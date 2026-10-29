import {test,expect} from '@playwright/test';
// A document left on Home while its Drive upload was still running must be
// uploaded by the next launch, promptly and with the last ink, even when the
// closed app left its sync lease and a long retry backoff behind.
const account='restart@example.test';
const grantedScopes='https://www.googleapis.com/auth/drive https://www.googleapis.com/auth/calendar.events';
function fakeDrive(context,drive){
 drive.meta={id:'restart-doc',name:'Restart.pdf',mimeType:'application/pdf',version:'1',headRevisionId:'h1',modifiedTime:'2026-10-29T10:00:00.000Z',
  parents:['root'],capabilities:{canEdit:true,canComment:true,canShare:true},ownedByMe:true,appProperties:{},properties:{}};
 drive.uploads=[];drive.hang=false;drive.pdf=Buffer.alloc(0);
 void context.route('https://accounts.google.com/**',r=>r.fulfill({body:''}));
 return context.route('https://www.googleapis.com/**',async route=>{
  const request=route.request(),url=new URL(request.url()),path=url.pathname,method=request.method();
  const json=(body,status=200)=>route.fulfill({status,json:body,headers:{ETag:`"${drive.meta.headRevisionId}"`,'Access-Control-Expose-Headers':'ETag'}});
  if(path.endsWith('/about'))return json({user:{emailAddress:account,displayName:'Restart',permissionId:'me'},storageQuota:{}});
  if(path.includes('/upload/')&&!path.includes('restart-doc')){
   const body=request.postData()?.match(/\{[^\r\n]*\}/)?.[0];let meta={};try{meta=JSON.parse(body||'{}');}catch(_){}
   return json({...meta,id:meta.id||`upload-${Date.now()}`,version:'1',headRevisionId:'u1',modifiedTime:new Date().toISOString()});
  }
  if(path.includes('/upload/')){
   if(drive.hang)return; // The app is closed while this request is still in flight.
   drive.meta={...drive.meta,version:String(+drive.meta.version+1),headRevisionId:`h${drive.uploads.length+2}`,modifiedTime:new Date().toISOString()};
   drive.uploads.push({path,frame:request.frame().url(),body:request.postDataBuffer(),revision:drive.meta.headRevisionId});
   return json(path.includes('/v2/')?{...drive.meta,title:drive.meta.name,modifiedDate:drive.meta.modifiedTime}:drive.meta);
  }
  if(path.includes('/drive/v2/files/restart-doc'))return json({etag:`"${drive.meta.headRevisionId}"`,version:drive.meta.version,headRevisionId:drive.meta.headRevisionId});
  if(path.includes('/comments'))return method==='GET'?json({comments:[]}):method==='DELETE'?route.fulfill({status:204,body:''}):json({id:`c${Date.now()}`,content:'',createdTime:new Date().toISOString(),modifiedTime:new Date().toISOString(),author:{me:true}});
  if(path.includes('/permissions'))return json({permissions:[]});
  if(path.includes('/revisions/')&&url.searchParams.get('alt')==='media')return route.fulfill({status:200,contentType:'application/pdf',body:drive.pdf});
  if(path.includes('/revisions/'))return json({id:drive.meta.headRevisionId,modifiedTime:drive.meta.modifiedTime});
  if(path.includes('/revisions'))return json({revisions:[{id:drive.meta.headRevisionId,modifiedTime:drive.meta.modifiedTime}]});
  if(path.endsWith('/files/restart-doc')){
   if(url.searchParams.get('alt')==='media')return route.fulfill({status:200,contentType:'application/pdf',body:drive.pdf,headers:{ETag:`"${drive.meta.headRevisionId}"`}});
   if(method==='PATCH'){const body=JSON.parse(request.postData()||'{}');drive.meta={...drive.meta,properties:{...drive.meta.properties,...body.properties},appProperties:{...drive.meta.appProperties,...body.appProperties},version:String(+drive.meta.version+1)};}
   return json(drive.meta);
  }
  if(path.endsWith('/files/root'))return json({id:'root-id'});
  if(path.endsWith('/files')&&method==='GET')return json({files:[]});
  if(path.endsWith('/files')&&method==='POST'){const body=JSON.parse(request.postData()||'{}');return json({...body,id:body.id||`created-${Date.now()}`,version:'1',headRevisionId:'x1',modifiedTime:new Date().toISOString()});}
  if(/\/files\/[^/]+$/.test(path))return method==='DELETE'?route.fulfill({status:204,body:''}):json({id:path.split('/').pop(),name:'other',version:'1',appProperties:{},properties:{}});
  return json({},404);
 });
}
async function launch(context){
 const page=await context.newPage();
 await page.addInitScript(({account,grantedScopes})=>{
  if(window.top!==window)return;
  localStorage.setItem('drive-hint-email',account);localStorage.setItem('drive-remember-v1','1');
  localStorage.setItem('google-scope-version-v1','drive-calendar-v2');localStorage.setItem('google-granted-scopes-v1',grantedScopes);
  localStorage.setItem('drive-token-v1',JSON.stringify({token:'restart-token',expiry:Date.now()+3600000,scopeVersion:'drive-calendar-v2',grantedScopes}));
  window.__workerFrames=0;
  new MutationObserver(list=>{for(const m of list)for(const n of m.addedNodes)if(n.tagName==='IFRAME'&&String(n.src).includes('offline-worker'))window.__workerFrames++;})
   .observe(document,{childList:true,subtree:true});
 },{account,grantedScopes});
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 return page;
}
async function leaveWithUploadInFlight(context,drive,{failedBefore=false}={}){
 const page=await launch(context);
 drive.pdf=Buffer.from(await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(1,'Restart');
  state.pages[0].strokes.push({id:'base-ink',tool:'pen',color:'#000000',width:2,points:[{x:200,y:200},{x:260,y:240}]});
  const bytes=[...new Uint8Array(await (await buildPdfBlob({status:false})).arrayBuffer())];
  showDriveHome();return bytes;
 }));
 await page.evaluate(async()=>{
  driveAccessToken='restart-token';driveTokenExpiry=Date.now()+3600000;driveSignedOut=false;
  await openDriveFile({id:'restart-doc',name:'Restart.pdf'});
 });
 await expect(page.locator('#editor-view')).toBeVisible();
 await expect.poll(()=>page.evaluate(()=>state.driveFileId)).toBe('restart-doc');
 drive.hang=true;
 await page.evaluate(()=>{
  setReadOnlyMode(false,{force:true});
  state.pages[0].strokes.push({id:'restart-ink',tool:'pen',color:'#000000',width:2,points:[{x:20,y:20},{x:120,y:140}]});
  markPageDirty(0,'full');bumpDriveContentVersion();driveDirty=true;
 });
 // The upload to Drive starts and never finishes in this session.
 await page.evaluate(()=>{void queueDriveSave(true).catch(()=>{});});
 await page.waitForTimeout(500);
 await page.locator('#btn-drive-home').click();
 await expect(page.locator('#drive-home')).toBeVisible({timeout:10000});
 await expect.poll(()=>page.evaluate(async account=>(await InhouseOfflineStore.read('documents',account,'restart-doc'))?.dirty,account)).toBe(true);
 if(failedBefore)await page.evaluate(async account=>{
  // Earlier attempts in this session failed: its retry backoff reached 8 minutes.
  await InhouseOfflineStore.patch('outbox',account,'restart-doc',old=>({...old,attempts:6,attemptedAt:Date.now(),lastError:'network',lastErrorMessage:'Failed to fetch'}));
 },account);
 // Process death while the upload holds the account's sync lease.
 await expect.poll(()=>page.evaluate(async account=>((await InhouseOfflineStore.read('leases',account,'sync'))?.expires||0)>Date.now(),account)).toBe(true);
 if(failedBefore)await page.evaluate(async account=>{
  // A lease written by an older release carries no liveness lock: its expiry decides.
  await InhouseOfflineStore.put('leases',account,'sync',{owner:'closed-app',expires:Date.now()+20000});
 },account);
 await page.close();
}
for(const failedBefore of [false,true])test(`a document left during its upload is uploaded after the app restarts (failedBefore=${failedBefore})`,async({context})=>{
 test.setTimeout(120000);
 const drive={};await fakeDrive(context,drive);
 await leaveWithUploadInFlight(context,drive,{failedBefore});
 drive.hang=false;
 const startedAt=Date.now();
 const page=await launch(context);
 await expect.poll(()=>page.evaluate(async account=>(await InhouseOfflineStore.list('outbox',account)).length,account),{timeout:60000,intervals:[500]}).toBe(0);
 const elapsed=Date.now()-startedAt;
 // A dead owner's lease is recognised at once; an old-format lease expires
 // after 20 s. The upload follows without polling delays either way.
 expect(elapsed).toBeLessThan(failedBefore?45000:20000);
 expect(drive.uploads.length).toBeGreaterThanOrEqual(1);
 const stored=await page.evaluate(async account=>{const doc=await InhouseOfflineStore.read('documents',account,'restart-doc');return {dirty:doc.dirty,revision:doc.baseMeta?.headRevisionId};},account);
 // (The editor's own pending-exit resume may upload the same content once more.)
 expect(stored.dirty).toBe(false);
 expect(drive.uploads.map(upload=>upload.revision)).toContain(stored.revision);
 // The uploaded PDF carries the stroke written just before leaving.
 const body=drive.uploads.find(upload=>upload.revision===stored.revision).body,start=body.indexOf('%PDF'),end=body.lastIndexOf('%%EOF')+5;
 const ids=await page.evaluate(async bytes=>{
  const pdf=await pdfjsLib.getDocument({data:new Uint8Array(bytes)}).promise;
  const pages=await decodeEmbeddedStrokesFromPdfKeywords((await pdf.getMetadata()).info.Keywords);await pdf.destroy();
  return pages.flatMap(p=>p.strokes.map(s=>s.id));
 },[...body.subarray(start,end)]);
 expect(ids).toContain('restart-ink');
 // One hidden processor at most; no reload loop while the stale lease expired.
 expect(await page.evaluate(()=>window.__workerFrames)).toBeLessThanOrEqual(1);
 await page.close();
});
