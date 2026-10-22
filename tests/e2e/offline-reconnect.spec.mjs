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
  await page.evaluate(async account=>{await InhouseOfflineStore.patch('leases',account,'sync',()=>null);window.retrySync=InhouseOfflineLibrary.kick();},account);
  await expect.poll(()=>uploads).toBe(1);
  expect(await page.evaluate(()=>getSavePresentation().driveLabel)).toBe('Uploading');
  await expect(page.locator('#offline-sync-status')).toHaveAttribute('data-phase','uploading');
  release();await page.evaluate(()=>window.retrySync);
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
 await expect(page.locator('#offline-sync-status')).toContainText('changes are retained');
 const doc=await page.evaluate(async account=>{
  const doc=await InhouseOfflineStore.read('documents',account,'broken-dirty');
  return {dirty:doc.dirty,body:await doc.blob.text(),queued:!!await InhouseOfflineStore.read('outbox',account,'broken-dirty')};
 },account);
 expect(doc).toEqual({dirty:true,body:'broken',queued:true});
 await page.evaluate(()=>openDriveFile({id:'good',name:'Good.pdf'}));
 await expect(page.locator('#editor-view')).toBeVisible();
 expect(await page.evaluate(()=>state.driveFileId)).toBe('good');
});
