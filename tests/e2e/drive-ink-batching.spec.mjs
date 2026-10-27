import { test, expect } from '@playwright/test';

async function setup(page) {
 await page.goto('/?e2e=1'); await page.evaluate(() => __IHN_TEST_API__.ready());
 await page.evaluate(async () => {
  await InhouseOfflineLibrary.useAccount('ink-batching@example.test');
  await __IHN_TEST_API__.resetLocalDocument(1, 'Ink batching');
  await __IHN_TEST_API__.showEditorForTest();
  stopCollabPolling(); beginDocumentSession(); InhouseDriveInk.stop();
  state.driveFileId='ink-batching'; state.driveCanEdit=true; state.driveAutosave=false;
  state.pages[0].pageId='batch-page'; state.collabStructure=null; state.collabFields=null;
  driveDirty=false; resetDriveContentSyncVersion();
  const Journal=InhouseDriveInkJournal.Journal;
  InhouseDriveInkJournal.Journal=class extends Journal {
   constructor(host){super({...host,actor:'batch-local'});window.batchJournal=this;}
   start(){return this;} wake(){}
  };
  // Isolate the real adapter and merge/persistence path from network activity.
  InhouseDriveLivePreview=null; startLiveCollaboration=()=>{};
  InhouseDriveInk.start(); await InhouseDriveInk.capture();
  const apply=applyRemotePages;
  window.batchSizes=[];
  applyRemotePages=async(pages,options)=>{batchSizes.push(pages.reduce((n,p)=>n+(p.strokes?.length||0),0));return apply(pages,options);};
  window.remoteOp=(id,clock,x=10)=>({v:1,kind:'stroke',pageId:'batch-page',stroke:{id,tool:'pen',color:'#0000ff',width:2,
   points:[{x,y:20,p:.5},{x:x+12,y:32,p:.6}],syncStamp:{clock,actor:'remote'}}});
 });
}

test('a backlog of 65 strokes uses three durable merges without changing its contents',async({page})=>{
 await setup(page);
 const result=await page.evaluate(async()=>{
  const ops=Array.from({length:65},(_,i)=>remoteOp(`remote-${i}`,i+1,20+i));
  const start=performance.now(); await batchJournal.host.apply(ops);
  const ms=performance.now()-start;
  const stored=await loadPageFromIndexedDb(0);
  return {ms,sizes:batchSizes,actual:state.pages[0].strokes.map(s=>({id:s.id,points:s.points})),
   persisted:stored.strokes.map(s=>s.id),expected:ops.map(op=>({id:op.stroke.id,points:op.stroke.points}))};
 });
 expect(result.sizes).toEqual([32,32,1]);
 const byId=items=>[...items].sort((a,b)=>a.id.localeCompare(b.id));
 expect(byId(result.actual)).toEqual(byId(result.expected));
 expect(result.persisted.sort()).toEqual(result.expected.map(s=>s.id).sort());
 console.log('INK_BACKLOG_MERGES',JSON.stringify({strokes:65,merges:result.sizes.length,ms:result.ms}));
});

test('repeated stroke revisions retain their ordering and deletion tombstones still win',async({page})=>{
 await setup(page);
 const result=await page.evaluate(async()=>{
  await batchJournal.host.apply([remoteOp('same',1,10),remoteOp('other',2,20),remoteOp('same',3,90),remoteOp('third',4,30)]);
  const sizes=batchSizes.slice(),latest=state.pages[0].strokes.find(s=>s.id==='same').points[0].x;
  const patch=buildTimelineSnapshotFromPages([{...state.pages[0],strokes:[],images:[],legacyCoverStrokes:null}])[0];
  patch.deletedStrokeIds=['same'];patch.deletedStrokeStamps={same:{clock:100,actor:'remote'}};
  await batchJournal.host.apply([{v:1,pages:[patch],structure:getCollabStructureSnapshot(),fields:getCollabFieldSnapshot(),
   name:state.exportName,calendar:null},remoteOp('same',3,90),remoteOp('surviving',101,50)]);
  return {sizes,latest,ids:state.pages[0].strokes.map(s=>s.id).sort(),deleted:state.pages[0].deletedStrokeIds};
 });
 expect(result.sizes).toEqual([2,2]);expect(result.latest).toBe(90);
 expect(result.ids).toEqual(['other','surviving','third']);expect(result.deleted).toContain('same');
});

test('an in-flight page capture cannot be presented as saved by an earlier stroke receipt',async({page})=>{
 await setup(page);
 const result=await page.evaluate(async()=>{
  state.driveAutosave=true; driveDirty=true; driveContentVersion=9;
  driveIsOffline=()=>false; driveAuthNeedsReconnect=false;
  const load=loadPageDataForExport;let release,entered=false;
  const gate=new Promise(resolve=>release=resolve);
  loadPageDataForExport=async(...args)=>{entered=true;await gate;return load(...args);};
  InhouseDriveInk.page(0);const capture=InhouseDriveInk.capture();
  try{
   const deadline=performance.now()+3000;
   while(!entered){if(performance.now()>deadline)throw Error('Capture never entered');await new Promise(r=>setTimeout(r,5));}
   batchJournal.host.receipt({docId:'ink-batching',actor:'batch-local',version:9,operations:[]});
   const during={pending:InhouseDriveInk.status().pendingCapture,label:getSavePresentation().driveLabel};
   release();await capture;
   return {during,after:InhouseDriveInk.status().pendingCapture};
  }finally{release();loadPageDataForExport=load;state.driveAutosave=false;clearDriveSyncTimers();}
 });
 expect(result.during.pending).toBe(true);expect(result.during.label).not.toBe('Strokes saved');expect(result.after).toBe(false);
});

test('long paths split by point budget without simplifying or dropping coordinates',async({page})=>{
 await setup(page);
 const result=await page.evaluate(async()=>{
  const received=[];
  // The earlier test covers the real merge/checkpoint. Here intercept its
  // input to check scheduling bounds without making the test rasterize 9000 points.
  applyRemotePages=async pages=>{received.push(pages[0].strokes);return {changed:false};};
  const ops=Array.from({length:3},(_,i)=>{
   const op=remoteOp(`long-${i}`,i+1);
   op.stroke.points=Array.from({length:3000},(_,j)=>({x:i*10+j*.01,y:j*.02,p:.5}));
   return op;
  });
  await batchJournal.host.apply(ops);
  return {sizes:received.map(strokes=>strokes.length),actual:received.flat().map(stroke=>stroke.points),expected:ops.map(op=>op.stroke.points)};
 });
 expect(result.sizes).toEqual([2,1]);expect(result.actual).toEqual(result.expected);
});
