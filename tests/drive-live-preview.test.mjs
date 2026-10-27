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

test('identical large live frames decode once across fifty reads without changing the path',async()=>{
 const api=fakeInkDrive(),reader=replica(api,'reader');reader.live.nextList=Infinity;
 const points=Array.from({length:6000},(_,i)=>({x:i*.25,y:i*.5,p:.75}));
 const raw=JSON.stringify({v:1,docId:'doc',actor:'writer',frame:1,at:Date.now(),packets:[{...packet(1,0,[]),points}]});
 api.files.set('cached',{id:'cached',raw});reader.live.hint('cached');
 const source=reader.live.previewSources.get('cached'),parse=JSON.parse;let parses=0;
 JSON.parse=(value,...args)=>{if(value===raw)parses++;return parse(value,...args);};
 try{
  for(let i=0;i<50;i++)await reader.live.readPreview('cached',source);
  assert.equal(parses,1);assert.equal(reader.frames.length,1);assert.deepEqual(reader.frames[0].points,points);
  assert.equal(reader.receipts.length,0);assert.equal(source.frame,1);
  console.log('DRIVE_PREVIEW_REPEATED_PARSES',JSON.stringify({reads:50,parses,points:points.length,chars:raw.length}));
 }finally{JSON.parse=parse;reader.live.stop();}
});

test('cached frames retain the original activity and freshness deadlines',async t=>{
 t.mock.timers.enable({apis:['Date'],now:100000});
 const api=fakeInkDrive(),reader=replica(api,'reader');
 const raw=JSON.stringify({v:1,docId:'doc',actor:'writer',frame:1,at:Date.now(),packets:[packet(1,0,[1,2])]});
 api.files.set('cached',{id:'cached',raw});reader.live.hint('cached');
 const source=reader.live.previewSources.get('cached');
 try{
  await reader.live.readPreview('cached',source);const activeUntil=source.activeUntil;
  t.mock.timers.setTime(104999);await reader.live.readPreview('cached',source);
  assert.equal(source.activeUntil,activeUntil);assert.equal(source.next,105099);
  // The activity window has closed; the writer stays warm (300 ms polls).
  t.mock.timers.setTime(105001);await reader.live.readPreview('cached',source);
  assert.equal(source.activeUntil,activeUntil);assert.equal(source.next,105301);
  t.mock.timers.setTime(120001);await reader.live.readPreview('cached',source);
  assert.equal(source.activeUntil,0);assert.equal(source.next,135001);assert.equal(reader.live.readCache.size,0);
  assert.equal(reader.frames.length,1);
 }finally{reader.live.stop();t.mock.timers.reset();}
});

test('changed bytes at the same frame number still validate before final and cancelled paths arrive',async()=>{
 const api=fakeInkDrive(),reader=replica(api,'reader');
 const initial={v:1,docId:'doc',actor:'writer',frame:1,at:Date.now(),packets:[packet(1,0,[1,2])]};
 const file={id:'cached',raw:JSON.stringify(initial)};api.files.set(file.id,file);reader.live.hint(file.id);
 const source=reader.live.previewSources.get(file.id),parse=JSON.parse;let parses=0;
 JSON.parse=(value,...args)=>{if(value===file.raw)parses++;return parse(value,...args);};
 try{
  await reader.live.readPreview(file.id,source);
  file.raw=JSON.stringify({...initial,packets:[packet(2,0,[9,10])]});await reader.live.readPreview(file.id,source);
  assert.equal(parses,2);assert.equal(reader.frames.length,1);
  file.raw=JSON.stringify({...initial,docId:'other'});await reader.live.readPreview(file.id,source);
  assert.equal(parses,3);assert.equal(reader.frames.length,1);assert.equal(source.frame,1);
  file.raw=JSON.stringify({...initial,at:Date.now()-60000});await reader.live.readPreview(file.id,source);
  assert.equal(parses,4);assert.equal(source.activeUntil,0);assert.ok(source.next>Date.now()+14000);
  file.raw=JSON.stringify({...initial,frame:2,packets:[packet(3,0,[4,5],{final:true,finalBatch:true,totalPoints:2})]});
  await reader.live.readPreview(file.id,source);assert.deepEqual(reader.frames.at(-1).points.map(point=>point.x),[4,5]);
  assert.equal(reader.frames.at(-1).final,true);
  file.raw=JSON.stringify({...initial,frame:3,packets:[packet(4,0,[],{cancel:true})]});
  await reader.live.readPreview(file.id,source);assert.equal(reader.frames.at(-1).cancel,true);assert.equal(source.frame,3);
  assert.equal(reader.receipts.length,0);
 }finally{JSON.parse=parse;reader.live.stop();}
});

