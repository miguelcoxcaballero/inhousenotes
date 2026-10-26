(function (root) {
    'use strict';
    const S=root.InhouseOfflineStore;
    let host, account='', entries=[], documents=new Map(), captureTimer, syncFrame;
    let initialized=false, loaded=false, resumed=false, cataloguePromise=null;
    let savedViews={}, kicking=false;
    let pendingOperations=new Map();
    let syncState={phase:'idle',pending:0,id:null,errors:[]};
    function setSyncProgress(progress) {
        syncState={...syncState,...progress};
        document.querySelectorAll('[data-drive-item-id]').forEach(card=>decorate(card,card.dataset.driveItemId));
        host?.syncChanged?.({...syncState});
    }
    async function syncFinished(result={}) {
        setSyncProgress({phase:'idle',id:null,errors:result.errors||[]});
        await reload();
        host?.refreshed?.();
    }
    const NONE=0xFFFFFFFF,pdfType='application/pdf';
    // The catalogue index lives in Rust (rust-core/src/offline/library.rs). It answers with indexes into `entries`;
    // JS keeps the entry objects. Folder names are ranked here because only the browser's ICU can collate.
    let folderIds=new Set(), documentAliases=new Map(), indexed=false, ranked=false, named=false, shadowModified=[], shadowStarred=[], handed=[], handedFlags=new Uint8Array(0);
    function rustLibrary() {
        const core=root.InhouseRust;
        if(!core?.available?.()||!core.offlineLibrary)throw new Error('Rust core is unavailable; reload the app');
        return core.offlineLibrary;
    }
    const parsedTime=value=>Date.parse(value)||0;
    const wellFormed=text=>text.isWellFormed?.()===false?text.toWellFormed():text;
    // Only entries given to callers can be changed behind our back (cards update observed records in place).
    function handOut(at) {
        const entry=entries[at];
        if(!handedFlags[at]) {handedFlags[at]=1;handed.push(at);shadowModified[at]=entry.modifiedTime;shadowStarred[at]=!!entry.starred;}
        return entry;
    }
    // Equal rank exactly when localeCompare reports 0; names that are not folders are passed as undefined.
    function collationRanks(names) {
        const collator=new Intl.Collator(),distinct=[...new Set(names.filter(name=>name!==undefined))].sort(collator.compare);
        const rank=new Map();let current=-1;
        distinct.forEach((name,at)=>{if(!at||collator.compare(distinct[at-1],name)!==0)current++;rank.set(name,current);});
        return names.map(name=>rank.get(name)??0);
    }
    const folderName=entry=>entry.mimeType===folderType?(entry.name||''):undefined;
    function indexCatalogue() {
        folderIds=new Set();documentAliases=new Map();ranked=false;named=false;handed=[];handedFlags=new Uint8Array(entries.length);
        for(const doc of documents.values())if(doc.remoteId&&!documentAliases.has(doc.remoteId))documentAliases.set(doc.remoteId,doc);
        if(!indexed&&!entries.length)return;
        const library=rustLibrary(),columns={fl:[],t:[],views:savedViews,docs:Array.from(documents.values(),({id,remoteId,lastOpenedAt})=>({id,remoteId,lastOpenedAt}))};
        for(const entry of entries) {
            const folder=entry.mimeType===folderType;
            if(folder)folderIds.add(entry.id);
            columns.fl.push((entry.trashed?1:0)|(folder?2:0)|(entry.mimeType===pdfType?4:0)|(entry.pending?8:0)|(entry.starred?16:0)|(entry.ownedByMe===false?32:0));
            columns.t.push(entry.trashed?null:entry.modifiedTime);
        }
        const unparsed=library.load(columns);
        if(unparsed.length)library.touch(Array.from(unparsed,at=>[at,parsedTime(entries[at].modifiedTime),!!entries[at].starred]));
        indexed=true;
    }
    // Cards can update an observed record before the next reload: hand Rust the timestamps and stars that moved.
    function refreshObserved(only=handed) {
        let changes=null;
        for(const at of only) {
            if(!handedFlags[at])continue;
            const entry=entries[at];
            if(entry.modifiedTime===shadowModified[at]&&!!entry.starred===shadowStarred[at])continue;
            (changes||=[]).push([at,parsedTime(entry.modifiedTime),!!entry.starred]);
            shadowModified[at]=entry.modifiedTime;shadowStarred[at]=!!entry.starred;
        }
        if(changes)rustLibrary().touch(changes);
        return !!changes;
    }
    function ensureRanks() {
        if(ranked)return;
        const ranks=collationRanks(entries.map(entry=>entry.trashed?undefined:folderName(entry)));
        if(ranks.some(Boolean))rustLibrary().ranks(ranks);
        ranked=true;
    }
    function ensureNames() {
        if(named)return;
        rustLibrary().names(entries.map(entry=>entry.trashed||typeof entry.name!=='string'?null:wellFormed(entry.name.toLowerCase())));
        named=true;
    }
    const isWorker=new URLSearchParams(location.search).has('offline-worker');
    const folderType='application/vnd.google-apps.folder';
    let disconnected=false;
    try {disconnected=sessionStorage.getItem('ihn-network-offline')==='1';}catch(_){}
    let reachable=false,networkEpoch=0;
    const offline=()=>disconnected||(navigator.onLine===false&&!reachable);
    function connectivity(value) {
        const before=offline();reachable=!!value;disconnected=!value;
        try {sessionStorage.setItem('ihn-network-offline',disconnected?'1':'0');}catch(_){}
        const changed=before!==offline();
        if(changed)host?.connectionChanged?.(value);
        if(changed&&loaded) {reload().then(()=>host.refreshed?.()).catch(()=>{});if(value){kick();catalogue().catch(()=>{});}}
    }
    let probing=false;
    async function probeConnection() {
        if(probing)return;
        probing=true;
        const epoch=networkEpoch;
        try {
            // Probe the upload service itself. Shell assets may be cached or
            // intercepted by Android, and their failure says nothing about Drive.
            // An unauthenticated 401/403 is still proof that Drive is reachable.
            const response=await fetch('https://www.googleapis.com/drive/v3/about?fields=kind',{
                cache:'no-store',credentials:'omit',signal:AbortSignal.timeout(5000)});
            if(epoch!==networkEpoch)return;
            connectivity(response.status>0);
            if(!offline())void kick();
        }catch(_){if(epoch===networkEpoch)connectivity(false);}finally{probing=false;}
    }
    function networkAvailable() {
        // A real authenticated response outranks an older failed probe and the
        // occasionally stale navigator.onLine flag in an Android WebView.
        networkEpoch++;connectivity(true);
    }
    function knownAccount() {
        try {return (localStorage.getItem('drive-hint-email')||'').trim().toLowerCase();} catch(_){return '';}
    }
    async function reload() {
        if (!account) return;
        const requestedAccount=account;
        const [items,docs,driveRoot,views,outbox]=await Promise.all([S.list('entries',requestedAccount),S.list('document-index',requestedAccount),S.read('settings',requestedAccount,'drive-root'),S.read('settings',requestedAccount,'home-views'),S.list('outbox',requestedAccount),root.InhouseRust?.ready]);
        if(account!==requestedAccount)return;
        const library=rustLibrary();
        // Rust decides which records survive, which remote copy overlays them and which parents are aliases.
        const pointed=new Set(),bare=[],merge={i:items.map(e=>e.id),r:[],p:[],m:[],a:[],np:[],root:driveRoot?.remoteId||''};
        for(let at=0;at<items.length;at++) {
            const e=items[at];
            if(e.remoteId) {merge.r.push([at,String(e.remoteId)]);pointed.add(e.remoteId);}
            if(e.pending)merge.p.push(at);
            if(Array.isArray(e.parents))merge.a.push(e.parents);else {merge.a.push(null);bare.push(at);}
        }
        // the remote-newer test only compares records that name a remote copy and the copies they name
        if(pointed.size)items.forEach((e,at)=>{
            if(e.remoteId||pointed.has(e.id))merge.m.push([at,parsedTime(e.modifiedTime)]);
            if(pointed.has(e.id)&&!Object.hasOwn(e,'parents'))merge.np.push(at);
        });
        const plan=library.merge(merge);
        if(plan.identity) {
            entries=items.slice();
            for(const at of bare)entries[at]={...items[at],parents:['root']};
        } else {
            entries=Array.from(plan.kept,(source,at)=>{
                const e=items[source],remote=plan.overlay[at];
                if(remote===NONE&&Array.isArray(e.parents))return e;
                const item=remote===NONE?e:{...e,...items[remote],id:e.id,remoteId:e.remoteId};
                return {...item,parents:(item.parents||['root']).slice()};
            });
            let copied=-1;
            for(let at=0;at<plan.patches.length;at+=3) {
                const position=plan.patches[at],target=plan.patches[at+2];
                if(copied!==position) {entries[position]={...entries[position],parents:entries[position].parents.slice()};copied=position;}
                entries[position].parents[plan.patches[at+1]]=target===NONE?'root':items[target].id;
            }
        }
        documents=new Map(docs.map(doc=>[doc.id,doc]));
        savedViews=views||{};indexCatalogue();loaded=true;
        pendingOperations=new Map(outbox.map(op=>[op.id,op]));
        setSyncProgress({pending:outbox.length,...(!outbox.length?{errors:[]}:{})});
        document.querySelectorAll('[data-drive-item-id]').forEach(card=>decorate(card,card.dataset.driveItemId));
    }
    function record(id) {return documents.get(id)||documentAliases.get(id);}
    function decorate(card,id) {
        if(folderIds.has(id))return;
        const doc=record(id);
        const available=!!doc?.available;
        card.classList.toggle('offline-unavailable',offline()&&!available);
        if (offline()&&!available) {card.setAttribute('aria-disabled','true');card.title='Connect to download this document';}
        else {card.removeAttribute('aria-disabled');card.removeAttribute('title');}
        card.dataset.offlineAvailable=available?'true':'false';
        card.dataset.offlinePinned=doc?.pinned?'true':'false';
        const op=pendingOperations.get(doc?.id||id),active=syncState.phase!=='idle'&&[id,doc?.id,doc?.remoteId].includes(syncState.id);
        let badge=card.querySelector('.document-sync-status');
        if(!op&&!active){badge?.remove();return;}
        if(!badge){badge=document.createElement('button');badge.type='button';badge.className='document-sync-status';badge.setAttribute('aria-live','polite');card.append(badge);}
        const error=syncState.errors.find(e=>!e.id||e.id===(doc?.id||id))||(op?.lastError?{code:op.lastError,message:op.lastErrorMessage,phase:op.lastErrorPhase}:null);
        badge.dataset.phase=active?syncState.phase:error?'error':'pending';
        badge.textContent=active?({checking:'Checking…',building:'Preparing…',merging:'Merging…',preserving:'Keeping both versions…',uploading:'Uploading…'}[syncState.phase]||'Syncing…'):error?'Retry upload':offline()?'Saved offline':'Waiting to upload';
        badge.title=error?syncErrorText(error):badge.textContent;
        badge.onclick=event=>{
            event.preventDefault();event.stopPropagation();
            if(active)return;
            if(error){showSyncFailure(doc?.entry?.name||doc?.name||card.querySelector('.file-name')?.textContent||'Document',error);return;}
            void probeConnection().then(()=>{if(!offline())host?.retrySync?.();});
        };
    }
    function showSyncFailure(name,error) {
        document.getElementById('offline-sync-error')?.remove();
        const dialog=document.createElement('dialog');dialog.id='offline-sync-error';dialog.className='offline-settings';
        dialog.innerHTML='<form method="dialog"><h2>Upload pending</h2><p class="sync-file-name"></p><p class="sync-error-summary"></p><pre class="sync-error-detail"></pre><div class="offline-settings-actions"><button value="close">Close</button><button type="button" class="copy-sync-error">Copy error</button><button value="retry">Retry</button></div></form>';
        dialog.querySelector('.sync-file-name').textContent=name;
        dialog.querySelector('.sync-error-summary').textContent=syncErrorText(error);
        const detail=String(error.message||'Unknown error').replace(/Bearer\s+[^\s"']+/gi,'Bearer [redacted]').slice(0,1200);
        const report=`Inhouse Notes ${host.version}\n${error.phase||'sync'} · ${error.code||'unknown'}\n${detail}`;
        dialog.querySelector('.sync-error-detail').textContent=report;
        dialog.querySelector('.copy-sync-error').onclick=async event=>{
            try{await navigator.clipboard.writeText(report);event.target.textContent='Copied';}catch(_){event.target.textContent='Select the error text';}
        };
        dialog.addEventListener('close',()=>{
            dialog.remove();if(dialog.returnValue!=='retry')return;
            if(!offline())host?.retrySync?.();
            else void probeConnection().then(()=>{if(!offline())host?.retrySync?.();});
        });
        document.body.appendChild(dialog);dialog.showModal();
    }
    function syncErrorText(error) {
        if(!error)return '';
        const text=String(error.message||'');
        if(error.code===401||/auth|sign.?in|refresh token|access token|token expired/i.test(text))return 'Reconnect Google Drive to upload. Your local copy is safe.';
        if(error.code===403)return /quota|storage/i.test(text)?'Google Drive storage is full. Your local copy is safe.':'Drive denied access. Check editing permission.';
        if(error.code===404||error.code==='remote-deleted')return 'The Drive file is missing or unavailable. Your local copy is safe.';
        if(error.code===412)return 'The document changed on Drive. Retrying the merge.';
        if(error.code==='needs-merge')return 'Waiting to verify the Drive revision. Your local copy is safe.';
        return 'Upload failed: '+(text.replace(/Drive API error \d+:.*$/s,'Drive request failed').slice(0,160)||'Check your connection and retry.');
    }
    function select(parent,needle,local) {
        if(!indexed)return [];
        ensureRanks();
        if(needle)ensureNames();
        const ask=()=>rustLibrary().folder(parent,wellFormed(needle),local);
        let found=ask();
        // Only listed entries can reorder this answer; ask again when an observed one moved.
        if(refreshObserved(found))found=ask();
        return Array.from(found,handOut);
    }
    function folder(id, search='') {return select(id||'root',search.toLowerCase(),false);}
    function combine(remote,parent,search='') {
        const local=select(parent||'root',search.toLowerCase(),true),rows=[...local,...remote];
        const ranks=collationRanks(rows.map(folderName));
        const order=rustLibrary().sort({local:local.length,i:rows.map(e=>String(e.id)),r:rows.map(e=>e.remoteId?String(e.remoteId):''),
            k:rows.map(e=>String(e.remoteId||e.id)),f:rows.map(e=>e.mimeType===folderType),m:rows.map(e=>parsedTime(e.modifiedTime)),n:ranks});
        return Array.from(order,at=>rows[at]);
    }
    function home() {
        if(!indexed)return {recents:[],starred:[],shared:[]};
        refreshObserved();
        const quick=[];
        entries.forEach((entry,at)=>{if(!entry.trashed&&entry.mimeType===pdfType&&host.quickAccess?.(entry))quick.push(at);});
        const [recents,starred,shared]=rustLibrary().home(quick).map(list=>Array.from(list,handOut));
        return {recents,starred,shared};
    }
    function recentItems(remote) {
        const library=rustLibrary(),listed=Array.from(remote);
        if(indexed)refreshObserved();
        const order=library.recents({i:listed.map(e=>String(e.id)),k:listed.map(e=>String(e.remoteId||e.id)),m:listed.map(e=>parsedTime(e.modifiedTime))});
        return Array.from(order,at=>at<entries.length?handOut(at):listed[at-entries.length]);
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
    const captureQueue = new root.InhouseRuntimeCore.CoalescingQueue(async request => {
        const captureAccount = request.account;
        const identity = host.identity?.();
        if (account !== captureAccount || identity?.id !== request.identity?.id || identity?.session !== request.identity?.session) return;
        const known = identity?.id ? documents.get(identity.id) : null;
        if (known && known.contentVersion === identity.version && known.session === identity.session) return;
        const value = await host.capture();
        if (!value?.id || !value.snapshot || account !== captureAccount) return;
        const previous = await S.read('documents', captureAccount, value.id);
        const current = host.identity?.();
        if (account !== captureAccount || (current && (current.id !== value.id || current.session !== value.session || current.version !== value.contentVersion))) return;
        // A delayed capture must not roll back a newer checkpoint from the
        // same editor session (e.g. while shutdown/background work completes).
        if (previous?.session === value.session && previous.contentVersion >= value.contentVersion) return;
        await S.checkpoint(captureAccount, value.id, { ...value, offlineEdited: offline() || previous?.offlineEdited || false, lastOpenedAt: Date.now() }, value.dirty);
        await scheduleBackground();
        await reload();
    });
    async function capture() {
        clearTimeout(captureTimer);
        if (isWorker || !initialized || !host || !account) return;
        const identity = host.identity?.();
        return captureQueue.request(JSON.stringify([account, identity?.session, identity?.id]), { account, identity });
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
        if (isWorker || !initialized || !resumed || !account || offline() || syncFrame || kicking || host.busy()) return;
        kicking=true;
        try {
            const priority=await S.read('settings',account,'sync-priority');
            if(priority?.documentId&&priority.expires>Date.now())return;
            let queue=await S.list('outbox',account);
            if(host.activeDocument?.()&&queue.some(op=>op.kind==='document'&&op.id===host.currentId())) {
                // The background editor deliberately skips a live document.
                // Its foreground owner must retry too, including after a busy lease.
                await host.syncActive?.();
                queue=await S.list('outbox',account);
            }
            if (!queue.some(op=>op.kind==='folder'||!host.activeDocument?.()||op.id!==host.currentId()) || syncFrame || offline()) return;
            const frame=document.createElement('iframe');frame.hidden=true;frame.title='Background document sync';
            frame.src=`/?offline-worker=1&v=${encodeURIComponent(host.version)}`;
            syncFrame=frame;
            setSyncProgress({phase:'checking',pending:queue.length,id:null,errors:[]});
            const finish=result=>{
                clearTimeout(timer);window.removeEventListener('message',listener);
                if(syncFrame!==frame)return;
                frame.remove();syncFrame=null;
                void syncFinished(result).catch(()=>{});
            };
            const listener=event=>{
                if(event.origin!==location.origin || event.source!==frame.contentWindow)return;
                if(event.data?.type==='ihn-offline-progress')setSyncProgress(event.data.progress);
                if(event.data?.type==='ihn-offline-complete')finish(event.data.result);
            };
            const timer=setTimeout(()=>finish({errors:[{message:'Sync timed out; retrying automatically'}]}),8*60*1000);
            window.addEventListener('message',listener);
            document.body.appendChild(frame);
        } catch(error) {
            await syncFinished({errors:[{message:error.message}]}).catch(()=>{});
        } finally {kicking=false;}
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
            window.addEventListener('offline',()=>{networkEpoch++;connectivity(false);});
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
        resumed=true;
        try {await capture();} finally {kick().catch(()=>{});catalogue().catch(()=>{});}
    }
    async function useAccount(email) {
        const next=String(email||'').trim().toLowerCase();
        if(!next||next===account)return;
        account=next;loaded=false;await reload();host.refreshed?.();kick();catalogue().catch(()=>{});
    }
    function disable() {initialized=false;resumed=false;}
    function logout() {setSyncProgress({phase:'idle',pending:0,id:null,errors:[]});account='';loaded=false;savedViews={};entries=[];documents.clear();indexCatalogue();if(syncFrame){syncFrame.remove();syncFrame=null;}try{root.InhouseBackground?.cancel?.();}catch(_){} }
    root.InhouseOfflineLibrary=Object.freeze({init,resume,disable,account:()=>account,offline,networkAvailable,ready:()=>initialized&&loaded&&!!account,folder,combine,home,recentItems,syncErrorText,observed,cache,capture,scheduleCapture,prepared,
        openRecord,pin,createFolder,catalogue,kick,setSyncProgress,syncFinished,syncStatus:()=>({...syncState}),scheduleBackground,decorate,reload,useAccount,logout,settings:showSettings,record,isWorker});
})(window);
