/* Integration with the existing editor and its tested collaboration merge. */
(function (root) {
    'use strict';
    const S=root.InhouseOfflineStore, L=root.InhouseOfflineLibrary;
    function currentId() {return state.offlineLocalId || state.driveFileId || '';}
    function metadata() {return {id:state.driveFileId,headRevisionId:state.driveHeadRevisionId,version:state.driveRevisionVersion,
        modifiedTime:state.driveLastKnownModified?new Date(state.driveLastKnownModified).toISOString():null};}
    async function capture() {
        const id=currentId();
        if(!id || !state.pages.length || driveOpenInProgress || state.driveCanEdit===false)return null;
        // Boot restores a single-editor cache, not necessarily the newest
        // per-document draft. Home must upload the durable outbox as-is until
        // this document has been explicitly opened/restored in this launch.
        const existing=L.record(id);
        if(!documentSessionActivated && existing?.available && existing.session!==getOfflineDocumentSessionToken())return null;
        const token=getDocumentSessionToken(),version=driveContentVersion;
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
            const page=await loadPageDataForExport(i);assertCurrent();
            const copy=buildTimelineSnapshotFromPages([{...page,strokes:[],images:[]}])[0];
            let sliceStarted=performance.now();
            for(const kind of ['strokes','images'])for(const item of page[kind]||[]){
                copy[kind].push(cloneTimelineValue(item));
                if(performance.now()-sliceStarted>=3){await yieldCapture();sliceStarted=performance.now();}
            }
            pages.push(copy);
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
            state.activePdfBytes=bytes;state.activePdfBytesFileId=state.driveFileId;
            if(doc.snapshot?.pages.some(p=>p.pdfPageIndex)) {
                if(activePdfDocument)await activePdfDocument.destroy().catch(()=>{});
                activePdfDocument=await loadSecurePdfDocument({data:bytes.slice(0),cMapUrl:PDFJS_CMAP_URL,cMapPacked:true,standardFontDataUrl:PDFJS_STANDARD_FONT_URL});
                activePdfPageCount=activePdfDocument.numPages;
            }
        }
        if(doc.snapshot) {
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
        const doc=await S.read('documents',L.account(),currentId());
        const marker=readPendingExitUpload();
        if(doc&&!doc.dirty&&doc.lastUploadedAt>(state.lastSavedAt||0)
            &&(!marker||marker.fileId===state.driveFileId&&marker.contentVersion<=doc.contentVersion))await restore(doc,false);
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
    }
    async function initialize() {
        await L.init({version:APP_VERSION,currentId,quickAccess:isQuickAccessDriveFile,activeDocument:()=>document.visibilityState==='visible'&&editorView&&!editorView.classList.contains('hidden'),busy:()=>hasSmoothInteraction()||driveOpenInProgress||offlineOpenInProgress,
            identity:()=>({id:currentId(),version:driveContentVersion,session:getOfflineDocumentSessionToken()}),
            retrySync:()=>driveAuthNeedsReconnect||L.syncStatus().errors.some(error=>error.code===401||/sign.?in|refresh token/i.test(error.message||''))?signInDrive():void L.kick(),syncChanged:renderSyncStatus,syncActive:()=>queueDriveSave(true),
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
        const heartbeat=()=>{
            const account=L.account();if(!account)return;
            return S.put('leases',account,'editor',{documentId:currentId(),expires:document.visibilityState==='visible'&&editorView&&!editorView.classList.contains('hidden')?Date.now()+20000:0});
        };
        setInterval(()=>{heartbeat();if(!L.offline())L.kick();},10000);
        document.addEventListener('visibilitychange',()=>{heartbeat();if(document.hidden)L.capture().then(()=>L.scheduleBackground()).catch(()=>{});});
        window.addEventListener('online',()=>{if(state.driveAutosave&&driveDirty)queueDriveSave(true);});
        await heartbeat();
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
    async function worker() {
        let result;
        try {
            await initialize();
            const account=L.account();if(!account)throw new Error('No saved account');
            // Home owns authentication (including native/silent refresh).
            // Its hidden processor must not depend on a stale storage copy or
            // initialize a second OAuth client while the visible session works.
            let owner=null;
            try {if(root.parent!==root&&root.parent.location.origin===location.origin)
                owner=root.parent.InhouseOfflineApp;}catch(_){}
            const access=owner?.backgroundAccess;
            if(access){
                const credentials=await access(account);
                driveAccessToken=credentials.token;driveTokenExpiry=credentials.expiry;
                // Use the same owner for refreshes during long PDF jobs too.
                let lastToken=credentials.token;
                ensureDriveToken=async()=>{
                    const next=await access(account,driveAccessToken?null:lastToken);
                    lastToken=next.token;
                    driveAccessToken=next.token;driveTokenExpiry=next.expiry;
                    return next.token;
                };
            }else{
                loadDriveTokenFromStorage();
                if(!driveAccessToken && !useAndroidPkceFlow())throw new Error('Sign in to renew background access');
                await ensureDriveToken(false);
            }
            const profile=await (await driveFetch('https://www.googleapis.com/drive/v3/about?fields=user(emailAddress)')).json();
            if(profile.user?.emailAddress?.toLowerCase()!==account)throw new Error('Background account changed');
            const editor=await S.read('leases',account,'editor');
            const guardedFetch=async(url,options)=>{
                const active=await S.read('settings','__session__','active');
                if(active?.enabled===false||(localStorage.getItem('drive-hint-email')||'').toLowerCase()!==account)throw new Error('Account signed out');
                return driveFetch(url,options);
            };
            result=await root.InhouseOfflineSync.run(account,{background:true,fetch:guardedFetch,build:workerBuild,merge:async(doc,meta,remoteBlob)=>{await restore(doc,false);return merge(doc,meta,remoteBlob);},
                progress:progress=>root.parent?.postMessage({type:'ihn-offline-progress',progress},location.origin),
                skip:id=>editor?.documentId===id&&editor.expires>Date.now()});
        } catch(error) {result={pending:1,errors:[{message:error.message}]};}
        finally {
            root.parent?.postMessage({type:'ihn-offline-complete',result},location.origin);
            try {root.InhouseBackground?.complete?.(JSON.stringify(result));}catch(_){}
        }
        return result;
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
                    return driveFetch(url,options);
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
            return result;
        }
        const result=await S.withLease(L.account(),'sync',()=>isDocumentPersistenceContextCurrent(requestContext)?normal(force,options):{cancelled:true});
        retryBusySave(result,force,options,requestContext);
        return result;
    }
    async function saveNow() {
        if(!L.ready()||L.offline())return queueDriveSave(true);
        const account=L.account(),id=currentId(),context=captureDocumentPersistenceContext(),token=crypto.randomUUID();
        if(!id)return queueDriveSave(true);
        const priority={documentId:id,token,expires:Date.now()+60000};
        await S.put('settings',account,'sync-priority',priority);
        const renew=setInterval(()=>{void S.patch('settings',account,'sync-priority',old=>old.token===token?{...old,expires:Date.now()+60000}:old).catch(()=>{});},10000);
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
    root.InhouseOfflineApp=Object.freeze({authorizeBackground,initialize,reconcileStartup,backgroundAccess,worker,restore,newDocument,confirmed,queueSave,saveNow,renderHome,renderFolder,currentId});
})(window);
