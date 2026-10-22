import {test,expect} from '@playwright/test';
const account='reconnect@example.test';
test.beforeEach(async({page})=>{
 await page.route('https://accounts.google.com/**',r=>r.fulfill({body:''}));
 await page.addInitScript(account=>{localStorage.setItem('drive-hint-email',account);localStorage.setItem('drive-remember-v1','1');},account);
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
});

test('local and remote folder refreshes keep the same deterministic order',async({page})=>{
 const result=await page.evaluate(async()=>{
  const L=InhouseOfflineLibrary;
  const items=[{id:'b',name:'B.pdf'},{id:'a',name:'A.pdf'}].map(e=>({...e,mimeType:'application/pdf',parents:['root'],modifiedTime:'2026-10-21T12:00:00Z'}));
  await L.observed(items);
  const pending={...items[0],id:'local-b',remoteId:'b',pending:true};
  await L.observed([pending]);
  const orders=[];
  for(let i=0;i<4;i++){
   await L.observed(i%2?items:[...items].reverse());
   orders.push(L.folder('root').map(e=>e.remoteId||e.id));
   orders.push(L.combine(i%2?items:[...items].reverse(),'root').map(e=>e.remoteId||e.id));
  }
  return orders;
 });
 for(const order of result)expect(order).toEqual(['a','b']);
});

test('opening from Home does not checkpoint the stale previous editor',async({page,context})=>{
 await context.setOffline(true);
 const result=await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(1,'Open');
  state.driveFileId='open';state.driveFileName='Open.pdf';state.driveCanEdit=true;
  await InhouseOfflineLibrary.capture();
  showDriveHome();
  const original=InhouseOfflineLibrary;let captures=0;
  window.InhouseOfflineLibrary={...original,capture:async()=>{captures++;throw Error('Old editor pages unavailable');}};
  try {await openDriveFile({id:'open',name:'Open.pdf'});return {captures,id:state.driveFileId,visible:!editorView.classList.contains('hidden'),busy:offlineOpenInProgress};}
  finally{window.InhouseOfflineLibrary=original;}
 });
 expect(result).toEqual({captures:0,id:'open',visible:true,busy:false});
});

test('active offline edits retry after a busy sync lease and show upload progress',async({page,context})=>{
 test.setTimeout(60000);
 let release,uploads=0;const held=new Promise(resolve=>release=resolve);
 await context.route('https://www.googleapis.com/**',async route=>{
  const url=new URL(route.request().url());
  const meta={id:'pending',version:'1',headRevisionId:'h1',modifiedTime:'2026-10-21T12:00:00Z',name:'Pending.pdf',parents:['root'],capabilities:{canEdit:true}};
  if(url.pathname.includes('/upload/')){uploads++;await held;return route.fulfill({json:{...meta,version:'2',headRevisionId:'h2'}});}
  if(url.pathname.endsWith('/about'))return route.fulfill({json:{user:{emailAddress:account}}});
  if(url.pathname.endsWith('/root'))return route.fulfill({json:{id:'root'}});
  if(url.pathname.endsWith('/files'))return route.fulfill({json:{files:[]}});
  return route.fulfill({headers:{ETag:'"h1"','Access-Control-Expose-Headers':'ETag'},json:meta});
 });
 await context.setOffline(true);
 await page.evaluate(async account=>{
  await __IHN_TEST_API__.resetLocalDocument(1,'Pending');
  state.driveFileId='pending';state.driveFileName='Pending.pdf';state.driveParentIds=['root'];state.driveCanEdit=true;state.driveAutosave=true;
  state.driveHeadRevisionId='h1';state.driveRevisionVersion='1';state.driveLastKnownModified=Date.parse('2026-10-21T12:00:00Z');
  state.pages[0].strokes.push({id:'offline-ink',tool:'pen',color:'#000000',width:2,points:[{x:10,y:10},{x:60,y:70}]});
  bumpDriveContentVersion();driveDirty=true;markPageDirty(0,'full');await InhouseOfflineLibrary.capture();
  driveSignedOut=false;driveAuthNeedsReconnect=false;driveAccessToken='test';driveTokenExpiry=Date.now()+600000;
  await InhouseOfflineStore.put('leases',account,'sync',{owner:'other-tab',expires:Date.now()+60000});
 },account);
 await context.setOffline(false);
 try {
  await page.waitForFunction(()=>!InhouseOfflineLibrary.offline());
  await page.evaluate(()=>queueDriveSave(true));
  expect(uploads).toBe(0);
  await page.evaluate(async account=>{await InhouseOfflineStore.patch('leases',account,'sync',()=>null);},account);
  await expect.poll(()=>uploads).toBe(1);
  expect(await page.evaluate(()=>getSavePresentation().driveLabel)).toBe('Uploading');
  await page.evaluate(()=>renderDriveRecents([{id:'pending',name:'Pending.pdf',mimeType:'application/pdf'}]));
  await expect(page.locator('#drive-recents .document-sync-status')).toHaveAttribute('data-phase','uploading');
  await expect(page.locator('#offline-sync-status')).toHaveCount(0);
  release();
  await expect.poll(()=>page.evaluate(async account=>(await InhouseOfflineStore.list('outbox',account)).length,account)).toBe(0);
  expect(await page.evaluate(()=>driveDirty)).toBe(false);
  expect(await page.evaluate(async account=>(await InhouseOfflineStore.read('documents',account,'pending')).blob.size,account)).toBeGreaterThan(100);
 }finally{release();}
});

