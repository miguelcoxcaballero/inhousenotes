import {test,expect} from '@playwright/test';
import {fakeInkDrive} from '../helpers/fake-ink-drive.mjs';
async function setup(page,api,actor){
 await page.exposeFunction('inkRequest',(url,options)=>api.request(url,options));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(async actor=>{
  await InhouseOfflineLibrary.useAccount('ink-test@example.test');
  await __IHN_TEST_API__.resetLocalDocument(1,'Ink');
  await __IHN_TEST_API__.showEditorForTest();
  stopCollabPolling();beginDocumentSession();InhouseDriveInk.stop();
  state.driveFileId='ink-doc';state.driveCanEdit=true;state.driveAutosave=false;state.pages[0].pageId='page-one';
  state.collabStructure=null;state.collabFields=null;driveDirty=false;resetDriveContentSyncVersion();
  driveFetch=async(url,options={})=>{
   const body=typeof options.body==='string'?options.body:await options.body?.text?.();
   const result=await inkRequest(url,{method:options.method,headers:options.headers,body});
   return new Response(result.body,{status:result.status,headers:{'Content-Type':'application/json'}});
  };
  // Expose this instance only in the test; the production adapter remains sealed.
  const Original=InhouseDriveInkJournal.Journal;
  InhouseDriveInkJournal.Journal=class extends Original{constructor(host){super({...host,actor});window.inkTestJournal=this;}start(){return this;}wake(){}};
  const Live=InhouseDriveLivePreview;
  InhouseDriveLivePreview=class extends Live{constructor(host){super(host);window.liveTest=this;}start(){return this;}scheduleFrame(){}schedulePoll(){}};
  startLiveCollaboration=()=>{};
  InhouseDriveInk.start();await InhouseDriveInk.capture();
 },actor);
}
async function stroke(page,id,color='#000000'){
 await page.evaluate(async({id,color})=>{
  const stroke={id,tool:'pen',color,width:2,points:[{x:100,y:100,p:.5},{x:150,y:150,p:.6}]};
  stampCollaborativeItems([stroke]);state.pages[0].strokes.push(stroke);enqueueStrokeOp(0,stroke);
  await flushStrokeOpsQueue({force:true});await InhouseDriveInk.capture();inkTestJournal.lastWrite=0;await inkTestJournal.flush();
 },{id,color});
}
async function pull(page){await page.evaluate(async()=>{inkTestJournal.nextDiscovery=0;for(const source of inkTestJournal.sources.values())source.next=0;await inkTestJournal.poll();});}

test('two isolated devices exchange durable pen edits and erasures without compiling a PDF',async({browser})=>{
 const api=fakeInkDrive(),ca=await browser.newContext(),cb=await browser.newContext(),a=await ca.newPage(),b=await cb.newPage();
 try{
  await setup(a,api,'device-a');await setup(b,api,'device-b');
  await a.evaluate(()=>{buildPdfBlob=async()=>{throw Error('PDF compilation must not gate ink');};});
  await stroke(a,'from-a');await pull(b);
  await expect.poll(()=>b.evaluate(()=>state.pages[0].strokes.map(s=>s.id))).toContain('from-a');
  await stroke(b,'from-b','#ff0000');await pull(a);
  await expect.poll(async()=>{await pull(a);return a.evaluate(()=>state.pages[0].strokes.map(s=>s.id).sort());}).toEqual(['from-a','from-b']);
  await a.evaluate(async()=>{
   trackDeletedStrokeIds(state.pages[0],[{id:'from-a'}]);state.pages[0].strokes=state.pages[0].strokes.filter(s=>s.id!=='from-a');markPageDirty(0);
   await InhouseDriveInk.capture();inkTestJournal.lastWrite=0;await inkTestJournal.flush();
  });
  await expect.poll(async()=>{await pull(b);return b.evaluate(()=>state.pages[0].strokes.map(s=>s.id));}).toEqual(['from-b']);
  expect(api.calls.filter(c=>c.url.includes('uploadType')).every(c=>!c.body?.startsWith('%PDF'))).toBe(true);
 }finally{await ca.close();await cb.close();}
});

test('a stored ink outbox recovers after reload from home without opening the PDF',async({page})=>{
 const api=fakeInkDrive();await setup(page,api,'first-process');
 await page.evaluate(async()=>{
  const s={id:'offline-ink',tool:'pen',color:'#000000',width:2,points:[{x:10,y:20,p:.5},{x:30,y:40,p:.5}]};
  stampCollaborativeItems([s]);state.pages[0].strokes.push(s);enqueueStrokeOp(0,s);await InhouseDriveInk.capture();InhouseDriveInk.stop();
 });
 await page.reload();await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(async()=>{
  InhouseDriveInk.stop();documentSessionActivated=false;
  await InhouseOfflineLibrary.useAccount('ink-test@example.test');
  driveFetch=async(url,options)=>{
   const r=await inkRequest(url,{method:options?.method,headers:options?.headers,body:typeof options?.body==='string'?options.body:await options?.body?.text?.()});return new Response(r.body,{status:r.status});
  };
  InhouseDriveInk.start();
 });
 await expect.poll(()=>page.evaluate(async()=>{
  const records=(await InhouseOfflineStore.list('settings','ink-test@example.test')).filter(r=>r.kind==='ink-journal-v1');
  return records.length>0&&records.every(r=>r.rev===r.ack);
 })).toBe(true);
 expect([...api.files.values()].some(f=>f.raw?.includes('offline-ink'))).toBe(true);
});

