'use strict';
const VERSION='5.16.59';
const CACHE=`inhouse-shell-${VERSION}`;
importScripts('offline-assets.js', 'offline-store-v5.js', 'offline-sync-v5.js');
self.addEventListener('install',event=>event.waitUntil((async()=>{
    const cache=await caches.open(CACHE);
    // Installation is atomic: an incomplete shell never displaces a working one.
    await cache.addAll(self.INHOUSE_OFFLINE_ASSETS);
    // Take over right away instead of waiting for every tab to close, so a
    // reload shows the release that was just published.
    await self.skipWaiting();
})()));
self.addEventListener('activate',event=>event.waitUntil((async()=>{
    for(const key of await caches.keys())if(key.startsWith('inhouse-shell-')&&key!==CACHE)await caches.delete(key);
    await self.clients.claim();
})()));
self.addEventListener('fetch',event=>{
    const url=new URL(event.request.url);
    if(event.request.method==='GET'&&event.request.destination==='image'&&url.origin===self.location.origin&&url.pathname.startsWith('/__offline-media__/')) {
        event.respondWith((async()=>{
            const session=await self.InhouseOfflineStore.read('settings','__session__','active');
            if(!session?.enabled||!session.accountId)return new Response('',{status:404});
            const media=await caches.open(`inhouse-media-${encodeURIComponent(session.accountId)}`);
            return (await media.match(url.href))||new Response('',{status:404});
        })());return;
    }
    // Never cache authenticated Drive responses, OAuth, arbitrary external URLs or mutations.
    if(event.request.method!=='GET'||url.origin!==self.location.origin)return;
    if(url.pathname.startsWith('/oauth-callback')||url.searchParams.has('oauth_callback'))return;
    const path=url.pathname==='/'?'/index.html':url.pathname;
    if(!self.INHOUSE_OFFLINE_ASSETS.includes(path))return;
    const requestedVersion=url.searchParams.get('v');
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
// background; boot-v5 checks published versions and reloads only outside editing.
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
async function backgroundSync() {
    const S=self.InhouseOfflineStore;
    const session=await S.read('settings','__session__','active');
    if(!session?.enabled||!session.accountId)return;
    const account=session.accountId;
    const request=async(url,options={})=>{
        const active=await S.read('settings','__session__','active');
        if(!active?.enabled||active.accountId!==account)throw new Error('Account signed out');
        let auth=await S.read('settings',account,'background-auth');
        if(!auth?.token||auth.expiry<Date.now()+60000) {
            if(!auth?.refreshToken||!auth.clientId)throw new Error('Open Notes to renew access');
            const response=await fetch('https://oauth2.googleapis.com/token',{method:'POST',headers:{'Content-Type':'application/x-www-form-urlencoded'},
                body:new URLSearchParams({grant_type:'refresh_token',refresh_token:auth.refreshToken,client_id:auth.clientId})});
            if(!response.ok)throw new Error('Open Notes to reconnect Drive');
            const token=await response.json();
            auth={...auth,token:token.access_token,expiry:Date.now()+token.expires_in*1000};
            await S.put('settings',account,'background-auth',auth);
        }
        const headers=new Headers(options.headers||{});headers.set('Authorization',`Bearer ${auth.token}`);
        return fetch(url,{...options,headers,signal:AbortSignal.timeout(60000)});
    };
    const result=await self.InhouseOfflineSync.run(account,{fetch:request});
    for(const client of await self.clients.matchAll({type:'window'}))client.postMessage({type:'ihn-offline-updated',result});
    if(result?.pending||result?.errors?.length)throw new Error('Pending documents retained for the next background attempt');
}
self.addEventListener('sync',event=>{if(event.tag==='inhouse-outbox')event.waitUntil(backgroundSync());});
self.addEventListener('periodicsync',event=>{if(event.tag==='inhouse-outbox')event.waitUntil(backgroundSync());});
