import {test,expect} from '@playwright/test';
import {fakeInkDrive} from '../helpers/fake-ink-drive.mjs';

const DOCUMENT='cache-invalidation',SOURCE='cache-source';
const IDS=['cache-ink','cache-photo','cache-pdf'];

async function setup(page,api,{duplicate=false,agenda=false}={}){
 await page.exposeFunction('cacheInkRequest',(url,options)=>api.request(url,options));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 return page.evaluate(async({documentId,ids,duplicate,agenda})=>{
  await InhouseOfflineLibrary.useAccount('cache-invalidation@example.test');
  await __IHN_TEST_API__.resetLocalDocument(3,agenda?'Agenda 2026-2027':'Cache validity');
  await __IHN_TEST_API__.showEditorForTest();
  stopCollabPolling();beginDocumentSession();InhouseDriveInk.stop();
  state.driveFileId=documentId;state.driveFileName='';state.driveCanEdit=true;state.driveAutosave=false;
  state.driveFolderKind=agenda?'agenda':null;state.calendarPageConfig=agenda?{mode:'week',startPage:0,startDateKey:'2026-09-07',nextDateKey:'2026-09-28'}:null;
  const canvas=document.createElement('canvas');canvas.width=canvas.height=4;
  const ctx=canvas.getContext('2d');ctx.fillStyle='#315bdb';ctx.fillRect(0,0,4,4);
  const photo=canvas.toDataURL();
  state.pages.forEach((p,i)=>{p.pageId=ids[i];p.unloaded=false;p.pendingUnload=false;});
  state.pages[1].backgroundSource='custom';state.pages[1].backgroundImage=photo;
  state.pages[2].backgroundSource='pdf';state.pages[2].pdfPageIndex=1;state.pages[2].backgroundImage=null;
  const pdf=new jspdf.jsPDF();pdf.text('Unchanged PDF background',20,30);state.cleanOriginalPdfBytes=pdf.output('arraybuffer');
  window.cacheStroke={id:'incoming-cache-ink',tool:'pen',color:'#0000ff',width:2,
   points:[{x:30,y:40,p:.5},{x:55,y:65,p:.75}],syncStamp:{clock:100,actor:'cache-remote'}};
  if(duplicate)state.pages[1].strokes=[{...cacheStroke,points:[{x:10,y:10,p:.5},{x:20,y:20,p:.5}],syncStamp:{clock:10,actor:'cache-original'}}];
  state.collabStructure=null;state.collabFields=null;driveDirty=false;resetDriveContentSyncVersion();pageDirty.clear();pageDirtyMode.clear();
  for(let i=0;i<state.pages.length;i++)if(!await savePageToIndexedDb(i,state.pages[i]))throw Error('Initial page checkpoint failed');
  const at=Date.now(),meta=buildMetaPayload(at);
  if(!await saveToIndexedDb(meta,at))throw Error('Initial metadata checkpoint failed');
  if(!scheduleLocalStorageBackup(meta,{immediate:true}))throw Error('Initial backup failed');
  queueSave=async()=>({ok:false,deferred:true});window.cacheDriveSchedules=0;
  scheduleDriveSync=()=>{cacheDriveSchedules++;};schedulePreparedDrivePdf=()=>{};
  scheduleLiveDocumentBroadcast=()=>{};startLiveCollaboration=()=>{};
  driveFetch=async(url,options={})=>{
   const body=typeof options.body==='string'?options.body:await options.body?.text?.();
   const r=await cacheInkRequest(url,{method:options.method,headers:options.headers,body});
   return new Response(r.body,{status:r.status,headers:{'Content-Type':'application/json'}});
  };
  const Journal=InhouseDriveInkJournal.Journal;
  InhouseDriveInkJournal.Journal=class extends Journal{
   constructor(host){super({...host,actor:'cache-reader'});window.cacheJournal=this;}
   start(){return this;}wake(){}
  };
  InhouseDriveLivePreview=null;InhouseDriveInk.start();await InhouseDriveInk.capture();
  const apply=applyRemotePages;window.cacheMergeResults=[];
  applyRemotePages=async(...args)=>{const r=await apply(...args);cacheMergeResults.push(r);return r;};
  const invalidate=invalidatePageMutationCaches;window.cacheInvalidations=[];
  invalidatePageMutationCaches=index=>{cacheInvalidations.push(state.pages[index]?.pageId);return invalidate(index);};
  // Disable only the thumbnail queue during strict cache measurements; real
  // merge rendering, page persistence and acknowledgment remain in operation.
  resetScrollPagePreviews();scheduleScrollPagePreviews=()=>{};
  window.seedCacheProbe=()=>{
   resetScrollPagePreviews();cacheInvalidations.length=0;
   window.cacheProbe=state.pages.map((p,i)=>{
    const raster={kind:'raster',id:p.pageId},overlay={kind:'overlay',id:p.pageId};
    pdfExportCache.set(i,raster);pdfLibOverlayCache.set(i,overlay);
    p.preview=photo;p.scrollPreviewDirty=false;p.scrollPreviewVerified=true;
    return {id:p.pageId,raster,overlay,preview:p.preview,generation:pageDirtyGeneration.get(i)||0};
   });
   preparedDrivePdf={blob:new Blob(['cached whole document']),version:driveContentVersion,sessionToken:getDocumentSessionToken()};
  };
  window.inspectCacheProbe=()=>state.pages.map((p,i)=>{
   const previous=cacheProbe.find(saved=>saved.id===p.pageId);
   return {id:p.pageId,rasterSame:pdfExportCache.get(i)===previous?.raster,overlaySame:pdfLibOverlayCache.get(i)===previous?.overlay,
    rasterPresent:pdfExportCache.has(i),overlayPresent:pdfLibOverlayCache.has(i),previewSame:p.preview===previous?.preview,
    previewDirty:!!p.scrollPreviewDirty,generationSame:(pageDirtyGeneration.get(i)||0)===previous?.generation};
  });
  window.cacheOperation=()=>({v:1,kind:'stroke',pageId:ids[0],stroke:cacheStroke});
  window.cacheContextOp=()=>({v:1,pages:[],structure:getCollabStructureSnapshot(),fields:getCollabFieldSnapshot(),name:state.exportName,calendar:state.calendarPageConfig||null});
  seedCacheProbe();return {photo,stroke:cacheStroke};
 },{documentId:DOCUMENT,ids:IDS,duplicate,agenda});
}

