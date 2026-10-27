import {test,expect} from '@playwright/test';
test.beforeEach(async({page})=>{await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());});

test('small PDF uses one conditional request and waits for the real receipt',async({page})=>{
 const result=await page.evaluate(async()=>{
  const old=driveFetch,calls=[];let release;
  driveFetch=async(url,options)=>{calls.push({url,method:options.method,etag:options.headers['If-Match'],bytes:await options.body.text()});
   await new Promise(resolve=>release=resolve);return new Response(JSON.stringify({id:'file',modifiedDate:'2026-10-26T12:00:00Z',version:'8'}));};
  try{let done=false;const pending=updateDriveFile('file',new Blob(['exact pdf']),{writeRevision:{id:'file',etag:'"v7"'}}).then(value=>{done=true;return value;});
   while(!release)await new Promise(resolve=>setTimeout(resolve,5));const before=done;release();const receipt=await pending;
   return {calls,before,receipt};
  }finally{driveFetch=old;}
 });
 expect(result.before).toBe(false);expect(result.calls).toHaveLength(1);
 expect(result.calls[0]).toMatchObject({method:'PUT',etag:'"v7"',bytes:'exact pdf'});
 expect(result.calls[0].url).toContain('/upload/drive/v2/files/file?uploadType=media');
 expect(result.receipt.modifiedTime).toBe('2026-10-26T12:00:00Z');
});

test('missing revisions, incomplete receipts and conflicts never acknowledge a PDF',async({page})=>{
 const result=await page.evaluate(async()=>{
  const old=driveFetch;let calls=0,mode=0;const errors=[];
  driveFetch=async()=>{calls++;if(mode===2){const err=new Error('precondition failed');err.status=412;throw err;}
   return new Response(JSON.stringify({id:mode===1?'other':'file'}),{status:mode===0?202:200});};
  const blob=new Blob(['pdf']),options={writeRevision:{id:'file',etag:'v1'}};
  try{
   await updateDriveFile('file',blob).catch(e=>errors.push(e.message));const invalidCalls=calls;
   for(mode=0;mode<3;mode++)await updateDriveFile('file',blob,options).catch(e=>errors.push(e.status||e.message));
   return {invalidCalls,errors,calls};
  }finally{driveFetch=old;}
 });
 expect(result.invalidCalls).toBe(0);expect(result.calls).toBe(3);expect(result.errors).toHaveLength(4);expect(result.errors[3]).toBe(412);
});

test('resumable uploads bind every request to the merged revision and discard conflicted sessions',async({page})=>{
 const result=await page.evaluate(async()=>{
  const old=driveFetch,calls=[];let starts=0,conflict=true;
  driveFetch=async(url,options)=>{
   calls.push(options.headers['If-Match']);
   if(url==='https://upload.test/start'){starts++;return new Response('',{headers:{Location:'https://upload.test/session'}});}
   if(conflict){const err=new Error('changed remotely');err.code=412;throw err;}
   return new Response(JSON.stringify({id:'file'}));
  };
  try{const blob=new Blob(['pdf']);const args={url:'https://upload.test/start',method:'PUT',blob,options:{conditionalETag:'v1'}};
   const failure=await uploadDriveBlobResumable(args).catch(e=>e.code);const sessionsAfterConflict=driveResumableSessions.size;
   conflict=false;await uploadDriveBlobResumable({...args,options:{conditionalETag:'v2'}});
   return {failure,sessionsAfterConflict,starts,calls};
  }finally{driveFetch=old;driveResumableSessions.clear();}
 });
 expect(result).toEqual({failure:412,sessionsAfterConflict:0,starts:2,calls:['v1','v1','v2','v2']});
});

test('save conflict keeps edits pending for a new merge, never reports Saved',async({page})=>{
 const result=await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(1,'Concurrent');
  state.driveAutosave=true;state.driveFileId='conflict';state.driveCanEdit=true;driveSignedOut=false;driveDirty=true;
  resetDriveContentSyncVersion();markDriveHydrated('conflict');
  prepareDrivePdfBlob=async()=>({blob:new Blob(['pdf']),version:driveContentVersion,contentHash:'draft'});
  ensureDriveFile=async()=>({fileId:'conflict',created:false});
  mergeLatestRemoteBeforeDriveUpload=async()=>({changed:false,writeRevision:{id:'conflict',etag:'v1'}});
  updateDriveFile=async()=>{const err=new Error('remote changed');err.status=412;throw err;};
  await queueDriveSaveOnline(true);
  const value={pending:hasPendingDriveSave(),mergePending:deferredRemotePullPending,label:getSavePresentation().driveLabel};
  state.driveAutosave=false;clearDriveSyncTimers();clearTimeout(deferredDriveSaveRetryTimer);return value;
 });
 expect(result.pending).toBe(true);expect(result.mergePending).toBe(true);expect(result.label).not.toBe('Saved');
});