test('preview decode cache is bounded, evicts least-recently-read frames and cannot cross source identities',async()=>{
 const api=fakeInkDrive(),reader=replica(api,'reader');reader.live.nextList=Infinity;
 const rawById=new Map(),parses=new Map();
 for(let i=0;i<3;i++){
  const id='cached-'+i,raw=JSON.stringify({v:1,docId:'doc',actor:'writer-'+i,frame:1,at:Date.now(),padding:'x'.repeat(1536*1024),packets:[packet(1,0,[i])]});
  rawById.set(id,raw);api.files.set(id,{id,raw});reader.live.hint(id);
 }
 const parse=JSON.parse;JSON.parse=(value,...args)=>{
  for(const [id,raw]of rawById)if(value===raw)parses.set(id,(parses.get(id)||0)+1);
  return parse(value,...args);
 };
 const read=id=>reader.live.readPreview(id,reader.live.previewSources.get(id));
 try{
  await read('cached-0');await read('cached-1');await read('cached-0');await read('cached-2');
  assert.ok(reader.live.readCacheChars<=4*1024*1024);assert.equal(reader.live.readCache.size,2);
  assert.equal(parses.get('cached-0'),1);await read('cached-1');assert.equal(parses.get('cached-1'),2);
  const emitted=reader.frames.length;reader.live.previewSources.delete('cached-1');reader.live.hint('cached-1');
  await read('cached-1');assert.equal(parses.get('cached-1'),3);assert.equal(reader.frames.length,emitted+1);
  reader.live.previewSources.delete('cached-1');
  for(const source of reader.live.previewSources.values())source.next=Infinity;
  await reader.live.pollFrames();assert.equal(reader.live.readCache.has('cached-1'),false);
  reader.live.stop();assert.equal(reader.live.readCache.size,0);assert.equal(reader.live.readCacheChars,0);
 }finally{JSON.parse=parse;reader.live.stop();}
});

test('failed preview callbacks and failed HTTP reads never become successful cache proofs',async t=>{
 t.mock.timers.enable({apis:['Date'],now:100000});
 const api=fakeInkDrive(),reader=replica(api,'reader');
 const raw=JSON.stringify({v:1,docId:'doc',actor:'writer',frame:1,at:Date.now(),packets:[packet(1,0,[1,2])]});
 api.files.set('cached',{id:'cached',raw});reader.live.hint('cached');
 const source=reader.live.previewSources.get('cached'),preview=reader.live.host.preview,fetch=reader.live.host.fetch,parse=JSON.parse;let parses=0;
 JSON.parse=(value,...args)=>{if(value===raw)parses++;return parse(value,...args);};
 try{
  reader.live.host.preview=()=>{throw Error('Preview draw failed');};await reader.live.readPreview('cached',source);
  assert.equal(source.frame,0);assert.equal(source.next,103000);assert.equal(reader.live.readCache.size,0);
  reader.live.host.preview=preview;await reader.live.readPreview('cached',source);
  assert.equal(parses,2);assert.equal(reader.frames.length,1);assert.equal(source.frame,1);
  reader.live.host.fetch=async()=>new Response('{}',{status:503});await reader.live.readPreview('cached',source);
  assert.equal(source.next,103000);assert.equal(reader.frames.length,1);
  reader.live.host.fetch=fetch;await reader.live.readPreview('cached',source);
  assert.equal(parses,2);assert.equal(reader.frames.length,1);
 }finally{JSON.parse=parse;reader.live.stop();t.mock.timers.reset();}
});

