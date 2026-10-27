import {test,expect} from '@playwright/test';
import {fakeInkDrive} from '../helpers/fake-ink-drive.mjs';

const SOURCE='remote-replay-journal';
const DOCUMENT='replay-durability';
const INITIAL_NAME='Before remote edit';
const REMOTE_NAME='Durably received remote edit';

async function setup(page,api){
 await page.exposeFunction('replayInkRequest',(url,options)=>api.request(url,options));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(async({documentId,name})=>{
  await InhouseOfflineLibrary.useAccount('replay-durability@example.test');
  await __IHN_TEST_API__.resetLocalDocument(1,name);
  await __IHN_TEST_API__.showEditorForTest();
  stopCollabPolling();beginDocumentSession();InhouseDriveInk.stop();
  state.driveFileId=documentId;state.driveCanEdit=true;state.driveAutosave=false;
  state.pages[0].pageId='replay-page';state.collabStructure=null;state.collabFields=null;
  driveDirty=false;resetDriveContentSyncVersion();
  // Seed a real durable, empty notebook before failure injection. Background
  // save timers are disabled; the actual merge must complete its own checkpoint.
  if(!await savePageToIndexedDb(0,state.pages[0]))throw Error('Initial page checkpoint failed');
  const savedAt=Date.now(),payload=buildMetaPayload(savedAt);
  if(!await saveToIndexedDb(payload,savedAt))throw Error('Initial metadata checkpoint failed');
  if(!scheduleLocalStorageBackup(payload,{immediate:true}))throw Error('Initial backup failed');
  window.replayOriginalQueueSave=queueSave;
  queueSave=async()=>({ok:false,deferred:true});scheduleDriveSync=()=>{};
  schedulePreparedDrivePdf=()=>{};scheduleLiveDocumentBroadcast=()=>{};startLiveCollaboration=()=>{};
  driveFetch=async(url,options={})=>{
   const body=typeof options.body==='string'?options.body:await options.body?.text?.();
   const r=await replayInkRequest(url,{method:options.method,headers:options.headers,body});
   return new Response(r.body,{status:r.status,headers:{'Content-Type':'application/json'}});
  };
  const Journal=InhouseDriveInkJournal.Journal;
  InhouseDriveInkJournal.Journal=class extends Journal{
   constructor(host){super({...host,actor:'replay-reader'});window.replayJournal=this;}
   start(){return this;}wake(){}
  };
  InhouseDriveLivePreview=null;InhouseDriveInk.start();await InhouseDriveInk.capture();
  window.replayStroke={id:'remote-durable-ink',tool:'pen',color:'#0000ff',width:2,
   points:[{x:10,y:20,p:.5},{x:20,y:35,p:.75},{x:45,y:50,p:.25}],syncStamp:{clock:100,actor:'remote-writer'}};
 },{documentId:DOCUMENT,name:INITIAL_NAME});
}

async function installSource(page,api,metadata){
 const operation=await page.evaluate(({metadata,name})=>{
  if(!metadata)return {v:1,kind:'stroke',pageId:'replay-page',stroke:replayStroke};
  const fields=getCollabFieldSnapshot();
  fields.fields[COLLAB_DOC_NAME_KEY]={value:name,stamp:{clock:100,actor:'remote-writer'}};
  fields.clock=Math.max(fields.clock,100);
  const patch=buildTimelineSnapshotFromPages([{...state.pages[0],strokes:[replayStroke]}])[0];
  return {v:1,pages:[patch],fields,structure:getCollabStructureSnapshot(),name,calendar:null};
 },{metadata,name:REMOTE_NAME});
 api.files.set(SOURCE,{id:SOURCE,appProperties:{ihnDoc:DOCUMENT,ihnKind:'ink-journal-v1'},
  raw:JSON.stringify({v:1,docId:DOCUMENT,actor:'remote-writer',part:0,rev:1,ops:[operation]})});
 api.files.set(`${SOURCE}-duplicate`,{...api.files.get(SOURCE),id:`${SOURCE}-duplicate`});
 return operation.kind==='stroke'?operation.stroke:operation.pages[0].strokes[0];
}

