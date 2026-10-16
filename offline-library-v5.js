(function (root) {
    'use strict';
    const S=root.InhouseOfflineStore;
    let host, account='', entries=[], documents=new Map(), captureTimer, capturePromise, syncFrame;
    let initialized=false, loaded=false, resumed=false, cataloguePromise=null;
    let savedViews={};
    const isWorker=new URLSearchParams(location.search).has('offline-worker');
    const folderType='application/vnd.google-apps.folder';
    let disconnected=false;
    try {disconnected=sessionStorage.getItem('ihn-network-offline')==='1';}catch(_){}
    const offline=()=>navigator.onLine===false||disconnected;
    function connectivity(value) {
        const changed=disconnected!==!value;disconnected=!value;
        try {sessionStorage.setItem('ihn-network-offline',disconnected?'1':'0');}catch(_){}
        if(changed)host?.connectionChanged?.(value);
        if(changed&&loaded) {reload().then(()=>host.refreshed?.()).catch(()=>{});if(value){kick();catalogue().catch(()=>{});}}
    }
    let probing=false;
    async function probeConnection() {
        if(probing||navigator.onLine===false)return;
        probing=true;
        try {
            // Android can report Wi-Fi as online even when it has no internet.
            // This tiny uncached request must never hold up the local library.
            const response=await fetch(`/service-worker.js?connectivity=${Date.now()}`,{cache:'no-store',signal:AbortSignal.timeout(5000)});
            connectivity(response.ok);
        }catch(_){connectivity(false);}finally{probing=false;}
    }
    function knownAccount() {
        try {return (localStorage.getItem('drive-hint-email')||'').trim().toLowerCase();} catch(_){return '';}
    }
    async function reload() {
        if (!account) return;
        const requestedAccount=account;
        const [items,docs,driveRoot,views]=await Promise.all([S.list('entries',requestedAccount),S.list('document-index',requestedAccount),S.read('settings',requestedAccount,'drive-root'),S.read('settings',requestedAccount,'home-views')]);
        if(account!==requestedAccount)return;
        const aliases=new Map(items.filter(e=>e.remoteId && e.remoteId!==e.id).map(e=>[e.remoteId,e.id]));
        if(driveRoot?.remoteId)aliases.set(driveRoot.remoteId,'root');
        const byId=new Map(items.map(e=>[e.id,e]));
        entries=items.filter(e=>!aliases.has(e.id)).map(e=>{
            const remote=!e.pending&&e.remoteId?byId.get(e.remoteId):null;
            const item=remote?{...e,...remote,id:e.id,remoteId:e.remoteId}:e;
            return {...item,parents:(item.parents||['root']).map(p=>aliases.get(p)||p)};
        });
        documents=new Map(docs.map(doc=>[doc.id,doc]));
        savedViews=views||{};loaded=true;
        document.querySelectorAll('[data-drive-item-id]').forEach(card=>decorate(card,card.dataset.driveItemId));
    }
    function record(id) {return documents.get(id)||[...documents.values()].find(doc=>doc.remoteId===id);}
    function decorate(card,id) {
        if(entries.some(e=>e.id===id&&e.mimeType===folderType))return;
        const available=!!record(id)?.available;
        card.classList.toggle('offline-unavailable',offline()&&!available);
        if (offline()&&!available) {card.setAttribute('aria-disabled','true');card.title='Connect to download this document';}
        else {card.removeAttribute('aria-disabled');card.removeAttribute('title');}
        card.dataset.offlineAvailable=available?'true':'false';
        card.dataset.offlinePinned=record(id)?.pinned?'true':'false';
    }
    function folder(id, search='') {
        const needle=search.toLowerCase();
        return entries.filter(e=>!e.trashed && (e.parents||[]).includes(id||'root') && (!needle||e.name?.toLowerCase().includes(needle)))
            .sort((a,b)=>a.mimeType===folderType&&b.mimeType===folderType ? (a.name||'').localeCompare(b.name||'') : (Date.parse(b.modifiedTime)||0)-(Date.parse(a.modifiedTime)||0));
    }
    function combine(remote,parent,search='') {
        const local=folder(parent,search).filter(e=>e.pending||e.id.startsWith('local-'));
        const ids=new Set(local.flatMap(e=>[e.id,e.remoteId]));
        return [...local,...remote.filter(e=>!ids.has(e.id))];
    }
    function home() {
        const active=entries.filter(e=>!e.trashed);
        const ordered=list=>list.sort((a,b)=>(Date.parse(b.modifiedTime)||0)-(Date.parse(a.modifiedTime)||0));
        const view=(name,fallback)=>Array.isArray(savedViews[name])
            ? savedViews[name].map(id=>active.find(e=>e.id===id||e.remoteId===id)).filter(Boolean) : fallback;
        const files=active.filter(e=>e.mimeType==='application/pdf'&&!host.quickAccess?.(e));
        const pending=files.filter(e=>e.pending||e.id.startsWith('local-'));
        const recents=view('recents',ordered(files).slice(0,12));
        return {recents:[...pending.filter(e=>!recents.some(r=>r.id===e.id)),...recents].slice(0,12),
            starred:view('starred',ordered(active.filter(e=>e.starred)).slice(0,12)),
            shared:view('shared',ordered(active.filter(e=>e.ownedByMe===false)).slice(0,24))};
    }
    async function observed(items,parent,view) {
        if (!account) return;
        const observedAccount=account;
        await S.cacheEntries(observedAccount,items.map(e=>({...e,...(!e.parents&&parent?{parents:[parent]}:{})})));
        if(view)await S.patch('settings',observedAccount,'home-views',old=>({...old,[view]:items.map(e=>e.id)}));
        await reload();
    }
    async function cache(fileId,blob,meta,analysis,details={}) {
        if (!account || isWorker) return;
        const old=await S.read('documents',account,fileId);
        if (old?.dirty) return; // A downloaded baseline must not replace an offline draft.
        await S.checkpoint(account,fileId,{blob,baseMeta:meta,analysis,details,snapshot:null,originalBlob:null,preparedBlob:null,
            entry:{id:fileId,name:details.fileName||'Untitled.pdf',mimeType:'application/pdf',parents:details.parentIds||['root'],modifiedTime:meta.modifiedTime,ownedByMe:details.ownedByMe!==false},lastOpenedAt:old?.lastOpenedAt||Date.now()},false);
        await reload();
    }
    async function capture() {
        clearTimeout(captureTimer);
        if (isWorker || !initialized || !host || !account) return;
        if (capturePromise) {await capturePromise;return capture();}
        const captureAccount=account;
        const task=(async()=>{
            const identity=host.identity?.();
            const known=identity?.id?documents.get(identity.id):null;
            if(known&&known.contentVersion===identity.version&&known.session===identity.session)return;
            const value=await host.capture();
            if (!value?.id || !value.snapshot || account!==captureAccount) return;
            const previous=await S.read('documents',captureAccount,value.id);
            if(account!==captureAccount)return;

            if (previous?.contentVersion===value.contentVersion && previous?.session===value.session) return;
            await S.checkpoint(account,value.id,{...value,offlineEdited:offline()||previous?.offlineEdited||false,lastOpenedAt:Date.now()},value.dirty);
            await scheduleBackground();
        })();
        const complete=task.then(()=>reload()).finally(()=>{if(capturePromise===complete)capturePromise=null;});
        capturePromise=complete;
        return complete;
    }
    function scheduleCapture() {
        if (isWorker || !initialized || !host || !account) return;
        clearTimeout(captureTimer);
        captureTimer=setTimeout(()=>{
            if (host.busy()) {scheduleCapture();return;}
            capture().then(()=>kick()).catch(error=>host.error(error));
        },600);
    }
    async function prepared(id,blob,version,session) {
        if (!account || isWorker) return;
        await S.patch('documents',account,id,old=> {
            if (!old.snapshot || old.contentVersion!==version || old.session!==session) return old;
            return {...old,preparedBlob:blob,preparedGeneration:old.generation};
        });
        await scheduleBackground();
    }
    async function openRecord(file) {
        if (!account) return null;
        const id=record(file.id)?.id||file.id;
        const doc=await S.read('documents',account,id);
        if (!doc) return null;
        await S.patch('documents',account,id,old=>({...old,lastOpenedAt:Date.now()}));
        return doc.blob?.size || doc.snapshot?.pages?.length ? doc : null;
    }
    async function pin(file) {
        if (!account) return;
        file={...file,id:record(file.id)?.id||file.id};
        let doc=await S.read('documents',account,file.id);
        if (doc?.pinned) {
            await S.patch('documents',account,file.id,old=>({...old,pinned:false}));
            host.status('Automatic offline retention restored');
        } else {
            if (!doc?.blob && !doc?.snapshot) {
                if (offline()) throw new Error('Connect once to download this document');
                const downloaded=await host.download(file);
                await cache(file.id,downloaded.blob,downloaded.meta,null,{fileName:file.name,parentIds:file.parents,canEdit:downloaded.meta.capabilities?.canEdit!==false,ownedByMe:file.ownedByMe});
            }
            await S.patch('documents',account,file.id,old=>({...old,pinned:true,lastOpenedAt:Date.now()}));
            await navigator.storage?.persist?.().catch(()=>false);
            host.status('Saved for offline use');
        }
        await reload();
    }
    async function createFolder(parent,name) {
        if (!account) throw new Error('Sign in once to choose your Drive account');
        const entry=await S.createFolder(account,parent,name);await reload();await scheduleBackground();kick();return entry;
    }
    async function catalogue() {
        if (cataloguePromise || !account || offline() || isWorker) return cataloguePromise;
        cataloguePromise=(async()=>{
            const catalogAccount=account;
            const rootResponse=await host.fetch('https://www.googleapis.com/drive/v3/files/root?fields=id');
            if(!rootResponse.ok)throw new Error('Could not read Drive root');
            const rootId=(await rootResponse.json()).id;
            if(account!==catalogAccount)return;
            await S.put('settings',catalogAccount,'drive-root',{remoteId:rootId});
            let pageToken='';const found=new Set();
            do {
                const params=new URLSearchParams({q:"trashed=false and (mimeType='application/pdf' or mimeType='application/vnd.google-apps.folder')",
                    pageSize:'1000',fields:'nextPageToken,files(id,name,mimeType,parents,modifiedTime,createdTime,thumbnailLink,starred,ownedByMe,resourceKey,folderColorRgb)',...(pageToken?{pageToken}:{})});
                const response=await host.fetch(`https://www.googleapis.com/drive/v3/files?${params}`);
                if(!response.ok)throw new Error('Could not read Drive folders');
                if(account!==catalogAccount)throw new Error('Account changed');
                const data=await response.json();
                await S.cacheEntries(account,(data.files||[]).map(item=>({...item,parents:item.parents?.length?item.parents.map(id=>id===rootId?'root':id):['root']})));
                for (const item of data.files||[]) found.add(item.id);
                pageToken=data.nextPageToken||'';
            } while(pageToken);
            // Only a complete successful enumeration can mark an item removed.
            const old=await S.list('entries',account);
            await S.cacheEntries(account,old.filter(e=>!e.pending&&!found.has(e.remoteId||e.id)).map(e=>({...e,trashed:true})));
            await reload();
        })().finally(()=>{cataloguePromise=null;});
        return cataloguePromise;
    }
    async function scheduleBackground() {
        if (!account) return;
        try {root.InhouseBackground?.schedule?.();}catch(_){}
        try {const registration=await navigator.serviceWorker?.getRegistration();await registration?.sync?.register('inhouse-outbox');}catch(_){}
    }
    async function kick() {
        if (isWorker || !initialized || !resumed || !account || offline() || syncFrame || host.busy()) return;
        const queue=await S.list('outbox',account);
        if (!queue.some(op=>op.kind==='folder'||!host.activeDocument?.()||op.id!==host.currentId()) || syncFrame) return;
        const frame=document.createElement('iframe');frame.hidden=true;frame.title='Background document sync';
        frame.src=`/?offline-worker=1&v=${encodeURIComponent(host.version)}`;
        syncFrame=frame;document.body.appendChild(frame);
        const timer=setTimeout(()=>{window.removeEventListener('message',listener);if(syncFrame===frame){frame.remove();syncFrame=null;}},8*60*1000);
        const listener=event=>{
            if(event.origin!==location.origin || event.source!==frame.contentWindow || event.data?.type!=='ihn-offline-complete')return;
            clearTimeout(timer);window.removeEventListener('message',listener);frame.remove();syncFrame=null;
            reload().then(()=>host.refreshed?.()).catch(()=>{});
        };
        window.addEventListener('message',listener);
    }
    async function showSettings() {
        if (!account) return;
        let dialog=document.getElementById('offline-settings');
        if (dialog) dialog.remove();
        dialog=document.createElement('dialog');dialog.id='offline-settings';dialog.className='offline-settings';
        const preferences=await S.settings(account);
        dialog.innerHTML='<form method="dialog"><h2>Offline documents</h2><label>Keep recently opened documents for <input name="days" type="number" min="1" max="3650" required> days</label><p>Pinned documents and unsynced changes are kept until you remove them.</p><p class="offline-queue-status" role="status"></p><div class="offline-recoveries"></div><div class="offline-settings-actions"><button value="cancel" formnovalidate>Cancel</button><button value="save">Save</button></div></form>';
        dialog.querySelector('input').value=preferences.retentionDays;
        const pending=await S.list('outbox',account);
        dialog.querySelector('.offline-queue-status').textContent=pending.length?`${pending.length} pending · ${offline()?'Waiting for connection':'Will sync automatically'}`:'Up to date';
        for (const recovery of (await S.list('recoveries',account)).sort((a,b)=>b.createdAt-a.createdAt)) {
            const row=document.createElement('p');row.textContent=`${recovery.name||'Document'} · versions before merge `;
            for (const [label,blob] of [['This device',recovery.localBlob],['Other device',recovery.remoteBlob]]) {
                if (!(blob instanceof Blob))continue;
                const button=document.createElement('button');button.type='button';button.textContent=label;
                button.onclick=()=>host.downloadBlob(blob,`${recovery.name||'Document'} (${label}).pdf`);row.appendChild(button);
            }
            dialog.querySelector('.offline-recoveries').appendChild(row);
        }
        dialog.addEventListener('close',async()=>{
            if(dialog.returnValue==='save') {await S.setRetention(account,Number(dialog.querySelector('input').value));await S.prune(account,Date.now(),[host.currentId()]);await reload();host.refreshed?.();}
            dialog.remove();
        });
        document.body.appendChild(dialog);dialog.showModal();
    }
    async function init(adapter) {
        host=adapter;account=knownAccount();initialized=true;
        if (!isWorker) {
            if ('serviceWorker' in navigator) {
                navigator.serviceWorker.register('/service-worker.js',{updateViaCache:'none'}).catch(()=>{});
                navigator.serviceWorker.addEventListener('message',event=>{if(event.data?.type==='ihn-offline-updated')reload().then(()=>host.refreshed?.());});
            }
            document.querySelectorAll('[data-offline-settings]').forEach(button=>button.addEventListener('click',()=>showSettings().catch(host.error)));
            window.addEventListener('offline',()=>connectivity(false));
            window.addEventListener('online',()=>probeConnection());
            document.addEventListener('visibilitychange',()=>{if(!document.hidden)probeConnection();});
            setInterval(()=>{if(offline())probeConnection();},30000);
            void probeConnection();
            window.addEventListener('message',event=>{if(event.origin===location.origin&&event.data?.type==='ihn-offline-dirty')reload();});
        }
        if (account) {await reload();if(!isWorker){await S.put('settings','__session__','active',{accountId:account,enabled:true});host.refreshed?.();setTimeout(()=>S.prune(account,Date.now(),[host.currentId()]).then(()=>reload()).catch(()=>{}),2000);}}
        
    }
    async function resume() {
        if(isWorker||!initialized)return;
        await capture();resumed=true;kick().catch(()=>{});catalogue().catch(()=>{});
    }
    async function useAccount(email) {
        const next=String(email||'').trim().toLowerCase();
        if(!next||next===account)return;
        account=next;loaded=false;await reload();host.refreshed?.();kick();catalogue().catch(()=>{});
    }
    function disable() {initialized=false;resumed=false;}
    function logout() {account='';loaded=false;savedViews={};entries=[];documents.clear();if(syncFrame){syncFrame.remove();syncFrame=null;}try{root.InhouseBackground?.cancel?.();}catch(_){} }
    root.InhouseOfflineLibrary=Object.freeze({init,resume,disable,account:()=>account,offline,ready:()=>initialized&&loaded&&!!account,folder,combine,home,observed,cache,capture,scheduleCapture,prepared,
        openRecord,pin,createFolder,catalogue,kick,scheduleBackground,decorate,reload,useAccount,logout,settings:showSettings,record,isWorker});
})(window);
