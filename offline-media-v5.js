/* Account-scoped thumbnails and avatars, independent of expiring Drive URLs. */
(function(root){
    'use strict';
    const memory=new Map(),pending=new Map();
    const cacheName=account=>`inhouse-media-${encodeURIComponent(account)}`;
    const key=id=>new URL(`/__offline-media__/${encodeURIComponent(id)}`,location.origin).href;
    async function url(account,id,source='',revision='') {
        if(!account||source.startsWith('data:'))return source;
        const identity=`${account}\0${id}`,requestKey=`${identity}\0${revision}`;
        const remembered=memory.get(identity);
        if(remembered?.revision===revision)return remembered.url;
        if(pending.has(requestKey))return pending.get(requestKey);
        const task=(async()=>{
            try {
                const cache=await caches.open(cacheName(account));
                let response=await cache.match(key(id));
                const previous=response;
                const stamp=await cache.match(`${key(id)}?revision`);
                const stale=revision&&stamp&&(await stamp.text())!==revision;
                if((!response||stale) && source && !(root.InhouseOfflineLibrary?.offline()||navigator.onLine===false)) {
                    try {response=await fetch(source,{credentials:'omit',signal:AbortSignal.timeout(6000)});}
                    catch(_){try{response=await fetch(source,{mode:'no-cors',credentials:'omit',signal:AbortSignal.timeout(6000)});}catch(_){response=previous;}}
                    if(response&&(response.ok||response.type==='opaque')){if(response!==previous){await cache.put(key(id),response.clone());await cache.put(`${key(id)}?revision`,new Response(revision));}}
                    else response=previous;
                }
                if(response) {
                    const result=response.type==='opaque'
                        ? (navigator.serviceWorker?.controller?key(id):source)
                        : URL.createObjectURL(await response.blob());
                    if(result){
                        const previous=memory.get(identity);memory.delete(identity);memory.set(identity,{revision,url:result});
                        if(previous?.url.startsWith('blob:')&&previous.url!==result)setTimeout(()=>URL.revokeObjectURL(previous.url),60000);
                        if(memory.size>128){const oldest=memory.keys().next().value,entry=memory.get(oldest);memory.delete(oldest);if(entry.url.startsWith('blob:'))setTimeout(()=>URL.revokeObjectURL(entry.url),60000);}
                    }
                    return result;
                }
            }catch(_){} // A failed preview never prevents opening the document.
            return source;
        })().finally(()=>pending.delete(requestKey));
        pending.set(requestKey,task);return task;
    }
    function clear(){for(const value of memory.values())if(value.url.startsWith('blob:'))URL.revokeObjectURL(value.url);memory.clear();}
    root.InhouseOfflineMedia=Object.freeze({url,clear});
})(window);
