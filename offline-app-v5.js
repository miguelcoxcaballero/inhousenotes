/* Integration with the existing editor and its tested collaboration merge. */
(function (root) {
    'use strict';
    const S=root.InhouseOfflineStore, L=root.InhouseOfflineLibrary;
    function currentId() {return state.offlineLocalId || state.driveFileId || '';}
    function metadata() {return {id:state.driveFileId,headRevisionId:state.driveHeadRevisionId,version:state.driveRevisionVersion,
        modifiedTime:state.driveLastKnownModified?new Date(state.driveLastKnownModified).toISOString():null};}
    // Immutable packed pages are reused only within one editor session and
    // mutation generation. Bound retained snapshots by both bytes and pages.
    const pageSnapshots = new Map();
    let pageSnapshotSession = null, pageSnapshotBytes = 0;
    const snapshotPageFields = ['pageId','strokes','images','legacyCoverStrokes',
        'deletedStrokeIds','deletedStrokeStamps','backgroundImage','backgroundSource',
        'templateKind','pdfPageIndex','pageWidth','pageHeight','sidePanel','unloaded'];
    function pageSnapshotKey(page, index) {
        return [pageDirtyGeneration.get(index) || 0,
            ...snapshotPageFields.map(key=>page?.[key]), page?.strokes?.length,
            page?.images?.length, page?.legacyCoverStrokes?.length];
    }
    function rememberPageSnapshot(owner, key, snapshot) {
        const previous = pageSnapshots.get(owner);
        if (previous) { pageSnapshotBytes -= previous.bytes; pageSnapshots.delete(owner); }
        const bytes = (snapshot.__inkCheckpoint?.bytes?.byteLength || 0)
            + (snapshot.backgroundImage?.length || 0) * 2
            + (snapshot.images || []).reduce((n,image)=>n+(image.src?.length || 0)*2+256,0)
            + [snapshot.strokes,snapshot.legacyCoverStrokes].reduce((n,strokes)=>n+(strokes || []).reduce((m,stroke)=>m+512+(Array.isArray(stroke.points)?stroke.points.length*48:0),0),0);
        if (bytes > 16 * 1024 * 1024) return;
        while (pageSnapshots.size && (pageSnapshots.size >= 64 || pageSnapshotBytes + bytes > 16 * 1024 * 1024)) {
            const oldest = pageSnapshots.keys().next().value;
            pageSnapshotBytes -= pageSnapshots.get(oldest).bytes; pageSnapshots.delete(oldest);
        }
        pageSnapshots.set(owner,{key,snapshot,bytes}); pageSnapshotBytes += bytes;
    }
    async function capture() {
        const id=currentId();
        if(!id || !state.pages.length || driveOpenInProgress || state.driveCanEdit===false)return null;
        // Boot restores a single-editor cache, not necessarily the newest
        // per-document draft. Home must upload the durable outbox as-is until
        // this document has been explicitly opened/restored in this launch.
        const existing=L.record(id);
        if(!documentSessionActivated && existing?.available && existing.session!==getOfflineDocumentSessionToken())return null;
        const token=getDocumentSessionToken(),version=driveContentVersion;
        if (pageSnapshotSession !== token) {
            pageSnapshots.clear(); pageSnapshotBytes = 0; pageSnapshotSession = token;
        }
        const pages=[];
        const assertCurrent=()=>{
            if(token!==getDocumentSessionToken()||version!==driveContentVersion)
                throw createAbortError('Document changed during offline checkpoint');
        };
        const yieldCapture=async()=>{
            await yieldProcessingForUi();
            await waitForHotInteractionToSettle({sessionToken:token,windowMs:PAGE_CHECKPOINT_IDLE_MS});
            assertCurrent();
        };
        // Copy incrementally: a dense scanned page must not serialize the whole
        // notebook in one main-thread task between two pen contacts.
        for(let i=0;i<state.pages.length;i++){
            await yieldCapture();
            const owner=state.pages[i],key=pageSnapshotKey(owner,i),cached=pageSnapshots.get(owner);
            if (cached && cached.key.every((value,index)=>value===key[index])) {
                pageSnapshots.delete(owner); pageSnapshots.set(owner,cached);
                pages.push(cached.snapshot); continue;
            }
            const page=await loadPageDataForExport(i);assertCurrent();
            const copy=buildTimelineSnapshotFromPages([{...page,strokes:[],images:[],legacyCoverStrokes:null}])[0];
            if(window.InhouseDocumentProcessing){
                const packed=await packInkPageForStorage({...copy,strokes:page.strokes||[],images:page.images||[],legacyCoverStrokes:page.legacyCoverStrokes||null},token,()=>{assertCurrent();return false;},true);
                assertCurrent();
                if (!packed) throw createAbortError('Page checkpoint interrupted');
                rememberPageSnapshot(owner,key,packed);pages.push(packed);continue;
            }
            copy.legacyCoverStrokes=cloneTimelineValue(page.legacyCoverStrokes||null);
            let sliceStarted=performance.now();
            for(const kind of ['strokes','images'])for(const item of page[kind]||[]){
                copy[kind].push(cloneTimelineValue(item));
                if(performance.now()-sliceStarted>=3){await yieldCapture();sliceStarted=performance.now();}
            }
            assertCurrent();rememberPageSnapshot(owner,key,copy);pages.push(copy);
        }
        assertCurrent();
        const snapshot={pages,collabStructure:getCollabStructureSnapshot(),collabFields:getCollabFieldSnapshot(),
            calendarPageConfig:state.calendarPageConfig,exportName:state.exportName,versionHistory:state.versionHistory,
            baseline:[...lastSyncedStrokeIds].map(([index,ids])=>[index,[...ids]]),baselinePages:[...lastSyncedPageIds]};
        const pendingExit=readPendingExitUpload();
        const dirty=driveContentVersion!==driveUploadedContentVersion || driveDirty || state.driveFileId?.startsWith('local-') || pendingExit?.fileId===state.driveFileId;
        return {id,snapshot,baseMeta:metadata(),contentVersion:version,session:getOfflineDocumentSessionToken(token),dirty,
            entry:{id,localPreview:state.pages[0]?.preview||null,name:state.driveFileName||`${state.exportName||'Untitled'}.pdf`,mimeType:'application/pdf',parents:state.driveParentIds?.length?state.driveParentIds:['root'],ownedByMe:state.driveOwnedByMe,pending:!!dirty},
            details:{fileName:state.driveFileName,parentIds:state.driveParentIds,canEdit:state.driveCanEdit,ownedByMe:state.driveOwnedByMe,resourceKey:state.driveResourceKey,folderKind:state.driveFolderKind,ownerName:state.driveOwnerName,ownerEmail:state.driveOwnerEmail,ownerPhoto:state.driveOwnerPhoto},
            originalBlob:state.cleanOriginalPdfBytes?.byteLength?new Blob([state.cleanOriginalPdfBytes],{type:'application/pdf'}):undefined};
    }
    function renderSyncStatus() {
        refreshSaveIndicatorState();
    }
    function renderHome() {
        if(!L.ready())return;
        const cached=L.home();
        {
            driveRecentsCache=cached.recents;driveStarredCache=cached.starred;driveSharedCache=cached.shared;
            renderDriveRecents(cached.recents);renderDriveStarred(cached.starred);renderDriveShared(cached.shared);
            renderFolder();
            if(driveStatusEl)driveStatusEl.textContent=L.offline()?'Offline · changes stay on this device':'Connecting to Drive…';
        }
        updateDriveAuthUI();
    }
    function renderFolder() {
        const items=L.folder(driveCurrentFolderId,driveSearchInput?.value?.trim()||'');
        driveFolderCache={folders:items.filter(e=>e.mimeType==='application/vnd.google-apps.folder'),files:items.filter(e=>e.mimeType==='application/pdf')};
        renderDriveFolderContents(driveFolderCache.folders,driveFolderCache.files);renderDriveBreadcrumbs();
    }
    async function restore(doc,render=true) {
        const token=beginDocumentSession();
        resetPdfState();
        state.history=[];state.historyIndex=-1;
        state.hasLegacyBakedOverlay=false;state.activePageIndex=0;
        state.offlineLocalId=doc.id.startsWith('local-')?doc.id:null;
        state.driveFileId=doc.remoteId||doc.id;
        state.driveFileName=doc.entry?.name||doc.details?.fileName||'Untitled.pdf';
        state.driveParentIds=doc.entry?.parents||doc.details?.parentIds||['root'];
        state.driveFolderId=state.driveParentIds[0];
        state.driveCanEdit=doc.details?.canEdit!==false;
        state.driveOwnedByMe=doc.details?.ownedByMe!==false;
        state.driveResourceKey=doc.details?.resourceKey||'';
        state.driveFolderKind=doc.details?.folderKind||null;
        state.driveOwnerName=doc.details?.ownerName||'';state.driveOwnerEmail=doc.details?.ownerEmail||'';state.driveOwnerPhoto=doc.details?.ownerPhoto||'';
        state.driveAutosave=false;
        // A current native snapshot is already editable; parsing an older PDF
        // before replacing it with these same strokes wastes time and memory.
        const snapshotNeedsPdf=doc.snapshot?.pages.some(p=>p.pdfPageIndex||p.backgroundSource==='pdf');
        if(doc.blob && (!doc.snapshot || snapshotNeedsPdf&&!doc.originalBlob?.size)) {
            await importPDFData(URL.createObjectURL(doc.blob),true,{sourceBlob:doc.blob,sessionToken:token,driveFileId:state.driveFileId,strictCollabMetadata:true,cachedAnalysis:doc.analysis||null,throwOnError:true});
            if(doc.snapshot&&cleanOriginalRecoveryPromise){startCleanOriginalRecovery?.();await cleanOriginalRecoveryPromise;}
        }
        if(doc.originalBlob?.size) {
            const bytes=await doc.originalBlob.arrayBuffer();
            state.cleanOriginalPdfBytes=bytes;
            if(doc.snapshot?.pages.some(p=>p.pdfPageIndex)) {
                if(activePdfDocument)await activePdfDocument.destroy().catch(()=>{});
                activePdfDocument=await loadSecurePdfDocument({data:bytes.slice(0),cMapUrl:PDFJS_CMAP_URL,cMapPacked:true,standardFontDataUrl:PDFJS_STANDARD_FONT_URL});
                activePdfPageCount=activePdfDocument.numPages;
            }
        }
        if(doc.snapshot) {
            // Decode only pages requested by the editor; hidden pages stay binary.
            await migrateLegacyPayload({...doc.snapshot,pages:doc.snapshot.pages.slice()},{awaitAllPages:false,deferPageWrites:true,initialPages:1});
            state.collabStructure=ihnNormalizeStructureMeta(doc.snapshot.collabStructure,state.pages.map(p=>p.pageId));
            state.collabFields=ihnNormalizeFieldMeta(doc.snapshot.collabFields);
            state.calendarPageConfig=doc.snapshot.calendarPageConfig||null;
            state.exportName=doc.snapshot.exportName||state.driveFileName.replace(/\.pdf$/i,'');
            state.versionHistory=doc.snapshot.versionHistory||[];
            lastSyncedStrokeIds.clear();
            for(const [index,ids] of doc.snapshot.baseline||[])lastSyncedStrokeIds.set(index,new Set(ids));
            lastSyncedPageIds.clear();for(const id of doc.snapshot.baselinePages||[])lastSyncedPageIds.add(id);
        } else updateSyncBaseline();
        state.driveHeadRevisionId=doc.baseMeta?.headRevisionId||null;
        state.driveRevisionVersion=normalizeDriveVersion(doc.baseMeta?.version)||null;
        state.driveLastKnownModified=typeof doc.baseMeta?.modifiedTime==='number'?doc.baseMeta.modifiedTime:(Date.parse(doc.baseMeta?.modifiedTime||'')||null);
        state.driveAutosave=!L.isWorker&&state.driveCanEdit;
        markDriveHydrated(state.driveFileId);
        resetDriveContentSyncVersion();
        if(doc.dirty){driveContentVersion=1;driveUploadedContentVersion=0;driveDirty=true;}
        else {driveDirty=false;clearPendingExitUpload(state.driveFileId);if(doc.id!==state.driveFileId)clearPendingExitUpload(doc.id);}
        if(render) {
            state.zoom=1;state.panX=0;state.panY=0;state.overscrollY=0;
            showEditorView();renderAllPages();centerViewOnTop();setReadOnlyMode(!state.driveCanEdit,{force:true});
            updateDocTitle();saveDriveSession();await saveToIndexedDb(buildMetaPayload(Date.now()),Date.now());
            hideLoading();showStatus(doc.dirty?'Saved on this device · waiting to sync':'Available offline',{preserveState:true});
            refreshSaveIndicatorState();
            if(!L.offline()&&!doc.id.startsWith('local-')) {
                startCollabPolling();
                const file={id:state.driveFileId,name:state.driveFileName};
                setTimeout(()=>{if(isDocumentSessionTokenValid(token))revalidateCachedDriveOpen(file,{meta:doc.baseMeta||{}},token);},0);
            }
        }
        return token;
    }
    async function reconcileStartup() {
        if(!L.ready()||!currentId())return;
        // Startup continues (and the outbox resumes) even if this optional
        // refresh of the restored editor fails; the editor copy stays as is.
        try {
            const doc=await S.read('documents',L.account(),currentId());
            const marker=readPendingExitUpload();
            if(doc&&!doc.dirty&&doc.lastUploadedAt>(state.lastSavedAt||0)
                &&(!marker||marker.fileId===state.driveFileId&&marker.contentVersion<=doc.contentVersion))await restore(doc,false);
        } catch(error) {console.warn('Offline startup reconciliation skipped:',error);}
    }
    async function refreshed() {
        const doc=L.record(currentId());
        if(doc&&!doc.dirty&&doc.lastUploadedAt&&doc.session===getOfflineDocumentSessionToken()
            &&doc.contentVersion===driveContentVersion&&driveDirty&&!hasSmoothInteraction()) {
            const readOnly=state.isReadOnly;
            await restore(await S.read('documents',L.account(),doc.id),!!editorView&&!editorView.classList.contains('hidden'));setReadOnlyMode(readOnly,{force:true});
        }
        renderHome();
    }
    async function workerBuild(doc) {
        await restore(doc,false);
        return buildPdfBlob({sessionToken:getDocumentSessionToken(),...DRIVE_UPLOAD_PDF_OPTIONS});
    }
    async function merge(doc,meta,remoteBlob) {
        // Called after local PDF construction, so this isolated editor contains
        // the exact local snapshot plus its persisted baseline and tombstones.
        const result=await pullRemoteChanges(meta,{allowWhileLocalPending:true,preserveLocalUnsynced:true,visualSync:false,uploadManagedExternally:true,
            downloadedSnapshot:remoteBlob instanceof Blob?{fileId:state.driveFileId,blob:remoteBlob,meta}:null});
        if(result?.error)throw result.cause||new Error('Concurrent changes could not be merged');
        if(!result || result.busy || result.deferred || result.stale)throw Object.assign(new Error(
            result?.backgroundRenderFailed?'The remote PDF background could not be reconstructed':
            result?.supersededRevision?'The Drive revision changed during merge':
            result?.revisionRecheckFailed?'The Drive revision could not be rechecked':
            'Concurrent changes could not be merged yet'),{code:result?.supersededRevision?412:'needs-merge'});
        if(result.noMetadata)throw Object.assign(new Error('The remote PDF has no editable merge data; both copies are retained'),{code:'merge-unavailable'});
        return buildPdfBlob({sessionToken:getDocumentSessionToken(),...DRIVE_UPLOAD_PDF_OPTIONS});
    }
    async function authorizeBackground() {
        const account=L.account(); if(!account || driveSignedOut)return;
        await S.put('settings','__session__','active',{accountId:account,enabled:true});
        await S.put('settings',account,'background-auth',{token:driveAccessToken,expiry:driveTokenExpiry,
            refreshToken:useAndroidPkceFlow()?localStorage.getItem(DRIVE_REFRESH_TOKEN_KEY):null,clientId:useAndroidPkceFlow()?ANDROID_OAUTH_CLIENT_ID:null});
        // Signing in is what a run waiting for authentication was waiting for.
        if(!L.isWorker)void L.kick({force:true,reason:'signed-in'});
    }
    // The editor lease tells a background processor in another window (or
    // Android's headless one) which document a visible editor owns. It is
    // written when that owner changes and renewed at half its lifetime only
    // while an editor is visible; a hidden or closed app releases it at once.
    const EDITOR_LEASE_MS=5*60000;
    let editorLease={key:null,expires:0},editorLeaseTimer=null;
    function editorActive() {return document.visibilityState==='visible'&&!!editorView&&!editorView.classList.contains('hidden');}
    function activeId() {return editorActive()?currentId():null;}
    function heartbeat(force=false) {
        const account=L.account();if(!account||L.isWorker)return;
        const active=editorActive(),documentId=currentId(),now=Date.now();
        const key=JSON.stringify([account,documentId,active]);
        if(!force&&key===editorLease.key&&(!active||editorLease.expires-now>EDITOR_LEASE_MS/2))return;
        editorLease={key,expires:active?now+EDITOR_LEASE_MS:0};
        clearTimeout(editorLeaseTimer);editorLeaseTimer=null;
        if(active)editorLeaseTimer=setTimeout(()=>heartbeat(),EDITOR_LEASE_MS/2+1000);
        return S.put('leases',account,'editor',{documentId,expires:editorLease.expires}).catch(()=>{editorLease.key=null;});
    }
    async function initialize() {
        await L.init({version:APP_VERSION,currentId,quickAccess:isQuickAccessDriveFile,activeDocument:()=>{void heartbeat();return editorActive();},busy:()=>hasSmoothInteraction()||driveOpenInProgress||offlineOpenInProgress,
            // Signed out, or a failed refresh with no usable token: nothing can
            // upload until the user signs in again (authorizeBackground kicks).
            canSync:()=>!driveSignedOut&&(!driveAuthNeedsReconnect||!!driveAccessToken),
            identity:()=>({id:currentId(),version:driveContentVersion,session:getOfflineDocumentSessionToken()}),
            retrySync:()=>driveAuthNeedsReconnect||L.syncStatus().errors.some(error=>error.code===401||/sign.?in|refresh token/i.test(error.message||''))?signInDrive():void L.kick({force:true,reason:'retry'}),syncChanged:renderSyncStatus,syncActive:()=>queueDriveSave(true),
            connectionChanged:connected=>{
                renderSyncStatus();
                if(connected&&L.ready()){
                    if(state.driveAutosave&&driveDirty)void queueDriveSave(true).catch(()=>{});
                    if(isDriveHomeVisible())void refreshDriveHomeQuiet();
                }
            },
            capture,fetch:driveFetch,status:text=>showStatus(text,{preserveState:true}),error:error=>{if(error?.name!=='AbortError')showStatus('Offline copy could not be saved. Keep this document open.',{error:true});},
            download:async file=>{
                const response=await driveFetch(`https://www.googleapis.com/drive/v3/files/${encodeURIComponent(file.id)}?fields=id,name,headRevisionId,version,modifiedTime,capabilities(canEdit)`);
                return fetchStableDrivePdfSnapshot(file.id,await response.json());
            },downloadBlob:(blob,name)=>{const link=document.createElement('a'),url=URL.createObjectURL(blob);link.href=url;link.download=name;link.click();setTimeout(()=>URL.revokeObjectURL(url),60000);},
            refreshed:()=>refreshed().catch(()=>{})});
        if(L.isWorker)return;
        // No polling: the lease follows the editor's visibility and document,
        // and the outbox is kicked by events (see InhouseOfflineLibrary.kick).
        if(editorView&&typeof MutationObserver==='function')new MutationObserver(()=>void heartbeat()).observe(editorView,{attributes:true,attributeFilter:['class']});
        document.addEventListener('visibilitychange',()=>{
            void heartbeat();
            // Returning to the app probes Drive and kicks the outbox (library).
            if(document.hidden)L.capture().then(()=>L.scheduleBackground()).catch(()=>{});
        });
        window.addEventListener('online',()=>{if(state.driveAutosave&&driveDirty)queueDriveSave(true);});
        await heartbeat(true);
        if(driveAccessToken)await authorizeBackground();
    }
    async function backgroundAccess(expectedAccount,rejectedToken=null) {
        const assertAccount=()=>{
            if(!expectedAccount||L.account()!==expectedAccount||driveSignedOut
                ||(localStorage.getItem('drive-hint-email')||'').trim().toLowerCase()!==expectedAccount)
                throw new Error('Background account changed or signed out');
        };
        assertAccount();
        // Invalidate only the token rejected by Drive; another request may
        // already have refreshed the shared session in the meantime.
        if(rejectedToken&&driveAccessToken===rejectedToken){driveAccessToken=null;driveTokenExpiry=0;}
        const token=await ensureDriveToken(false);
        assertAccount();
        return {token,expiry:driveTokenExpiry};
    }
    const documentFetchControllers = new Set();
    function pauseForEditing() {
        for (const controller of documentFetchControllers) controller.abort();
    }
    async function documentFetch(url, options = {}) {
        await waitForDocumentBackgroundTurn({signal:options.signal});
        const controller = new AbortController();
        const abort = () => controller.abort();
        options.signal?.addEventListener('abort', abort, {once:true});
        if (options.signal?.aborted) controller.abort();
        documentFetchControllers.add(controller);
        try { return await driveFetch(url, {...options, signal:controller.signal}); }
        finally {
            documentFetchControllers.delete(controller);
            options.signal?.removeEventListener('abort', abort);
        }
    }
    // The hidden processor stays loaded while its owner page reuses it: each
    // 'ihn-offline-run' message starts one more pass over the outbox instead
    // of loading the whole app again. Android's headless processor (no owner
    // page) runs once and reports to WorkManager.
    async function worker() {
        let prepared=null,profileChecked=false,running=null,again=false;
        const embedded=root.parent!==root;
        const prepare=async()=>{
            await initialize();
            const account=L.account();if(!account)throw new Error('No saved account');
            // Home owns authentication (including native/silent refresh).
            // Its hidden processor must not depend on a stale storage copy or
            // initialize a second OAuth client while the visible session works.
            let owner=null;
            try {if(embedded&&root.parent.location.origin===location.origin)
                owner=root.parent.InhouseOfflineApp;}catch(_){}
            const access=owner?.backgroundAccess;
            const session={account,owner,access,lastToken:null};
            if(access){
                // Use the same owner for refreshes during long PDF jobs too.
                ensureDriveToken=async()=>{
                    const next=await access(account,driveAccessToken?null:session.lastToken);
                    session.lastToken=next.token;
                    driveAccessToken=next.token;driveTokenExpiry=next.expiry;
                    return next.token;
                };
            }
            return session;
        };
        const runOnce=async()=>{
            let result;
            try {
                if(!prepared)prepared=prepare().catch(error=>{prepared=null;throw error;});
                const session=await prepared,{account,owner,access}=session;
                if(access){
                    const credentials=await access(account);
                    driveAccessToken=credentials.token;driveTokenExpiry=credentials.expiry;
                    session.lastToken=credentials.token;
                }else{
                    loadDriveTokenFromStorage();
                    if(!driveAccessToken && !useAndroidPkceFlow())throw new Error('Sign in to renew background access');
                    await ensureDriveToken(false);
                }
                if(!profileChecked){
                    const profile=await (await driveFetch('https://www.googleapis.com/drive/v3/about?fields=user(emailAddress)')).json();
                    if(profile.user?.emailAddress?.toLowerCase()!==account)throw new Error('Background account changed');
                    profileChecked=true;
                }
                const editor=await S.read('leases',account,'editor');
                const guardedFetch=async(url,options)=>{
                    const active=await S.read('settings','__session__','active');
                    if(active?.enabled===false||(localStorage.getItem('drive-hint-email')||'').toLowerCase()!==account)throw new Error('Account signed out');
                    return documentFetch(url,options);
                };
                let ownerActive=null;try{ownerActive=owner?.activeId?.()||null;}catch(_){}
                result=await root.InhouseOfflineSync.run(account,{background:true,fetch:guardedFetch,build:workerBuild,merge:async(doc,meta,remoteBlob)=>{await restore(doc,false);return merge(doc,meta,remoteBlob);},
                    progress:progress=>root.parent?.postMessage({type:'ihn-offline-progress',progress},location.origin),
                    skip:id=>id===ownerActive?'owner':editor?.documentId===id&&editor.expires>Date.now()});
            } catch(error) {result={pending:1,errors:[{message:error.message}]};}
            finally {
                root.parent?.postMessage({type:'ihn-offline-complete',result,reusable:embedded},location.origin);
                try {root.InhouseBackground?.complete?.(JSON.stringify(result));}catch(_){}
            }
            return result;
        };
        const schedule=async()=>{
            if(running){again=true;return running;}
            running=(async()=>{
                let result;
                do {again=false;result=await runOnce();} while(again);
                return result;
            })().finally(()=>{running=null;});
            return running;
        };
        if(embedded)root.addEventListener('message',event=>{
            if(event.origin===location.origin&&event.source===root.parent&&event.data?.type==='ihn-offline-run')void schedule();
        });
        return schedule();
    }
    async function newDocument() {
        const id=`local-${crypto.randomUUID()}`;
        state.offlineLocalId=id;state.driveFileId=id;state.driveAutosave=true;
        state.driveHeadRevisionId=null;state.driveRevisionVersion=null;state.driveLastKnownModified=null;
        markDriveHydrated(id);driveDirty=true;
        await L.capture();saveDriveSession();L.kick();
        if(!L.offline())setTimeout(()=>queueDriveSave(true).catch(()=>{}),0);
        return id;
    }
    async function confirmed(fileId,blob,meta,version,session) {
        if(!L.account()||L.isWorker)return;
        const id=currentId();if((state.driveFileId!==fileId&&id!==fileId)||getDocumentSessionToken()!==session)return;
        const doc=await S.read('documents',L.account(),id);
        if(doc?.contentVersion===version&&doc.session===getOfflineDocumentSessionToken(session)){
            await S.acknowledge(L.account(),id,doc.generation,meta,blob);
            await L.reload().catch(()=>{});
        }
    }
    let syncRetryTimer=null;
    function retryBusySave(result,force,options,context) {
        if(!result?.busy||options?.priority)return;
        clearTimeout(syncRetryTimer);
        syncRetryTimer=setTimeout(()=>{
            syncRetryTimer=null;
            if(isDocumentPersistenceContextCurrent(context)&&driveDirty)
                void queueDriveSave(force,options).catch(()=>{});
        },1000);
    }
    async function queueSave(force,options,normal) {
        if(L.isWorker)return;
        if(!L.ready())return normal(force,options);
        const requestContext=captureDocumentPersistenceContext();
        await L.capture();
        if(!isDocumentPersistenceContextCurrent(requestContext))return {cancelled:true};
        if(L.offline()){refreshSaveIndicatorState();return;}
        const pending=await S.read('documents',L.account(),currentId());
        if(!isDocumentPersistenceContextCurrent(requestContext))return {cancelled:true};
        if(state.driveFileId?.startsWith('local-') || pending?.offlineEdited) {
            const id=currentId(),session=getDocumentSessionToken(),version=driveContentVersion;
            L.setSyncProgress({phase:'checking',id,errors:[]});
            let result;
            try {result=await root.InhouseOfflineSync.run(L.account(),{
                targetId:options?.priority?id:null,
                fetch:(url,options)=>{
                    if(!isDocumentPersistenceContextCurrent(requestContext))throw createAbortError('Sync document changed');
                    return documentFetch(url,options);
                },skip:other=>other!==id,
                progress:L.setSyncProgress,
                build:()=>buildPdfBlob({sessionToken:session,...DRIVE_UPLOAD_PDF_OPTIONS}),merge,
                saved:async(localId,meta,blob)=>{
                    if(getDocumentSessionToken()!==session)return;
                    state.driveFileId=meta.id;state.driveHeadRevisionId=meta.headRevisionId;state.driveRevisionVersion=normalizeDriveVersion(meta.version);
                    state.driveLastKnownModified=Date.parse(meta.modifiedTime)||Date.now();markDriveHydrated(meta.id);
                    markDriveContentUploaded(version);if(version===driveContentVersion){driveDirty=false;updateSyncBaseline();}
                    saveDriveSession();updateDocTitle();refreshSaveIndicatorState();
                }});
            } catch(error) {result={errors:[{message:error.message}]};}
            finally {await L.syncFinished(result||{});}
            if(result?.errors?.length)showStatus(L.syncErrorText(result.errors[0]),{preserveState:true});
            retryBusySave(result,force,options,requestContext);
            // A failed attempt now has a backoff deadline; let the library wait for it.
            if(!result?.busy)void L.kick({reason:'after-save'});
            return result;
        }
        let result;
        try {result=await S.withLease(L.account(),'sync',()=>isDocumentPersistenceContextCurrent(requestContext)?normal(force,options):{cancelled:true});}
        finally {if(!result?.busy)L.leaseReleased();}
        retryBusySave(result,force,options,requestContext);
        return result;
    }
    async function saveNow() {
        if(!L.ready()||L.offline())return queueDriveSave(true);
        const account=L.account(),id=currentId(),context=captureDocumentPersistenceContext(),token=crypto.randomUUID();
        if(!id)return queueDriveSave(true);
        const priority={documentId:id,token,expires:Date.now()+60000};
        await S.put('settings',account,'sync-priority',priority);
        const renew=setInterval(()=>{void S.patch('settings',account,'sync-priority',old=>old.token===token?{...old,expires:Date.now()+60000}:old).catch(()=>{});},30000);
        try {
            const deadline=Date.now()+120000;
            while(isDocumentPersistenceContextCurrent(context)&&L.account()===account&&!L.offline()){
                const result=await queueDriveSave(true,{priority:true});
                if(!result?.busy)return result;
                if(Date.now()>=deadline){showStatus('Another upload is still finishing. Your local copy is safe; retry shortly.',{preserveState:true});return result;}
                // Let the current network request finish, then take the lease
                // before any further background documents are started.
                await new Promise(resolve=>setTimeout(resolve,250));
            }
            return {cancelled:true};
        } finally {
            clearInterval(renew);
            await S.patch('settings',account,'sync-priority',old=>old.token===token?null:old);
            void L.kick();
        }
    }
    root.InhouseOfflineApp=Object.freeze({pauseForEditing,authorizeBackground,initialize,reconcileStartup,backgroundAccess,worker,restore,newDocument,confirmed,queueSave,saveNow,renderHome,renderFolder,currentId,activeId});
})(window);
