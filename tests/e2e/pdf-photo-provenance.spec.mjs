import {test,expect} from '@playwright/test';
import {fakeInkDrive} from '../helpers/fake-ink-drive.mjs';

const DOCUMENT='pdf-photo-provenance',SOURCE='raw-photo-source';

async function setup(page,api){
 await page.route('https://accounts.google.com/**',route=>route.fulfill({body:''}));
 await page.exposeFunction('pdfPhotoInkRequest',(url,options)=>api.request(url,options));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 return page.evaluate(async documentId=>{
  await InhouseOfflineLibrary.useAccount('pdf-photo@example.test');
  await __IHN_TEST_API__.resetLocalDocument(1,'Photo notebook');
  stopCollabPolling();beginDocumentSession();InhouseDriveInk.stop();
  state.driveFileId=documentId;state.driveCanEdit=true;state.driveAutosave=false;
  state.pages[0].pageId='pdf-photo-page';state.collabStructure=null;state.collabFields=null;
  const canvas=document.createElement('canvas');canvas.width=canvas.height=12;const ctx=canvas.getContext('2d');
  const [localPhoto,remotePhoto]=['#cc3322','#27b74a'].map(color=>{ctx.fillStyle=color;ctx.fillRect(0,0,12,12);return canvas.toDataURL();});
  const stroke=id=>({id,tool:'pen',color:'#000000',width:2,points:[{x:15,y:15,p:.5},{x:35,y:35,p:.5}],syncStamp:{clock:100,actor:id}});
  const remoteStroke=stroke('pdf-remote-ink'),localStroke=stroke('pdf-local-ink');
  state.pages[0].backgroundSource='custom';state.pages[0].backgroundImage=remotePhoto;
  state.pages[0].strokes=[remoteStroke];state.pages[0].needsRedraw=true;
  noteCollabPageFieldChanged(state.pages[0],'background');
  schedulePreparedDrivePdf=()=>{};scheduleDriveSync=()=>{};scheduleLiveDocumentBroadcast=()=>{};startLiveCollaboration=()=>{};
  const blob=await buildPdfBlob({status:false});
  const pdfDocument=await pdfjsLib.getDocument({data:await blob.arrayBuffer()}).promise;
  const keywords=(await pdfDocument.getMetadata()).info.Keywords;
  const pages=await decodeEmbeddedStrokesFromPdfKeywords(keywords),fields=decodeCollabFieldsFromKeywords(keywords,{throwOnInvalid:true});
  const structure=decodeCollabStructureFromKeywords(keywords,pages.map(p=>p.pageId),{throwOnInvalid:true});
  await pdfDocument.destroy();
  await __IHN_TEST_API__.resetLocalDocument(1,'Photo notebook');
  stopCollabPolling();beginDocumentSession();InhouseDriveInk.stop();
  state.driveFileId=documentId;state.driveFileName='Photo notebook.pdf';state.driveCanEdit=true;state.driveAutosave=false;
  state.driveHeadRevisionId='photo-head-1';state.driveRevisionVersion='1';state.driveLastKnownModified=0;
  driveAccessToken='fixture-token';driveTokenExpiry=Date.now()+600000;
  state.pages[0].pageId='pdf-photo-page';state.pages[0].backgroundSource='custom';state.pages[0].backgroundImage=localPhoto;
  state.pages[0].strokes=[localStroke];state.collabStructure=null;state.collabFields=null;
  driveDirty=false;resetDriveContentSyncVersion();pageDirty.clear();pageDirtyMode.clear();
  if(!await savePageToIndexedDb(0,state.pages[0]))throw Error('Local photo checkpoint failed');
  const savedAt=Date.now(),payload=buildMetaPayload(savedAt);
  if(!await saveToIndexedDb(payload,savedAt)||!scheduleLocalStorageBackup(payload,{immediate:true}))throw Error('Local metadata checkpoint failed');
  const key=getCollabPageFieldKey(state.pages[0].pageId,'background'),localRegister=getCollabFieldSnapshot().fields[key];
  const meta={id:documentId,version:'2',headRevisionId:'photo-head-2',modifiedTime:'2026-10-27T12:00:00.000Z'};
  window.pdfPhotoFixture={blob,pages,fields,structure,meta,localPhoto,localRegister,localStroke,remoteStroke};
  driveFetch=async(url,options={})=>{
   const parsed=new URL(url);
   if(parsed.pathname.endsWith('/'+documentId)&&!parsed.searchParams.has('alt'))return new Response(JSON.stringify(meta),{headers:{'Content-Type':'application/json'}});
   const body=typeof options.body==='string'?options.body:await options.body?.text?.();
   const r=await pdfPhotoInkRequest(url,{method:options.method,headers:options.headers,body});
   return new Response(r.body,{status:r.status,headers:{'Content-Type':'application/json'}});
  };
  queueSave=async()=>({ok:false,deferred:true});
  const Journal=InhouseDriveInkJournal.Journal;
  InhouseDriveInkJournal.Journal=class extends Journal{
   constructor(host){super({...host,actor:'pdf-photo-reader'});window.pdfPhotoJournal=this;}
   start(){return this;}wake(){}
  };
  InhouseDriveLivePreview=null;InhouseDriveInk.start();await InhouseDriveInk.capture();
  const render=renderPdfPageToBackground;window.pdfPhotoRenders=0;
  renderPdfPageToBackground=async(...args)=>{pdfPhotoRenders++;return render(...args);};
  window.pullPhotoFixture=()=>pullRemoteChanges(meta,{allowWhileLocalPending:true,preserveLocalUnsynced:true,uploadManagedExternally:true,
   downloadedSnapshot:{fileId:documentId,blob,meta}});
  window.photoState=async()=>{
   const p=state.pages[0],stored=await loadPageFromIndexedDb(0),meta=await loadFromIndexedDb();
   let pixel=null;
   if(p.backgroundImage){const img=new Image();img.src=p.backgroundImage;await img.decode();const canvas=document.createElement('canvas');canvas.width=canvas.height=1;const ctx=canvas.getContext('2d');ctx.drawImage(img,0,0,1,1);pixel=[...ctx.getImageData(0,0,1,1).data];}
   return {photo:p.backgroundImage,pixel,source:p.backgroundSource,pdfPageIndex:p.pdfPageIndex,
    ids:p.strokes.map(s=>s.id).sort(),storedIds:stored.strokes.map(s=>s.id).sort(),
    register:getCollabFieldSnapshot().fields[key],storedRegister:meta.data.collabFields.fields[key],
    renders:pdfPhotoRenders,pending:hasPendingLocalSave(),revision:state.driveHeadRevisionId};
  };
  return {localPhoto,localRegister,remoteStroke,localStroke,operation:{v:1,pages,fields,structure,name:'Photo notebook',calendar:null},
   pdfRepresentation:{source:pages[0].backgroundSource,pdfPageIndex:pages[0].pdfPageIndex,inline:pages[0].backgroundImage||null,
    hash:fields.fields[key].value.backgroundImageHash}};
 },DOCUMENT);
}