test('stopping during a cached-source read releases memory and ignores its late response',async()=>{
 const api=fakeInkDrive(),reader=replica(api,'reader'),gate=previewGate();
 const frame={v:1,docId:'doc',actor:'writer',frame:1,at:Date.now(),packets:[packet(1,0,[1,2])]};
 api.files.set('cached',{id:'cached',raw:JSON.stringify(frame)});reader.live.hint('cached');
 const source=reader.live.previewSources.get('cached');await reader.live.readPreview('cached',source);
 assert.ok(reader.live.readCacheChars>0);
 reader.live.host.fetch=async()=>{await gate.promise;return new Response(JSON.stringify({...frame,frame:2,packets:[packet(2,0,[3,4])]}));};
 const reading=reader.live.readPreview('cached',source);
 try{
  await nextPreviewTurn();reader.live.stop();assert.equal(reader.live.readCacheChars,0);
  gate.release();await reading;
  assert.equal(reader.frames.length,1);assert.equal(reader.live.readCache.size,0);assert.equal(reader.live.readCacheChars,0);
 }finally{gate.release();await reading;reader.live.stop();}
});

test('fresh preview activity preempts a sleeping timer and later polling cannot postpone it',async t=>{
 t.mock.timers.enable({apis:['Date','setTimeout'],now:100000});
 const api=fakeInkDrive(),reader=replica(api,'reader');reader.live.nextList=Infinity;
 reader.live.schedulePoll=LivePreview.prototype.schedulePoll;
 const source={next:115000,retryAt:0,frame:0,activeUntil:0};reader.live.previewSources.set('sleeping',source);
 api.files.set('sleeping',{id:'sleeping',raw:JSON.stringify({v:1,docId:'doc',actor:'writer',frame:1,at:Date.now(),packets:[packet(1,0,[1,2])]})});
 try{
  reader.live.schedulePoll(15000);const oldTimer=reader.live.pollTimer;
  reader.live.hint('sleeping',true);const wakeTimer=reader.live.pollTimer;
  assert.notEqual(wakeTimer,oldTimer);assert.equal(reader.live.pollAt,100000);assert.equal(source.next,100000);
  reader.live.schedulePoll(100);assert.equal(reader.live.pollTimer,wakeTimer);
  t.mock.timers.tick(0);await nextPreviewTurn();
  assert.equal(api.calls.filter(call=>call.url.includes('alt=media')).length,1);assert.equal(reader.frames.length,1);
  assert.deepEqual(reader.frames[0].points.map(point=>point.x),[1,2]);assert.equal(reader.receipts.length,0);
  console.log('DRIVE_PREVIEW_HINT_WAKE_MS',Date.now()-100000);
 }finally{reader.live.stop();t.mock.timers.reset();}
});

test('passive discovery leaves existing deadlines alone and gives new sources a timely first probe',async t=>{
 t.mock.timers.enable({apis:['Date'],now:100000});
 const api=fakeInkDrive(),reader=replica(api,'reader'),gate=previewGate();reader.live.nextList=Infinity;
 const passive={next:115000,retryAt:0,frame:0,activeUntil:0};reader.live.previewSources.set('sleeping',passive);
 api.files.set('sleeping',{id:'sleeping',appProperties:{ihnKind:'ink-preview-v1',ihnDoc:'doc'}});
 const read=[];reader.live.host.fetch=async(url,options)=>{
  if(!url.includes('alt=media'))return api.fetch(url,options);
  read.push(new URL(url).pathname.split('/').at(-1));await gate.promise;return new Response('{}');
 };
 let polling;
 try{
  reader.live.hint('sleeping',false);await reader.live.discoverPreviews();
  assert.equal(passive.next,115000);assert.equal(passive.activeUntil,0);
  for(let i=0;i<4;i++)reader.live.previewSources.set('busy-'+i,{next:0,retryAt:0,frame:0,activeUntil:104999});
  reader.live.hint('new-passive',false);const first=reader.live.previewSources.get('new-passive');
  assert.equal(first.activeUntil,105000);assert.equal(first.next,0);
  polling=reader.live.pollFrames();await nextPreviewTurn();
  assert.equal(read.length,4);assert.equal(read[0],'new-passive');assert.equal(first.inFlight,true);
  t.mock.timers.setTime(100100);reader.live.hint('new-passive',false);
  assert.equal(first.activeUntil,105000,'a repeated listing cannot renew the first-probe priority');
 }finally{reader.live.stop();gate.release();await polling;t.mock.timers.reset();}
});

