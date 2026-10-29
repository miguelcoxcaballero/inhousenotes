import {test,expect} from '@playwright/test';
test.beforeEach(async({page})=>{await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());});
test('PDF commands wait before dispatch and before delivering a result; checkpoint queue also pauses',async({page})=>{
 const result=await page.evaluate(async()=>{
  const prototype=InhouseRuntimeCore.WorkerRpcClient.prototype,original=prototype.run;
  const calls=[];let release;
  prototype.run=function(type){calls.push(type);return type==='pdfCreate'?new Promise(resolve=>release=resolve):Promise.resolve('checkpoint');};
  const pause=()=>new Promise(r=>setTimeout(r,80));
  try{
   isPenWriting=true;let delivered=false;
   const pdf=InhouseDocumentProcessing.run('pdfCreate',{}).then(value=>{delivered=true;return value;});
   const checkpointWork=InhouseDocumentProcessing.run('packInkCheckpoint',{});
   await pause();const before=calls.slice();
   isPenWriting=false;lastUserInteractionAt=0;
   while(!release)await pause();
   isPenWriting=true;release({id:'ready'});
   await pause();const during=delivered;
   isPenWriting=false;lastUserInteractionAt=0;
   const after=await pdf;const checkpoint=await checkpointWork;
   return {before,checkpoint,during,after};
  }finally{isPenWriting=false;prototype.run=original;}
 });expect(result).toEqual({before:[],checkpoint:'checkpoint',during:false,after:{id:'ready'}});
});
test('upload pauses between chunks without cancelling or resending accepted bytes',async({page})=>{
 const result=await page.evaluate(async()=>{
  const original=driveFetch;let started;const first=new Promise(r=>started=r);const ranges=[];let finished=false;
  driveFetch=async(url,options)=>{
   if(options.method==='POST')return new Response('',{status:200,headers:{Location:'https://upload.test/session'}});
   ranges.push(options.headers['Content-Range']);
   if(ranges.length===1){isPenWriting=true;started();return new Response('',{status:308,headers:{Range:'bytes=0-1048575'}});}
   return new Response(JSON.stringify({id:'saved'}),{status:200});
  };
  try{
   const upload=uploadDriveBlobResumable({url:'https://upload.test/start',method:'POST',blob:new Blob([new Uint8Array(1048586)])}).then(r=>{finished=true;return r;});
   await first;await new Promise(r=>setTimeout(r,120));const during={chunks:ranges.length,finished};
   isPenWriting=false;lastUserInteractionAt=0;const saved=await upload;
   return {during,ranges,id:saved.id};
  }finally{isPenWriting=false;driveFetch=original;}
 });expect(result).toEqual({during:{chunks:1,finished:false},ranges:['bytes 0-1048575/1048586','bytes 1048576-1048585/1048586'],id:'saved'});
});
test('aborting work waiting for the pen never starts an upload',async({page})=>{
 const result=await page.evaluate(async()=>{
  const original=driveFetch;let requests=0;driveFetch=async()=>{requests++;throw Error('must not start');};
  const controller=new AbortController();isPenWriting=true;
  try{
   const work=uploadDriveBlobResumable({url:'https://upload.test/start',method:'POST',blob:new Blob(['data']),options:{signal:controller.signal}}).catch(e=>e.name);
   await new Promise(r=>setTimeout(r,60));controller.abort();return {error:await work,requests};
  }finally{isPenWriting=false;driveFetch=original;}
 });expect(result).toEqual({error:'AbortError',requests:0});
});
test('a real pen contact terminates active processing and aborts the active upload',async({page,context})=>{
 await page.addInitScript(()=>{
  const NativeWorker=Worker;window.blockProcessing=false;window.processingPosted=false;window.processingTerminated=0;
  window.Worker=class extends NativeWorker{
   constructor(url,...args){super(url,...args);this.processing=String(url).includes('document-processing-worker');}
   postMessage(message,...args){if(this.processing&&window.blockProcessing){window.processingPosted=true;return;}return super.postMessage(message,...args);}
   terminate(){if(this.processing)window.processingTerminated++;return super.terminate();}
  };
 });
 await page.reload();await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(async()=>{await __IHN_TEST_API__.resetLocalDocument(1,'Pause');await __IHN_TEST_API__.showEditorForTest();
  window.blockProcessing=true;window.processingTerminated=0;
  window.activeUpload=new AbortController();activeDriveSaveController=activeUpload;
  window.job=InhouseDocumentProcessing.run('pdfReady',{}).then(()=> 'unexpected',e=>e.name);
 });
 await page.waitForFunction(()=>processingPosted);
 const position=await page.evaluate(()=>{const r=document.querySelector('.page-canvas').getBoundingClientRect();return {x:r.left+100,y:r.top+100};});
 const cdp=await context.newCDPSession(page);
 await cdp.send('Input.dispatchMouseEvent',{type:'mousePressed',...position,button:'left',buttons:1,clickCount:1,pointerType:'pen'});
 expect(await page.evaluate(async()=>({aborted:activeUpload.signal.aborted,terminated:processingTerminated,error:await job})))
  .toEqual({aborted:true,terminated:1,error:'AbortError'});
 await cdp.send('Input.dispatchMouseEvent',{type:'mouseReleased',...position,button:'left',buttons:0,clickCount:1,pointerType:'pen'});
 await page.evaluate(()=>{window.blockProcessing=false;activeDriveSaveController=null;});
 await page.evaluate(()=>InhouseDocumentProcessing.run('pdfReady',{}));
});
test('an interrupted PDF worker never switches to an expensive main-thread export',async({page})=>{
 const error=await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(1,'Interrupted export');
  const processing=InhouseDocumentProcessing;
  window.InhouseDocumentProcessing={...processing,pdfLib:async()=>{throw new DOMException('Paused for editing','AbortError');}};
  try{await buildPdfBlob({status:false});return 'unexpected export';}
  catch(error){return error.name;}
  finally{window.InhouseDocumentProcessing=processing;}
 });
 expect(error).toBe('AbortError');
});
test('native background sync stops once per writing burst and is requested again once the app is hidden',async({page,context})=>{
 await context.setOffline(true);
 const result=await page.evaluate(async()=>{
  await InhouseOfflineLibrary.useAccount('native-pause@example.test');
  let stopped=0,scheduled=0;
  window.InhouseBackground={cancel:()=>stopped++,schedule:()=>scheduled++};
  isPenWriting=true;
  InhouseOfflineLibrary.pauseForEditing();InhouseOfflineLibrary.pauseForEditing();
  await InhouseOfflineLibrary.scheduleBackground();
  const during={stopped,scheduled};
  isPenWriting=false;lastUserInteractionAt=0;
  InhouseOfflineLibrary.resumeAfterEditing();
  await new Promise(resolve=>setTimeout(resolve,0));
  // The visible app keeps uploading its own outbox: no second (headless)
  // app instance is started while the user is in the app.
  const visible={stopped,scheduled,paused:InhouseOfflineLibrary.diagnostics().nativePaused};
  await InhouseOfflineStore.checkpoint('native-pause@example.test','queued',{blob:new Blob(['pdf']),entry:{id:'queued',name:'Queued.pdf',parents:['root']}});
  Object.defineProperty(document,'visibilityState',{configurable:true,get:()=>'hidden'});
  await InhouseOfflineLibrary.scheduleBackground();
  return {during,visible,hidden:{stopped,scheduled}};
 });
 expect(result).toEqual({during:{stopped:1,scheduled:0},visible:{stopped:1,scheduled:0,paused:false},hidden:{stopped:1,scheduled:1}});
});
test('the editor uses Rust and a shared alarm for concurrent document waits',async({page})=>{
 const result=await page.evaluate(async()=>{
  await InhouseRust.ready;
  isPenWriting=true;let released=0;
  const jobs=Array.from({length:64},()=>waitForHotInteractionToSettle({windowMs:1200,sessionToken:getDocumentSessionToken()}).then(ok=>{if(ok)released++;}));
  const before=documentWorkQueue.diagnostics.wakes;
  await new Promise(r=>setTimeout(r,1200));
  const during={released,wakes:documentWorkQueue.diagnostics.wakes-before};
  isPenWriting=false;lastUserInteractionAt=0;wakeDocumentBackgroundWork();
  await Promise.all(jobs);
  return {during,released,...documentWorkQueue.diagnostics};
 });
 expect(result.during.released).toBe(0);expect(result.during.wakes).toBeLessThanOrEqual(16);
 expect(result.released).toBe(64);expect(result.backend).toBe('rust');expect(result.maxBatch).toBeLessThanOrEqual(8);
 console.log('Document queue:',JSON.stringify(result));
});