async function installOperation(page,api,id=SOURCE,operation=null){
 const op=operation||await page.evaluate(()=>cacheOperation());
 api.files.set(id,{id,appProperties:{ihnDoc:DOCUMENT,ihnKind:'ink-journal-v1'},
  raw:JSON.stringify({v:1,docId:DOCUMENT,actor:'cache-remote',part:0,rev:1,ops:[op]})});
 return op;
}
async function readSource(page,id=SOURCE){
 return page.evaluate(async id=>{
  cacheJournal.hint(id);await cacheJournal.readSource(id,cacheJournal.sources.get(id));
  return {seen:cacheJournal.seen.get(id)||0,results:cacheMergeResults,invalidations:cacheInvalidations.slice(),
   caches:inspectCacheProbe(),version:driveContentVersion,prepared:!!preparedDrivePdf,driveDirty,schedules:cacheDriveSchedules};
 },id);
}
function expectUntouched(entry){expect(entry).toMatchObject({rasterSame:true,overlaySame:true,previewSame:true,previewDirty:false,generationSame:true});}
function expectInvalidated(entry){expect(entry).toMatchObject({rasterPresent:false,overlayPresent:false,generationSame:false});}
async function durablePages(page){
 return page.evaluate(async()=>{
  const result=[];
  for(let i=0;i<state.pages.length;i++){
   const p=await loadPageFromIndexedDb(i);result.push({id:p.pageId,strokes:p.strokes.map(cloneStroke),deleted:p.deletedStrokeIds||[],
    background:p.backgroundSource,photo:p.backgroundImage||null,pdfPageIndex:p.pdfPageIndex});
  }
  return result;
 });
}

