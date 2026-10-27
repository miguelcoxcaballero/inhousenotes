import {test,expect} from '@playwright/test';
import {fakeInkDrive} from '../helpers/fake-ink-drive.mjs';

test('the immediate ink journal freezes the original stroke without another point-array clone',async({page})=>{
 const api=fakeInkDrive();
 await page.exposeFunction('snapshotInkRequest',(url,options)=>api.request(url,options));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  await InhouseOfflineLibrary.useAccount('ink-snapshot@example.test');
  await __IHN_TEST_API__.resetLocalDocument(1,'Immutable ink');
  stopCollabPolling();beginDocumentSession();InhouseDriveInk.stop();
  state.driveFileId='snapshot-doc';state.driveCanEdit=true;state.driveAutosave=false;
  state.pages[0].pageId='snapshot-page';state.collabStructure=null;state.collabFields=null;
  driveDirty=false;resetDriveContentSyncVersion();
  driveFetch=async(url,options={})=>{
   const body=typeof options.body==='string'?options.body:await options.body?.text?.();
   const response=await snapshotInkRequest(url,{method:options.method,headers:options.headers,body});
   return new Response(response.body,{status:response.status,headers:{'Content-Type':'application/json'}});
  };
  const Journal=InhouseDriveInkJournal.Journal;
  InhouseDriveInkJournal.Journal=class extends Journal{
   constructor(host){super({...host,actor:'snapshot-writer'});window.snapshotJournal=this;}
   start(){return this;}wake(){}
  };
  const Preview=InhouseDriveLivePreview;
  InhouseDriveLivePreview=class extends Preview{start(){return this;}scheduleFrame(){}schedulePoll(){}};
  startLiveCollaboration=()=>{};
  InhouseDriveInk.start();await InhouseDriveInk.capture();
  const stroke={id:'original-id',tool:'pen',color:'#123456',width:2,
   __pendingOpId:'local-only-operation',syncStamp:{clock:7,actor:'original-actor'},
   points:Array.from({length:4000},(_,i)=>({x:i,y:i*2,p:.5}))};
  const originalClone=cloneStroke;let copies=0;
  cloneStroke=value=>{copies++;return originalClone(value);};
  const started=performance.now();
  try{InhouseDriveInk.stroke(0,stroke);}finally{cloneStroke=originalClone;}
  const elapsed=performance.now()-started;
  // Mutation happens before persistence/upload resolve, exercising the exact
  // boundary that used to be protected by a second full point-array copy.
  stroke.id='mutated-id';stroke.color='#ff0000';stroke.width=20;
  stroke.syncStamp.clock=88;stroke.syncStamp.actor='mutated-actor';
  stroke.points[0].x=999;stroke.points.push({x:9000,y:9000,p:1});
  await snapshotJournal.persistChain;snapshotJournal.lastWrite=0;await snapshotJournal.flush();
  const pending=snapshotJournal.status().pending;InhouseDriveInk.stop();
  return {copies,elapsed,pending};
 });
 const file=[...api.files.values()].find(file=>file.appProperties?.ihnKind==='ink-journal-v1');
 expect(file).toBeTruthy();const op=JSON.parse(file.raw).ops.find(op=>op.kind==='stroke');
 expect(op.stroke.__pendingOpId).toBeUndefined();expect(op.stroke.id).toBe('original-id');expect(op.stroke.color).toBe('#123456');expect(op.stroke.width).toBe(2);
 expect(op.stroke.syncStamp).toEqual({clock:7,actor:'original-actor'});
 expect(op.stroke.points).toHaveLength(4000);expect(op.stroke.points[0]).toEqual({x:0,y:0,p:.5});
 expect(result.pending).toBe(0);expect(result.copies).toBe(0);
 console.log('IMMEDIATE_INK_SNAPSHOT',result);
});
