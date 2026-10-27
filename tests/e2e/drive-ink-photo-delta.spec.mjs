import {test,expect} from '@playwright/test';
import {fakeInkDrive} from '../helpers/fake-ink-drive.mjs';
async function setup(page,api,actor='photo-writer'){
 await page.exposeFunction('photoInkRequest',(url,options)=>api.request(url,options));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(async actor=>{
  await InhouseOfflineLibrary.useAccount('photo-delta@example.test');
  await __IHN_TEST_API__.resetLocalDocument(1,'Photo delta');
  stopCollabPolling();beginDocumentSession();InhouseDriveInk.stop();
  state.driveFileId='photo-delta';state.driveCanEdit=true;state.driveAutosave=false;
  state.pages[0].pageId='photo-page';state.collabStructure=null;state.collabFields=null;
  // The adapter transports opaque image bytes; photo decode/export has its own
  // persistence tests. A large payload exposes accidental retransmission here.
  window.originalPhoto='data:image/png;base64,'+'A'.repeat(1024*1024);
  state.pages[0].backgroundSource='custom';state.pages[0].backgroundImage=originalPhoto;
  driveDirty=false;resetDriveContentSyncVersion();
  driveFetch=async(url,options={})=>{
   const body=typeof options.body==='string'?options.body:await options.body?.text?.();
   const r=await photoInkRequest(url,{method:options.method,headers:options.headers,body});
   return new Response(r.body,{status:r.status,headers:{'Content-Type':'application/json'}});
  };
  const Journal=InhouseDriveInkJournal.Journal;
  InhouseDriveInkJournal.Journal=class extends Journal{
   constructor(host){super({...host,actor});window.photoJournal=this;}
   start(){return this;}wake(){}
  };
  InhouseDriveLivePreview=null;startLiveCollaboration=()=>{};schedulePreparedDrivePdf=()=>{};
  window.photoOperations=()=>[...photoJournal.records.values()].flatMap(record=>record.ops.map(op=>JSON.parse(op)));
  InhouseDriveInk.start();await InhouseDriveInk.capture();
 },actor);
}

test('writing on an unchanged one-megabyte photo sends only the new stroke',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);
 const result=await page.evaluate(async()=>{
  const stroke={id:'new-ink',tool:'pen',color:'#000000',width:2,points:[{x:10,y:20,p:.5},{x:20,y:30,p:.5}]};
  stampCollaborativeItems([stroke]);state.pages[0].strokes.push(stroke);enqueueStrokeOp(0,stroke);
  await photoJournal.persistChain;await InhouseDriveInk.capture();
  const ops=photoOperations(),bytes=JSON.stringify(ops).length;
  photoJournal.lastWrite=0;await photoJournal.flush();
  const before=InhouseDriveInk.status().ackVersion;
  // An empty page capture cannot manufacture a receipt for a newer version.
  driveContentVersion++;InhouseDriveInk.page(0);await InhouseDriveInk.capture();
  const after=InhouseDriveInk.status().ackVersion;
  return {kinds:ops.map(op=>op.kind),bytes,before,after,version:driveContentVersion,photo:state.pages[0].backgroundImage===originalPhoto};
 });
 expect(result.kinds).toEqual(['stroke']);expect(result.bytes).toBeLessThan(2000);expect(result.photo).toBe(true);
 expect(result.after).toBe(result.before);expect(result.after).toBeLessThan(result.version);
 expect([...api.files.values()].filter(f=>f.raw).every(f=>!f.raw.includes('data:image/png'))).toBe(true);
 console.log('PHOTO_INK_DELTA_BYTES',result.bytes);
});