test('one incoming stroke preserves unrelated photo and PDF caches and duplicate sources do no extra work',async({page})=>{
 const api=fakeInkDrive(),fixture=await setup(page,api);await installOperation(page,api);
 const result=await readSource(page);
 expect(result.seen).toBe(1);expect(result.results.at(-1).invalidatedPageIds).toEqual([IDS[0]]);
 expect(result.invalidations).toEqual([IDS[0]]);expectInvalidated(result.caches[0]);result.caches.slice(1).forEach(expectUntouched);
 expect(result.prepared).toBe(false);expect(result.driveDirty).toBe(true);expect(result.schedules).toBeGreaterThan(0);
 const stored=await durablePages(page);expect(stored[0].strokes).toEqual([fixture.stroke]);
 expect(stored[1]).toMatchObject({background:'custom',photo:fixture.photo});expect(stored[2]).toMatchObject({background:'pdf',pdfPageIndex:1});
 await page.evaluate(()=>seedCacheProbe());await installOperation(page,api,'duplicate-cache-source');
 const duplicate=await readSource(page,'duplicate-cache-source');
 expect(duplicate.seen).toBe(1);expect(duplicate.invalidations).toEqual([]);expect(duplicate.version).toBe(result.version);
 duplicate.caches.forEach(expectUntouched);
 await page.reload();await page.evaluate(()=>__IHN_TEST_API__.ready());
 expect(await durablePages(page)).toEqual(stored);
});

test('moving one item invalidates the losing page even when that page is absent from the packet',async({page})=>{
 const api=fakeInkDrive(),fixture=await setup(page,api,{duplicate:true});await installOperation(page,api);
 const result=await readSource(page),stored=await durablePages(page);
 expect(result.seen).toBe(1);expect(result.results.at(-1).invalidatedPageIds.sort()).toEqual(IDS.slice(0,2).sort());
 expect(result.invalidations.sort()).toEqual(IDS.slice(0,2).sort());result.caches.slice(0,2).forEach(expectInvalidated);expectUntouched(result.caches[2]);
 expect(stored[0].strokes).toEqual([fixture.stroke]);expect(stored[1].strokes).toEqual([]);expect(stored[1].deleted).toContain(fixture.stroke.id);
 expect(stored[1].photo).toBe(fixture.photo);
});

for(const kind of ['reorder','legacy','rename','calendar','sidePanel'])test(`incoming ${kind} conservatively invalidates every page`,async({page})=>{
 const api=fakeInkDrive(),fixture=await setup(page,api,{agenda:['rename','calendar','sidePanel'].includes(kind)});
 const op=await page.evaluate(kind=>{
  const op=cacheContextOp(),snapshot=buildTimelineSnapshotFromPages(state.pages);
  if(kind==='reorder'){
   const ids=state.pages.map(p=>p.pageId),next=[ids[2],ids[0],ids[1]];
   op.structure=ihnRecordStructureReplacement(op.structure,ids,next,'cache-remote',100);
   op.pages=next.map(id=>snapshot.find(p=>p.pageId===id));
  }else if(kind==='legacy'){
   op.pages=snapshot.map(p=>{const copy={...p};delete copy.pageId;return copy;});op.pages[0].strokes=[cacheStroke];
  }else if(kind==='rename'){
   op.name='Ordinary notes';op.fields.fields[COLLAB_DOC_NAME_KEY]={value:op.name,stamp:{clock:100,actor:'cache-remote'}};
   op.fields.clock=100;
  }else if(kind==='sidePanel'){
   const panel={title:'New calendar panel',dateKeys:['2026-09-07'],html:'<p>New calendar panel</p>'};
   op.pages=[{...snapshot[0],sidePanel:panel}];
   op.fields.fields[getCollabPageFieldKey(state.pages[0].pageId,'sidePanel')]={value:panel,stamp:{clock:100,actor:'cache-remote'}};op.fields.clock=100;
  }else{
   op.calendar={mode:'day',startPage:0,startDateKey:'2026-09-07',nextDateKey:'2026-09-10'};
   op.fields.fields[COLLAB_DOC_CALENDAR_KEY]={value:op.calendar,stamp:{clock:100,actor:'cache-remote'}};op.fields.clock=100;
  }
  return op;
 },kind);
 await installOperation(page,api,SOURCE,op);const result=await readSource(page);
 expect(result.seen).toBe(1);expect(result.results.at(-1).invalidatedPageIds).toBeNull();result.caches.forEach(expectInvalidated);
 // Legacy alignment legitimately adopts deterministic page identities. All
 // resulting pages must be invalidated, not just the identities at entry.
 expect(result.caches).toHaveLength(3);
 expect(new Set(result.invalidations)).toEqual(new Set(result.caches.map(p=>p.id)));
 if(kind==='legacy'){
  const stored=await durablePages(page);
  expect(new Set(stored.map(p=>p.id)).size).toBe(3);
  expect(stored[0].strokes).toEqual([fixture.stroke]);expect(stored[1].photo).toBe(fixture.photo);
  expect(stored[2]).toMatchObject({background:'pdf',pdfPageIndex:1});
 }
 if(kind==='reorder')expect((await durablePages(page)).map(p=>p.id)).toEqual([IDS[2],IDS[0],IDS[1]]);
 if(kind==='rename')expect(await page.evaluate(()=>({name:state.exportName,calendar:canUseAgendaCalendarForCurrentDocument()}))).toEqual({name:'Ordinary notes',calendar:false});
});

