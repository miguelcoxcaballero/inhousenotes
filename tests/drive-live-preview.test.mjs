import test from 'node:test';import assert from 'node:assert/strict';import {createRequire} from 'node:module';
import {fakeInkDrive,memoryInkStore} from './helpers/fake-ink-drive.mjs';
const require=createRequire(import.meta.url),{LivePreview}=require('../drive-live-preview-v5.js');
const {Journal}=require('../drive-ink-journal-v5.js');
function replica(api,actor){
 const frames=[],receipts=[];
 const live=new LivePreview({store:memoryInkStore(),account:'a',docId:'doc',actor,fetch:api.fetch,preview:p=>frames.push(p),receipt:r=>receipts.push(r)});
 live.scheduleFrame=()=>{};live.schedulePoll=()=>{};return {live,frames,receipts};
}
const packet=(sequence,offset,points,extra={})=>({v:1,type:'live-stroke',fileId:'doc',pageId:'page',actorId:'writer',strokeId:'pen',sequence,offset,points:points.map(x=>({x,y:x,p:.5})),tool:'pen',width:2,color:'#000000',...extra});
test('unfinished strokes arrive as complete previews even after missed frames, without save receipts',async()=>{
 const api=fakeInkDrive(),a=replica(api,'writer'),b=replica(api,'reader');
 a.live.push(packet(1,0,[1,2]));await a.live.flushFrame();
 a.live.push(packet(2,1,[2,3,4]));await a.live.flushFrame();
 b.live.hint(a.live.fileId);await b.live.pollFrames();
 assert.deepEqual(b.frames[0].points.map(p=>p.x),[1,2,3,4]);assert.ok(!b.frames[0].final);assert.equal(a.receipts.length,0);
 assert.equal([...a.live.store.data.values()].filter(r=>r.kind).length,0);
 a.live.stop();b.live.stop();
});
test('one in-flight preview coalesces new samples and cannot acknowledge or lose the latest frame',async()=>{
 const api=fakeInkDrive(),a=replica(api,'writer');let release;api.hold=new Promise(r=>release=r);
 a.live.push(packet(1,0,[1,2]));const send=a.live.flushFrame();
 while(![...api.files.values()].some(f=>f.raw))await new Promise(r=>setTimeout(r,1));
 a.live.push(packet(2,1,[2,3]));a.live.push(packet(3,2,[3,4]));
 release();await send;api.hold=null;assert.equal(a.live.sentFrame,1);await a.live.flushFrame();
 assert.equal(JSON.parse(api.files.get(a.live.fileId).raw).frame,3);
 assert.deepEqual(JSON.parse(api.files.get(a.live.fileId).raw).packets[0].points.map(p=>p.x),[1,2,3,4]);a.live.stop();
});
test('stale frames and another document cannot overwrite the live preview',async()=>{
 const api=fakeInkDrive(),a=replica(api,'writer'),b=replica(api,'reader');
 a.live.push(packet(1,0,[1,2]));await a.live.flushFrame();b.live.hint(a.live.fileId);await b.live.pollFrames();
 const file=api.files.get(a.live.fileId),frame=JSON.parse(file.raw);
 file.raw=JSON.stringify({...frame,frame:99,docId:'other'});b.live.previewSources.get(a.live.fileId).next=0;await b.live.pollFrames();
 file.raw=JSON.stringify({...frame,frame:99,at:Date.now()-60000});b.live.previewSources.get(a.live.fileId).next=0;await b.live.pollFrames();
 assert.equal(b.frames.length,1);a.live.stop();b.live.stop();
});
test('a pen-gated receiver cannot block the next durable stroke upload',async()=>{
 const api=fakeInkDrive(),store=memoryInkStore();let release;
 const journal=new Journal({store,account:'a',docId:'doc',actor:'writer',fetch:api.fetch});
 journal.poll=()=>new Promise(r=>release=r);
 journal.start();await journal.enqueue({stroke:'one'},1);
 const deadline=Date.now()+1500;
 while(journal.status().pending&&Date.now()<deadline)await new Promise(r=>setTimeout(r,10));
 await journal.enqueue({stroke:'two'},2);
 while(journal.status().pending&&Date.now()<deadline)await new Promise(r=>setTimeout(r,10));
 assert.equal(journal.status().pending,0);assert.ok([...api.files.values()].some(f=>f.raw?.includes('two')));
 journal.stop();release?.();
});