test('changed photos, deletion stamps and document metadata are never skipped',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);
 const result=await page.evaluate(async()=>{
  const changed='data:image/png;base64,'+'B'.repeat(512*1024);
  state.pages[0].backgroundImage=changed;noteCollabPageFieldChanged(state.pages[0],'background');markPageDirty(0);
  await InhouseDriveInk.capture();
  const photo=photoOperations().at(-1).pages[0];
  state.pages[0].deletedStrokeIds=['deleted'];state.pages[0].deletedStrokeStamps={deleted:{clock:123,actor:'writer'}};
  markPageDirty(0);await InhouseDriveInk.capture();
  const tombstone=photoOperations().at(-1).pages[0];
  state.exportName='Renamed photo';noteCollabDocumentFieldChanged('exportName');bumpDriveContentVersion();
  InhouseDriveInk.page(0);await InhouseDriveInk.capture();
  const metadata=photoOperations().at(-1);
  // Same value with a newer field stamp is a real causal update.
  const before=photoOperations().length;noteCollabPageFieldChanged(state.pages[0],'background');bumpDriveContentVersion();
  await InhouseDriveInk.capture();
  return {photo:photo.backgroundImage===changed,deleted:tombstone.deletedStrokeIds,stamp:tombstone.deletedStrokeStamps.deleted,
   metadata:{name:metadata.name,pages:metadata.pages.length},stampUpdate:photoOperations().length>before};
 });
 expect(result.photo).toBe(true);expect(result.deleted).toEqual(['deleted']);expect(result.stamp).toEqual({clock:123,actor:'writer'});
 expect(result.metadata).toEqual({name:'Renamed photo',pages:0});expect(result.stampUpdate).toBe(true);
});

test('new pages and dirty session recovery retain complete photo payloads',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);
 const result=await page.evaluate(async()=>{
  const page={...createBlankPageData(),pageId:'new-photo',backgroundSource:'custom',backgroundImage:originalPhoto};
  state.pages.push(page);noteCollabPageAdded(page.pageId,1);markPageDirty(1);
  await InhouseDriveInk.capture();
  const newPage=photoOperations().flatMap(op=>op.pages||[]).find(page=>page.pageId==='new-photo');
  InhouseDriveInk.stop();driveDirty=true;InhouseDriveInk.start();await InhouseDriveInk.capture();
  const recovered=photoOperations().flatMap(op=>op.pages||[]);
  return {newPage:newPage?.backgroundImage===originalPhoto,
   recovered:recovered.filter(page=>page.backgroundImage===originalPhoto).map(page=>page.pageId).sort()};
 });
 expect(result.newPage).toBe(true);expect(result.recovered).toEqual(['new-photo','photo-page']);
});

test('a failed photo capture does not install a baseline that would skip its retry',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);
 const result=await page.evaluate(async()=>{
  const changed=originalPhoto+'AA';state.pages[0].backgroundImage=changed;markPageDirty(0);
  const enqueue=photoJournal.enqueue;let failed=false;
  photoJournal.enqueue=()=>Promise.reject(Error('storage unavailable'));
  try{await InhouseDriveInk.capture();}catch(_){failed=true;}
  photoJournal.enqueue=enqueue;await InhouseDriveInk.capture();
  const patch=photoOperations().flatMap(op=>op.pages||[]).at(-1);
  return {failed,retried:patch?.backgroundImage===changed};
 });
 expect(result).toEqual({failed:true,retried:true});
});

async function coloredPhotoState(page,colors,clock){
 return page.evaluate(async({colors,clock})=>{
  InhouseDriveInk.stop();
  const canvas=document.createElement('canvas');canvas.width=canvas.height=4;
  const ctx=canvas.getContext('2d');
  const photos=colors.map(color=>{ctx.fillStyle=color;ctx.fillRect(0,0,4,4);return canvas.toDataURL('image/png');});
  state.pages=photos.map((backgroundImage,index)=>({...createBlankPageData(),pageId:`photo-${index}`,
   backgroundSource:'custom',backgroundImage}));
  state.collabStructure=null;state.collabFields=null;ensureCollabFieldMetadata();
  for(const p of state.pages)state.collabFields.fields[getCollabPageFieldKey(p.pageId,'background')].stamp={clock,actor:'writer'};
  state.collabFields.clock=clock;driveDirty=false;resetDriveContentSyncVersion();
  InhouseDriveInk.start();await InhouseDriveInk.capture();return photos;
 },{colors,clock});
}
async function pushPhotoJournal(page){
 await page.evaluate(async()=>{await photoJournal.persistChain;for(let i=0;i<16&&photoJournal.status().pending;i++){photoJournal.lastWrite=0;await photoJournal.flush();}if(photoJournal.status().pending)throw Error('Photo journal did not flush');});
}
async function pullPhotoJournal(page){
 await page.evaluate(async()=>{photoJournal.nextDiscovery=0;for(const source of photoJournal.sources.values())source.next=0;await photoJournal.poll();});
}

