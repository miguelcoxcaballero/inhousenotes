'use strict';
// Inhouse Notes 6 service worker (ARCHITECTURE §12): shell cache only. It stays a classic script at the URL every
// 5.x page registered, and keeps the VERSION line that boot.js reads to detect a published release.
const VERSION='6.0.0';
const CACHE=`inhouse-shell-${VERSION}`;
// Kept until 6.0.1 for 5.18.0 tabs that are still open (ARCHITECTURE §10.4).
const LEGACY_VERSION='5.18.0';
const LEGACY_CACHE=`inhouse-shell-${LEGACY_VERSION}`;
// One entry per window that announced itself as v6; survives worker restarts.
const CLIENTS='ihn6-clients';
importScripts('offline-assets.js');
// On localhost `?e2e=1` boots the test build (host/main.js): keep it in the shell there so offline tests match production.
if(['127.0.0.1','localhost','[::1]'].includes(self.location.hostname))self.INHOUSE_OFFLINE_ASSETS.push('/wasm/ihn-test.wasm');
self.addEventListener('install',event=>event.waitUntil((async()=>{
    const cache=await caches.open(CACHE);
    // Installation is atomic: an incomplete shell never displaces a working one.
    await cache.addAll(self.INHOUSE_OFFLINE_ASSETS);
    // Take over right away instead of waiting for every tab to close, so a
    // reload shows the release that was just published.
    await self.skipWaiting();
})()));
self.addEventListener('activate',event=>event.waitUntil((async()=>{
    for(const key of await caches.keys())if(key.startsWith('inhouse-shell-')&&key!==CACHE&&key!==LEGACY_CACHE)await caches.delete(key);
    await self.clients.claim();
})()));
self.addEventListener('fetch',event=>{
    const url=new URL(event.request.url);
    // Never cache authenticated Drive responses, OAuth, arbitrary external URLs or mutations.
    if(event.request.method!=='GET'||url.origin!==self.location.origin)return;
    if(url.pathname.startsWith('/oauth-callback'))return;
    const path=url.pathname==='/'?'/index.html':url.pathname;
    const requestedVersion=url.searchParams.get('v');
    if(requestedVersion===LEGACY_VERSION) {
        // A 5.18.0 page that has not reloaded yet: its own shell first, then the network.
        event.respondWith((async()=>(await caches.match(path,{cacheName:LEGACY_CACHE}))||fetch(event.request))());
        return;
    }
    if(!self.INHOUSE_OFFLINE_ASSETS.includes(path))return;
    if(requestedVersion&&/^\d+\.\d+\.\d+$/.test(requestedVersion)&&requestedVersion!==VERSION)return;
    event.respondWith(path.endsWith('.html')?freshPage(event.request,path,event):(async()=>{
        const cache=await caches.open(CACHE);
        const cached=await cache.match(path);
        if(cached)return cached;
        const response=await fetch(event.request);
        if(response.ok)await cache.put(path,response.clone());
        return response;
    })());
});
// A complete versioned shell is immediately usable. Revalidate in the
// background; boot.js checks published versions and reloads only outside editing.
async function freshPage(request,path,event){
    const cache=await caches.open(CACHE);
    const cached=await cache.match(path);
    const network=fetch(request,{cache:'no-cache'}).then(async response=>{
        if(response.ok&&(await response.clone().text()).includes(`v=${VERSION}`))await cache.put(path,response.clone());
        return response;
    });
    event.waitUntil(network.then(()=>{},()=>{}));
    return cached||network;
}
// v6 client registry for the 5.x data migration (ARCHITECTURE §10.2): a v6 window posts 'ihn6-hello' at boot; the
// migration asks {type:'ihn6-clients'} (reply on the message port, else to the sender) and deletes 5.x data only
// when every open window is v6, i.e. windows === v6.
async function v6Clients(){
    const registry=await caches.open(CLIENTS);
    const windows=await self.clients.matchAll({type:'window',includeUncontrolled:true});
    const open=new Set(windows.map(client=>client.id));
    let v6=0;
    for(const request of await registry.keys()) {
        const id=new URL(request.url).pathname.split('/').pop();
        if(open.has(id))v6+=1;else await registry.delete(request);
    }
    return {type:'ihn6-clients',windows:windows.length,v6};
}
self.addEventListener('message',event=>{
    const id=event.source?.id;
    if(event.data==='ihn6-hello'&&id) {
        event.waitUntil(caches.open(CLIENTS).then(registry=>registry.put(`/__ihn6-client__/${id}`,new Response(''))));
    } else if(event.data?.type==='ihn6-clients') {
        event.waitUntil(v6Clients().then(answer=>(event.ports[0]||event.source)?.postMessage(answer)));
    }
});