test('Rust vector ink keeps positions, color, every sample and removable PDF overlays',async({page})=>{
 const result=await page.evaluate(async()=>{
  const processor=InhouseDocumentProcessing,lib=await processor.pdfLib(PDFLib);
  const strokes=[{tool:'pen',color:'#142dd2',width:4,points:[{x:30,y:40},{x:50,y:40},{x:50,y:70}]}];
  const args={json:JSON.stringify({strokes,images:[]}),bounds:{x:20,y:30,width:100,height:100},scale:1,pdfRaster:true,vectorInk:true};
  const overlay=await processor.run('renderOverlay',args);
  const source=await new Response(new Blob([overlay.contentBytes]).stream().pipeThrough(new DecompressionStream('deflate'))).text();
  const doc=await lib.PDFDocument.create(),sheet=await doc.addPage([200,200]);
  const ink=await doc.embedPng(overlay);await sheet.drawImage(ink,{x:20,y:70,width:100,height:100});
  const name=ink.workerDrawName;const bytes=await doc.save();await doc.dispose();
  async function sample(data){const pdf=await pdfjsLib.getDocument({data}).promise,pg=await pdf.getPage(1),canvas=new OffscreenCanvas(200,200),ctx=canvas.getContext('2d');
   await pg.render({canvasContext:ctx,viewport:pg.getViewport({scale:1})}).promise;
   const value=[...ctx.getImageData(40,40,1,1).data];await pdf.destroy();return value;}
  const before=await sample(bytes.slice());
  const cleaned=await stripOverlaysFromPdfBytes(bytes,new Map([[0,{name}]]));
  const after=cleaned?await sample(cleaned):null;
  const erased=await processor.run('renderOverlay',{...args,json:JSON.stringify({strokes:[...strokes,{tool:'eraser-area',width:8,points:[{x:40,y:30},{x:40,y:60}]}]})});
  return {vector:overlay.pdfVector,source,before,after,eraserRaster:!erased.pdfVector,name};
 });
 expect(result.vector).toBe(true);expect(result.source).toContain('30.000000 40.000000 m\n50.000000 40.000000 l\n50.000000 70.000000 l');
 expect(result.before).toEqual([20,45,210,255]);expect(result.after).toEqual([255,255,255,255]);expect(result.eraserRaster).toBe(true);
});

test('sparse pen export benchmark compares Rust vectors with the raster path',async({page})=>{
 const result=await page.evaluate(async()=>{
  await __IHN_TEST_API__.seedDensePageForTest(30,60);
  state.pages[0].strokes.forEach((stroke,index)=>stroke.points.forEach((point,p)=>{
   point.x=30+(index%5)*120+p*.4;
   point.y=30+Math.floor(index/5)*130+Math.sin(p*.3)*3;
  }));
  markPageDirty(0,'full');
  // Warm the worker, template and metadata caches before comparing paths.
  await buildPdfBlob({status:false,optimizeForUpload:true,forceRasterInk:true});
  const timings={raster:[],vector:[]};
  for(const forceRasterInk of [false,true,false,true]){
   const start=performance.now();const blob=await buildPdfBlob({status:false,optimizeForUpload:true,forceRasterInk});
   timings[forceRasterInk?'raster':'vector'].push({ms:performance.now()-start,bytes:blob.size});
  }
  return timings;
 });
 console.log('SPARSE_SAVE_BENCHMARK',JSON.stringify(result));
 expect(result.vector.every(sample=>sample.bytes>0)).toBe(true);
});


test('a remote revision must finish merging before its write token is returned',async({page})=>{
 const result=await page.evaluate(async()=>{
  state.driveFileId='file';let newer=true,pulls=0;
  waitForCollabPullIdle=async()=>true;
  driveFetch=async()=>new Response(JSON.stringify({id:'file',etag:'v2',version:'2',modifiedDate:'2026-10-26T12:00:00Z'}));
  isRemoteMetaNewerThanTracked=()=>newer;
  pullRemoteChanges=async()=>{pulls++;newer=false;return {changed:false,hasLocalMerges:false};};
  const first=await mergeLatestRemoteBeforeDriveUpload('file',new AbortController());
  const second=await mergeLatestRemoteBeforeDriveUpload('file',new AbortController());
  return {pulls,first,second};
 });
 expect(result.pulls).toBe(1);expect(result.first.changed).toBe(true);expect(result.first.writeRevision).toBeUndefined();
 expect(result.second.changed).toBe(false);expect(result.second.writeRevision.etag).toBe('v2');
});

test('small upload obeys pen pause and aborts without sending a request',async({page})=>{
 const result=await page.evaluate(async()=>{
  const old=driveFetch;let calls=0;driveFetch=async()=>{calls++;throw Error('unexpected request');};
  const controller=new AbortController();isPenWriting=true;
  try{const pending=updateDriveFile('file',new Blob(['pdf']),{writeRevision:{id:'file',etag:'v1'},signal:controller.signal}).catch(e=>e.name);
   await new Promise(resolve=>setTimeout(resolve,60));controller.abort();return {error:await pending,calls};
  }finally{isPenWriting=false;driveFetch=old;}
 });
 expect(result).toEqual({error:'AbortError',calls:0});
});

test('a revision conflict during timeout recovery discards the resumable session too',async({page})=>{
 const result=await page.evaluate(async()=>{
  const old=driveFetch;let requests=0;
  driveFetch=async(url,options)=>{
   requests++;
   if(options.method==='POST')return new Response('',{headers:{Location:'https://upload.test/session'}});
   const error=new Error('request failed');error.status=options.headers['Content-Range'].startsWith('bytes */')?412:503;throw error;
  };
  try{const error=await uploadDriveBlobResumable({url:'https://upload.test/start',method:'POST',blob:new Blob(['pdf']),options:{conditionalETag:'v1'}}).catch(e=>e.status);
   return {error,requests,sessions:driveResumableSessions.size};
  }finally{driveFetch=old;driveResumableSessions.clear();}
 });
 expect(result).toEqual({error:412,requests:3,sessions:0});
});