test('a same-value newer photo register carries bytes that replace a divergent replica',async({browser})=>{
 const api=fakeInkDrive(),ca=await browser.newContext(),cb=await browser.newContext(),a=await ca.newPage(),b=await cb.newPage();
 try{
  await setup(a,api,'photo-sender');await setup(b,api,'photo-reader');
  const [original]=await coloredPhotoState(a,['#ff0000'],10);await coloredPhotoState(b,['#0000ff'],20);
  await a.evaluate(async()=>{
   // Equivalent to A -> B -> A: value returns to the local baseline while
   // its causal stamp must supersede B on the other device. No page() call.
   state.collabFields.fields[getCollabPageFieldKey('photo-0','background')].stamp={clock:30,actor:'writer'};
   state.collabFields.clock=30;bumpDriveContentVersion();await InhouseDriveInk.capture();
  });
  expect(await a.evaluate(()=>photoOperations().flatMap(op=>op.pages||[]).length)).toBe(1);
  await pushPhotoJournal(a);await pullPhotoJournal(b);
  expect(await b.evaluate(()=>state.pages[0].backgroundImage)).toBe(original);
  expect(await b.evaluate(()=>state.collabFields.fields[getCollabPageFieldKey('photo-0','background')].stamp.clock)).toBe(30);
 }finally{await ca.close();await cb.close();}
});

test('two photo updates deliver each register with its own page payload',async({browser})=>{
 const api=fakeInkDrive(),ca=await browser.newContext(),cb=await browser.newContext(),a=await ca.newPage(),b=await cb.newPage();
 try{
  await setup(a,api,'photo-sender');await setup(b,api,'photo-reader');
  await coloredPhotoState(a,['#ffffff','#ffffff'],10);await coloredPhotoState(b,['#ff0000','#0000ff'],20);
  const expected=await a.evaluate(async()=>{
   const canvas=document.createElement('canvas');canvas.width=canvas.height=4;const ctx=canvas.getContext('2d');
   const photos=['#00ff00','#ffff00'].map(color=>{ctx.fillStyle=color;ctx.fillRect(0,0,4,4);return canvas.toDataURL('image/png');});
   state.pages.forEach((p,index)=>{
    p.backgroundImage=photos[index];
    state.collabFields.fields[getCollabPageFieldKey(p.pageId,'background')]={value:getCollabBackgroundValue(p),stamp:{clock:30,actor:'writer'}};
   });
   state.collabFields.clock=30;bumpDriveContentVersion();await InhouseDriveInk.capture();
   return {photos,ops:photoOperations()};
  });
  expect(expected.ops).toHaveLength(2);
  for(const op of expected.ops){
   const id=op.pages[0].pageId;
   expect(Object.keys(op.fields.fields).filter(key=>key.startsWith('page:')).every(key=>key.startsWith(`page:${id}:`))).toBe(true);
  }
  await pushPhotoJournal(a);await pullPhotoJournal(b);
  expect(await b.evaluate(()=>state.pages.map(p=>p.backgroundImage))).toEqual(expected.photos);
 }finally{await ca.close();await cb.close();}
});

