/* Drive ink transport adapter. PDF checkpoints remain the export/recovery path;
 * these causal edits travel independently of PDF compilation and revision checks. */
(function(root){
    'use strict';
    const L=root.InhouseOfflineLibrary,S=root.InhouseOfflineStore;
    let live=null,previewRaf=null;const incomingPreviews=new Map();
    let engine=null,session=null,account='',timer=null,applying=0,ackVersion=-1;
    let pages=new Set(),baseline=new Map(),metadataDirty=false,baselineReady=false,capturing=null;
    const announced=new Map();
    const stamp=item=>`${item?.syncStamp?.clock||0}:${item?.syncStamp?.actor||''}`;
    const usable=()=>documentSessionActivated&&!driveOpenInProgress&&L.account()&&state.driveFileId&&!state.driveFileId.startsWith('local-');
    const current=journal=>journal===engine&&journal.account===L.account()
        &&session===getDocumentSessionToken()&&journal.docId===(usable()?state.driveFileId:'');
    function context(){return {structure:getCollabStructureSnapshot(),fields:getCollabFieldSnapshot(),
        name:state.exportName,calendar:state.calendarPageConfig||null};}
    function shell(page){return buildTimelineSnapshotFromPages([{...page,strokes:[],images:[],legacyCoverStrokes:null}])[0];}
    function remember(page){
        baseline.set(page.pageId,new Map([...(page.strokes||[]),...(page.images||[])].map(item=>[item.id,stamp(item)])));
    }
    function stop(){live?.stop();live=null;if(previewRaf)cancelAnimationFrame(previewRaf);previewRaf=null;incomingPreviews.clear();capturing=null;clearTimeout(timer);timer=null;engine?.stop();engine=null;baseline.clear();pages.clear();baselineReady=false;metadataDirty=false;ackVersion=-1;}
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
                for(let op of ordered){
                    if(op?.v===1&&op.kind==='stroke'){
                        const local=state.pages.find(page=>page.pageId===op.pageId);
                        if(!local&&state.collabStructure?.tombstones?.[op.pageId])continue;
                        if(!local)throw new Error('Waiting for the journal page metadata');
                        if(!op.stroke?.id||!Array.isArray(op.stroke.points))throw new Error('Invalid ink stroke');
                        const patch=shell(local);patch.strokes=[op.stroke];
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
        return task.then(result=>{if(capturing===task){capturing=null;if(engine?.docId&&(pages.size||metadataDirty))schedule();}return result;},error=>{if(capturing===task)capturing=null;throw error;});
    }
    async function captureEdits(){
        const journal=engine||start();if(!journal||!journal.docId||!current(journal))return;
        const token=session;
        await waitForHotInteractionToSettle({sessionToken:token,windowMs:getDocumentSaveIdleWindow()});
        if(!current(journal)||applying)return;
        if(!baselineReady){
            const dirty=driveContentVersion!==driveUploadedContentVersion||driveDirty;
            for(let i=0;i<state.pages.length;i++){
                if(dirty)pages.add(i);else if(!state.pages[i].unloaded)remember(state.pages[i]);
            }
            baselineReady=true;
        }
        const changed=[...pages];pages.clear();const includeMetadata=metadataDirty;metadataDirty=false;
        const version=driveContentVersion;
        try{
            for(const index of changed){
                await yieldProcessingForUi();
                await waitForHotInteractionToSettle({sessionToken:token,windowMs:getDocumentSaveIdleWindow()});
                if(!current(journal))return;
                const page=await loadPageDataForExport(index);
                if(!current(journal))return;
                if(!page?.pageId)continue;
                const known=baseline.get(page.pageId)||new Map();
                const capturedStamps=new Map([...(page.strokes||[]),...(page.images||[])].map(item=>[item.id,stamp(item)]));
                const strokes=(page.strokes||[]).filter(item=>known.get(item.id)!==stamp(item));
                const images=(page.images||[]).filter(item=>known.get(item.id)!==stamp(item));
                // Bound each operation so receiving a long offline session does
                // not decode the whole document in a single task.
                const count=Math.max(1,Math.ceil(strokes.length/32),Math.ceil(images.length/4));
                for(let part=0;part<count;part++){
                    if(!current(journal))return;
                    const patch=shell(page);patch.strokes=strokes.slice(part*32,(part+1)*32);patch.images=images.slice(part*4,(part+1)*4);
                    await journal.enqueue({v:1,...context(),pages:[patch]},version);
                    await yieldProcessingForUi();
                }
                if(!current(journal))return;
                baseline.set(page.pageId,capturedStamps);
            }
            if(includeMetadata&&!changed.length)await journal.enqueue({v:1,...context(),pages:[]},version);
        }catch(error){if(current(journal)){for(const index of changed)pages.add(index);metadataDirty ||= includeMetadata;}throw error;}
    }
    function stroke(index,item){
        if(applying||!item?.id)return;
        const journal=start(),page=state.pages[index];
        if(!journal?.docId||!page?.pageId)return;
        // Serialize just the finished stroke. No page scan or PDF export runs
        // here, so another pen contact need not hold up network delivery.
        const frozen=cloneStroke(item),version=driveContentVersion;
        void journal.enqueue({v:1,kind:'stroke',pageId:page.pageId,stroke:frozen},version)
            .then(()=>{if(current(journal)&&baselineReady){const known=baseline.get(page.pageId);known?.set(item.id,stamp(frozen));}})
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
    function hint(fileId,id,preview=false){if(fileId!==state.driveFileId)return;const journal=start();if(preview)live?.hint(id);else journal?.hint(id);}
    function hints(fileId,props){for(const [key,id]of Object.entries(props||{}))if(key.startsWith('ihn_ink_')||key.startsWith('ihn_live_'))hint(fileId,id,key.startsWith('ihn_live_'));}
    function status(){return engine&&current(engine)?{...engine.status(),ackVersion,pendingCapture:!!pages.size||metadataDirty}:null;}
    root.InhouseDriveInk=Object.freeze({start,stop,page,stroke,preview,change,hint,hints,status,capture});
    // Home also drains journals committed before a process exit. It never
    // replaces the PDF's local snapshot with journal upload acknowledgements.
    setInterval(()=>{
        if(engine&&(!current(engine)||!L.account()))stop();
        if(!engine&&L.account())start();
    },2000);
    root.addEventListener('online',()=>{start()?.retry().catch(()=>{});});
})(window);