test('a corrupt clean copy falls back to Drive without waiting for folder classification',async({page,context})=>{
 const pdf=await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(1,'Fresh');
  state.pages[0].strokes.push({id:'server-ink',tool:'pen',color:'#000000',width:2,points:[{x:10,y:10},{x:50,y:70}]});markPageDirty(0,'full');
  const bytes=[...new Uint8Array(await (await buildPdfBlob({status:false})).arrayBuffer())];
  await InhouseOfflineLibrary.cache('broken',new Blob(['broken pdf']),{id:'broken',version:'1'},null,{fileName:'Fresh.pdf',canEdit:true,parentIds:['root']});
  showDriveHome();driveSignedOut=false;driveAccessToken='test';driveTokenExpiry=Date.now()+600000;
  return bytes;
 });
 let downloads=0;
 await context.route('https://www.googleapis.com/**',route=>{
  const url=new URL(route.request().url());
  if(url.searchParams.get('alt')==='media'){downloads++;return route.fulfill({contentType:'application/pdf',body:Buffer.from(pdf)});}
  if(url.pathname.endsWith('/files'))return route.fulfill({json:{files:[]}});
  return route.fulfill({json:{id:'broken',name:'Fresh.pdf',version:'1',headRevisionId:'h1',parents:['root'],capabilities:{canEdit:true},ownedByMe:true}});
 });
 await page.evaluate(()=>{window.resolveDriveQuickAccessKindForFile=()=>new Promise(()=>{});window.openingFresh=openDriveFile({id:'broken',name:'Fresh.pdf'});});
 await page.waitForFunction(()=>!driveOpenInProgress&&!offlineOpenInProgress,{},{timeout:4000});
 await page.evaluate(()=>window.openingFresh);
 await expect(page.locator('#editor-view')).toBeVisible();
 expect(downloads).toBeGreaterThan(0);
 expect(await page.evaluate(async()=>{await ensurePageDataLoaded(0);return state.pages[0].strokes.map(s=>s.id);})).toContain('server-ink');
 expect(await page.evaluate(()=>driveOpenInProgress||offlineOpenInProgress)).toBe(false);
});

test('a failed dirty draft open preserves it and releases the lock for another file',async({page,context})=>{
 await context.setOffline(true);
 await page.evaluate(async account=>{
  await __IHN_TEST_API__.resetLocalDocument(1,'Good');state.driveFileId='good';state.driveFileName='Good.pdf';state.driveCanEdit=true;
  await InhouseOfflineLibrary.capture();showDriveHome();
  await InhouseOfflineStore.checkpoint(account,'broken-dirty',{blob:new Blob(['broken']),entry:{name:'Dirty.pdf',parents:['root'],mimeType:'application/pdf'}},true);
  await InhouseOfflineLibrary.reload();
  await openDriveFile({id:'broken-dirty',name:'Dirty.pdf'});
 },account);
 await expect(page.locator('#drive-status')).toContainText('changes are retained');
 const doc=await page.evaluate(async account=>{
  const doc=await InhouseOfflineStore.read('documents',account,'broken-dirty');
  return {dirty:doc.dirty,body:await doc.blob.text(),queued:!!await InhouseOfflineStore.read('outbox',account,'broken-dirty')};
 },account);
 expect(doc).toEqual({dirty:true,body:'broken',queued:true});
 await page.evaluate(()=>openDriveFile({id:'good',name:'Good.pdf'}));
 await expect(page.locator('#editor-view')).toBeVisible();
 expect(await page.evaluate(()=>state.driveFileId)).toBe('good');
});

test('production Drive adapter creates an offline document after reserved ID returns 404',async({page,context})=>{
 const writes=[];
 await context.route('https://www.googleapis.com/**',route=>{
  const req=route.request(),url=new URL(req.url());
  if(url.pathname.includes('/upload/')){writes.push(req.method());return route.fulfill({json:{id:'reserved',version:'1'}});}
  if(url.pathname.endsWith('/generateIds'))return route.fulfill({json:{ids:['reserved']}});
  return route.fulfill({status:404,json:{error:{code:404,message:'File not found'}}});
 });
 const result=await page.evaluate(async account=>{
  driveSignedOut=false;driveAccessToken='test';driveTokenExpiry=Date.now()+600000;
  await InhouseOfflineStore.checkpoint(account,'local-create',{blob:new Blob(['pdf']),entry:{name:'Offline.pdf',parents:['root']}});
  const result=await InhouseOfflineSync.run(account,{fetch:driveFetch});
  return {result,pending:(await InhouseOfflineStore.list('outbox',account)).length};
 },account);
 expect(result.result.errors).toEqual([]);expect(result.result.saved).toBe(1);expect(result.pending).toBe(0);expect(writes).toEqual(['POST']);
});