async function exerciseFailedReplays(page,metadata,earlyFailure=null){
 return page.evaluate(async({sourceId,metadata,earlyFailure})=>{
  const put=IDBObjectStore.prototype.put,backup=scheduleLocalStorageBackup,savePage=savePageToIndexedDb;
  const versionBefore=driveContentVersion;
  preparedDrivePdf={blob:new Blob(['stale pdf']),version:versionBefore,sessionToken:getDocumentSessionToken()};
  pdfExportCache.set(0,{test:'stale raster'});pdfLibOverlayCache.set(0,{test:'stale overlay'});
  let failedWrites=0,failedBackups=0;
  if(earlyFailure)savePageToIndexedDb=async()=>{
   failedWrites++;
   if(earlyFailure==='throw')throw Error('Injected failure before the page transaction');
   return false;
  };
  else IDBObjectStore.prototype.put=function(...args){
   if(this.name===(metadata?DB_STORE:PAGE_STORE)){
    failedWrites++;throw new DOMException('Injected persistent checkpoint failure','UnknownError');
   }
   return put.apply(this,args);
  };
  if(metadata)scheduleLocalStorageBackup=()=>{failedBackups++;return false;};
  replayJournal.hint(sourceId);const source=replayJournal.sources.get(sourceId);
  const inspect=async()=>{
   const storedPage=await loadPageFromIndexedDb(0),storedMeta=await loadFromIndexedDb();
   const local=JSON.parse(localStorage.getItem(DATA_STORAGE_KEY));
   return {seen:replayJournal.seen.get(sourceId)||0,rev:source.rev,rawCached:!!source.raw,
    stored:storedPage?.strokes.map(cloneStroke),memory:state.pages[0].strokes.map(cloneStroke),
    name:storedMeta?.data.collabFields.fields[COLLAB_DOC_NAME_KEY]?.value,
    backupName:local?.collabFields.fields[COLLAB_DOC_NAME_KEY]?.value,
    memoryName:state.exportName,dirty:pageDirty.has(0),failedWrites,failedBackups,
    version:driveContentVersion,driveDirty,pdfCached:!!preparedDrivePdf,
    rasterCached:pdfExportCache.has(0),overlayCached:pdfLibOverlayCache.has(0)};
  };
  let first,retry;
  try{
   await replayJournal.readSource(sourceId,source);first=await inspect();
   // Identical bytes and an already-mutated live page must not count as a
   // successful receipt while the same storage failure is still present.
   await replayJournal.readSource(sourceId,source);retry=await inspect();
  }finally{
   IDBObjectStore.prototype.put=put;scheduleLocalStorageBackup=backup;savePageToIndexedDb=savePage;
  }
  await replayJournal.readSource(sourceId,source);
  const recovered=await inspect(),duplicateWrites={page:0,metadata:0},duplicateWriteStacks=[];
  // Recovery redraw also queues thumbnail-only writes. Cancel that independent
  // renderer while counting duplicate merge writes; keep the actual merge and
  // all page/metadata persistence functions untouched.
  const queuePreviews=queueScrollPagePreviews;resetScrollPagePreviews();queueScrollPagePreviews=()=>{};
  IDBObjectStore.prototype.put=function(...args){
   if(this.name===PAGE_STORE)duplicateWrites.page++;
   if(this.name===DB_STORE)duplicateWrites.metadata++;
   if(this.name===PAGE_STORE||this.name===DB_STORE)duplicateWriteStacks.push(new Error(`Write to ${this.name}`).stack);
   return put.apply(this,args);
  };
  let duplicateSeen;
  try{
   // The same acknowledged source is rejected by the journal; a different
   // source carrying identical ink still traverses the real merge path.
   await replayJournal.readSource(sourceId,source);
   const duplicateId=`${sourceId}-duplicate`;replayJournal.hint(duplicateId);
   await replayJournal.readSource(duplicateId,replayJournal.sources.get(duplicateId));
   duplicateSeen=replayJournal.seen.get(duplicateId)||0;
  }finally{IDBObjectStore.prototype.put=put;queueScrollPagePreviews=queuePreviews;}
  const versionAfterDuplicate=driveContentVersion;InhouseDriveInk.stop();
  return {first,retry,recovered,versionBefore,duplicateWrites,duplicateWriteStacks,duplicateSeen,versionAfterDuplicate};
 },{sourceId:SOURCE,metadata,earlyFailure});
}