test('a photo edited during an asynchronous capture is retried with a matching register',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);await coloredPhotoState(page,['#ff0000'],10);
 const result=await page.evaluate(async()=>{
  const load=loadPageDataForExport;let release,entered=false;
  const gate=new Promise(resolve=>release=resolve);
  loadPageDataForExport=async(...args)=>{entered=true;await gate;return load(...args);};
  InhouseDriveInk.page(0);const capture=InhouseDriveInk.capture();
  try{
   const deadline=performance.now()+3000;
   while(!entered){if(performance.now()>deadline)throw Error('capture did not enter');await new Promise(r=>setTimeout(r,5));}
   const canvas=document.createElement('canvas');canvas.width=canvas.height=4;const ctx=canvas.getContext('2d');
   ctx.fillStyle='#0000ff';ctx.fillRect(0,0,4,4);state.pages[0].backgroundImage=canvas.toDataURL('image/png');
   noteCollabPageFieldChanged(state.pages[0],'background');bumpDriveContentVersion();
   const expected=getCollabBackgroundValue(state.pages[0]);release();await capture;
   const premature=photoOperations().length;loadPageDataForExport=load;await InhouseDriveInk.capture();
   const op=photoOperations().at(-1),patch=op.pages[0];
   return {premature,expected,actual:{backgroundSource:patch.backgroundSource,backgroundImageHash:simpleHash(patch.backgroundImage)},
    register:op.fields.fields[getCollabPageFieldKey(patch.pageId,'background')].value};
  }finally{release();loadPageDataForExport=load;}
 });
 expect(result.premature).toBe(0);expect(result.actual).toEqual(result.expected);expect(result.register).toEqual(result.expected);
});

test('two newly added photo pages are fully materialized on a replica with only the original page',async({browser})=>{
 const api=fakeInkDrive(),ca=await browser.newContext(),cb=await browser.newContext(),a=await ca.newPage(),b=await cb.newPage();
 try{
  await setup(a,api,'new-photo-sender');await setup(b,api,'new-photo-reader');
  const [original]=await coloredPhotoState(a,['#ffffff'],10);await coloredPhotoState(b,['#ffffff'],10);
  const added=await a.evaluate(async()=>{
   const canvas=document.createElement('canvas');canvas.width=canvas.height=4;const ctx=canvas.getContext('2d');
   const photos=['#00ff00','#ffff00'].map(color=>{ctx.fillStyle=color;ctx.fillRect(0,0,4,4);return canvas.toDataURL('image/png');});
   photos.forEach((backgroundImage,index)=>{
    const page={...createBlankPageData(),pageId:`added-photo-${index+1}`,backgroundSource:'custom',backgroundImage};
    state.pages.push(page);noteCollabPageAdded(page.pageId,index+1);markPageDirty(index+1);
   });
   await InhouseDriveInk.capture();return photos;
  });
  await pushPhotoJournal(a);await pullPhotoJournal(b);
  const actual=await b.evaluate(async()=>({ids:state.pages.map(p=>p.pageId),photos:state.pages.map(p=>p.backgroundImage),
   stored:await Promise.all(state.pages.map(async(_p,index)=>(await loadPageFromIndexedDb(index))?.backgroundImage))}));
  expect(actual.ids).toEqual(['photo-0','added-photo-1','added-photo-2']);
  expect(actual.photos).toEqual([original,...added]);expect(actual.stored.slice(1)).toEqual(added);
 }finally{await ca.close();await cb.close();}
});

async function seedEditablePhotoInk(page){
 await page.evaluate(async()=>{
  InhouseDriveInk.stop();
  state.pages[0].strokes=[{id:'editable-ink',tool:'pen',color:'#000000',width:2,
   points:[{x:10,y:20,p:.5},{x:20,y:30,p:.6}],syncStamp:{clock:10,actor:'original-writer'}}];
  driveDirty=false;resetDriveContentSyncVersion();InhouseDriveInk.start();await InhouseDriveInk.capture();
 });
}