test('active preview hints cannot bypass failed-read backoff and success restores normal cadence',async t=>{
 t.mock.timers.enable({apis:['Date'],now:100000});
 const api=fakeInkDrive(),reader=replica(api,'reader');reader.live.nextList=Infinity;reader.live.hint('failed');
 let calls=0,fail=true;reader.live.host.fetch=async()=>{
  calls++;return fail?new Response('{}',{status:503}):new Response(JSON.stringify({v:1,docId:'doc',actor:'writer',frame:1,at:Date.now(),packets:[packet(1,0,[1,2])]}));
 };
 const source=reader.live.previewSources.get('failed');
 try{
  await reader.live.pollFrames();assert.equal(calls,1);assert.equal(source.retryAt,103000);
  for(const now of [100001,100100,101000,102999]){
   t.mock.timers.setTime(now);reader.live.hint('failed',true);reader.live.hint('failed',false);
   source.next=0;await reader.live.pollFrames();assert.equal(calls,1,'the independent cooldown survives earlier poll wakes');
  }
  fail=false;t.mock.timers.setTime(103000);reader.live.hint('failed',true);await reader.live.pollFrames();
  assert.equal(calls,2);assert.equal(source.retryAt,0);assert.equal(source.next,103100);assert.equal(reader.frames.length,1);
 }finally{reader.live.stop();t.mock.timers.reset();}
});

for(const invalid of ['document','version','packets','oversize'])test(`active hints retain the cooldown for an invalid ${invalid} preview response`,async t=>{
 t.mock.timers.enable({apis:['Date'],now:100000});
 const api=fakeInkDrive(),reader=replica(api,'reader');reader.live.nextList=Infinity;reader.live.hint('invalid');
 const frame={v:1,docId:'doc',actor:'writer',frame:1,at:Date.now(),packets:[packet(1,0,[1,2])]};
 if(invalid==='document')frame.docId='other';if(invalid==='version')frame.v=2;if(invalid==='packets')frame.packets={};
 const raw=invalid==='oversize'?'x'.repeat(2*1024*1024+1):JSON.stringify(frame);let calls=0;
 reader.live.host.fetch=async()=>{calls++;return new Response(raw);};
 const source=reader.live.previewSources.get('invalid');
 try{
  await reader.live.pollFrames();assert.equal(source.retryAt,102000);assert.equal(source.frame,0);assert.equal(reader.live.readCache.size,0);
  for(const now of [100001,101000,101999]){
   t.mock.timers.setTime(now);reader.live.hint('invalid');source.next=0;await reader.live.pollFrames();assert.equal(calls,1);
  }
  t.mock.timers.setTime(102000);reader.live.hint('invalid');await reader.live.pollFrames();assert.equal(calls,2);
  assert.equal(reader.frames.length,0);assert.equal(reader.receipts.length,0);
 }finally{reader.live.stop();t.mock.timers.reset();}
});

for(const failed of [false,true])test(`a hint during an in-flight ${failed?'failed':'stale'} read preserves one safe follow-up`,async t=>{
 t.mock.timers.enable({apis:['Date'],now:100000});
 const api=fakeInkDrive(),reader=replica(api,'reader'),gate=previewGate();reader.live.nextList=Infinity;reader.live.hint('remote');
 const frame={v:1,docId:'doc',actor:'writer',frame:1,at:Date.now(),packets:[packet(1,0,[1,2])]};let calls=0;
 reader.live.host.fetch=async()=>{
  const attempt=++calls;
  if(attempt===1){await gate.promise;return failed?new Response('{}',{status:503}):new Response(JSON.stringify({...frame,at:Date.now()-60000}));}
  return new Response(JSON.stringify(frame));
 };
 const source=reader.live.previewSources.get('remote'),reading=reader.live.pollFrames();
 try{
  await nextPreviewTurn();assert.equal(source.inFlight,true);
  reader.live.hint('remote');reader.live.hint('remote');await reader.live.pollFrames();assert.equal(calls,1);
  gate.release();await reading;
  assert.equal(source.next,failed?103000:100000);assert.equal(source.retryAt,failed?103000:0);
  await reader.live.pollFrames();assert.equal(calls,failed?1:2);
  if(failed){t.mock.timers.setTime(103000);await reader.live.pollFrames();assert.equal(calls,2);}
  assert.equal(reader.frames.length,1);assert.equal(source.retryAt,0);assert.equal(reader.receipts.length,0);
  await reader.live.pollFrames();assert.equal(calls,2,'the hints coalesce, rather than queue duplicate reads');
 }finally{reader.live.stop();gate.release();await reading;t.mock.timers.reset();}
});