function expectRecoveredCheckpoint(result){
 expect(result.recovered.driveDirty).toBe(true);expect(result.recovered.version).toBeGreaterThan(result.versionBefore);
 expect(result.recovered.pdfCached).toBe(false);expect(result.recovered.rasterCached).toBe(false);expect(result.recovered.overlayCached).toBe(false);
 expect(result.duplicateWrites,result.duplicateWriteStacks.join('\n')).toEqual({page:0,metadata:0});expect(result.duplicateSeen).toBe(1);
 expect(result.versionAfterDuplicate).toBe(result.recovered.version);
}

async function reopenedNotebook(page){
 await page.reload();await page.evaluate(()=>__IHN_TEST_API__.ready());
 return page.evaluate(async()=>{
  InhouseDriveInk.stop();stopCollabPolling();
  await ensurePageDataLoaded(0,{keepLoaded:true});
  return {ids:state.pages.map(p=>p.pageId),strokes:state.pages[0].strokes.map(cloneStroke),name:state.exportName};
 });
}

test('identical ink replay cannot acknowledge a page whose IndexedDB writes still fail',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);const stroke=await installSource(page,api,false);
 const result=await exerciseFailedReplays(page,false),{first,retry,recovered}=result;
 for(const attempt of [first,retry]){
  expect(attempt.memory).toEqual([stroke]);expect(attempt.stored).toEqual([]);
  expect(attempt.seen).toBe(0);expect(attempt.rev).toBe(0);expect(attempt.rawCached).toBe(false);
 }
 expect(retry.failedWrites).toBeGreaterThan(first.failedWrites);
 expect(recovered.seen).toBe(1);expect(recovered.rev).toBe(1);expect(recovered.stored).toEqual([stroke]);
 expectRecoveredCheckpoint(result);
 expect(await reopenedNotebook(page)).toEqual({ids:['replay-page'],strokes:[stroke],name:INITIAL_NAME});
});

test('identical replay retries its failed metadata checkpoint before marking the source received',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);const stroke=await installSource(page,api,true);
 const result=await exerciseFailedReplays(page,true),{first,retry,recovered}=result;
 for(const attempt of [first,retry]){
  expect(attempt.memory).toEqual([stroke]);expect(attempt.stored).toEqual([stroke]);
  expect(attempt.memoryName).toBe(REMOTE_NAME);expect(attempt.name).toBe(INITIAL_NAME);expect(attempt.backupName).toBe(INITIAL_NAME);
  expect(attempt.seen).toBe(0);expect(attempt.rev).toBe(0);expect(attempt.rawCached).toBe(false);
 }
 expect(retry.failedWrites).toBeGreaterThan(first.failedWrites);expect(retry.failedBackups).toBeGreaterThan(first.failedBackups);
 expect(recovered.seen).toBe(1);expect(recovered.rev).toBe(1);expect(recovered.name).toBe(REMOTE_NAME);
 expectRecoveredCheckpoint(result);
 expect(await reopenedNotebook(page)).toEqual({ids:['replay-page'],strokes:[stroke],name:REMOTE_NAME});
});

