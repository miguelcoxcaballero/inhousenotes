import {test,expect} from '@playwright/test';
test('a real prepared PDF reaches a conditional media upload and shows uploading',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  beginDocumentSession();await __IHN_TEST_API__.resetLocalDocument(1,'Save progress');
  state.driveFileId='progress-doc';state.driveCanEdit=true;state.driveAutosave=true;driveAccessToken='test';driveTokenExpiry=Date.now()+600000;driveSignedOut=false;driveAuthNeedsReconnect=false;ensureDriveToken=async()=> 'test';
  driveUserProfile={name:'Test',email:'save@example.test'};state.driveHeadRevisionId='h1';state.driveRevisionVersion='1';markDriveHydrated('progress-doc');
  const s={id:'ink',tool:'pen',color:'#000000',width:2,points:[{x:10,y:10,p:.5},{x:100,y:100,p:.5}]};
  state.pages[0].strokes.push(s);enqueueStrokeOp(0,s);driveDirty=true;
  const events=[];let uploaded='',etag=1;
  driveFetch=async(url,options={})=>{
   driveAuthNeedsReconnect=false;
   if(options.method==='PATCH'&&!url.includes('/upload/'))etag++;
   if(url.includes('uploadType=media')){events.push(getSavePresentation().driveLabel);if(options.headers['If-Match']!==String(etag))throw Object.assign(new Error('Presence invalidated precondition'),{status:412});uploaded=await options.body.text();}
   return new Response(JSON.stringify({id:'progress-doc',etag:String(etag),headRevisionId:url.includes('uploadType')?'h2':'h1',version:url.includes('uploadType')?'2':'1',modifiedDate:'2026-10-27T10:00:00Z'}),{headers:{'Content-Type':'application/json'}});
  };
  startCollabPolling=()=>{};
  const saved=queueDriveSaveOnline(true);
  const done=await Promise.race([saved.then(()=>true),new Promise(r=>setTimeout(()=>r(false),4000))]);
  const outcome={done,events,pdf:uploaded.startsWith('%PDF'),phase:driveSavePhase,error:driveLastSaveError?.message,version:driveContentVersion,uploaded:driveUploadedContentVersion,locks:!!remotePageMergeToken};
  state.driveAutosave=false;clearDriveSyncTimers();return outcome;
 });
 console.log('SAVE PROGRESS',result);expect(result.done).toBe(true);expect(result.pdf).toBe(true);expect(result.events).toContain('Uploading');expect(result.uploaded).toBe(result.version);
});

for(const external of [false,true])test(`PDF body cancellation stays active after headers arrive (${external?'pen interruption':'stalled network'})`,async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async external=>{
  ensureDriveToken=async()=> 'token';let cancelled=false;
  window.fetch=async()=>new Response(new ReadableStream({start(c){c.enqueue(new TextEncoder().encode('%PDF-'));},cancel(){cancelled=true;}}),{headers:{'Content-Type':'application/pdf'}});
  const controller=new AbortController();
  const response=await driveFetch('https://www.googleapis.com/drive/v3/files/stalled?alt=media',{timeout:1000,bodyIdleTimeout:80,signal:controller.signal});
  if(external)setTimeout(()=>controller.abort(),20);
  const error=await Promise.race([response.blob().then(()=>null,e=>({name:e.name,message:e.message})),new Promise(resolve=>setTimeout(()=>resolve({message:'still blocked'}),500))]);
  return {error,cancelled};
 },external);
 expect(result.cancelled).toBe(true);expect(result.error.message).toContain(external?'aborted':'timed out');
 expect(result.error.name).toBe(external?'AbortError':'Error');
});

for(const realEdit of [false,true])test(`home outbox distinguishes ${realEdit?'a new PDF':'presence metadata'} during preparation`,async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async realEdit=>{
  const S=InhouseOfflineStore,account='progress@example.test';let version=1,uploads=0,builds=0;
  const base={id:'outbox-doc',version:'1',headRevisionId:'h1',parents:['root'],name:'Test.pdf',capabilities:{canEdit:true}};
  await S.checkpoint(account,'outbox-doc',{snapshot:{pages:[]},baseMeta:base,entry:base,contentVersion:1,session:'test'},true);
  const result=await InhouseOfflineSync.run(account,{
   build:async()=>{builds++;version++;return new Blob(['%PDF-test'],{type:'application/pdf'});},
   fetch:async(url,options={})=>{
    if(url.includes('uploadType=')){uploads++;return new Response(JSON.stringify({...base,version:'3',headRevisionId:'h3'}));}
    return new Response(JSON.stringify({...base,version:String(version),headRevisionId:realEdit&&version>1?'h2':'h1',...(url.includes('/v2/')?{etag:'e'+version}:{})}));
   }
  });
  return {saved:result.saved,uploads,builds,dirty:(await S.read('documents',account,'outbox-doc')).dirty};
 },realEdit);
 expect(result).toEqual({saved:realEdit?0:1,uploads:realEdit?0:1,builds:1,dirty:realEdit});
});

for(const realEdit of [false,true])test(`conditional upload retries only ${realEdit?'after merging a real edit':'metadata conflicts without rebuilding'}`,async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async realEdit=>{
  let attempts=0;driveFetch=async(url,options)=>{
   if(url.includes('/upload/')){attempts++;if(attempts===1)throw Object.assign(new Error('conflict'),{status:412});return new Response(JSON.stringify({id:'doc',headRevisionId:'h2'}));}
   return new Response(JSON.stringify({id:'doc',etag:'fresh',headRevisionId:realEdit?'edited':'same'}));
  };
  try{await updateDriveFile('doc',new Blob(['%PDF']),{writeRevision:{id:'doc',etag:'old',headRevisionId:'same'}});return {attempts,ok:true};}
  catch(error){return {attempts,ok:false,code:error.status};}
 },realEdit);
 expect(result).toEqual(realEdit?{attempts:1,ok:false,code:412}:{attempts:2,ok:true});
});