function expectGreen(pixel){expect(pixel[0]).toBeLessThan(80);expect(pixel[1]).toBeGreaterThan(140);expect(pixel[2]).toBeLessThan(110);}
function expectRawPreserved(actual,fixture){
 expect(actual.photo).toBe(fixture.localPhoto);expect(actual.register).toEqual(fixture.localRegister);
 expect(actual.storedRegister).toEqual(fixture.localRegister);expect(actual.ids).toEqual([fixture.localStroke.id]);
 expect(actual.renders).toBe(0);expect(actual.revision).toBe('photo-head-1');
}

test('a stable real PDF supplies its embedded photo without inline JSON image bytes',async({page})=>{
 const api=fakeInkDrive(),fixture=await setup(page,api);
 expect(fixture.pdfRepresentation).toMatchObject({source:'custom',pdfPageIndex:1,inline:null});expect(fixture.pdfRepresentation.hash).toBeTruthy();
 const result=await page.evaluate(async()=>{const result=await pullPhotoFixture();return {error:!!result.error,deferred:!!result.deferred,state:await photoState()};});
 expect(result.error).toBe(false);expect(result.deferred).toBe(false);expectGreen(result.state.pixel);
 expect(result.state.ids).toEqual(['pdf-local-ink','pdf-remote-ink']);expect(result.state.storedIds).toEqual(result.state.ids);
 expect(result.state.renders).toBeGreaterThan(0);expect(result.state.revision).toBe('photo-head-2');
});