for(const failure of ['false','throw'])test(`a page write returning ${failure} before its transaction leaves a durable retry pending`,async({page})=>{
 const api=fakeInkDrive();await setup(page,api);const stroke=await installSource(page,api,false);
 const result=await exerciseFailedReplays(page,false,failure),{first,retry,recovered}=result;
 for(const attempt of [first,retry]){
  expect(attempt.memory).toEqual([stroke]);expect(attempt.stored).toEqual([]);
  expect(attempt.seen).toBe(0);expect(attempt.rev).toBe(0);expect(attempt.dirty).toBe(true);
 }
 expect(retry.failedWrites).toBeGreaterThan(first.failedWrites);
 expect(recovered.seen).toBe(1);expect(recovered.rev).toBe(1);expect(recovered.stored).toEqual([stroke]);
 expectRecoveredCheckpoint(result);
 expect(await reopenedNotebook(page)).toEqual({ids:['replay-page'],strokes:[stroke],name:INITIAL_NAME});
});

test('a failed remote checkpoint cannot leak into the next document session',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);await installSource(page,api,true);
 const result=await page.evaluate(async sourceId=>{
  const put=IDBObjectStore.prototype.put,backup=scheduleLocalStorageBackup;
  IDBObjectStore.prototype.put=function(...args){
   if(this.name===DB_STORE)throw new DOMException('Old document metadata unavailable','UnknownError');
   return put.apply(this,args);
  };
  scheduleLocalStorageBackup=()=>false;
  replayJournal.hint(sourceId);const oldJournal=replayJournal;
  try{await oldJournal.readSource(sourceId,oldJournal.sources.get(sourceId));}
  finally{IDBObjectStore.prototype.put=put;scheduleLocalStorageBackup=backup;}
  const oldSeen=oldJournal.seen.get(sourceId)||0,oldSession=getDocumentSessionToken();
  beginDocumentSession();InhouseDriveInk.stop();
  await __IHN_TEST_API__.resetLocalDocument(1,'Other document');InhouseDriveInk.stop();
  state.driveFileId='other-replay-document';state.driveCanEdit=true;state.driveAutosave=false;
  state.pages[0].pageId='other-page';state.collabStructure=null;state.collabFields=null;
  driveDirty=false;resetDriveContentSyncVersion();pageDirty.clear();pageDirtyMode.clear();
  if(!await savePageToIndexedDb(0,state.pages[0]))throw Error('New session page failed');
  const savedAt=Date.now(),payload=buildMetaPayload(savedAt);
  if(!await saveToIndexedDb(payload,savedAt))throw Error('New session metadata failed');
  scheduleLocalStorageBackup(payload,{immediate:true});
  InhouseDriveInk.start();await InhouseDriveInk.capture();
  const writes={page:0,metadata:0},version=driveContentVersion;
  IDBObjectStore.prototype.put=function(...args){
   if(this.name===PAGE_STORE)writes.page++;
   if(this.name===DB_STORE)writes.metadata++;
   return put.apply(this,args);
  };
  try{
   await replayJournal.host.apply([{v:1,pages:buildTimelineSnapshotFromPages(state.pages),
    structure:getCollabStructureSnapshot(),fields:getCollabFieldSnapshot(),name:state.exportName,calendar:null}]);
  }finally{IDBObjectStore.prototype.put=put;}
  const stored=await loadPageFromIndexedDb(0);
  return {oldSeen,sessionChanged:getDocumentSessionToken()!==oldSession,writes,versionChanged:driveContentVersion!==version,
   driveDirty,ids:state.pages.map(p=>p.pageId),strokes:stored.strokes,name:state.exportName};
 },SOURCE);
 expect(result).toEqual({oldSeen:0,sessionChanged:true,writes:{page:0,metadata:0},versionChanged:false,
  driveDirty:false,ids:['other-page'],strokes:[],name:'Other document'});
});