test('finished strokes reach the small Drive journal while heavy document work is paused for the next contact',async({page})=>{
 const api=fakeInkDrive();await setup(page,api,'active-pen');
 const result=await page.evaluate(async()=>{
  const stroke={id:'quick-ink',tool:'pen',color:'#000000',width:2,points:[{x:40,y:40,p:.5},{x:60,y:60,p:.5}]};
  stampCollaborativeItems([stroke]);state.pages[0].strokes.push(stroke);enqueueStrokeOp(0,stroke);
  isPenWriting=true;
  try{
   await inkTestJournal.persistChain;inkTestJournal.lastWrite=0;await inkTestJournal.flush();
   return {pending:inkTestJournal.status().pending,receipt:InhouseDriveInk.status().ackVersion,version:driveContentVersion};
  }finally{isPenWriting=false;wakeDocumentBackgroundWork();}
 });
 expect(result.pending).toBe(0);expect(result.receipt).toBe(result.version);
 expect([...api.files.values()].some(f=>f.raw?.includes('quick-ink'))).toBe(true);
});

test('Drive previews show a growing stroke before pen-up without P2P or durable merge waits',async({browser})=>{
 const api=fakeInkDrive(),ca=await browser.newContext(),cb=await browser.newContext(),a=await ca.newPage(),b=await cb.newPage();
 try{
  await setup(a,api,'drawing-device');await setup(b,api,'viewing-device');
  const id=await a.evaluate(async()=>{
   window.testLiveStroke={id:'unfinished',tool:'pen',color:'#0000ff',width:2,points:[{x:10,y:10,p:.5},{x:20,y:20,p:.5}]};
   publishLiveStrokePreview('page-one',testLiveStroke);ihnFlushLiveStrokePreview('unfinished');
   isPenWriting=true;await liveTest.flushFrame();return liveTest.fileId;
  });
  await b.evaluate(async id=>{liveTest.hint(id);await liveTest.pollFrames();},id);
  await expect.poll(()=>b.evaluate(()=>[...remoteLiveStrokePreviews.values()].find(p=>p.strokeId==='unfinished')?.points.length)).toBe(2);
  expect(await b.evaluate(()=>state.pages[0].strokes.some(s=>s.id==='unfinished'))).toBe(false);
  await a.evaluate(async()=>{testLiveStroke.points.push({x:30,y:35,p:.7});publishLiveStrokePreview('page-one',testLiveStroke);ihnFlushLiveStrokePreview('unfinished');await liveTest.flushFrame();});
  await b.evaluate(async()=>{for(const s of liveTest.previewSources.values())s.next=0;await liveTest.pollFrames();});
  await expect.poll(()=>b.evaluate(()=>[...remoteLiveStrokePreviews.values()].find(p=>p.strokeId==='unfinished')?.points.length)).toBe(3);
  expect(await a.evaluate(()=>isPenWriting)).toBe(true);
  // Pen-up may refine the path; the final preview must replace all old
  // coordinates, not append a shortened path onto the stale prefix.
  await a.evaluate(async()=>{
   testLiveStroke.points=[{x:12,y:14,p:.5},{x:32,y:38,p:.7}];stampCollaborativeItems([testLiveStroke]);
   publishLiveStrokePreview('page-one',testLiveStroke,{final:true});await liveTest.flushFrame();
  });
  await b.evaluate(async()=>{for(const s of liveTest.previewSources.values())s.next=0;await liveTest.pollFrames();});
  await expect.poll(()=>b.evaluate(()=>[...remoteLiveStrokePreviews.values()].find(p=>p.strokeId==='unfinished')?.points.map(p=>p.x))).toEqual([12,32]);
  await a.evaluate(async()=>{
   isPenWriting=false;wakeDocumentBackgroundWork();state.pages[0].strokes.push(testLiveStroke);enqueueStrokeOp(0,testLiveStroke);
   await inkTestJournal.persistChain;inkTestJournal.lastWrite=0;await inkTestJournal.flush();
  });
  expect(await a.evaluate(()=>liveTest.paths.size)).toBe(0);
  await pull(b);
  await expect.poll(()=>b.evaluate(()=>state.pages[0].strokes.some(s=>s.id==='unfinished'))).toBe(true);
  expect(await b.evaluate(()=>remoteLiveStrokePreviews.size)).toBe(0);
 }finally{await ca.close();await cb.close();}
});