test('stopping removes an activity wake and later hints cannot restart it',async t=>{
 t.mock.timers.enable({apis:['Date','setTimeout'],now:100000});
 const api=fakeInkDrive(),reader=replica(api,'reader');reader.live.nextList=Infinity;
 reader.live.schedulePoll=LivePreview.prototype.schedulePoll;
 try{
  reader.live.hint('remote');assert.ok(reader.live.pollTimer);reader.live.stop();reader.live.hint('remote');
  t.mock.timers.tick(20000);await nextPreviewTurn();
  assert.equal(reader.live.pollTimer,null);assert.equal(reader.live.pollAt,0);assert.equal(reader.live.previewSources.size,0);assert.equal(api.calls.length,0);
 }finally{reader.live.stop();t.mock.timers.reset();}
});

async function idlePreviewTrace(t,scenario,baseline){
 t.mock.timers.enable({apis:['Date','setTimeout'],now:100000});
 const requests=[];let polls=0;
 const live=new LivePreview({store:memoryInkStore(),account:'a',docId:'doc',actor:'reader',canPublish:false,
  fetch:async url=>{
   const media=url.includes('alt=media');requests.push({kind:media?'media':'discovery',at:Date.now()});
   return new Response(JSON.stringify(media?{v:1,docId:'doc',actor:'writer',frame:1,
    at:scenario==='inactive-frame'?100000:Date.now()-60000,packets:[]}:{files:[]}));
  }});
 const poll=live.pollFrames.bind(live);live.pollFrames=()=>{polls++;return poll();};
 // Reproduce the previous fixed 100ms scheduling policy for comparison,
 // retaining the same transport, sources, responses and discovery cadence.
 if(baseline)live.schedulePoll=function(delay=100){return LivePreview.prototype.schedulePoll.call(this,Math.min(delay,100));};
 if(scenario!=='no-sources')live.previewSources.set('remote',{next:scenario==='inactive-frame'?102000:115000,retryAt:0,frame:1,activeUntil:0});
 try{
  live.start();t.mock.timers.tick(0);await nextPreviewTurn();
  for(let i=0;i<150;i++){t.mock.timers.tick(100);await nextPreviewTurn();}
  const trace={polls,requests};live.stop();t.mock.timers.tick(60000);await nextPreviewTurn();
  assert.equal(polls,trace.polls);assert.equal(live.pollTimer,null);return trace;
 }finally{live.stop();t.mock.timers.reset();}
}

for(const scenario of ['no-sources','expired-source','inactive-frame'])test(`quiet ${scenario} polling sleeps without changing any request timestamp`,async t=>{
 const baseline=await idlePreviewTrace(t,scenario,true),optimized=await idlePreviewTrace(t,scenario,false);
 assert.deepEqual(optimized.requests,baseline.requests);assert.equal(baseline.polls,151);
 assert.deepEqual(optimized.requests.filter(request=>request.kind==='discovery').map(request=>request.at),[100000,105000,110000,115000]);
 if(scenario==='inactive-frame')assert.ok(optimized.polls<30);
 else assert.equal(optimized.polls,7);
 console.log('DRIVE_PREVIEW_IDLE_PASSES',JSON.stringify({scenario,before:baseline.polls,after:optimized.polls,requests:optimized.requests.length}));
});