test('a complete local checkpoint repairs failed remote metadata without another peer replay',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);const stroke=await installSource(page,api,true);
 const result=await page.evaluate(async sourceId=>{
  const put=IDBObjectStore.prototype.put,backup=scheduleLocalStorageBackup,versionBefore=driveContentVersion;
  preparedDrivePdf={blob:new Blob(['stale pdf']),version:versionBefore,sessionToken:getDocumentSessionToken()};
  pdfExportCache.set(0,{test:'stale raster'});pdfLibOverlayCache.set(0,{test:'stale overlay'});
  IDBObjectStore.prototype.put=function(...args){
   if(this.name===DB_STORE)throw new DOMException('Remote metadata checkpoint unavailable','UnknownError');
   return put.apply(this,args);
  };
  scheduleLocalStorageBackup=()=>false;
  replayJournal.hint(sourceId);const source=replayJournal.sources.get(sourceId);
  let before,afterOldReceipt;
  try{
   await replayJournal.readSource(sourceId,source);
   before={pending:hasPendingLocalSave(),statusPending:getLocalSaveStatus().pending,seen:replayJournal.seen.get(sourceId)||0};
   // Neither an ordinary local stroke append nor an older network receipt
   // proves that the failed document metadata checkpoint was repaired.
   confirmLocalSave(Date.now(),'stroke');
   replayJournal.host.receipt({docId:state.driveFileId,actor:replayJournal.actor,version:versionBefore,operations:[]});
   afterOldReceipt={pending:hasPendingLocalSave(),statusPending:getLocalSaveStatus().pending,seen:replayJournal.seen.get(sourceId)||0};
  }finally{IDBObjectStore.prototype.put=put;scheduleLocalStorageBackup=backup;}
  queueSave=replayOriginalQueueSave;
  const outcome=await queueSave();
  const stored=await loadPageFromIndexedDb(0),meta=await loadFromIndexedDb();
  const after={pending:hasPendingLocalSave(),statusPending:getLocalSaveStatus().pending,error:getLocalSaveStatus().error,
   seen:replayJournal.seen.get(sourceId)||0,rev:source.rev,version:driveContentVersion,driveDirty,
   pdfCached:!!preparedDrivePdf,rasterCached:pdfExportCache.has(0),overlayCached:pdfLibOverlayCache.has(0),
   stored:stored.strokes.map(cloneStroke),name:meta.data.collabFields.fields[COLLAB_DOC_NAME_KEY].value};
  InhouseDriveInk.stop();return {before,afterOldReceipt,outcome,versionBefore,after};
 },SOURCE);
 expect(result.before).toEqual({pending:true,statusPending:true,seen:0});expect(result.afterOldReceipt).toEqual(result.before);
 expect(result.outcome.ok).toBe(true);
 expect(result.after.pending).toBe(false);expect(result.after.statusPending).toBe(false);expect(result.after.error).toBe('');
 expect(result.after.seen).toBe(0);expect(result.after.rev).toBe(0);
 expect(result.after.driveDirty).toBe(true);expect(result.after.version).toBeGreaterThan(result.versionBefore);
 expect(result.after.pdfCached).toBe(false);expect(result.after.rasterCached).toBe(false);expect(result.after.overlayCached).toBe(false);
 expect(result.after.stored).toEqual([stroke]);expect(result.after.name).toBe(REMOTE_NAME);
 // The button also contains the editable document title. Its center lands
 // inside a long title, where clicks intentionally edit rather than navigate.
 await page.locator('#btn-drive-home > svg').click();await expect(page.locator('#drive-home')).toBeVisible({timeout:3000});
 expect(await reopenedNotebook(page)).toEqual({ids:['replay-page'],strokes:[stroke],name:REMOTE_NAME});
});