test('a prewarmed preview delivers before stroke completion with bounded scheduling delay',async()=>{
 const api=fakeInkDrive();const frames=[];
 const writer=new LivePreview({store:memoryInkStore(),account:'a',docId:'doc',actor:'writer',fetch:api.fetch});
 const reader=new LivePreview({store:memoryInkStore(),account:'a',docId:'doc',actor:'reader',fetch:api.fetch,canPublish:false,preview:p=>frames.push({packet:p,at:performance.now()})});
 writer.start();reader.start();
 try{
  let deadline=Date.now()+1500;
  while(!writer.exists&&Date.now()<deadline)await new Promise(r=>setTimeout(r,5));
  assert.ok(writer.exists);reader.hint(writer.fileId);
  const start=performance.now();writer.push(packet(1,0,[1,2]));
  while(!frames.length&&Date.now()<deadline)await new Promise(r=>setTimeout(r,5));
  assert.equal(frames.length,1);assert.ok(!frames[0].packet.final);
  const latency=frames[0].at-start;console.log('DRIVE_PREVIEW_SCHEDULING_MS',latency.toFixed(1));assert.ok(latency<350);
 }finally{writer.stop();reader.stop();}
});

test('a slow discovery hint cannot freeze unfinished stroke updates',async()=>{
 const api=fakeInkDrive(),a=replica(api,'writer');let release;
 a.live.host.announce=()=>new Promise(r=>release=r);
 a.live.push(packet(1,0,[1,2]));
 await Promise.race([a.live.flushFrame(),new Promise((_,reject)=>{const t=setTimeout(()=>reject(Error('hint blocked preview')),500);t.unref();})]);
 a.live.push(packet(2,1,[2,3]));await a.live.flushFrame();
 assert.equal(a.live.sentFrame,2);assert.equal(JSON.parse(api.files.get(a.live.fileId).raw).packets[0].points.length,3);
 a.live.stop();release();
});

test('final geometry replaces the preview prefix and does not retain a stale tail',async()=>{
 const api=fakeInkDrive(),a=replica(api,'writer'),b=replica(api,'reader');
 a.live.push(packet(1,0,[1,2,3,4,5]));await a.live.flushFrame();
 a.live.push(packet(2,0,[10,20],{finalBatch:true,totalPoints:3}));
 a.live.push(packet(3,2,[30],{finalBatch:true,totalPoints:3,final:true}));await a.live.flushFrame();
 b.live.hint(a.live.fileId);await b.live.pollFrames();
 assert.deepEqual(b.frames[0].points.map(p=>p.x),[10,20,30]);a.live.stop();b.live.stop();
});

test('only the exact confirmed final stroke is retired from live payloads',async()=>{
 const api=fakeInkDrive(),a=replica(api,'writer');const stamp={clock:2,actor:'writer'};
 a.live.push(packet(1,0,[1,2],{strokeId:'finished',final:true,finalBatch:true,totalPoints:2,syncStamp:stamp}));
 a.live.push(packet(1,0,[3,4],{strokeId:'drawing',syncStamp:stamp}));
 const confirm=(id,syncStamp)=>JSON.stringify({kind:'stroke',stroke:{id,syncStamp}});
 a.live.retireConfirmed([confirm('finished',{clock:1,actor:'writer'}),confirm('drawing',stamp)]);
 assert.equal(a.live.paths.size,2);
 a.live.retireConfirmed([confirm('finished',stamp)]);await a.live.flushFrame();
 const wire=JSON.parse(api.files.get(a.live.fileId).raw);
 assert.deepEqual(wire.packets.map(p=>p.strokeId),['drawing']);assert.equal(a.receipts.length,0);a.live.stop();
});

test('a stalled preview source does not freeze successive frames from another device',async()=>{
 const api=fakeInkDrive(),a=replica(api,'writer'),reader=replica(api,'reader');
 a.live.push(packet(1,0,[1,2]));await a.live.flushFrame();
 let release,slowCalls=0;const blocked=new Promise(r=>release=r),fetch=reader.live.host.fetch;
 reader.live.host.fetch=async(url,options)=>{
  if(url.includes('/slow?alt=media')){slowCalls++;await blocked;return new Response('{}');}
  return fetch(url,options);
 };
 reader.live.hint('slow');reader.live.hint(a.live.fileId);
 const first=reader.live.pollFrames();
 try{
  const deadline=Date.now()+1000;
  while(!reader.frames.length&&Date.now()<deadline)await new Promise(r=>setTimeout(r,2));
  assert.equal(reader.frames.length,1);
  a.live.push(packet(2,1,[2,3,4]));await a.live.flushFrame();
  reader.live.previewSources.get(a.live.fileId).next=0;
  await reader.live.pollFrames();
  assert.equal(reader.frames.length,2);assert.deepEqual(reader.frames[1].points.map(p=>p.x),[1,2,3,4]);
  assert.equal(slowCalls,1);
 }finally{reader.live.stop();a.live.stop();release();await first;}
});

