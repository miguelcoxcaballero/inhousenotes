import {test,expect} from '@playwright/test';
test('headless processor merges real PDFs after all foreground pages close',async({context,page})=>{
 test.setTimeout(90000);
 await context.route('https://accounts.google.com/**',r=>r.fulfill({status:200,body:''}));
 await page.addInitScript(()=>localStorage.setItem('drive-hint-email','worker@example.test'));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const remote=await page.evaluate(async()=>{
  await __IHN_TEST_API__.seedLiveReplicaForTest('remote');
  return [...new Uint8Array(await (await buildPdfBlob({status:false})).arrayBuffer())];
 });
 await context.setOffline(true);
 await page.evaluate(async()=>{
  await __IHN_TEST_API__.seedLiveReplicaForTest('first');
  state.driveFileId='worker-doc';state.driveFileName='Offline.pdf';state.driveParentIds=['root'];
  state.driveCanEdit=true;state.driveAutosave=false;state.driveHeadRevisionId='h1';state.driveRevisionVersion='1';
  driveDirty=true;bumpDriveContentVersion();await InhouseOfflineLibrary.capture();
  await InhouseOfflineStore.patch('leases','worker@example.test','editor',()=>null);
  const grantedScopes='https://www.googleapis.com/auth/drive https://www.googleapis.com/auth/calendar.events';
  localStorage.setItem('google-scope-version-v1','drive-calendar-v2');
  localStorage.setItem('google-granted-scopes-v1',grantedScopes);
  localStorage.setItem('drive-token-v1',JSON.stringify({token:'test',expiry:Date.now()+600000,scopeVersion:'drive-calendar-v2',grantedScopes}));
 });
 await page.close();
 let uploads=0;
 const meta={id:'worker-doc',version:'2',headRevisionId:'h2',modifiedTime:'2026-10-14T12:00:00.000Z',name:'Offline.pdf',capabilities:{canEdit:true},parents:['root']};
 await context.route('https://www.googleapis.com/**',async route=>{
  const url=new URL(route.request().url());
  if(url.pathname.endsWith('/about'))return route.fulfill({json:{user:{emailAddress:'worker@example.test'}}});
  if(url.searchParams.get('alt')==='media')return route.fulfill({contentType:'application/pdf',body:Buffer.from(remote)});
  if(url.pathname.includes('/upload/')&&route.request().method()==='PATCH'){
   expect(route.request().headers()['if-match']).toBe('"h2"');uploads++;
   return route.fulfill({json:{...meta,version:'3',headRevisionId:'h3'}});
  }
  return route.fulfill({headers:{ETag:'"h2"','Access-Control-Expose-Headers':'ETag'},json:meta});
 });
 await context.setOffline(false);
 const worker=await context.newPage();
 await worker.addInitScript(()=>{
  window.requestAnimationFrame=()=>0; // No painted frames in Android's headless WebView.
  Object.defineProperty(document,'visibilityState',{get:()=> 'hidden'});
  addEventListener('message',e=>{if(e.data?.type==='ihn-offline-complete')window.result=e.data.result;});
 });
 await worker.goto('/?offline-worker=1');
 await worker.waitForFunction(()=>window.result,{},{timeout:60000,polling:100});
 const result=await worker.evaluate(async()=>{
  const stored=await InhouseOfflineStore.read('documents','worker@example.test','worker-doc');
  if(stored.dirty)return {summary:window.result,dirty:true};
  const pdf=await pdfjsLib.getDocument({data:await stored.blob.arrayBuffer()}).promise;
  const keywords=(await pdf.getMetadata()).info.Keywords;
  const pages=await decodeEmbeddedStrokesFromPdfKeywords(keywords);await pdf.destroy();
  return {summary:window.result,dirty:stored.dirty,ids:pages.flatMap(p=>p.strokes.map(s=>s.id)),recoveries:(await InhouseOfflineStore.list('recoveries','worker@example.test')).length};
 });
 expect(result.summary.errors).toEqual([]);expect(result.dirty).toBe(false);expect(uploads).toBe(1);
 for(let i=0;i<3;i++)expect(result.ids).toEqual(expect.arrayContaining([`first-${i}`,`remote-${i}`]));
 expect(result.ids).not.toContain('base-1-0');expect(result.recoveries).toBe(1);
 await worker.close();
});