async function installTwoPagePhotoSource(page,api,{stalePayload=false,invalidPayload=false}={}){
 const fixture=await page.evaluate(async({name,stalePayload,invalidPayload})=>{
  InhouseDriveInk.stop();
  const canvas=document.createElement('canvas');canvas.width=canvas.height=4;const ctx=canvas.getContext('2d');
  const [oldPhoto,newPhoto,thirdPhoto]=['#cc4411','#2277dd','#44bb66'].map(color=>{
   ctx.fillStyle=color;ctx.fillRect(0,0,4,4);return canvas.toDataURL('image/png');
  });
  const existing=(id,x)=>({id,tool:'pen',color:'#000000',width:2,
   points:[{x,y:10,p:.5},{x:x+10,y:20,p:.5}],syncStamp:{clock:10,actor:'original-writer'}});
  const kept=[existing('kept-first',20),existing('kept-photo-ink',40)];
  state.pages[0].strokes=[kept[0]];
  state.pages.push({...createBlankPageData(),pageId:'second-photo',backgroundSource:'custom',
   backgroundImage:stalePayload?newPhoto:oldPhoto,strokes:[kept[1]]});
  state.collabStructure=null;state.collabFields=null;driveDirty=false;resetDriveContentSyncVersion();
  pageDirty.clear();pageDirtyMode.clear();
  for(let index=0;index<state.pages.length;index++){
   if(!await savePageToIndexedDb(index,state.pages[index]))throw Error('Two-page fixture checkpoint failed');
  }
  const savedAt=Date.now(),meta=buildMetaPayload(savedAt);
  if(!await saveToIndexedDb(meta,savedAt))throw Error('Two-page metadata checkpoint failed');
  scheduleLocalStorageBackup(meta,{immediate:true});
  InhouseDriveInk.start();await InhouseDriveInk.capture();
  const pages=buildTimelineSnapshotFromPages(state.pages);pages[0].strokes.push(replayStroke);
  pages[1].backgroundImage=invalidPayload?thirdPhoto:stalePayload?oldPhoto:newPhoto;
  const fields=getCollabFieldSnapshot(),photoKey=getCollabPageFieldKey('second-photo','background');
  const oldRegister=fields.fields[photoKey];
  const register={value:getCollabBackgroundValue({...pages[1],backgroundImage:newPhoto}),stamp:{clock:100,actor:'remote-writer'}};
  fields.fields[photoKey]=register;fields.fields[COLLAB_DOC_NAME_KEY]={value:name,stamp:{clock:100,actor:'remote-writer'}};
  fields.clock=Math.max(fields.clock,100);
  return {oldPhoto,newPhoto,thirdPhoto,register,oldRegister,kept,remote:replayStroke,
   op:{v:1,pages,fields,structure:getCollabStructureSnapshot(),name,calendar:null}};
 },{name:REMOTE_NAME,stalePayload,invalidPayload});
 api.files.set(SOURCE,{id:SOURCE,appProperties:{ihnDoc:DOCUMENT,ihnKind:'ink-journal-v1'},
  raw:JSON.stringify({v:1,docId:DOCUMENT,actor:'remote-writer',part:0,rev:1,ops:[fixture.op]})});
 return fixture;
}

async function readTwoPagePhotoState(page){
 return page.evaluate(async()=>{
  await ensurePageDataLoaded(0,{keepLoaded:true});await ensurePageDataLoaded(1,{keepLoaded:true});
  const stored=await Promise.all([loadPageFromIndexedDb(0),loadPageFromIndexedDb(1)]),meta=await loadFromIndexedDb();
  const key=getCollabPageFieldKey('second-photo','background');
  return {ids:state.pages.map(p=>p.pageId),strokes:state.pages.map(p=>p.strokes.map(cloneStroke)),
   storedStrokes:stored.map(p=>p.strokes.map(cloneStroke)),photo:state.pages[1].backgroundImage,storedPhoto:stored[1].backgroundImage,
   register:getCollabFieldSnapshot().fields[key],storedRegister:meta.data.collabFields.fields[key],
   name:state.exportName,storedName:meta.data.collabFields.fields[COLLAB_DOC_NAME_KEY].value};
 });
}