test('preview polling bounds concurrency and ignores late responses after leaving',async()=>{
 const api=fakeInkDrive(),reader=replica(api,'reader');let release,active=0,maximum=0,calls=0;
 const gate=new Promise(r=>release=r);reader.live.nextList=Infinity;
 reader.live.host.fetch=async()=>{
  calls++;maximum=Math.max(maximum,++active);await gate;active--;
  return new Response(JSON.stringify({v:1,docId:'doc',actor:'writer',frame:1,at:Date.now(),packets:[packet(1,0,[1,2])]}));
 };
 for(let i=0;i<10;i++)reader.live.hint('remote-'+i);
 const first=reader.live.pollFrames();await reader.live.pollFrames();
 assert.equal(calls,4);assert.equal(maximum,4);
 reader.live.stop();release();await first;
 assert.equal(reader.frames.length,0);
});

function previewGate(){let release;const promise=new Promise(resolve=>release=resolve);return {promise,release};}
const nextPreviewTurn=()=>new Promise(resolve=>setImmediate(resolve));

test('cold preview folder discovery and ID reservation begin concurrently',async()=>{
 const api=fakeInkDrive(),a=replica(api,'writer'),folderGate=previewGate(),idGate=previewGate();
 const ensure=a.live.ensureFolder.bind(a.live),fetch=a.live.host.fetch;let folderStarted=false,idStarted=false;
 a.live.ensureFolder=async()=>{folderStarted=true;await folderGate.promise;return ensure();};
 a.live.host.fetch=async(url,options)=>{if(url.includes('/generateIds?')){idStarted=true;await idGate.promise;}return fetch(url,options);};
 a.live.push(packet(1,0,[1,2]));const sending=a.live.flushFrame();
 try{
  await nextPreviewTurn();assert.ok(folderStarted&&idStarted,'both independent requests start before either is released');
  assert.equal(a.live.writing,true);assert.equal(api.calls.filter(c=>c.url.includes('uploadType')).length,0);
  folderGate.release();await nextPreviewTurn();assert.equal(a.live.writing,true);
  idGate.release();await sending;
  assert.equal(a.live.exists,true);assert.equal(a.live.sentFrame,1);
  assert.equal(api.calls.filter(c=>c.url.includes('/generateIds?')).length,1);
 }finally{folderGate.release();idGate.release();await sending;a.live.stop();}
});

test('existing preview updates need neither folder discovery nor another reserved ID',async()=>{
 const api=fakeInkDrive(),a=replica(api,'writer');
 a.live.push(packet(1,0,[1,2]));await a.live.flushFrame();const id=a.live.fileId;
 a.live.folder=null;a.live.ensureFolder=async()=>{throw Error('An existing media update must not resolve its folder');};
 a.live.push(packet(2,1,[2,3]));await a.live.flushFrame();
 assert.equal(a.live.fileId,id);assert.equal(a.live.sentFrame,2);
 assert.equal(api.calls.filter(c=>c.url.includes('/generateIds?')).length,1);
 assert.deepEqual(JSON.parse(api.files.get(id).raw).packets[0].points.map(p=>p.x),[1,2,3]);a.live.stop();
});

for(const failed of ['folder','reservation'])test(`failed ${failed} setup waits for the other branch before a safe preview retry`,async()=>{
 const api=fakeInkDrive(),a=replica(api,'writer'),gate=previewGate();
 const ensure=a.live.ensureFolder.bind(a.live),fetch=a.live.host.fetch;let first=true,reservations=0,folders=0;
 a.live.ensureFolder=async()=>{
  folders++;
  if(first&&failed==='folder')throw Error('Folder temporarily unavailable');
  if(first)await gate.promise;
  return ensure();
 };
 a.live.host.fetch=async(url,options)=>{
  if(url.includes('/generateIds?')){
   reservations++;
   if(first&&failed==='reservation')throw Error('Reservation temporarily unavailable');
   if(first)await gate.promise;
  }
  return fetch(url,options);
 };
 a.live.push(packet(1,0,[1,2]));const sending=a.live.flushFrame();
 try{
  await nextPreviewTurn();assert.equal(a.live.writing,true,'one rejection cannot release the write lock');
  await a.live.flushFrame();assert.equal(folders,1);assert.equal(reservations,1);
  gate.release();await sending;assert.equal(a.live.writing,false);assert.equal(a.live.sentFrame,0);
  assert.equal(api.calls.filter(c=>c.url.includes('uploadType')).length,0);
  const reservedId=a.live.fileId;first=false;await a.live.flushFrame();
  assert.equal(a.live.exists,true);assert.equal(a.live.sentFrame,1);
  if(failed==='folder'){assert.ok(reservedId);assert.equal(a.live.fileId,reservedId);assert.equal(reservations,1);}
  else assert.equal(reservations,2);
 }finally{gate.release();await sending;a.live.stop();}
});