test('journal and direct P2P cannot authorize missing photo bytes with PDF locators or proof-like flags',async({page})=>{
 const api=fakeInkDrive(),fixture=await setup(page,api);
 fixture.operation.pages[0]={...fixture.operation.pages[0],verifiedPdfBackground:true,trustedPdf:true,pdfProof:{pdfPageIndex:1}};
 api.files.set(SOURCE,{id:SOURCE,appProperties:{ihnDoc:DOCUMENT,ihnKind:'ink-journal-v1'},
  raw:JSON.stringify({v:1,docId:DOCUMENT,actor:'spoofed-pdf',part:0,rev:1,ops:[fixture.operation]})});
 const result=await page.evaluate(async({source,operation})=>{
  pdfPhotoJournal.hint(source);const reader=pdfPhotoJournal.sources.get(source);await pdfPhotoJournal.readSource(source,reader);
  const journal={seen:pdfPhotoJournal.seen.get(source)||0,rev:reader.rev,state:await photoState()};
  let error='';
  try{await applyRemotePages(operation.pages,{sessionToken:getDocumentSessionToken(),fileId:state.driveFileId,
   remoteFields:operation.fields,remoteStructure:operation.structure,remoteExportName:operation.name,
   preserveLocalUnsynced:true,additiveById:true,captureContentHash:true,
   verifiedPdfBackground:true,trustedPdf:true,remotePdfDoc:{numPages:1},pdfBackgroundProof:{pdfPageIndex:1}});}catch(e){error=e.message;}
  return {journal,p2p:{error,state:await photoState()}};
 },{source:SOURCE,operation:fixture.operation});
 expect(result.journal.seen).toBe(0);expect(result.journal.rev).toBe(0);expectRawPreserved(result.journal.state,fixture);
 expect(result.p2p.error).toContain('photo does not match');expectRawPreserved(result.p2p.state,fixture);
});

for(const mutation of ['options identity','page identity','page index','background record','inline image'])test(`trusted PDF authorization does not survive altered ${mutation}`,async({page})=>{
 const api=fakeInkDrive(),fixture=await setup(page,api);
 const result=await page.evaluate(async mutation=>{
  const apply=applyRemotePages;
  applyRemotePages=async(pages,options)=>{
   if(mutation==='options identity')options={...options};
   if(mutation==='page identity')pages=pages.map(p=>({...p}));
   if(mutation==='page index')pages[0].pdfPageIndex=2;
   if(mutation==='inline image')pages[0].backgroundImage=pdfPhotoFixture.localPhoto;
   if(mutation==='background record'){
    const key=getCollabPageFieldKey(pages[0].pageId,'background'),record=options.remoteFields.fields[key];
    options.remoteFields.fields[key]={...record,value:{...record.value,backgroundImageHash:'wrong-registered-photo'}};
   }
   return apply(pages,options);
  };
  try{const result=await pullPhotoFixture();return {error:!!result.error,cause:result.cause?.message,state:await photoState()};}
  finally{applyRemotePages=apply;}
 },mutation);
 expect(result.error).toBe(true);expect(result.cause).toContain('photo does not match');expectRawPreserved(result.state,fixture);
});

test('a failed pull retires its exact options authorization before any direct replay',async({page})=>{
 const api=fakeInkDrive(),fixture=await setup(page,api);
 const result=await page.evaluate(async()=>{
  const apply=applyRemotePages;let capturedPages,capturedOptions;
  applyRemotePages=async(pages,options)=>{capturedPages=pages;capturedOptions=options;throw Error('Interrupted before applying the PDF');};
  let first;
  try{first=await pullPhotoFixture();}finally{applyRemotePages=apply;}
  let replayError='';try{await applyRemotePages(capturedPages,capturedOptions);}catch(error){replayError=error.message;}
  return {first:!!first.error,replayError,state:await photoState()};
 });
 expect(result.first).toBe(true);expect(result.replayError).toContain('photo does not match');expectRawPreserved(result.state,fixture);
});

test('an identical PDF replay after a failed metadata checkpoint still renders its missing photo',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);
 const result=await page.evaluate(async()=>{
  const put=IDBObjectStore.prototype.put,backup=scheduleLocalStorageBackup;
  IDBObjectStore.prototype.put=function(...args){if(this.name===DB_STORE)throw Error('Injected PDF metadata checkpoint failure');return put.apply(this,args);};
  scheduleLocalStorageBackup=()=>false;let first;
  try{first=await pullPhotoFixture();}finally{IDBObjectStore.prototype.put=put;scheduleLocalStorageBackup=backup;}
  const before={error:!!first.error,photo:state.pages[0].backgroundImage,renders:pdfPhotoRenders,pending:hasPendingLocalSave()};
  const retry=await pullPhotoFixture();return {before,retry:{error:!!retry.error,deferred:!!retry.deferred,state:await photoState()}};
 });
 expect(result.before).toEqual({error:true,photo:null,renders:0,pending:true});
 expect(result.retry.error).toBe(false);expect(result.retry.deferred).toBe(false);expectGreen(result.retry.state.pixel);
 expect(result.retry.state.renders).toBeGreaterThan(0);expect(result.retry.state.ids).toEqual(['pdf-local-ink','pdf-remote-ink']);
 expect(result.retry.state.revision).toBe('photo-head-2');
});
