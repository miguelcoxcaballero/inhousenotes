import {test,expect} from '@playwright/test';
test.beforeEach(async({page})=>{
 // These cases simulate online upload failures. Keep the connectivity probe
 // deterministic so a host-network outage does not select the offline UI.
 await page.route('https://www.googleapis.com/drive/v3/about?fields=kind',route=>route.fulfill({status:401,json:{error:'Authentication required'}}));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await expect.poll(()=>page.evaluate(()=>InhouseOfflineLibrary.offline())).toBe(false);
});
test('equal-size PDFs with the same logical hash never reuse each other’s upload session',async({page})=>{
 const result=await page.evaluate(async()=>{
  const old=driveFetch;let sessions=0,fail=true;const sent=[];
  driveFetch=async(url,options)=>{
   if(options.method==='POST')return new Response('',{status:200,headers:{Location:`https://upload.test/${++sessions}`}});
   if(fail){const e=new Error('connection interrupted');e.name='AbortError';throw e;}
   sent.push({url,bytes:await options.body.text()});return new Response(JSON.stringify({id:'file'}),{status:200});
  };
  try{
   const a=new Blob(['AAAA']),b=new Blob(['BBBB']);a.inhouseContentHash=b.inhouseContentHash='same-logical-hash';
   await uploadDriveBlobResumable({url:'https://upload.test/start',method:'POST',blob:a}).catch(()=>{});
   fail=false;await uploadDriveBlobResumable({url:'https://upload.test/start',method:'POST',blob:b});
   return {sessions,sent};
  }finally{driveFetch=old;driveResumableSessions.clear();}
 });
 expect(result).toEqual({sessions:2,sent:[{url:'https://upload.test/2',bytes:'BBBB'}]});
});
test('an expired 410 upload session restarts for the same blob',async({page})=>{
 const result=await page.evaluate(async()=>{
  const old=driveFetch;let sessions=0,fail=true;
  driveFetch=async(url,options)=>{
   if(options.method==='POST')return new Response('',{status:200,headers:{Location:`https://upload.test/${++sessions}`}});
   if(fail){const e=new Error('interrupted');e.name='AbortError';throw e;}
   if(url.endsWith('/1'))throw new Error('Drive API error 410: Gone');
   return new Response(JSON.stringify({id:'file'}),{status:200});
  };
  try{const blob=new Blob(['test']);await uploadDriveBlobResumable({url:'https://upload.test/start',method:'POST',blob}).catch(()=>{});fail=false;
   const response=await uploadDriveBlobResumable({url:'https://upload.test/start',method:'POST',blob});return {sessions,id:response.id};
  }finally{driveFetch=old;driveResumableSessions.clear();}
 });expect(result).toEqual({sessions:2,id:'file'});
});
test('durable pending edits and upload failures cannot display Saved',async({page})=>{
 const result=await page.evaluate(()=>{
  const old=InhouseOfflineLibrary;state.driveFileId='file';state.driveAutosave=true;driveDirty=false;driveContentVersion=driveUploadedContentVersion=3;
  window.InhouseOfflineLibrary={...old,record:()=>({dirty:true})};
  try{
   const pending=getSavePresentation();driveLastSaveError=new Error('Drive upload session expired');driveSavePhase='retrying';
   const failed=getSavePresentation();
   return {pending:pending.pendingDrive,phase:pending.phase,label:pending.driveLabel,errorPhase:failed.phase,detail:failed.driveDetail};
  }finally{window.InhouseOfflineLibrary=old;}
 });expect(result.pending).toBe(true);expect(result.phase).not.toBe('saved');expect(result.label).not.toBe('Saved');expect(result.errorPhase).toBe('error');expect(result.detail).toContain('expired');
});
test('confirmed upload retains a recovery draft and an old receipt cannot clear new edits',async({page})=>{
 const result=await page.evaluate(async()=>{
  const s=InhouseOfflineStore,account='receipt-test',id='file';
  const snapshot={pages:[{strokes:[{id:'valuable-ink',points:[{x:1,y:2}]}]}]};
  await s.put('documents',account,id,{generation:2,dirty:true,snapshot});
  const stale=await s.acknowledge(account,id,1,{id},new Blob(['old']));
  const pending=await s.read('documents',account,id);
  const current=await s.acknowledge(account,id,2,{id},new Blob(['current']));
  const recovery=await s.read('recoveries',account,`last-upload-draft:${id}`);
  return {stale,pending:pending.dirty,current,stroke:recovery.snapshot.pages[0].strokes[0].id};
 });expect(result).toEqual({stale:false,pending:true,current:true,stroke:'valuable-ink'});
});
test('dirty intent cannot take the no-upload Saved shortcut when version counters match',async({page})=>{
 const result=await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(1,'Pending');
  state.driveAutosave=true;state.driveFileId='pending';state.driveCanEdit=true;driveSignedOut=false;driveDirty=true;
  resetDriveContentSyncVersion();markDriveHydrated('pending');
  let uploads=0;
  prepareDrivePdfBlob=async()=>({blob:new Blob(['pdf']),version:driveContentVersion,contentHash:'draft'});
  ensureDriveFile=async()=>({fileId:'pending',created:false});
  mergeLatestRemoteBeforeDriveUpload=async()=>({changed:false});
  updateDriveFile=async()=>{uploads++;throw new Error('test upload rejected');};
  await queueDriveSaveOnline(true);
  const result={uploads,pending:hasPendingDriveSave(),label:getSavePresentation().driveLabel};
  state.driveAutosave=false;clearDriveSyncTimers();clearTimeout(deferredDriveSaveRetryTimer);return result;
 });expect(result.uploads).toBe(1);expect(result.pending).toBe(true);expect(result.label).toBe('Upload failed');
});