for(const manualCheckpoint of [false,true])test(`a partially applied two-page photo edit recovers after ${manualCheckpoint?'a local checkpoint then replay':'direct replay'}`,async({page})=>{
 const api=fakeInkDrive();await setup(page,api);const fixture=await installTwoPagePhotoSource(page,api);
 const result=await page.evaluate(async({sourceId,manualCheckpoint})=>{
  const save=savePageToIndexedDb;
  savePageToIndexedDb=async(index,...args)=>{
   if(index===0)throw Error('First page write interrupted before the second photo was applied');
   return save(index,...args);
  };
  replayJournal.hint(sourceId);const source=replayJournal.sources.get(sourceId);
  try{await replayJournal.readSource(sourceId,source);}finally{savePageToIndexedDb=save;}
  const key=getCollabPageFieldKey('second-photo','background');
  const failed={seen:replayJournal.seen.get(sourceId)||0,rev:source.rev,photo:state.pages[1].backgroundImage,
   register:getCollabFieldSnapshot().fields[key],storedFirst:(await loadPageFromIndexedDb(0)).strokes.map(cloneStroke),
   storedPhoto:(await loadPageFromIndexedDb(1)).backgroundImage};
  let manual=null;
  if(manualCheckpoint){
   queueSave=replayOriginalQueueSave;const outcome=await queueSave();
   manual={ok:outcome.ok,pending:hasPendingLocalSave(),seen:replayJournal.seen.get(sourceId)||0,
    photo:(await loadPageFromIndexedDb(1)).backgroundImage,
    register:(await loadFromIndexedDb()).data.collabFields.fields[key]};
  }
  await replayJournal.readSource(sourceId,source);
  const recovered={seen:replayJournal.seen.get(sourceId)||0,rev:source.rev};
  InhouseDriveInk.stop();return {failed,manual,recovered};
 },{sourceId:SOURCE,manualCheckpoint});
 expect(result.failed).toEqual({seen:0,rev:0,photo:fixture.oldPhoto,register:fixture.oldRegister,
  storedFirst:[fixture.kept[0]],storedPhoto:fixture.oldPhoto});
 if(manualCheckpoint)expect(result.manual).toEqual({ok:true,pending:false,seen:0,photo:fixture.oldPhoto,register:fixture.oldRegister});
 expect(result.recovered).toEqual({seen:1,rev:1});
 const strokes=[[fixture.kept[0],fixture.remote],[fixture.kept[1]]];
 const expected={ids:['replay-page','second-photo'],strokes,storedStrokes:strokes,photo:fixture.newPhoto,storedPhoto:fixture.newPhoto,
  register:fixture.register,storedRegister:fixture.register,name:REMOTE_NAME,storedName:REMOTE_NAME};
 expect(await readTwoPagePhotoState(page)).toEqual(expected);
 await page.reload();await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(()=>{InhouseDriveInk.stop();stopCollabPolling();});
 expect(await readTwoPagePhotoState(page)).toEqual(expected);
});

test('a newer photo register with mismatched payload bytes cannot replace the matching local photo',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);const fixture=await installTwoPagePhotoSource(page,api,{stalePayload:true});
 await page.evaluate(async sourceId=>{
  replayJournal.hint(sourceId);await replayJournal.readSource(sourceId,replayJournal.sources.get(sourceId));InhouseDriveInk.stop();
 },SOURCE);
 const actual=await readTwoPagePhotoState(page);
 expect(actual.photo).toBe(fixture.newPhoto);expect(actual.storedPhoto).toBe(fixture.newPhoto);
 expect(actual.register).toEqual(fixture.register);expect(actual.storedRegister).toEqual(fixture.register);
 expect(actual.strokes).toEqual([[fixture.kept[0],fixture.remote],[fixture.kept[1]]]);
});