test('new activity interrupts an idle deadline immediately and successive frames keep their 100ms cadence',async t=>{
 t.mock.timers.enable({apis:['Date','setTimeout'],now:100000});
 const requests=[],frames=[];let frame=1;
 const live=new LivePreview({store:memoryInkStore(),account:'a',docId:'doc',actor:'reader',canPublish:false,
  preview:value=>frames.push({value,at:Date.now()}),fetch:async url=>{
   if(!url.includes('alt=media'))return new Response('{"files":[]}');
   requests.push(Date.now());return new Response(JSON.stringify({v:1,docId:'doc',actor:'writer',frame,at:Date.now(),packets:[packet(frame,0,[1,frame+1])]}));
  }});
 live.previewSources.set('sleeping',{next:115000,retryAt:0,frame:0,activeUntil:0});
 try{
  live.start();t.mock.timers.tick(0);await nextPreviewTurn();
  for(let i=0;i<60;i++){t.mock.timers.tick(100);await nextPreviewTurn();}
  assert.equal(live.pollAt,110000);assert.equal(requests.length,0);
  live.hint('sleeping');assert.equal(live.pollAt,106000);t.mock.timers.tick(0);await nextPreviewTurn();
  assert.deepEqual(requests,[106000]);assert.equal(frames.length,1);
  frame=2;t.mock.timers.tick(100);await nextPreviewTurn();
  assert.deepEqual(requests,[106000,106100]);assert.equal(frames.length,2);assert.deepEqual(frames[1].value.points.map(point=>point.x),[1,3]);
 }finally{live.stop();t.mock.timers.reset();}
});

test('quiet failure retries and discovery retain their absolute deadlines despite hints',async t=>{
 t.mock.timers.enable({apis:['Date','setTimeout'],now:100000});
 const requests=[];
 const live=new LivePreview({store:memoryInkStore(),account:'a',docId:'doc',actor:'reader',canPublish:false,fetch:async url=>{
  const media=url.includes('alt=media');requests.push({kind:media?'media':'discovery',at:Date.now()});
  return media?new Response('{}',{status:503}):new Response('{"files":[]}');
 }});
 try{
  live.hint('failing');live.start();t.mock.timers.tick(0);await nextPreviewTurn();
  for(let i=0;i<51;i++){
   t.mock.timers.tick(100);await nextPreviewTurn();
   if(i===9||i===19){live.hint('failing');live.hint('failing',false);}
  }
  assert.deepEqual(requests.filter(request=>request.kind==='media').map(request=>request.at),[100000,103000]);
  assert.deepEqual(requests.filter(request=>request.kind==='discovery').map(request=>request.at),[100000,105000]);
  assert.equal(live.previewSources.get('failing').retryAt,106000);assert.equal(live.pollAt,106000);
 }finally{live.stop();t.mock.timers.reset();}
});

test('no finite deadline waits for a real hint instead of clamping an infinite timer into a busy loop',async t=>{
 t.mock.timers.enable({apis:['Date','setTimeout'],now:100000});
 const api=fakeInkDrive(),reader=replica(api,'reader');reader.live.nextList=Infinity;
 reader.live.schedulePoll=LivePreview.prototype.schedulePoll;
 const source={next:Infinity,retryAt:Infinity,frame:0,activeUntil:0};reader.live.previewSources.set('sleeping',source);
 api.files.set('sleeping',{id:'sleeping',raw:JSON.stringify({v:1,docId:'doc',actor:'writer',frame:1,at:Date.now(),packets:[packet(1,0,[1,2])]})});
 try{
  await reader.live.pollFrames();reader.live.schedulePoll(Infinity);reader.live.hint('sleeping');
  assert.equal(reader.live.pollTimer,null);t.mock.timers.tick(1000);await nextPreviewTurn();assert.equal(api.calls.length,0);
  reader.live.schedulePoll(Number.MAX_VALUE);assert.equal(reader.live.pollAt,Date.now()+2147483647);
  t.mock.timers.tick(1);await nextPreviewTurn();assert.equal(api.calls.length,0);
  source.retryAt=0;reader.live.hint('sleeping');assert.equal(reader.live.pollAt,Date.now());
  t.mock.timers.tick(0);await nextPreviewTurn();assert.equal(reader.frames.length,1);assert.equal(api.calls.length,1);
 }finally{reader.live.stop();t.mock.timers.reset();}
});