for(const edit of ['width','color','geometry'])test(`changing existing ink ${edit} does not retransmit a one-megabyte photo`,async({page})=>{
 const api=fakeInkDrive();await setup(page,api);await seedEditablePhotoInk(page);
 const result=await page.evaluate(async edit=>{
  const item=state.pages[0].strokes[0];
  if(edit==='width')item.width=4;
  if(edit==='color')item.color='#ff0000';
  if(edit==='geometry')item.points=[{x:30,y:40,p:.7},{x:50,y:60,p:.8},{x:60,y:65,p:.6}];
  item.__pendingOpId='local-only';stampCollaborativeItems([item]);markPageDirty(0);
  const expected={id:item.id,tool:item.tool,color:item.color,width:item.width,points:item.points,syncStamp:item.syncStamp};
  await InhouseDriveInk.capture();const ops=photoOperations();
  const before=InhouseDriveInk.status().ackVersion,version=driveContentVersion;
  photoJournal.lastWrite=0;await photoJournal.flush();
  return {ops,expected,bytes:JSON.stringify(ops).length,before,version,after:InhouseDriveInk.status().ackVersion,
   photo:state.pages[0].backgroundImage===originalPhoto};
 },edit);
 expect(result.ops).toEqual([{v:1,kind:'stroke',pageId:'photo-page',stroke:result.expected}]);
 expect(result.bytes).toBeLessThan(2000);expect(result.photo).toBe(true);
 expect(result.before).toBeLessThan(result.version);expect(result.after).toBe(result.version);
 expect([...api.files.values()].filter(f=>f.raw).every(f=>!f.raw.includes('data:image/png'))).toBe(true);
 console.log('EDITED_PHOTO_INK_BYTES',edit,result.bytes);
});

test('edited stroke deltas update another replica and persist without replacing its photo',async({browser})=>{
 const api=fakeInkDrive(),ca=await browser.newContext(),cb=await browser.newContext(),a=await ca.newPage(),b=await cb.newPage();
 try{
  await setup(a,api,'edit-photo-sender');await setup(b,api,'edit-photo-reader');
  const [photo]=await coloredPhotoState(a,['#eeccee'],10);await coloredPhotoState(b,['#eeccee'],10);
  await seedEditablePhotoInk(a);await seedEditablePhotoInk(b);
  const expected=await a.evaluate(async()=>{
   const item=state.pages[0].strokes[0];item.color='#0000ff';item.width=4;
   item.points=[{x:25,y:45,p:.7},{x:40,y:65,p:.8},{x:65,y:70,p:.6}];
   stampCollaborativeItems([item]);markPageDirty(0);await InhouseDriveInk.capture();
   return {stroke:cloneStroke(item),ops:photoOperations()};
  });
  expect(expected.ops).toHaveLength(1);expect(expected.ops[0].kind).toBe('stroke');
  await pushPhotoJournal(a);await pullPhotoJournal(b);
  const actual=await b.evaluate(async()=>{
   const stored=await loadPageFromIndexedDb(0);
   return {photo:state.pages[0].backgroundImage,strokes:state.pages[0].strokes.map(cloneStroke),
    storedPhoto:stored?.backgroundImage,storedStrokes:stored?.strokes.map(cloneStroke)};
  });
  expect(actual).toEqual({photo,strokes:[expected.stroke],storedPhoto:photo,storedStrokes:[expected.stroke]});
 }finally{await ca.close();await cb.close();}
});