test('a winning photo register without matching local or incoming bytes stays unacknowledged and consistent',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);const fixture=await installTwoPagePhotoSource(page,api,{invalidPayload:true});
 const result=await page.evaluate(async sourceId=>{
  replayJournal.hint(sourceId);const source=replayJournal.sources.get(sourceId),key=getCollabPageFieldKey('second-photo','background');
  const receipt=()=>({seen:replayJournal.seen.get(sourceId)||0,rev:source.rev,rawCached:!!source.raw,
   photo:state.pages[1].backgroundImage,register:getCollabFieldSnapshot().fields[key]});
  await replayJournal.readSource(sourceId,source);const failed=receipt();
  queueSave=replayOriginalQueueSave;const outcome=await queueSave();
  const persisted={ok:outcome.ok,photo:(await loadPageFromIndexedDb(1)).backgroundImage,
   register:(await loadFromIndexedDb()).data.collabFields.fields[key]};
  await replayJournal.readSource(sourceId,source);const retry=receipt();
  InhouseDriveInk.stop();return {failed,persisted,retry};
 },SOURCE);
 const unacknowledged={seen:0,rev:0,rawCached:false,photo:fixture.oldPhoto,register:fixture.oldRegister};
 expect(result.failed).toEqual(unacknowledged);expect(result.retry).toEqual(unacknowledged);
 expect(result.persisted).toEqual({ok:true,photo:fixture.oldPhoto,register:fixture.oldRegister});
 await page.reload();await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(()=>{InhouseDriveInk.stop();stopCollabPolling();});
 const reopened=await readTwoPagePhotoState(page);
 expect(reopened.photo).toBe(fixture.oldPhoto);expect(reopened.storedPhoto).toBe(fixture.oldPhoto);
 expect(reopened.register).toEqual(fixture.oldRegister);expect(reopened.storedRegister).toEqual(fixture.oldRegister);
 expect(reopened.strokes).toEqual([[fixture.kept[0],fixture.remote],[fixture.kept[1]]]);
});

test('manual saving defers unregistered photo bytes after a partial merge until a stamped edit resolves them',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);const fixture=await installTwoPagePhotoSource(page,api);
 const result=await page.evaluate(async({sourceId,thirdPhoto})=>{
  const save=savePageToIndexedDb,key=getCollabPageFieldKey('second-photo','background');
  savePageToIndexedDb=async(index,...args)=>{
   if(index===0){state.pages[1].backgroundImage=thirdPhoto;throw Error('Photo replaced before its causal field was stamped');}
   return save(index,...args);
  };
  replayJournal.hint(sourceId);const source=replayJournal.sources.get(sourceId);
  try{await replayJournal.readSource(sourceId,source);}finally{savePageToIndexedDb=save;}
  const failed={photo:state.pages[1].backgroundImage,register:getCollabFieldSnapshot().fields[key]};
  const outcome=await saveToStorage(),meta=await loadFromIndexedDb();
  const deferred={ok:outcome.ok,deferred:outcome.deferred,pending:hasPendingLocalSave(),
   register:meta.data.collabFields.fields[key],name:meta.data.collabFields.fields[COLLAB_DOC_NAME_KEY].value,
   seen:replayJournal.seen.get(sourceId)||0};
  // A real local photo edit supplies its own causal register, removing the
  // ambiguity without replacing its bytes with either older remote image.
  noteCollabPageFieldChanged(state.pages[1],'background');markPageDirty(1);
  const expected=getCollabFieldSnapshot().fields[key],resolved=await saveToStorage();
  const stored=await loadPageFromIndexedDb(1),storedMeta=await loadFromIndexedDb();
  const repaired={ok:resolved.ok,pending:hasPendingLocalSave(),photo:stored.backgroundImage,
   register:storedMeta.data.collabFields.fields[key]};
  await replayJournal.readSource(sourceId,source);
  const replayed={seen:replayJournal.seen.get(sourceId)||0,photo:state.pages[1].backgroundImage,
   register:getCollabFieldSnapshot().fields[key]};
  InhouseDriveInk.stop();return {failed,deferred,expected,repaired,replayed};
 },{sourceId:SOURCE,thirdPhoto:fixture.thirdPhoto});
 expect(result.failed).toEqual({photo:fixture.thirdPhoto,register:fixture.register});
 expect(result.deferred).toEqual({ok:false,deferred:true,pending:true,register:fixture.oldRegister,name:INITIAL_NAME,seen:0});
 expect(result.repaired).toEqual({ok:true,pending:false,photo:fixture.thirdPhoto,register:result.expected});
 expect(result.replayed).toEqual({seen:1,photo:fixture.thirdPhoto,register:result.expected});
});
