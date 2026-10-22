import {test,expect} from '@playwright/test';

test.beforeEach(async({page})=>{
 await page.route('https://accounts.google.com/**',r=>r.fulfill({body:''}));
 await page.addInitScript(()=>{localStorage.setItem('drive-hint-email','cached@example.test');localStorage.setItem('drive-remember-v1','1');});
});

test('downloaded Drive PDF opens offline with its ink and survives a cold reopen',async({page,context})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(1,'Downloaded');
  state.pages[0].strokes.push({id:'downloaded-ink',tool:'pen',color:'#000000',width:2,points:[{x:20,y:30},{x:90,y:80}]});markPageDirty(0,'full');
  const blob=await buildPdfBlob({status:false});
  await InhouseOfflineLibrary.cache('downloaded',blob,{version:'1',modifiedTime:'2026-10-16T00:00:00Z'},null,{fileName:'Downloaded.pdf',parentIds:['root'],canEdit:true});
  await __IHN_TEST_API__.resetLocalDocument(1,'Other');showDriveHome();
 });
 await page.evaluate(()=>navigator.serviceWorker.ready);await page.waitForFunction(()=>!!navigator.serviceWorker.controller);
 await context.setOffline(true);
 await page.reload();await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.locator('[data-drive-item-id="downloaded"]').first().click();
 await expect(page.locator('#editor-view')).toBeVisible();
 await page.waitForFunction(()=>state.pages.length===1&&!offlineOpenInProgress);
 const result=await page.evaluate(async()=>{await ensurePageDataLoaded(0);return {ids:state.pages[0].strokes.map(s=>s.id),id:state.driveFileId,pages:state.pages.length};});
 expect(result).toEqual({ids:['downloaded-ink'],id:'downloaded',pages:1});
});

test('online and cold offline home keep the same lists, folder colours, preview, avatar and fonts',async({page,context})=>{
 await page.route('**/test-preview.png',r=>r.fulfill({contentType:'image/png',body:Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==','base64')}));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(async()=>{
  const photo='/test-preview.png';
  localStorage.setItem('drive-profile-v1',JSON.stringify({email:'cached@example.test',name:'Cached User',photo}));restoreCachedDriveProfile();
  const folder={id:'folder-b',name:'Biology',mimeType:'application/vnd.google-apps.folder',parents:['root'],folderColorRgb:'#aaddcc'};
  const remote={id:'only-remote',name:'Remote.pdf',mimeType:'application/pdf',parents:['root'],thumbnailLink:photo,modifiedTime:'2026-10-15T00:00:00Z'};
  const inside={...remote,id:'inside',name:'Inside.pdf',parents:['folder-b']};
  await InhouseOfflineLibrary.observed([remote],null,'recents');
  await InhouseOfflineLibrary.observed([folder],null,'starred');
  await InhouseOfflineLibrary.observed([],null,'shared');
  await InhouseOfflineLibrary.observed([folder,remote],'root');
  await InhouseOfflineLibrary.observed([inside],'folder-b');
  await InhouseOfflineMedia.url('cached@example.test','preview:only-remote',photo);
  await InhouseOfflineMedia.url('cached@example.test','preview:inside',photo);
  InhouseOfflineApp.renderHome();
 });
 await expect(page.locator('#drive-recents .drive-card-preview img')).toBeVisible();
 const layout=()=>page.evaluate(()=>Object.fromEntries(['drive-recents','drive-starred','drive-shared','drive-folders','drive-folder-files'].map(id=>[id,[...document.querySelectorAll(`#${id} [data-drive-item-id]`)].map(e=>e.dataset.driveItemId)])));
 const online=await layout();
 await page.evaluate(()=>navigator.serviceWorker.ready);await page.waitForFunction(()=>!!navigator.serviceWorker.controller);
 await page.unroute('**/test-preview.png');
 await context.setOffline(true);await page.reload();await page.evaluate(()=>__IHN_TEST_API__.ready());
 expect(await layout()).toEqual(online);
 await expect(page.locator('#drive-recents [data-drive-item-id="only-remote"]')).toHaveClass(/offline-unavailable/);
 await expect.poll(()=>page.locator('#drive-recents .drive-card-preview img').evaluate(img=>img.complete&&img.naturalWidth>0)).toBe(true);
 expect(await page.evaluate(async()=>{await document.fonts.ready;return document.fonts.check('600 20px Comfortaa')&&document.fonts.check('400 20px "DM Sans"');})).toBe(true);
 expect(await page.evaluate(()=>driveProfileName.textContent)).toBe('Cached User');
 await expect.poll(()=>page.evaluate(()=>driveProfileAvatar.complete&&driveProfileAvatar.naturalWidth>0&&driveProfileAvatar.src!==DEFAULT_AVATAR)).toBe(true);
 await page.locator('#home-nav [data-home-tab="files"]').click();
 await page.locator('#drive-folders [data-drive-item-id="folder-b"]').click();
 await expect(page.locator('#drive-folder-view [data-drive-item-id="inside"]')).toBeVisible();
 await expect(page.locator('#drive-folder-view [data-drive-item-id="inside"]')).toHaveClass(/offline-unavailable/);
});

test('cached document opens online without waiting for Drive and listing never loads its stroke payload',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(3,'Fast');
  state.driveFileId='fast-open';state.driveFileName='Fast.pdf';state.driveParentIds=['root'];state.driveCanEdit=true;
  state.pages[0].strokes.push({id:'local-pen',tool:'pen',color:'#000000',width:2,points:[{x:1,y:2},{x:3,y:4}]});
  driveDirty=true;bumpDriveContentVersion();await InhouseOfflineLibrary.capture();
  const index=InhouseOfflineLibrary.record('fast-open');
  await __IHN_TEST_API__.resetLocalDocument(1,'Other');showDriveHome();
  const start=performance.now();await openDriveFile({id:'fast-open',name:'Fast.pdf'});
  await ensurePageDataLoaded(0);
  return {ms:performance.now()-start,pages:state.pages.length,ink:state.pages[0].strokes.map(s=>s.id),available:index.available,payload:['blob','snapshot','originalBlob','analysis'].some(key=>key in index)};
 });
 console.log('LOCAL_OPEN',JSON.stringify(result));
 expect(result.pages).toBe(3);expect(result.ink).toContain('local-pen');expect(result.available).toBe(true);expect(result.payload).toBe(false);
 expect(result.ms).toBeLessThan(2000);
});

