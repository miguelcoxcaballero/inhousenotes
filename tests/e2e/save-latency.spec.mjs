import {test,expect} from '@playwright/test';
test('a released pen stroke reaches durable local storage promptly',async({page,context})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(async()=>{await __IHN_TEST_API__.resetLocalDocument(1,'Latency');await __IHN_TEST_API__.showEditorForTest();});
 const position=await page.evaluate(()=>{const rect=document.querySelector('.page-canvas').getBoundingClientRect();return {x:rect.left+100,y:rect.top+100};});
 const cdp=await context.newCDPSession(page);
 await cdp.send('Input.dispatchMouseEvent',{type:'mousePressed',...position,button:'left',buttons:1,clickCount:1,pointerType:'pen'});
 await cdp.send('Input.dispatchMouseEvent',{type:'mouseMoved',x:position.x+30,y:position.y+30,buttons:1,pointerType:'pen'});
 await cdp.send('Input.dispatchMouseEvent',{type:'mouseReleased',x:position.x+30,y:position.y+30,button:'left',buttons:0,clickCount:1,pointerType:'pen'});
 const result=await page.evaluate(async()=>{
  const start=performance.now(),stroke=state.pages[0].strokes.at(-1);
  while(strokeOpsQueue.length||strokeOpsFlushInProgress||pendingStrokeOps.get(0)?.length){
   // Pending operations remain as an overlay after journal commit; wait for
   // the committed journal's confirmation, then inspect IndexedDB itself.
   if(!strokeOpsQueue.length&&!strokeOpsFlushInProgress)break;
   await new Promise(resolve=>setTimeout(resolve,10));
   if(performance.now()-start>5000)throw Error('Journal did not commit');
  }
  const db=await openNotebookDb();
  const ops=await new Promise((resolve,reject)=>{const request=db.transaction(OPS_STORE,'readonly').objectStore(OPS_STORE).getAll();request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);});
  return {ms:performance.now()-start,stored:ops.some(op=>op.stroke?.id===stroke.id)};
 });
 console.log('LOCAL_SAVE_LATENCY',JSON.stringify(result));expect(result.stored).toBe(true);expect(result.ms).toBeLessThan(700);
});

test('authentication cannot leave a Drive check pending forever or send a late request',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  const original=ensureDriveToken,request=window.fetch;let release,calls=0;
  ensureDriveToken=()=>new Promise(resolve=>release=resolve);window.fetch=async()=>{calls++;return new Response('{}');};
  try{const error=await driveFetch('https://www.googleapis.com/drive/v3/files/test',{timeout:60}).catch(e=>e.message);
   release('token');await new Promise(resolve=>setTimeout(resolve,30));return {error,calls};
  }finally{ensureDriveToken=original;window.fetch=request;}
 });expect(result.calls).toBe(0);expect(result.error).toContain('timed out');
});

test('metadata-only updates do not reject an already verified PDF head',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(()=>{
  state.driveHeadRevisionId='content-8';state.driveRevisionVersion='24';state.driveLastKnownModified=Date.parse('2026-10-26T10:00:00Z');
  const accepted=applyDriveRemoteMeta({headRevisionId:'content-8',version:'21',modifiedTime:'2026-10-26T09:00:00Z'},{verifiedContent:true});
  const rejected=applyDriveRemoteMeta({headRevisionId:'content-7',version:'20',modifiedTime:'2026-10-26T08:00:00Z'},{verifiedContent:true});
  return {accepted,rejected,head:state.driveHeadRevisionId,version:state.driveRevisionVersion,time:state.driveLastKnownModified};
 });expect(result).toEqual({accepted:true,rejected:false,head:'content-8',version:'24',time:Date.parse('2026-10-26T10:00:00Z')});
});

test('offline captures reuse intact pages and invalidate new pen, edits and metadata',async({page,context})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());await context.setOffline(true);
 const result=await page.evaluate(async()=>{
  await InhouseOfflineLibrary.useAccount('capture-cache@example.test');
  await __IHN_TEST_API__.resetLocalDocument(4,'Cache');
  state.driveFileId='capture-cache';state.offlineLocalId='';state.driveCanEdit=true;
  state.pages.forEach((p,i)=>{p.strokes=[{id:'old-'+i,tool:'pen',color:'#000000',width:2,points:[{x:20,y:30},{x:40,y:50}]}];});
  const original=packInkPageForStorage;let packed=0;
  packInkPageForStorage=async(...args)=>{packed++;return original(...args);};
  async function capture(){packed=0;await InhouseOfflineLibrary.capture();return packed;}
  try{
   const cold=await capture();
   const stroke={id:'new-ink',tool:'pen',color:'#123456',width:3,points:[{x:30,y:20},{x:40,y:60}]};
   state.pages[1].strokes.push(stroke);enqueueStrokeOp(1,stroke);const added=await capture();
   const first=await InhouseOfflineStore.read('documents',InhouseOfflineLibrary.account(),'capture-cache');
   const decoded=await unpackInkPageFromStorage(first.snapshot.pages[1]);
   stroke.width=5;markPageDirty(1,'full');const edited=await capture();
   state.pages[2].templateKind='agenda';bumpDriveContentVersion();const metadata=await capture();
   const saved=await InhouseOfflineStore.read('documents',InhouseOfflineLibrary.account(),'capture-cache');
   const latest=await unpackInkPageFromStorage(saved.snapshot.pages[1]);
   return {cold,added,edited,metadata,ids:decoded.strokes.map(s=>s.id),width:latest.strokes.find(s=>s.id==='new-ink').width};
  }finally{packInkPageForStorage=original;clearDriveSyncTimers();}
 });expect(result).toEqual({cold:4,added:1,edited:1,metadata:1,ids:['old-1','new-ink'],width:5});
});

test('a pen appended after a PDF save cannot reuse stale page metadata or overlay',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  await __IHN_TEST_API__.seedDensePageForTest(2,10);await buildPdfBlob({status:false,optimizeForUpload:true});
  const old=pdfCompactStrokeCache.get(state.pages[0]);
  const stroke={id:'after-pdf-save',tool:'pen',color:'#ff0000',width:3,points:[{x:200,y:200},{x:220,y:220}]};
  state.pages[0].strokes.push(stroke);enqueueStrokeOp(0,stroke);
  const blob=await buildPdfBlob({status:false,optimizeForUpload:true});
  const pdf=await pdfjsLib.getDocument({data:await blob.arrayBuffer()}).promise;
  const keywords=(await pdf.getMetadata()).info.Keywords;await pdf.destroy();
  const pages=await decodeEmbeddedStrokesFromPdfKeywords(keywords);
  return {invalidated:pdfCompactStrokeCache.get(state.pages[0])!==old,present:pages[0].strokes.some(s=>s.id===stroke.id)};
 });expect(result).toEqual({invalidated:true,present:true});
});
