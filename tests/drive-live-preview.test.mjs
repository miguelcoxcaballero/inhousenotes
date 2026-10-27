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