test('Drive v3 without ETag obtains a revision-matched token without discarding conditional writes',async({page,context})=>{
 const writes=[];
 await context.route('https://www.googleapis.com/**',route=>{
  const req=route.request(),url=new URL(req.url());
  if(req.method()==='PATCH'){writes.push({match:req.headers()['if-match'],path:url.pathname,body:req.postData()});return route.fulfill({json:{id:'existing',title:'Existing.pdf',modifiedDate:'2026-10-22T12:00:00Z',version:'8'}});}
  if(url.pathname.includes('/v2/'))return route.fulfill({json:{etag:'"revision-7"',version:'7'}});
  return route.fulfill({json:{id:'existing',version:'7',capabilities:{canEdit:true}}});
 });
 const result=await page.evaluate(async account=>{
  driveSignedOut=false;driveAccessToken='test';driveTokenExpiry=Date.now()+600000;
  await InhouseOfflineStore.checkpoint(account,'existing',{blob:new Blob(['offline edit']),baseMeta:{version:'7'},entry:{parents:['root']}});
  return InhouseOfflineSync.run(account,{fetch:driveFetch});
 },account);
 expect(result.errors).toEqual([]);expect(result.saved).toBe(1);expect(writes).toHaveLength(1);expect(writes[0].match).toBe('"revision-7"');expect(writes[0].path).toContain('/upload/drive/v2/');expect(writes[0].body).toContain('"visibility":"PRIVATE"');
});

test('a changed revision during token recovery never overwrites Drive',async({page})=>{
 const result=await page.evaluate(async account=>{
  const S=InhouseOfflineStore;let writes=0;
  await S.checkpoint(account,'race',{blob:new Blob(['local ink']),baseMeta:{version:'7'},entry:{parents:['root']}});
  const result=await InhouseOfflineSync.run(account,{fetch:async(url,o={})=>{
   if(o.method==='PATCH'){writes++;return Response.json({});}
   return Response.json(url.includes('/v2/')?{etag:'"revision-8"',version:'8'}:{id:'race',version:'7'});
  }});
  return {result,writes,local:await (await S.read('documents',account,'race')).blob.text(),pending:(await S.list('outbox',account)).length};
 },account);
 expect(result.result.errors[0].code).toBe(412);expect(result.writes).toBe(0);expect(result.local).toBe('local ink');expect(result.pending).toBe(1);
});

test('recents stay ordered across cached and network renders with pending local aliases',async({page})=>{
 const orders=await page.evaluate(async()=>{
  const items=['b','a'].map(id=>({id,name:id+'.pdf',mimeType:'application/pdf',parents:['root'],modifiedTime:'2026-10-22T12:00:00Z'}));
  await InhouseOfflineLibrary.observed([{...items[0],id:'local-b',remoteId:'b',pending:true}]);
  const orders=[];
  for(let i=0;i<4;i++){
   const files=i%2?items:[...items].reverse();
   await InhouseOfflineLibrary.observed(files,null,'recents');
   renderDriveRecents(files);orders.push([...document.querySelectorAll('#drive-recents [data-drive-item-id]')].map(e=>e.dataset.driveItemId));
   renderDriveRecents(InhouseOfflineLibrary.home().recents);orders.push([...document.querySelectorAll('#drive-recents [data-drive-item-id]')].map(e=>e.dataset.driveItemId));
  }
  return orders;
 });
 for(const order of orders)expect(order).toEqual(['a','local-b']);
});

test('eight queued documents drain through the production adapter without exposed v3 ETags',async({page,context})=>{
 let uploaded=0;
 await context.route('https://www.googleapis.com/**',route=>{
  const request=route.request(),url=new URL(request.url()),id=url.pathname.split('/').pop();
  if(request.method()==='PATCH'){uploaded++;return route.fulfill({json:{id,version:'2'}});}
  return route.fulfill({json:url.pathname.includes('/v2/')?{etag:'"v1"',version:'1'}:{id,version:'1'}});
 });
 const result=await page.evaluate(async account=>{
  driveSignedOut=false;driveAccessToken='test';driveTokenExpiry=Date.now()+600000;
  for(let i=0;i<8;i++)await InhouseOfflineStore.checkpoint(account,'queued-'+i,{blob:new Blob(['ink-'+i]),baseMeta:{version:'1'},entry:{name:'Note '+i,parents:['root']}});
  const summary=await InhouseOfflineSync.run(account,{fetch:driveFetch});
  await InhouseOfflineLibrary.syncFinished(summary);
  return {summary,pending:InhouseOfflineLibrary.syncStatus().pending};
 },account);
 expect(result.summary.errors).toEqual([]);expect(result.summary.saved).toBe(8);expect(result.pending).toBe(0);expect(uploaded).toBe(8);
});