test('an ordinary stroke in an unchanged calendar keeps unrelated page caches',async({page})=>{
 const api=fakeInkDrive();await setup(page,api,{agenda:true});await installOperation(page,api);const result=await readSource(page);
 expect(result.seen).toBe(1);expect(result.results.at(-1).invalidatedPageIds).toEqual([IDS[0]]);result.caches.slice(1).forEach(expectUntouched);
});

test('recovery after failed metadata invalidates every cache before acknowledging the source',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);await installOperation(page,api);
 const failed=await page.evaluate(async id=>{
  const put=IDBObjectStore.prototype.put,backup=scheduleLocalStorageBackup;
  IDBObjectStore.prototype.put=function(...args){if(this.name===DB_STORE)throw Error('Injected metadata failure');return put.apply(this,args);};
  scheduleLocalStorageBackup=()=>false;cacheJournal.hint(id);
  try{await cacheJournal.readSource(id,cacheJournal.sources.get(id));}
  finally{IDBObjectStore.prototype.put=put;scheduleLocalStorageBackup=backup;}
  const result={seen:cacheJournal.seen.get(id)||0,pending:hasPendingLocalSave()};seedCacheProbe();return result;
 },SOURCE);
 expect(failed).toEqual({seen:0,pending:true});const recovered=await readSource(page);
 expect(recovered.seen).toBe(1);expect(recovered.results.at(-1).invalidatedPageIds).toBeNull();recovered.caches.forEach(expectInvalidated);
 expect(new Set(recovered.invalidations)).toEqual(new Set(IDS));
});

test('direct P2P merge returns its targeted cache proof and exact content hash without adapter invalidation',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);
 const result=await page.evaluate(async()=>{
  const patch=buildTimelineSnapshotFromPages([{...state.pages[0],strokes:[cacheStroke]}])[0],op=cacheContextOp();
  const merged=await applyRemotePages([patch],{sessionToken:getDocumentSessionToken(),fileId:state.driveFileId,
   additiveById:true,preserveLocalUnsynced:true,remoteStructure:op.structure,remoteFields:op.fields,
   remoteExportName:op.name,remoteCalendarPageConfig:op.calendar,captureContentHash:true});
  return {merged,caches:inspectCacheProbe(),invalidations:cacheInvalidations,
   expected:ihnCanonicalDocumentHash(state.pages.map(p=>sanitizePageForStorage(p)),getCollabStructureSnapshot(),state.calendarPageConfig,state.exportName,getCollabFieldSnapshot())};
 });
 expect(result.merged.invalidatedPageIds).toEqual([IDS[0]]);expect(result.merged.contentHash).toBe(result.expected);
 expect(result.invalidations).toEqual([IDS[0]]);result.caches.slice(1).forEach(expectUntouched);
});

test('adapter falls back safely for older or malformed proofs without repeating completed invalidation',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);
 for(const proof of ['missing','undefined','false','number-id','null-id','empty-id','array','null']){
  const result=await page.evaluate(async proof=>{
   seedCacheProbe();
   applyRemotePages=async()=>{
    if(proof==='null')state.pages.forEach((_,i)=>invalidatePageMutationCaches(i));
    const values={undefined, false:false, 'number-id':[123], 'null-id':[null], 'empty-id':[''], array:[], null:null};
    return {changed:true,...(proof==='missing'?{}:{invalidatedPageIds:values[proof]})};
   };
   await cacheJournal.host.apply([cacheOperation()]);return cacheInvalidations;
  },proof);
  expect(result,proof).toEqual(proof==='array'?[]:IDS);
 }
});
