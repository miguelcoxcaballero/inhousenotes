/* Account-scoped thumbnails and avatars, independent of expiring Drive URLs. */
(function(root){
    'use strict';
    const memory=new Map(),pending=new Map();
    const cacheName=account=>`inhouse-media-${encodeURIComponent(account)}`;
    const key=id=>new URL(`/__offline-media__/${encodeURIComponent(id)}`,location.origin).href;
    async function url(account,id,source='',revision='') {
        if(!account||source.startsWith('data:'))return source;
        const identity=`${account}\0${id}\0${revision}`;
        if(memory.has(identity))return memory.get(identity);
        if(pending.has(identity))return pending.get(identity);
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
                    if(result)memory.set(identity,result);
                    return result;
                }
            }catch(_){} // A failed preview never prevents opening the document.
            return source;
        })().finally(()=>pending.delete(identity));
        pending.set(identity,task);return task;
    }
    function clear(){for(const value of memory.values())if(value.startsWith('blob:'))URL.revokeObjectURL(value);memory.clear();}
    root.InhouseOfflineMedia=Object.freeze({url,clear});
})(window);
