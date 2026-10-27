/* Drive ink transport adapter. PDF checkpoints remain the export/recovery path;
 * these causal edits travel independently of PDF compilation and revision checks. */
(function(root){
    'use strict';
    const L=root.InhouseOfflineLibrary,S=root.InhouseOfflineStore;
    let live=null,previewRaf=null;const incomingPreviews=new Map();
    let engine=null,session=null,account='',timer=null,applying=0,ackVersion=-1;
    let pages=new Set(),baseline=new Map(),metadataBaseline=new Map(),fieldBaseline=new Map(),contextBaseline=null,metadataDirty=false,baselineReady=false,capturing=null;
    const announced=new Map(),previewHeartbeats=new Map();
    const stamp=item=>`${item?.syncStamp?.clock||0}:${item?.syncStamp?.actor||''}`;
    const usable=()=>documentSessionActivated&&!driveOpenInProgress&&L.account()&&state.driveFileId&&!state.driveFileId.startsWith('local-');
    const current=journal=>journal===engine&&journal.account===L.account()
        &&session===getDocumentSessionToken()&&journal.docId===(usable()?state.driveFileId:'');
    function context(){return {structure:getCollabStructureSnapshot(),fields:getCollabFieldSnapshot(),
        name:state.exportName,calendar:state.calendarPageConfig||null};}
    function strokeOperation(pageId,item){
        return {v:1,kind:'stroke',pageId,stroke:{id:item.id,tool:item.tool,color:item.color,width:item.width,points:item.points,
            ...(item.syncStamp?{syncStamp:item.syncStamp}:{})}};
    }
    function shell(page){return buildTimelineSnapshotFromPages([{...page,strokes:[],images:[],legacyCoverStrokes:null}])[0];}
    function metadataSnapshot(patch){
        const {backgroundImage,...metadata}=patch;
        // The immutable photo string is compared separately, never re-hashed
        // or JSON-encoded merely to detect an unchanged page shell.
        return {backgroundImage,key:JSON.stringify(metadata)};
    }
    function sameMetadata(left,right){return !!left&&left.backgroundImage===right.backgroundImage&&left.key===right.key;}
    const pageFields=['background','template','size','sidePanel'];
    function pageFieldsKey(pageId,metadata){
        return JSON.stringify(pageFields.map(field=>metadata.fields?.[`page:${pageId}:${field}`]||null));
    }
    function contextForPage(value,pageId=null){
        const allowed=new Set(pageId?pageFields.map(field=>`page:${pageId}:${field}`):[]);
        const fields=Object.fromEntries(Object.entries(value.fields.fields||{}).filter(([key])=>!key.startsWith('page:')||allowed.has(key)));
        // A winning photo register must arrive with that page's bytes. Sending
        // another page's field early would make its later payload look stale.
        return {...value,fields:{...value.fields,fields}};
    }
    function contextKey(value){
        const documentContext=contextForPage(value);
        // Page field records/stamps have their own baselines. Item stamps carry
        // summary clocks; those alone are not a reorder or document field edit.
        return JSON.stringify({...documentContext,structure:{...value.structure,clock:0},fields:{...documentContext.fields,clock:0}});
    }
    function remember(page){
        baseline.set(page.pageId,new Map([...(page.strokes||[]),...(page.images||[])].map(item=>[item.id,stamp(item)])));
        metadataBaseline.set(page.pageId,metadataSnapshot(shell(page)));
    }
    function stop(){live?.stop();live=null;if(previewRaf)cancelAnimationFrame(previewRaf);previewRaf=null;incomingPreviews.clear();previewHeartbeats.clear();capturing=null;clearTimeout(timer);timer=null;engine?.stop();engine=null;baseline.clear();metadataBaseline.clear();fieldBaseline.clear();contextBaseline=null;pages.clear();baselineReady=false;metadataDirty=false;ackVersion=-1;}
    function start(){
        if(!L.account())return null;
        if(engine&&current(engine))return engine;
        stop();account=L.account();session=getDocumentSessionToken();
        const token=session,docId=usable()?state.driveFileId:'';
        const host={store:S,account,docId,
            fetch:async(url,options)=>{
                if(journal.account!==L.account())throw createAbortError('Account changed');
                if(navigator.onLine===false)throw createAbortError('Offline');
                return driveFetch(url,options);
            },
            idle:()=>waitForHotInteractionToSettle({sessionToken:token,windowMs:getDocumentSaveIdleWindow()}),
            apply:async(operations)=>{
                if(!current(journal))throw createAbortError('Document changed');
                // Materialize new page metadata before its small stroke packets.
                const ordered=[...operations].sort((a,b)=>(a?.kind==='stroke'?1:0)-(b?.kind==='stroke'?1:0));
                for(let operationIndex=0;operationIndex<ordered.length;operationIndex++){
                    let op=ordered[operationIndex];
                    if(op?.v===1&&op.kind==='stroke'){
                        const local=state.pages.find(page=>page.pageId===op.pageId);
                        if(!local&&state.collabStructure?.tombstones?.[op.pageId])continue;
                        if(!local)throw new Error('Waiting for the journal page metadata');
                        if(!op.stroke?.id||!Array.isArray(op.stroke.points))throw new Error('Invalid ink stroke');
                        const patch=shell(local);patch.strokes=[op.stroke];
                        const ids=new Set([op.stroke.id]);let pointCount=op.stroke.points.length;
                        // Keep the same causal merge path, but amortize its
                        // document metadata/checkpoint work across a bounded
                        // run of distinct strokes on this page (also bounded by
                        // point count). A repeated ID
                        // stays a separate operation so revisions retain order.
                        while(patch.strokes.length<32){
                            const next=ordered[operationIndex+1];
                            if(next?.v!==1||next.kind!=='stroke'||next.pageId!==op.pageId
                                ||!next.stroke?.id||!Array.isArray(next.stroke.points)||ids.has(next.stroke.id)
                                ||pointCount+next.stroke.points.length>8192)break;
                            patch.strokes.push(next.stroke);ids.add(next.stroke.id);pointCount+=next.stroke.points.length;operationIndex++;
                        }
                        op={v:1,...context(),pages:[patch]};
                    }
                    if(op?.v!==1||!Array.isArray(op.pages)||!op.structure||!op.fields)throw new Error('Invalid ink edit');
                    if(!current(journal))throw createAbortError('Document changed');
                    await waitForHotInteractionToSettle({sessionToken:token,windowMs:getDocumentSaveIdleWindow()});
                    if(!current(journal))throw createAbortError('Document changed');
                    applying++;
                    try{
                        const result=await applyRemotePages(op.pages,{sessionToken:token,fileId:docId,
                            additiveById:true,preserveLocalUnsynced:true,remoteStructure:op.structure,
                            remoteFields:op.fields,remoteExportName:op.name,remoteCalendarPageConfig:op.calendar});
                        clearRemoteLiveStrokePreviews('',Infinity,op.pages);
                        if(result.changed){
                            // A journal receipt is not a PDF receipt. Keep PDF dirty
                            // and invalidate exports before its next checkpoint.
                            for(let i=0;i<state.pages.length;i++)invalidatePageMutationCaches(i);
                            bumpDriveContentVersion();driveDirty=true;
                            scheduleDriveSync();L.scheduleCapture();
                        }
                    }finally{applying--;}
                }
            },
            announce:async(fileId,id,actor,preview=false)=>{
                root.publishDriveInkHint?.(fileId,id,preview);
                if(driveSaveInProgress)return;
                const device=getPresenceClientId().replace(/[^a-zA-Z0-9]/g,'').slice(-24);
                const key=`${preview?'ihn_live_':'ihn_ink_'}${device}`,announcementKey=`${fileId}:${preview}`;
                if(announced.get(announcementKey)===id)return;
                if(journal.account!==L.account())throw createAbortError('Account changed');
                const response=await driveFetch(`https://www.googleapis.com/drive/v3/files/${encodeURIComponent(fileId)}?fields=id`,{
                    method:'PATCH',timeout:8000,headers:{'Content-Type':'application/json'},body:JSON.stringify({properties:{[key]:id}})});
                if(!response.ok)throw new Error(`Ink discovery hint failed (${response.status})`);
                announced.set(announcementKey,id);
            },
            receipt:receipt=>{if(current(journal)&&receipt.docId===docId&&receipt.actor===journal.actor){live?.retireConfirmed(receipt.operations);ackVersion=Math.max(ackVersion,receipt.version);refreshSaveIndicatorState();}},
            error:error=>{if(current(journal))console.warn('Drive ink sync:',error.message);},
            changed:()=>{if(current(journal))refreshSaveIndicatorState();}
        };
        const journal=new root.InhouseDriveInkJournal.Journal(host);
        engine=journal;
        if(docId&&root.InhouseDriveLivePreview)live=new root.InhouseDriveLivePreview({...host,actor:journal.actor,
            canPublish:state.driveCanEdit!==false,preview:packet=>queuePreview(journal,packet)}).start();
        // Baseline establishment happens at idle, not on the pointerdown path.
        schedule();journal.start();return journal;
    }
    function schedule(){if(timer||applying)return;timer=setTimeout(()=>{timer=null;void capture().catch(error=>{console.warn('Ink checkpoint:',error.message);if(engine&&!timer)timer=setTimeout(()=>{timer=null;schedule();},1000);});},60);}
    function capture(){
        if(capturing)return capturing;
        const task=captureEdits();capturing=task;
        return task.then(result=>{if(capturing===task){capturing=null;if(engine?.docId&&(pages.size||metadataDirty))schedule();if(engine)refreshSaveIndicatorState();}return result;},error=>{if(capturing===task){capturing=null;if(engine)refreshSaveIndicatorState();}throw error;});
    }
    async function captureEdits(){
        const journal=engine||start();if(!journal||!journal.docId||!current(journal))return;
        const token=session;
        await waitForHotInteractionToSettle({sessionToken:token,windowMs:getDocumentSaveIdleWindow()});
        if(!current(journal)||applying)return;
        // A slow local transaction must finish before comparing item stamps;
        // the immediate stroke path installs its immutable baseline on commit.
        await journal.persistChain;
        if(!current(journal)||applying)return;
        const capturedDocument=context(),capturedDocumentKey=contextKey(capturedDocument);
        if(!baselineReady){
            const dirty=driveContentVersion!==driveUploadedContentVersion||driveDirty;
            for(let i=0;i<state.pages.length;i++){
                const page=state.pages[i];
                if(dirty)pages.add(i);
                else{
                    fieldBaseline.set(page.pageId,pageFieldsKey(page.pageId,capturedDocument.fields));
                    if(!page.unloaded)remember(page);
                }
            }
            if(!dirty)contextBaseline=capturedDocumentKey;
            baselineReady=true;
        }
        if(metadataDirty)for(let i=0;i<state.pages.length;i++){
            const pageId=state.pages[i].pageId;
            if(fieldBaseline.get(pageId)!==pageFieldsKey(pageId,capturedDocument.fields))pages.add(i);
        }
        const changed=[...pages];pages.clear();const includeMetadata=metadataDirty;metadataDirty=false;
        const version=driveContentVersion;
        let capturedContext=contextBaseline;
        const deferChangedCapture=()=>{
            if(!current(journal))return true;
            if(!applying&&driveContentVersion===version)return false;
            for(const index of changed)pages.add(index);
            metadataDirty=true;
            return true;
        };
        try{
            for(const index of changed){
                await yieldProcessingForUi();
                await waitForHotInteractionToSettle({sessionToken:token,windowMs:getDocumentSaveIdleWindow()});
                if(deferChangedCapture())return;
                const page=await loadPageDataForExport(index);
                if(deferChangedCapture())return;
                if(!page?.pageId)continue;
                const known=baseline.get(page.pageId)||new Map();
                const capturedStamps=new Map([...(page.strokes||[]),...(page.images||[])].map(item=>[item.id,stamp(item)]));
                const strokes=(page.strokes||[]).filter(item=>known.get(item.id)!==stamp(item));
                const images=(page.images||[]).filter(item=>known.get(item.id)!==stamp(item));
                const pageShell=shell(page),capturedMetadata=metadataSnapshot(pageShell);
                const capturedFields=pageFieldsKey(page.pageId,capturedDocument.fields);
                const unchangedPage=sameMetadata(metadataBaseline.get(page.pageId),capturedMetadata)
                    &&fieldBaseline.get(page.pageId)===capturedFields;
                if(!strokes.length&&!images.length&&unchangedPage)continue;
                // Editing existing ink does not need to retransmit its photo.
                // Only an established, otherwise unchanged page can use this
                // envelope; metadata/tombstones, images and recovery stay full.
                const strokeOnly=unchangedPage&&!images.length&&baseline.has(page.pageId)
                    &&capturedDocumentKey===contextBaseline&&strokes.every(item=>known.has(item.id));
                if(strokeOnly){
                    for(const item of strokes){
                        if(deferChangedCapture())return;
                        // enqueue freezes points and causal identity synchronously.
                        await journal.enqueue(strokeOperation(page.pageId,item),version);
                        await yieldProcessingForUi();
                    }
                }else{
                    // Bound each operation so receiving a long offline session
                    // does not decode the whole document in a single task.
                    const count=Math.max(1,Math.ceil(strokes.length/32),Math.ceil(images.length/4));
                    for(let part=0;part<count;part++){
                        if(deferChangedCapture())return;
                        const patch={...pageShell,strokes:strokes.slice(part*32,(part+1)*32),images:images.slice(part*4,(part+1)*4)};
                        const operationContext=contextForPage(capturedDocument,page.pageId);
                        await journal.enqueue({v:1,...operationContext,pages:[patch]},version);
                        capturedContext=capturedDocumentKey;
                        await yieldProcessingForUi();
                    }
                }
                if(deferChangedCapture())return;
                baseline.set(page.pageId,capturedStamps);
                metadataBaseline.set(page.pageId,capturedMetadata);
                fieldBaseline.set(page.pageId,capturedFields);
            }
            if(deferChangedCapture())return;
            if(includeMetadata&&capturedDocumentKey!==capturedContext){
                await journal.enqueue({v:1,...contextForPage(capturedDocument),pages:[]},version);
                capturedContext=capturedDocumentKey;
            }
            if(deferChangedCapture())return;
            contextBaseline=capturedContext;
            const currentIds=new Set(state.pages.map(page=>page.pageId));
            for(const map of [metadataBaseline,fieldBaseline,baseline])for(const id of map.keys())if(!currentIds.has(id))map.delete(id);
        }catch(error){if(current(journal)){for(const index of changed)pages.add(index);metadataDirty ||= includeMetadata;}throw error;}
    }
    function stroke(index,item){
        if(applying||!item?.id)return;
        const journal=start(),page=state.pages[index];
        if(!journal?.docId||!page?.pageId)return;
        // Serialize just the finished stroke. No page scan or PDF export runs
        // here, so another pen contact need not hold up network delivery.
        // enqueue freezes the operation synchronously as JSON. Capture the
        // identity/stamp now, without copying every point a second time.
        const itemId=item.id,capturedStamp=stamp(item),version=driveContentVersion;
        void journal.enqueue(strokeOperation(page.pageId,item),version)
            .then(()=>{if(current(journal)&&baselineReady){const known=baseline.get(page.pageId);known?.set(itemId,capturedStamp);}})
            .catch(error=>{if(current(journal)){pages.add(index);schedule();console.warn('Ink stroke checkpoint:',error.message);}});
    }
    function page(index){if(applying)return;if(!start()?.docId)return;pages.add(index);schedule();}
    function change(){if(applying)return;if(!start()?.docId)return;metadataDirty=true;schedule();}
    function preview(packet){if(packet?.fileId!==state.driveFileId)return;start();live?.push(packet);}
    function queuePreview(journal,packet){
        if(!current(journal)||packet.actorId===`${ihnGetLivePeerId()}:${ihnGetLiveTabId()}`)return;
        const key=`${packet.actorId}:${packet.strokeId}`,queued=incomingPreviews.get(key);
        if(queued&&queued.sequence>=packet.sequence)return;
        incomingPreviews.set(key,packet);
        if(previewRaf)return;
        previewRaf=requestAnimationFrame(()=>{
            previewRaf=null;
            const packets=[...incomingPreviews.values()];incomingPreviews.clear();
            if(!current(journal))return;
            for(const packet of packets){
                const page=state.pages.find(page=>page.pageId===packet.pageId);
                if(!page)continue;
                if(packet.cancel||page.strokes?.some(stroke=>stroke.id===packet.strokeId)
                    ||page.deletedStrokeIds?.includes(packet.strokeId)){
                    clearRemoteLiveStrokePreview(packet.actorId,packet.strokeId);continue;
                }
                const old=remoteLiveStrokePreviews.get(`${packet.actorId}:${packet.strokeId}`);
                if(old&&old.sequence>=packet.sequence)continue;
                const replacement=!!packet.finalBatch;
                if(replacement&&old)clearRemoteLiveStrokePreview(packet.actorId,packet.strokeId);
                const offset=replacement?0:Math.max(0,Math.min(packet.points.length,old?.points.length||0)-1);
                applyRemoteLiveStrokePreview({...packet,offset,points:packet.points.slice(offset),
                    finalBatch:replacement,final:false,totalPoints:replacement?packet.points.length:0});
            }
        });
    }
    function hint(fileId,id,preview=false,active=true){if(fileId!==state.driveFileId)return;const journal=start();if(preview)live?.hint(id,active);else journal?.hint(id);}
    function hints(fileId,props){
        if(fileId!==state.driveFileId)return;
        const journal=start();if(!journal?.docId||!current(journal))return;
        const now=Date.now(),maxAge=PRESENCE_HEARTBEAT_INTERVAL*PRESENCE_HEARTBEAT_MISS_LIMIT+1200;
        const freshDevices=new Map();
        // Presence is refreshed from this same metadata response immediately
        // before hints(). Only a new, recent heartbeat proves activity; an old
        // property, identity timestamp or sticky online badge does not.
        for(const user of latestPresenceUsers||[]){
            const heartbeat=Number(user?.h),device=String(user?.c||'').replace(/[^a-zA-Z0-9]/g,'').slice(-24);
            if(!device||!Number.isFinite(heartbeat)||heartbeat<=0||heartbeat>now+30000||now-heartbeat>maxAge)continue;
            freshDevices.set(device,Math.max(freshDevices.get(device)||0,heartbeat));
        }
        for(const [id,previous]of previewHeartbeats)if(live?.previewSources.get(id)!==previous.source)previewHeartbeats.delete(id);
        for(const [key,id]of Object.entries(props||{})){
            if(key.startsWith('ihn_ink_')){journal.hint(id);continue;}
            if(!key.startsWith('ihn_live_'))continue;
            live?.hint(id,false);
            const source=live?.previewSources.get(id),device=key.slice('ihn_live_'.length),heartbeat=freshDevices.get(device);
            if(!source||!heartbeat||id===live.fileId)continue;
            const previous=previewHeartbeats.get(id);
            if(previous?.source===source&&previous.device===device&&heartbeat<=previous.heartbeat)continue;
            previewHeartbeats.set(id,{source,device,heartbeat});
            live.hint(id,true);
        }
    }
    function status(){return engine&&current(engine)?{...engine.status(),ackVersion,pendingCapture:!!capturing||!!pages.size||metadataDirty}:null;}
    root.InhouseDriveInk=Object.freeze({start,stop,page,stroke,preview,change,hint,hints,status,capture});
    // Home also drains journals committed before a process exit. It never
    // replaces the PDF's local snapshot with journal upload acknowledgements.
    setInterval(()=>{
        if(engine&&(!current(engine)||!L.account()))stop();
        if(!engine&&L.account())start();
    },2000);
    root.addEventListener('online',()=>{start()?.retry().catch(()=>{});});
})(window);
