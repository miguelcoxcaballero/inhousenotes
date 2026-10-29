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
    function compareEntries(a,b) {
        const af=a.mimeType===folderType,bf=b.mimeType===folderType;
        if(af!==bf)return af?-1:1;
        const order=af?(a.name||'').localeCompare(b.name||''):modifiedTime(b)-modifiedTime(a);
        const ak=String(a.remoteId||a.id),bk=String(b.remoteId||b.id);
        return order||(ak<bk?-1:ak>bk?1:0);
    }
    let folderEntries=new Map(), folderIds=new Set(), activeEntries=[], activeAliases=new Map();
    let documentAliases=new Map(), modifiedTimes=new WeakMap();
    function indexCatalogue() {
        folderEntries=new Map();folderIds=new Set();activeEntries=[];activeAliases=new Map();
        documentAliases=new Map();modifiedTimes=new WeakMap();
        for (const entry of entries) {
            if(entry.mimeType===folderType)folderIds.add(entry.id);
            if(entry.trashed)continue;
            activeEntries.push(entry);
            // Saved Home views historically resolve the first matching ID or
            // remote alias. Preserve that priority when aliases overlap.
            for(const id of [entry.id,entry.remoteId])if(id&&!activeAliases.has(id))activeAliases.set(id,entry);
            for(const parent of new Set(entry.parents||[])) {
                if(!folderEntries.has(parent))folderEntries.set(parent,[]);
                folderEntries.get(parent).push(entry);
            }
        }
        for(const doc of documents.values())if(doc.remoteId&&!documentAliases.has(doc.remoteId))documentAliases.set(doc.remoteId,doc);
    }
    function modifiedTime(entry) {
        let cached=modifiedTimes.get(entry);
        // Cards can update an observed record before the next reload. Cache
        // parsing, not the record's mutable timestamp or its resulting order.
        if(!cached||cached.source!==entry.modifiedTime) {
            cached={source:entry.modifiedTime,value:Date.parse(entry.modifiedTime)||0};
            modifiedTimes.set(entry,cached);
        }
        return cached.value;
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
        if(changed&&loaded) {reload().then(()=>host.refreshed?.()).catch(()=>{});if(value){void kick({force:true,reason:'online'});catalogue().catch(()=>{});}}
        if(value)stopOfflineProbes();else if(changed)scheduleOfflineProbe();
    }
    // While offline, Drive is probed with a growing interval (30 s up to
    // 5 min) and only while the app is visible; 'online' and returning to
    // the app probe at once. Nothing is probed while connected.
    const PROBE_BASE_MS=30000, PROBE_MAX_MS=5*60000;
    let probeTimer=null, probeDelay=PROBE_BASE_MS;
    function stopOfflineProbes() {clearTimeout(probeTimer);probeTimer=null;probeDelay=PROBE_BASE_MS;}
    function scheduleOfflineProbe() {
        clearTimeout(probeTimer);probeTimer=null;
        if(isWorker||!initialized||!offline()||document.hidden)return;
        probeTimer=setTimeout(()=>{
            probeTimer=null;probeDelay=Math.min(PROBE_MAX_MS,probeDelay*2);
            void probeConnection();
        },probeDelay);
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
            if(!offline())void kick({reason:'probe'});
        }catch(_){if(epoch===networkEpoch)connectivity(false);}finally{probing=false;if(offline()&&!probeTimer)scheduleOfflineProbe();}
    }
    function networkAvailable() {
        // A real authenticated response outranks an older failed probe and the
        // occasionally stale navigator.onLine flag in an Android WebView.
        networkEpoch++;connectivity(true);
        // An authenticated response also proves that sign-in works again.
        if(waitingForAuth&&(!host?.canSync||host.canSync()))void kick({reason:'signed-in'});
    }
    function knownAccount() {
        try {return (localStorage.getItem('drive-hint-email')||'').trim().toLowerCase();} catch(_){return '';}
    }
    async function reload() {
        if (!account) return;
        const requestedAccount=account;
        const [items,docs,driveRoot,views,outbox]=await Promise.all([S.list('entries',requestedAccount),S.list('document-index',requestedAccount),S.read('settings',requestedAccount,'drive-root'),S.read('settings',requestedAccount,'home-views'),S.list('outbox',requestedAccount)]);
        if(account!==requestedAccount)return;
        const aliases=new Map(items.filter(e=>e.remoteId && e.remoteId!==e.id).map(e=>[e.remoteId,e.id]));
        if(driveRoot?.remoteId)aliases.set(driveRoot.remoteId,'root');
        const byId=new Map(items.map(e=>[e.id,e]));storedEntries=byId;
        entries=items.filter(e=>!aliases.has(e.id)).map(e=>{
            const remote=!e.pending&&e.remoteId?byId.get(e.remoteId):null;
            const remoteIsNewer=remote&&(Date.parse(remote.modifiedTime)||0)>=(Date.parse(e.modifiedTime)||0);
            const item=remoteIsNewer?{...e,...remote,id:e.id,remoteId:e.remoteId}:e;
            return {...item,parents:(item.parents||['root']).map(p=>aliases.get(p)||p)};
        });
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
    function folder(id, search='') {
        const needle=search.toLowerCase();
        // Read this directory only; rebuilding every card no longer searches
        // the entire Drive catalogue. Filter returns a fresh sortable array.
        return (folderEntries.get(id||'root')||[]).filter(e=>!needle||e.name?.toLowerCase().includes(needle))
            .sort(compareEntries);
    }
    function combine(remote,parent,search='') {
        const local=folder(parent,search).filter(e=>e.pending||e.id.startsWith('local-'));
        const ids=new Set(local.flatMap(e=>[e.id,e.remoteId]));
        return [...local,...remote.filter(e=>!ids.has(e.id))].sort(compareEntries);
    }
    function home() {
        const active=activeEntries;
        const ordered=list=>list.sort(compareRecents);
        const view=(name,fallback)=>Array.isArray(savedViews[name])
            ? savedViews[name].map(id=>activeAliases.get(id)).filter(Boolean) : fallback;
        const files=active.filter(e=>e.mimeType==='application/pdf');
        const recents=view('recents',ordered(files).slice(0,12));
        return {recents:recentItems(recents),
            starred:view('starred',ordered(active.filter(e=>e.starred)).slice(0,12)),
            shared:view('shared',ordered(active.filter(e=>e.ownedByMe===false)).slice(0,24))};
    }
    function compareRecents(a,b) {
        const openedA=record(a.id)?.lastOpenedAt||0,openedB=record(b.id)?.lastOpenedAt||0;
        const delta=openedB-openedA||modifiedTime(b)-modifiedTime(a);
        const ak=String(a.remoteId||a.id),bk=String(b.remoteId||b.id);
        return delta||(ak<bk?-1:ak>bk?1:0);
    }
    function recentItems(remote) {
        const byId=new Map();
        // Both cached and network renders use the same ordering and pending
        // aliases. Never prepend the outbox in IndexedDB key order.
        for(const entry of remote)byId.set(entry.remoteId||entry.id,entry);
        for(const entry of activeEntries)if(entry.mimeType==='application/pdf'&&(entry.pending||entry.id.startsWith('local-')))byId.set(entry.remoteId||entry.id,entry);
        return [...byId.values()].sort(compareRecents).slice(0,12);
    }
    // Home refreshes its lists every 30 s. When Drive returns exactly what
    // is already cached (and the same view order), there is nothing to write
    // or to re-render.
    let storedEntries=new Map();
    const sameValue=(a,b)=>a===b||JSON.stringify(a)===JSON.stringify(b);
    function alreadyObserved(items,view) {
        if(!loaded)return false;
        if(view&&!sameValue(savedViews[view],items.map(e=>e.id)))return false;
        return items.every(item=>{
            const stored=storedEntries.get(item.id);
            return !!stored&&Object.keys(item).every(key=>sameValue(stored[key],item[key]));
        });
    }
    async function observed(items,parent,view) {
        if (!account) return;
        const observedAccount=account;
        const observedItems=items.map(e=>({...e,...(!e.parents&&parent?{parents:[parent]}:{})}));
        if(alreadyObserved(observedItems,view))return;
        await S.cacheEntries(observedAccount,observedItems);
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
            capture().then(()=>kick({reason:'checkpoint'})).catch(error=>host.error(error));
        },180);
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
        const entry=await S.createFolder(account,parent,name);await reload();await scheduleBackground();void kick({reason:'checkpoint'});return entry;
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
    // ── Platform background work (Android WorkManager, Background Sync) ──
    // Each starts a second, complete instance of the app. While this page is
    // visible it processes the outbox itself, so platform work is requested
    // only once the app is hidden with operations pending, and withdrawn
    // while the app is visible or nothing is pending. WorkManager jobs
    // survive restarts, so their state is unknown until first cancelled.
    let nativePausedForEditing = false, nativeScheduled = null;
    const appVisible = () => document.visibilityState === 'visible';
    function cancelNative() {
        if (isWorker || nativeScheduled === false || !root.InhouseBackground?.cancel) return;
        try { root.InhouseBackground.cancel(); nativeScheduled = false; } catch (_) { }
    }
    async function scheduleBackground() {
        if (!account || isWorker || (nativePausedForEditing && root.hasHotUserInteraction?.(root.getDocumentSaveIdleWindow?.() ?? 180))) return;
        nativePausedForEditing = false;
        if (appVisible()) return;
        const scheduledAccount = account;
        const pending = (await S.list('outbox', scheduledAccount)).length;
        if (account !== scheduledAccount || appVisible()) return;
        if (!pending) { cancelNative(); return; }
        try { if (root.InhouseBackground?.schedule) { root.InhouseBackground.schedule(); nativeScheduled = true; } } catch (_) { }
        try {const registration=await navigator.serviceWorker?.getRegistration();await registration?.sync?.register('inhouse-outbox');}catch(_){}
    }
    let nativeEditingResumeTimer = null;
    function pauseForEditing() {
        clearTimeout(nativeEditingResumeTimer); nativeEditingResumeTimer = null;
        // The hidden editor must yield with its foreground owner as well.
        try { syncFrame?.contentWindow?.interruptDocumentBackgroundWork?.(); } catch (_) { }
        // Android also has an independent headless sync WebView. Stop its
        // WorkManager job once per writing burst.
        if (!nativePausedForEditing && root.InhouseBackground?.cancel) {
            nativePausedForEditing = true;
            try { root.InhouseBackground.cancel(); nativeScheduled = false; } catch (_) { }
        }
    }
    function resumeAfterEditing() {
        if (!nativePausedForEditing) return;
        if (root.hasHotUserInteraction?.(root.getDocumentSaveIdleWindow?.() ?? 180)) {
            if (!nativeEditingResumeTimer) nativeEditingResumeTimer = setTimeout(() => {
                nativeEditingResumeTimer = null;
                resumeAfterEditing();
            }, root.getDocumentSaveIdleWindow?.() ?? 180);
            return;
        }
        clearTimeout(nativeEditingResumeTimer); nativeEditingResumeTimer = null;
        nativePausedForEditing = false;
        // A visible app keeps processing its own outbox; the platform job
        // is requested again when the app is hidden (see scheduleBackground).
        void scheduleBackground();
        if (deferredKick) void kick({reason:'again'});
    }

    // ── Outbox scheduling ──
    // The outbox is processed only when that can make progress: a new
    // checkpoint, regained connectivity, the app returning to the
    // foreground, an explicit retry, a released sync lease or the backoff
    // deadline of an unfinished operation (InhouseOfflineSync.retryAt).
    // There is no fixed polling: at most one timer waits for the next
    // deadline. One hidden document processor is reused between runs and
    // released once idle.
    const LAUNCH_CAPTURE_WAIT_MS = 3000, BUSY_RECHECK_MS = 3000, MIN_RUN_GAP_MS = 5000, WORKER_IDLE_MS = 90000, WORKER_RUN_TIMEOUT_MS = 8 * 60 * 1000;
    const RUN_RETRY_BASE_MS = 15000, RUN_RETRY_MAX_MS = 15 * 60000;
    let kickTimer = null, kickTimerAt = 0, kickAgain = false, forceAgain = false, deferredKick = false, waitingForLease = false, waitingForAuth = false;
    let worker = null, workerRunning = false, workerIdleTimer = null, lastRunEndedAt = 0, runFailures = 0, runRetryAt = 0;
    const diagnostics = { kicks: 0, runs: 0, frames: 0, reused: 0 };
    const retryAt = op => root.InhouseOfflineSync?.retryAt?.(op) || 0;
    // Kicks caused by this app's own saving never force another save of the
    // open document (which could otherwise feed itself).
    const OWN_WORK = new Set(['checkpoint', 'after-save', 'after-run', 'lease', 'again']);
    function armKick(at) {
        if (isWorker || !(at > 0)) return;
        if (kickTimer && kickTimerAt <= at) return;
        clearTimeout(kickTimer); kickTimerAt = at;
        kickTimer = setTimeout(() => { kickTimer = null; kickTimerAt = 0; void kick({reason:'timer'}); }, Math.max(0, at - Date.now()));
    }
    function stopScheduling() {
        if (kickTimer) clearTimeout(kickTimer);
        kickTimer = null; kickTimerAt = 0;
        kickAgain = forceAgain = deferredKick = waitingForLease = waitingForAuth = false;
        runFailures = 0; runRetryAt = 0;
    }
    async function kick(options = {}) {
        if (!options || typeof options !== 'object') options = {};
        if (isWorker || !initialized || !resumed || !account || offline()) return;
        diagnostics.kicks++;
        if (options.force) { forceAgain = true; runRetryAt = 0; }
        if (kicking || workerRunning) { kickAgain = true; return; }
        if (host.busy()) { deferredKick = true; armKick(Date.now() + BUSY_RECHECK_MS); return; }
        if (host.canSync && !host.canSync()) { waitingForAuth = true; return; } // signing in kicks again
        waitingForAuth = false;
        deferredKick = false; kicking = true;
        // A forced kick (launch, reconnection, sign-in, explicit retry) stays
        // forced while it is only deferred by a lease, a manual save or input.
        const force = forceAgain;
        const kickAccount = account;
        let pending = null;
        try {
            const priority=await S.read('settings',account,'sync-priority');
            // saveNow kicks again when it releases the priority.
            if(priority?.documentId&&priority.expires>Date.now()){armKick(priority.expires+50);return;}
            let queue=await S.list('outbox',account);
            const activeId = host.activeDocument?.() ? host.currentId() : null;
            let now = Date.now();
            const due = op => force || retryAt(op) <= now;
            const active = activeId ? queue.find(op => op.kind === 'document' && op.id === activeId) : null;
            // A backoff timer only concerns the open document when its own
            // foreground attempt failed (InhouseOfflineApp.queueSave).
            if (active && due(active) && !OWN_WORK.has(options.reason) && (options.reason !== 'timer' || active.attemptedAt)) {
                // The background editor deliberately skips a live document.
                // Its foreground owner retries it on the same events. A fresh
                // checkpoint or a finished save needs no forced save: the
                // editor's own autosave already follows every edit.
                await host.syncActive?.();
                queue = await S.list('outbox', account); now = Date.now();
            }
            if (account !== kickAccount) return;
            const background = queue.filter(op => op.kind === 'folder' || op.id !== activeId);
            const later = queue.map(retryAt).filter(at => at > now);
            const nextDeadline = later.length ? Math.min(...later) : 0;
            if (!background.some(due)) { forceAgain = false; armKick(nextDeadline); return; }
            if (!force && runRetryAt > now) { armKick(runRetryAt); return; }
            if (!force && lastRunEndedAt && now - lastRunEndedAt < MIN_RUN_GAP_MS) { armKick(lastRunEndedAt + MIN_RUN_GAP_MS); return; }
            // An upload in this or another window holds the account's sync
            // lease; starting a processor now would only find it busy.
            const lease = S.activeLease ? await S.activeLease(account, 'sync') : await S.read('leases', account, 'sync');
            if (lease?.expires > Date.now()) { waitingForLease = true; armKick(lease.expires + 50); return; }
            waitingForLease = false;
            if (offline() || host.busy() || account !== kickAccount) { deferredKick = true; armKick(Date.now() + BUSY_RECHECK_MS); return; }
            pending = queue.length; forceAgain = false;
        } catch(error) {
            await syncFinished({errors:[{message:error.message}]}).catch(()=>{});
        } finally {
            kicking=false;
            // Re-evaluate once for kicks that arrived meanwhile (a run does so when it ends).
            if (pending === null && kickAgain) { kickAgain = false; void Promise.resolve().then(() => kick({reason:'again'})); }
        }
        if (pending !== null) runWorker(pending);
    }
    function leaseReleased() {
        if (waitingForLease) { waitingForLease = false; void kick({reason:'lease'}); }
    }
    function retireWorker() {
        if (workerIdleTimer) clearTimeout(workerIdleTimer);
        workerIdleTimer = null;
        if (!worker) return;
        const current = worker; worker = null;
        window.removeEventListener('message', current.listener);
        current.run?.finish({errors:[{message:'Background sync stopped'}]}, true);
        current.frame.remove();
        if (syncFrame === current.frame) syncFrame = null;
    }
    function runWorker(pending) {
        workerRunning = true; diagnostics.runs++;
        clearTimeout(workerIdleTimer); workerIdleTimer = null;
        setSyncProgress({phase:'checking',pending,id:null,errors:[]});
        const reuse = !!worker?.reusable && !worker.run;
        if (!reuse) retireWorker();
        const current = worker || (worker = { frame: null, reusable: false, run: null, listener: null });
        const run = { done: false };
        current.run = run;
        run.finish = (result = {}, retiring = false) => {
            if (run.done) return; run.done = true;
            clearTimeout(run.timer);
            if (current.run === run) current.run = null;
            workerRunning = false; lastRunEndedAt = Date.now();
            // A run that could not start (authentication, a busy lease, a
            // crashed or hung processor) backs off as a whole.
            const failed = !!(result.busy || result.errors?.some(error => !error.id));
            if (failed) { runFailures++; runRetryAt = Date.now() + Math.min(RUN_RETRY_MAX_MS, RUN_RETRY_BASE_MS * 2 ** Math.min(runFailures - 1, 16)); }
            else { runFailures = 0; runRetryAt = 0; }
            if (!retiring) {
                // Release the processor once nothing is left; keep it for the
                // next attempt when work remains or another upload held the lease.
                if (result.timedOut || !current.reusable || (!result.busy && !result.pending && !result.errors?.length)) { if (worker === current) retireWorker(); }
                else workerIdleTimer = setTimeout(() => { if (worker === current && !current.run) retireWorker(); }, WORKER_IDLE_MS);
            }
            void syncFinished(result).catch(() => {}).finally(() => { kickAgain = false; void kick({reason:'after-run'}); });
        };
        run.timer = setTimeout(() => run.finish({errors:[{message:'Sync timed out; retrying automatically'}],timedOut:true}), WORKER_RUN_TIMEOUT_MS);
        if (reuse) {
            diagnostics.reused++;
            try { current.frame.contentWindow.postMessage({type:'ihn-offline-run'}, location.origin); }
            catch (error) { run.finish({errors:[{message:error.message}]}); }
            return;
        }
        diagnostics.frames++;
        const frame=document.createElement('iframe');frame.hidden=true;frame.title='Background document sync';
        frame.src=`/?offline-worker=1&v=${encodeURIComponent(host.version)}`;
        current.frame = frame; syncFrame = frame;
        current.listener = event => {
            if (event.origin !== location.origin || event.source !== frame.contentWindow) return;
            if (event.data?.type === 'ihn-offline-progress') setSyncProgress(event.data.progress);
            if (event.data?.type === 'ihn-offline-complete') {
                current.reusable = event.data.reusable === true;
                current.run?.finish(event.data.result || {});
            }
        };
        window.addEventListener('message', current.listener);
        document.body.appendChild(frame);
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
            window.addEventListener('online',()=>{stopOfflineProbes();void probeConnection();});
            document.addEventListener('visibilitychange',()=>{
                if(document.hidden){clearTimeout(probeTimer);probeTimer=null;return;}
                // A visible app does its own background work (see scheduleBackground).
                cancelNative();
                probeDelay=PROBE_BASE_MS;
                void probeConnection();
            });
            if(appVisible())cancelNative();
            void probeConnection();
            window.addEventListener('message',event=>{if(event.origin===location.origin&&event.data?.type==='ihn-offline-dirty')reload();});
        }
        if (account) {await reload();if(!isWorker){await S.put('settings','__session__','active',{accountId:account,enabled:true});host.refreshed?.();setTimeout(()=>S.prune(account,Date.now(),[host.currentId()]).then(()=>reload()).catch(()=>{}),2000);}}
        
    }
    async function resume() {
        if(isWorker||!initialized)return;
        resumed=true;
        // A new launch retries everything left by the previous one at once,
        // whatever backoff that session had reached. The restored editor is
        // checkpointed first, but a slow or stuck checkpoint must not hold
        // back documents that were left waiting before the app closed.
        const captured=capture();
        try {await Promise.race([captured.catch(()=>{}),new Promise(resolve=>setTimeout(resolve,LAUNCH_CAPTURE_WAIT_MS))]);}
        finally {kick({force:true,reason:'launch'}).catch(()=>{});catalogue().catch(()=>{});}
        await captured;
    }
    async function useAccount(email) {
        const next=String(email||'').trim().toLowerCase();
        if(!next||next===account)return;
        // The hidden processor belongs to the previous account.
        retireWorker();stopScheduling();
        account=next;loaded=false;await reload();host.refreshed?.();void kick({force:true,reason:'account'});catalogue().catch(()=>{});
    }
    function disable() {initialized=false;resumed=false;stopScheduling();stopOfflineProbes();}
    function logout() {setSyncProgress({phase:'idle',pending:0,id:null,errors:[]});account='';loaded=false;savedViews={};entries=[];storedEntries=new Map();documents.clear();indexCatalogue();retireWorker();stopScheduling();nativeScheduled=null;cancelNative();}
    root.InhouseOfflineLibrary=Object.freeze({pauseForEditing,resumeAfterEditing,init,resume,disable,account:()=>account,offline,networkAvailable,ready:()=>initialized&&loaded&&!!account,folder,combine,home,recentItems,syncErrorText,observed,cache,capture,scheduleCapture,prepared,
        openRecord,pin,createFolder,catalogue,kick,leaseReleased,setSyncProgress,syncFinished,syncStatus:()=>({...syncState}),scheduleBackground,decorate,reload,useAccount,logout,settings:showSettings,record,isWorker,
        diagnostics:()=>({...diagnostics,workerAlive:!!worker,workerRunning,timerAt:kickTimerAt||null,runRetryAt:runRetryAt||null,probing:!!probeTimer,nativePaused:nativePausedForEditing})});
})(window);