test('stopping during cold preview setup awaits both branches without publishing or accepting a late ID',async()=>{
 const api=fakeInkDrive(),a=replica(api,'writer'),folderGate=previewGate(),idGate=previewGate();
 const ensure=a.live.ensureFolder.bind(a.live),fetch=a.live.host.fetch;
 a.live.ensureFolder=async()=>{await folderGate.promise;return ensure();};
 a.live.host.fetch=async(url,options)=>{if(url.includes('/generateIds?'))await idGate.promise;return fetch(url,options);};
 a.live.push(packet(1,0,[1,2]));const sending=a.live.flushFrame();
 try{
  await nextPreviewTurn();a.live.stop();folderGate.release();await nextPreviewTurn();
  assert.equal(a.live.writing,true,'cancellation still waits for the reserved-ID request to settle');
  idGate.release();await sending;
  assert.equal(a.live.writing,false);assert.equal(a.live.fileId,'');assert.equal(a.live.exists,false);assert.equal(a.live.sentFrame,0);
  assert.equal(api.calls.filter(c=>c.url.includes('uploadType')).length,0);assert.equal(a.receipts.length,0);
 }finally{folderGate.release();idGate.release();await sending;a.live.stop();}
});

test('the first cold preview upload includes points that arrived during setup',async()=>{
 const api=fakeInkDrive(),a=replica(api,'writer'),gate=previewGate();
 const ensure=a.live.ensureFolder.bind(a.live);a.live.ensureFolder=async()=>{await gate.promise;return ensure();};
 a.live.frame=1; // Empty prewarm frame, as start() would schedule.
 const sending=a.live.flushFrame();
 try{
  await nextPreviewTurn();a.live.push(packet(1,0,[1,2]));a.live.push(packet(2,1,[2,3,4]));
  assert.equal(api.calls.filter(c=>c.url.includes('uploadType')).length,0);
  gate.release();await sending;
  const first=JSON.parse(api.files.get(a.live.fileId).raw);
  assert.equal(first.frame,3);assert.equal(a.live.sentFrame,3);assert.equal(a.live.sentFrame,a.live.frame);
  assert.deepEqual(first.packets[0].points.map(point=>point.x),[1,2,3,4]);
  assert.equal(api.calls.filter(c=>c.url.includes('uploadType')).length,1);
 }finally{gate.release();await sending;a.live.stop();}
});

for(const change of ['cancel','retire'])test(`a ${change} during cold setup cannot publish the superseded stroke`,async()=>{
 const api=fakeInkDrive(),a=replica(api,'writer'),gate=previewGate();
 const ensure=a.live.ensureFolder.bind(a.live);a.live.ensureFolder=async()=>{await gate.promise;return ensure();};
 const stamp={clock:2,actor:'writer'};
 a.live.push(packet(1,0,[1,2],{final:true,finalBatch:true,totalPoints:2,syncStamp:stamp}));
 const sending=a.live.flushFrame();
 try{
  await nextPreviewTurn();
  if(change==='cancel')a.live.push(packet(2,0,[],{cancel:true}));
  else a.live.retireConfirmed([JSON.stringify({kind:'stroke',stroke:{id:'pen',syncStamp:stamp}})]);
  gate.release();await sending;
  const first=JSON.parse(api.files.get(a.live.fileId).raw);
  assert.equal(first.frame,2);assert.equal(a.live.sentFrame,2);
  if(change==='cancel'){assert.equal(first.packets[0].cancel,true);assert.deepEqual(first.packets[0].points,[]);}
  else assert.deepEqual(first.packets,[]);
 }finally{gate.release();await sending;a.live.stop();}
});

test('latest-frame serialization retains the byte bound and can recover with the reserved identity',async()=>{
 const api=fakeInkDrive(),a=replica(api,'writer'),gate=previewGate();
 const ensure=a.live.ensureFolder.bind(a.live);a.live.ensureFolder=async()=>{await gate.promise;return ensure();};
 a.live.frame=1;const sending=a.live.flushFrame();
 try{
  await nextPreviewTurn();
  a.live.push({...packet(1,0,[1]),points:[{x:1,y:1,p:.5,extra:'x'.repeat(2*1024*1024)}]});
  gate.release();await sending;
  const id=a.live.fileId;assert.ok(id);assert.equal(a.live.sentFrame,0);assert.equal(a.live.exists,false);
  assert.equal(api.calls.filter(c=>c.url.includes('uploadType')).length,0);
  a.live.push(packet(2,0,[],{cancel:true}));await a.live.flushFrame();
  assert.equal(a.live.fileId,id);assert.equal(a.live.sentFrame,3);assert.equal(a.live.exists,true);
  assert.equal(api.calls.filter(c=>c.url.includes('/generateIds?')).length,1);
 }finally{gate.release();await sending;a.live.stop();}
});