for(const change of ['tombstone','page-field','document','image'])test(`an ink edit with a ${change} change retains the full page payload`,async({page})=>{
 const api=fakeInkDrive();await setup(page,api);await seedEditablePhotoInk(page);
 const result=await page.evaluate(async change=>{
  const page=state.pages[0],item=page.strokes[0];item.width=4;stampCollaborativeItems([item]);
  if(change==='tombstone'){
   page.deletedStrokeIds=['removed-ink'];page.deletedStrokeStamps={'removed-ink':{clock:20,actor:'eraser'}};
  }else if(change==='page-field'){
   // Even a field whose value returned to baseline still needs its winning
   // register to accompany the full page payload.
   noteCollabPageFieldChanged(page,'background');
  }else if(change==='document'){
   state.exportName='Edited photo name';noteCollabDocumentFieldChanged('exportName');
  }else{
   page.images=[{id:'new-image',src:'data:image/png;base64,BBBB',x:4,y:8,width:20,height:30}];
   stampCollaborativeItems(page.images);
  }
  markPageDirty(0);await InhouseDriveInk.capture();const ops=photoOperations();
  return {count:ops.length,kind:ops[0]?.kind||null,photo:ops[0]?.pages?.[0]?.backgroundImage===originalPhoto,
   strokes:ops[0]?.pages?.[0]?.strokes.map(s=>s.id),deleted:ops[0]?.pages?.[0]?.deletedStrokeIds,
   images:ops[0]?.pages?.[0]?.images.map(image=>image.id),name:ops[0]?.name,
   field:ops[0]?.fields?.fields[getCollabPageFieldKey(page.pageId,'background')],
   expectedField:getCollabFieldSnapshot().fields[getCollabPageFieldKey(page.pageId,'background')]};
 },change);
 expect(result.count).toBe(1);expect(result.kind).toBeNull();expect(result.photo).toBe(true);expect(result.strokes).toEqual(['editable-ink']);
 if(change==='tombstone')expect(result.deleted).toEqual(['removed-ink']);
 if(change==='page-field')expect(result.field).toEqual(result.expectedField);
 if(change==='document')expect(result.name).toBe('Edited photo name');
 if(change==='image')expect(result.images).toEqual(['new-image']);
});

test('capture-only new strokes retain the page shell needed by a receiver without that page',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);await seedEditablePhotoInk(page);
 const result=await page.evaluate(async()=>{
  const item={...cloneStroke(state.pages[0].strokes[0]),id:'new-captured-ink'};
  stampCollaborativeItems([item]);state.pages[0].strokes.push(item);markPageDirty(0);await InhouseDriveInk.capture();
  const ops=photoOperations();return {count:ops.length,kind:ops[0]?.kind||null,
   photo:ops[0]?.pages?.[0]?.backgroundImage===originalPhoto,ids:ops[0]?.pages?.[0]?.strokes.map(s=>s.id)};
 });
 expect(result).toEqual({count:1,kind:null,photo:true,ids:['new-captured-ink']});
});

test('an edited stroke is immutable during persistence and a newer revision is captured again',async({page})=>{
 const api=fakeInkDrive();await setup(page,api);await seedEditablePhotoInk(page);
 const result=await page.evaluate(async()=>{
  const item=state.pages[0].strokes[0];item.width=4;stampCollaborativeItems([item]);markPageDirty(0);
  const original=cloneStroke(item),enqueue=photoJournal.enqueue;let release,entered=false;
  const gate=new Promise(resolve=>release=resolve);
  photoJournal.enqueue=function(...args){
   const pending=enqueue.apply(this,args);
   if(entered)return pending;entered=true;return pending.then(()=>gate);
  };
  const capture=InhouseDriveInk.capture();
  try{
   const deadline=performance.now()+3000;
   while(!entered){if(performance.now()>deadline)throw Error('stroke capture did not enter');await new Promise(r=>setTimeout(r,5));}
   item.points[0].x=99;item.points.push({x:80,y:90,p:.9});item.color='#ff0000';
   // Mutate the same stamp object as well as points, after serialization.
   item.syncStamp.clock++;item.syncStamp.actor='newer-writer';markPageDirty(0);
   const latest=cloneStroke(item),version=driveContentVersion;
   const during={pending:InhouseDriveInk.status().pendingCapture,ack:InhouseDriveInk.status().ackVersion};
   release();await capture;photoJournal.enqueue=enqueue;await InhouseDriveInk.capture();
   const ops=photoOperations();photoJournal.lastWrite=0;await photoJournal.flush();
   return {ops,original,latest,version,during,after:InhouseDriveInk.status().ackVersion};
  }finally{release();photoJournal.enqueue=enqueue;}
 });
 expect(result.during.pending).toBe(true);expect(result.during.ack).toBeLessThan(result.version);
 expect(result.ops).toEqual([result.original,result.latest].map(stroke=>({v:1,kind:'stroke',pageId:'photo-page',stroke})));
 expect(result.after).toBe(result.version);
});