test('cached home paints while Drive authentication and listing requests are stalled',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(async()=>{
  await InhouseOfflineLibrary.observed([{id:'immediate',name:'Immediate.pdf',mimeType:'application/pdf',parents:['root']}],null,'recents');
  const scopes='https://www.googleapis.com/auth/drive https://www.googleapis.com/auth/calendar.events';
  localStorage.setItem('google-scope-version-v1','drive-calendar-v2');localStorage.setItem('google-granted-scopes-v1',scopes);
  localStorage.setItem('drive-token-v1',JSON.stringify({token:'test',expiry:Date.now()+600000,scopeVersion:'drive-calendar-v2',grantedScopes:scopes}));
 });
 let release;const stalled=new Promise(resolve=>{release=resolve;});
 await page.route('https://www.googleapis.com/**',async r=>{await stalled;await r.fulfill({json:{files:[]}}).catch(()=>{});});
 try {
  await page.reload({waitUntil:'domcontentloaded'});
  await expect(page.locator('#drive-recents [data-drive-item-id="immediate"]')).toBeVisible({timeout:2500});
  await expect(page.locator('#welcome-view')).toBeHidden();
 }finally{release();}
});

test('schema upgrade indexes existing offline drafts without modifying their content or outbox',async({page})=>{
 // Seed the deployed v1 schema before any app or service worker opens it.
 await page.route('**/seed-offline.html',r=>r.fulfill({contentType:'text/html',body:'<!doctype html><title>Seed</title>'}));
 await page.goto('/seed-offline.html');
 await page.evaluate(()=>new Promise((resolve,reject)=>{
  const request=indexedDB.open('inhouse-offline-v1',1);
  request.onupgradeneeded=()=>{for(const name of ['entries','documents','outbox','settings','leases','recoveries']){const store=request.result.createObjectStore(name,{keyPath:'key'});store.createIndex('account','account');}};
  request.onerror=()=>reject(request.error);
  request.onsuccess=()=>{
   const db=request.result,account='cached@example.test',id='legacy-draft',key=`${account}\0${id}`;
   const tx=db.transaction(['entries','documents','outbox'],'readwrite');
   tx.objectStore('entries').put({key,account,id,name:'Legacy.pdf',parents:['root'],mimeType:'application/pdf'});
   tx.objectStore('documents').put({key,account,id,dirty:true,generation:7,snapshot:{pages:[{pageId:'old-page',strokes:[{id:'old-ink'}]}]},lastOpenedAt:Date.now()});
   tx.objectStore('outbox').put({key,account,id,generation:7,kind:'document'});
   tx.oncomplete=()=>{db.close();resolve();};tx.onerror=()=>reject(tx.error);
  };
 }));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>({index:InhouseOfflineLibrary.record('legacy-draft'),doc:await InhouseOfflineStore.read('documents','cached@example.test','legacy-draft'),pending:await InhouseOfflineStore.read('outbox','cached@example.test','legacy-draft')}));
 expect(result.index.available).toBe(true);expect(result.index.snapshot).toBeUndefined();
 expect(result.doc.snapshot.pages[0].strokes[0].id).toBe('old-ink');expect(result.doc.generation).toBe(7);expect(result.pending.generation).toBe(7);
});

