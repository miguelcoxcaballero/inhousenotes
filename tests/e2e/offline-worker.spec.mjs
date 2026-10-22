import {test,expect} from '@playwright/test';
for (const mode of ['strokes','pdf','home-frame','home-memory-auth','home-auth-401']) test(`background processor merges and uploads real PDFs (${mode})`,async({context,page})=>{
 test.setTimeout(90000);
 const liveAuth=mode==='home-memory-auth'||mode==='home-auth-401';
 const withPhoto=mode!=='strokes';
 await context.route('https://accounts.google.com/**',r=>r.fulfill({status:200,body:''}));
 await page.addInitScript(()=>localStorage.setItem('drive-hint-email','worker@example.test'));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const remote=await page.evaluate(async(withPhoto)=>{
  await __IHN_TEST_API__.seedLiveReplicaForTest('remote');
  if(withPhoto){
   const canvas=document.createElement('canvas');canvas.width=100;canvas.height=100;
   const ctx=canvas.getContext('2d');ctx.fillStyle='#27b74a';ctx.fillRect(0,0,100,100);
   state.pages[0].backgroundSource='custom';state.pages[0].backgroundImage=canvas.toDataURL();
   noteCollabPageFieldChanged(state.pages[0],'background');
   markPageDirty(0,'full');bumpDriveContentVersion();
   state.pages[0].needsRedraw=true;
   document.querySelector('canvas.page-canvas[data-page="0"]')?.remove();
  }
  const bytes=await (await buildPdfBlob({status:false})).arrayBuffer();
  if(!withPhoto)return [...new Uint8Array(bytes)];
  // A distinctive PDF-only background tests reconstruction, independently of
  // any image URL/cache in the foreground editor that is about to be closed.
  const pdf=await PDFLib.PDFDocument.load(bytes),first=pdf.getPage(0);
  first.drawRectangle({x:0,y:0,width:first.getWidth(),height:first.getHeight(),color:PDFLib.rgb(0.15,0.72,0.29)});
  return [...await pdf.save()];
 },withPhoto);
 await context.setOffline(true);
 await page.evaluate(async(withPhoto)=>{
  await __IHN_TEST_API__.seedLiveReplicaForTest('first');
  if(withPhoto){
   const pdf=new jspdf.jsPDF();pdf.text('Original background',20,30);
   state.cleanOriginalPdfBytes=pdf.output('arraybuffer');
   state.pages[0].backgroundSource='pdf';state.pages[0].pdfPageIndex=1;state.pages[0].backgroundImage=null;
  }
  state.driveFileId='worker-doc';state.driveFileName='Offline.pdf';state.driveParentIds=['root'];
  state.driveCanEdit=true;state.driveAutosave=false;state.driveHeadRevisionId='h1';state.driveRevisionVersion='1';
  driveDirty=true;bumpDriveContentVersion();await InhouseOfflineLibrary.capture();
  await InhouseOfflineStore.patch('leases','worker@example.test','editor',()=>null);
  const grantedScopes='https://www.googleapis.com/auth/drive https://www.googleapis.com/auth/calendar.events';
  localStorage.setItem('google-scope-version-v1','drive-calendar-v2');
  localStorage.setItem('google-granted-scopes-v1',grantedScopes);
  localStorage.setItem('drive-token-v1',JSON.stringify({token:'test',expiry:Date.now()+600000,scopeVersion:'drive-calendar-v2',grantedScopes}));
 },withPhoto);
 if(liveAuth)await page.evaluate(()=>{
  driveOpenInProgress=true;driveAccessToken='live-home-token';driveTokenExpiry=Date.now()+600000;driveSignedOut=false;
  window.refreshes=0;
  requestDriveToken=async()=>{window.refreshes++;driveAccessToken='refreshed-home-token';driveTokenExpiry=Date.now()+600000;return driveAccessToken;};
  localStorage.removeItem('drive-token-v1');editorView.classList.add('hidden');
 });
 else await page.close();
 let uploads=0,downloads=0,rejections=0;
 const meta={id:'worker-doc',version:'2',headRevisionId:'h2',modifiedTime:'2026-10-14T12:00:00.000Z',name:'Offline.pdf',capabilities:{canEdit:true},parents:['root']};
 await context.route('https://www.googleapis.com/**',async route=>{
  const url=new URL(route.request().url());
  if(mode==='home-auth-401'&&route.request().frame().url().includes('offline-worker')&&url.pathname.endsWith('/worker-doc')&&route.request().headers().authorization==='Bearer live-home-token'){
   rejections++;return route.fulfill({status:401,json:{error:'expired'}});
  }
  if(mode==='home-auth-401'&&url.pathname.includes('/upload/'))expect(route.request().headers().authorization).toBe('Bearer refreshed-home-token');
  if(url.pathname.includes('/drive/v2/')&&!url.pathname.includes('/upload/'))return route.fulfill({json:{etag:'"h2"',version:'2'}});
  if(url.pathname.endsWith('/about'))return route.fulfill({json:{user:{emailAddress:'worker@example.test'}}});
  if(url.searchParams.get('alt')==='media'){if(route.request().frame().url().includes('offline-worker'))downloads++;return route.fulfill({contentType:'application/pdf',body:Buffer.from(remote)});}
  if(url.pathname.includes('/upload/')&&route.request().method()==='PATCH'){
   expect(route.request().headers()['if-match']).toBe('"h2"');uploads++;
   if(mode==='home-frame')expect(url.pathname).toContain('/upload/drive/v2/');
   return route.fulfill({json:{...meta,title:meta.name,modifiedDate:meta.modifiedTime,version:'3',headRevisionId:'h3'}});
  }
  return route.fulfill({headers:mode==='home-frame'?{}:{ETag:'"h2"','Access-Control-Expose-Headers':'ETag'},json:meta});
 });
 await context.setOffline(false);
 const host=liveAuth?page:await context.newPage();
 await host.addInitScript(()=>{
  window.requestAnimationFrame=()=>0; // No painted frames in Android's headless WebView.
  Object.defineProperty(document,'visibilityState',{get:()=> 'hidden'});
  addEventListener('message',e=>{if(e.data?.type==='ihn-offline-complete')window.result=e.data.result;});
 });
 let worker=host;
 if(mode==='home-frame'||liveAuth){
  await context.route('**/sync-test-host',r=>r.fulfill({contentType:'text/html',body:'<!doctype html><title>Home sync host</title>'}));
  if(mode==='home-frame')await host.goto('/sync-test-host');
  else await host.evaluate(()=>addEventListener('message',e=>{if(e.data?.type==='ihn-offline-complete')window.result=e.data.result;}));
  await host.evaluate(()=>{const frame=document.createElement('iframe');frame.hidden=true;frame.src='/?offline-worker=1';document.body.append(frame);});
  await expect.poll(()=>host.frames().find(f=>f.url().includes('offline-worker'))?.url()).toBeTruthy();
  worker=host.frames().find(f=>f.url().includes('offline-worker'));
  await host.waitForFunction(()=>window.result,{},{timeout:60000,polling:100});
  await worker.evaluate(()=>{window.result=parent.result;});
 }else await worker.goto('/?offline-worker=1');
 await worker.waitForFunction(()=>window.result,{},{timeout:60000,polling:100});
 const result=await worker.evaluate(async()=>{
  const stored=await InhouseOfflineStore.read('documents','worker@example.test','worker-doc');
  if(stored.dirty)return {summary:window.result,dirty:true};
  const pdf=await pdfjsLib.getDocument({data:await stored.blob.arrayBuffer()}).promise;
  const keywords=(await pdf.getMetadata()).info.Keywords;
  const pages=await decodeEmbeddedStrokesFromPdfKeywords(keywords);
  const first=await pdf.getPage(1),viewport=first.getViewport({scale:0.2});
  const canvas=document.createElement('canvas');canvas.width=viewport.width;canvas.height=viewport.height;
  const ctx=canvas.getContext('2d');await first.render({canvasContext:ctx,viewport,intent:'print'}).promise;
  const pixel=[...ctx.getImageData(Math.floor(canvas.width/2),Math.floor(canvas.height/2),1,1).data];
  await pdf.destroy();
  const recovery=(await InhouseOfflineStore.list('recoveries','worker@example.test'))[0];
  const rp=await pdfjsLib.getDocument({data:await recovery.remoteBlob.arrayBuffer()}).promise;
  const rpage=await rp.getPage(1);await rpage.render({canvasContext:ctx,viewport:rpage.getViewport({scale:0.2}),intent:'print'}).promise;
  const remotePixel=[...ctx.getImageData(Math.floor(canvas.width/2),Math.floor(canvas.height/2),1,1).data];await rp.destroy();
  return {summary:window.result,dirty:stored.dirty,pixel,remotePixel,ids:pages.flatMap(p=>p.strokes.map(s=>s.id)),recoveries:(await InhouseOfflineStore.list('recoveries','worker@example.test')).length};
 });
 expect(result.summary.errors).toEqual([]);expect(result.dirty).toBe(false);expect(uploads).toBe(1);expect(downloads).toBe(1);
 for(let i=0;i<3;i++)expect(result.ids).toEqual(expect.arrayContaining([`first-${i}`,`remote-${i}`]));
 if(withPhoto){expect(result.remotePixel[0]).toBeLessThan(90);expect(result.pixel[1]).toBeGreaterThan(130);expect(result.pixel[0]).toBeLessThan(90);}
 expect(result.ids).not.toContain('base-1-0');expect(result.recoveries).toBe(1);
 if(mode==='home-auth-401'){expect(rejections).toBe(1);expect(await host.evaluate(()=>window.refreshes)).toBe(1);}
 await host.close();
});

test('background access rejects account changes while authentication is pending',async({page})=>{
 await page.route('https://accounts.google.com/**',r=>r.fulfill({body:''}));
 await page.addInitScript(()=>localStorage.setItem('drive-hint-email','worker@example.test'));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  let resolve,calls=0;
  ensureDriveToken=()=>{calls++;return new Promise(done=>resolve=done);};
  const wrong=await InhouseOfflineApp.backgroundAccess('other@example.test').catch(error=>error.message);
  const pending=InhouseOfflineApp.backgroundAccess('worker@example.test').catch(error=>error.message);
  localStorage.setItem('drive-hint-email','other@example.test');resolve('must-not-escape');
  return {wrong,pending:await pending,calls};
 });
 expect(result.calls).toBe(1);expect(result.wrong).toMatch(/account changed/i);expect(result.pending).toMatch(/account changed/i);
});