test('four stalled read lanes retain bounded 100ms scheduling and a queued source starts when a lane returns',async t=>{
 t.mock.timers.enable({apis:['Date','setTimeout'],now:100000});
 const gates=new Map(),calls=[];let active=0,maximum=0,polls=0;
 const live=new LivePreview({store:memoryInkStore(),account:'a',docId:'doc',actor:'reader',canPublish:false,fetch:async url=>{
  const id=new URL(url).pathname.split('/').at(-1);calls.push(id);maximum=Math.max(maximum,++active);
  const gate=gates.get(id);if(gate)await gate.promise;active--;
  return new Response(JSON.stringify({v:1,docId:'doc',actor:'writer',frame:1,at:Date.now()-60000,packets:[]}));
 }});
 live.nextList=Infinity;const poll=live.pollFrames.bind(live);live.pollFrames=()=>{polls++;return poll();};
 for(let i=0;i<4;i++){gates.set('slow-'+i,previewGate());live.hint('slow-'+i);}
 try{
  live.start();t.mock.timers.tick(0);await nextPreviewTurn();
  for(let i=0;i<10;i++){t.mock.timers.tick(100);await nextPreviewTurn();}
  assert.equal(polls,11);assert.equal(calls.length,4);assert.equal(maximum,4);assert.equal(live.pollAt,101100);
  live.hint('waiting',false);t.mock.timers.tick(0);await nextPreviewTurn();assert.equal(calls.length,4);assert.equal(live.pollAt,101100);
  gates.get('slow-0').release();await nextPreviewTurn();t.mock.timers.tick(100);await nextPreviewTurn();
  assert.equal(calls.at(-1),'waiting');assert.equal(calls.length,5);assert.equal(maximum,4);
 }finally{live.stop();for(const gate of gates.values())gate.release();await nextPreviewTurn();t.mock.timers.reset();}
});

test('an in-flight discovery retains bounded polling while a direct hint delivers new ink immediately',async t=>{
 t.mock.timers.enable({apis:['Date','setTimeout'],now:100000});
 const gate=previewGate(),frames=[];let polls=0,listings=0,reads=0;
 const live=new LivePreview({store:memoryInkStore(),account:'a',docId:'doc',actor:'reader',canPublish:false,
  preview:value=>frames.push(value),fetch:async url=>{
   if(!url.includes('alt=media')){listings++;await gate.promise;return new Response('{"files":[]}');}
   reads++;return new Response(JSON.stringify({v:1,docId:'doc',actor:'writer',frame:1,at:Date.now(),packets:[packet(1,0,[1,2])]}));
  }});
 const poll=live.pollFrames.bind(live);live.pollFrames=()=>{polls++;return poll();};
 try{
  live.start();t.mock.timers.tick(0);await nextPreviewTurn();
  for(let i=0;i<10;i++){t.mock.timers.tick(100);await nextPreviewTurn();}
  assert.equal(polls,11);assert.equal(listings,1);assert.equal(reads,0);assert.equal(live.pollAt,101100);
  live.hint('active');assert.equal(live.pollAt,101000);t.mock.timers.tick(0);await nextPreviewTurn();
  assert.equal(reads,1);assert.equal(frames.length,1);assert.equal(listings,1);assert.equal(live.pollAt,101100);
 }finally{live.stop();gate.release();await nextPreviewTurn();t.mock.timers.reset();}
});

test('a device that wrote recently stays warm: a new stroke after a pause is read within 300 ms',async t=>{
 t.mock.timers.enable({apis:['Date'],now:100000});
 let frame=0;const reads=[];
 const raw=()=>JSON.stringify({v:1,docId:'doc',actor:'writer',frame,at:Date.now(),packets:[packet(frame,0,[1,2])]});
 const live=new LivePreview({store:memoryInkStore(),account:'a',docId:'doc',actor:'reader',
  fetch:async()=>{reads.push(Date.now());return new Response(raw());},preview:()=>{}});
 live.scheduleFrame=()=>{};live.schedulePoll=()=>{};
 const source={next:0,retryAt:0,frame:0,activeUntil:0,wakeEpoch:0};live.previewSources.set('remote',source);
 frame=1;await live.readPreview('remote',source);
 assert.equal(source.frame,1);assert.equal(source.next-Date.now(),100);
 // Ten seconds of pause: the activity window has closed, the device is warm.
 t.mock.timers.setTime(110000);await live.readPreview('remote',source);
 assert.equal(source.next-Date.now(),300);
 // After two quiet minutes polling relaxes to the ordinary interval.
 t.mock.timers.setTime(100000+120001);await live.readPreview('remote',source);
 assert.equal(source.next-Date.now(),2000);
 live.stop();
});