test('private Google image without CORS remains visible offline through the account cache',async({page,context})=>{
 await page.route('https://lh3.googleusercontent.com/offline-preview',r=>r.fulfill({contentType:'image/png',body:Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==','base64')}));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(()=>navigator.serviceWorker.ready);await page.waitForFunction(()=>!!navigator.serviceWorker.controller);
 const warmed=await page.evaluate(async()=>{
  const fetchNetwork=window.fetch;window.fetch=(url,options)=>String(url).includes('googleusercontent.com/offline-preview')&&options?.mode!=='no-cors'?Promise.reject(new TypeError('CORS denied')):fetchNetwork(url,options);
  const source=await InhouseOfflineMedia.url('cached@example.test','opaque','https://lh3.googleusercontent.com/offline-preview');
  const cache=await caches.open('inhouse-media-cached%40example.test');
  return {source,type:(await cache.match(new URL('/__offline-media__/opaque',location.origin))).type};
 });
 expect(warmed.type).toBe('opaque');
 await page.unroute('https://lh3.googleusercontent.com/offline-preview');await context.setOffline(true);
 await page.reload();await page.evaluate(()=>__IHN_TEST_API__.ready());
 expect(await page.evaluate(async()=>{
  const img=new Image();img.src=await InhouseOfflineMedia.url('cached@example.test','opaque');
  document.body.appendChild(img);await img.decode();return img.naturalWidth;
 })).toBe(1);
});

test('a new confirmed PDF cannot reuse the previous revision snapshot or decoded metadata',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  const S=InhouseOfflineStore,a=InhouseOfflineLibrary.account();
  const old={blob:new Blob(['old']),snapshot:{pages:[{strokes:[{id:'obsolete'}]}]},analysis:{keywords:'old'},originalBlob:new Blob(['old-background']),entry:{id:'revision',name:'Revision.pdf'}};
  const checkpoint=await S.checkpoint(a,'revision',old,true);
  await S.acknowledge(a,'revision',checkpoint.generation,{id:'revision',version:'2'},new Blob(['confirmed']));
  const ack=await S.read('documents',a,'revision');
  await S.checkpoint(a,'download',old,false);
  await InhouseOfflineLibrary.cache('download',new Blob(['new download']),{version:'3'},{keywords:'new'},{});
  const downloaded=await S.read('documents',a,'download');
  return {ack:[ack.snapshot,ack.analysis,ack.originalBlob,await ack.blob.text()],download:[downloaded.snapshot,downloaded.originalBlob,downloaded.analysis.keywords,await downloaded.blob.text()]};
 });
 expect(result.ack).toEqual([null,null,null,'confirmed']);expect(result.download).toEqual([null,null,'new','new download']);
});

test('cold Home resume never replaces the durable outbox draft with the last editor cache',async({page,context})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(async()=>{
  await __IHN_TEST_API__.seedLiveReplicaForTest('before');
  state.driveFileId='cold-draft';state.driveFileName='Draft.pdf';state.driveCanEdit=true;state.driveAutosave=false;
  driveDirty=true;bumpDriveContentVersion();
  await __IHN_TEST_API__.checkpointBeforeLeaving();saveDriveSession();
  // The per-document draft can be newer than the single-editor cache, e.g.
  // after an interrupted checkpoint or after opening another document.
  const doc=await InhouseOfflineStore.read('documents','cached@example.test','cold-draft');
  doc.snapshot.pages[0].strokes.push({id:'last-durable-stroke',tool:'pen',color:'#000000',width:2,points:[{x:10,y:10},{x:80,y:80}]});
  await InhouseOfflineStore.checkpoint('cached@example.test','cold-draft',{...doc,contentVersion:doc.contentVersion+1},true);
  showDriveHome();
 });
 await context.route('https://www.googleapis.com/**',r=>r.abort());
 await page.reload();await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(()=>InhouseOfflineLibrary.resume());
 const ids=await page.evaluate(async()=>{
  const doc=await InhouseOfflineStore.read('documents','cached@example.test','cold-draft');
  return doc.snapshot.pages[0].strokes.map(s=>s.id);
 });
 expect(ids).toContain('last-durable-stroke');
});

for(const legacy of [false,true])test(`cold notebook restores the persisted page session (legacy=${legacy})`,async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const before=await page.evaluate(async legacy=>{
  beginDocumentSession();
  await __IHN_TEST_API__.seedLiveReplicaForTest('persisted');
  await __IHN_TEST_API__.checkpointBeforeLeaving();
  if(legacy){
   const record=await loadFromIndexedDb();delete record.data.sessionId;
   await saveToIndexedDb(record.data,record.data.savedAt);
   const local=JSON.parse(localStorage.getItem(DATA_STORAGE_KEY));delete local.sessionId;
   localStorage.setItem(DATA_STORAGE_KEY,JSON.stringify(local));
  }
  return {session:getDocumentSessionToken(),ids:state.pages.map(p=>p.strokes.map(s=>s.id))};
 },legacy);
 expect(before.session).toBeGreaterThan(1);
 await page.reload();await page.evaluate(()=>__IHN_TEST_API__.ready());
 const after=await page.evaluate(async()=>{
  for(let i=0;i<state.pages.length;i++)await __IHN_TEST_API__.loadPage(i);
  return {session:getDocumentSessionToken(),ids:state.pages.map(p=>p.strokes.map(s=>s.id))};
 });
 expect(after).toEqual(before);
});
