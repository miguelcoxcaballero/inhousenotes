import test from 'node:test';import assert from 'node:assert/strict';import {createRequire} from 'node:module';
import {fakeInkDrive,memoryInkStore} from './helpers/fake-ink-drive.mjs';
const {Journal}=createRequire(import.meta.url)('../drive-ink-journal-v5.js');
function replica(api,actor,store=memoryInkStore(),docId='doc'){
 const applied=[],receipts=[];
 const j=new Journal({store,account:'a',actor,docId,fetch:api.fetch,apply:async ops=>applied.push(...ops),receipt:r=>receipts.push(r)});
 j.wake=()=>{};return {j,store,applied,receipts};
}
async function flush(j){j.lastWrite=0;for(const record of await j.store.list('settings',j.account))if(record.retryAt)await j.patch(record.id,old=>({...old,retryAt:0}));await j.flush();}
async function poll(j){for(const s of j.sources.values())s.next=0;await j.poll();}
test('direct IDs carry consecutive strokes during search-index lag and search failure',async()=>{
 const api=fakeInkDrive(),a=replica(api,'a'),b=replica(api,'b');
 await a.j.enqueue({stroke:'one'},1);await flush(a.j);
 const id=a.j.current.fileId;assert.ok(id);b.j.hint(id);api.hideSearch=true;
 await poll(b.j);assert.deepEqual(b.applied,[{stroke:'one'}]);
 await a.j.enqueue({stroke:'two'},2);await flush(a.j);api.failSearch=true;b.j.nextDiscovery=0;
 await poll(b.j);await poll(b.j);assert.deepEqual(b.applied,[{stroke:'one'},{stroke:'two'}]);
 assert.equal(a.j.status().pending,0);assert.equal([...api.files.values()].filter(f=>f.name==='inhouse notes').length,1);
 assert.equal([...a.store.data.values()].find(v=>v.kind)?.id,a.j.key(a.j.current));
});
test('a late receipt cannot acknowledge a newer stroke, and restart drains durable edits',async()=>{
 const api=fakeInkDrive(),a=replica(api,'a');await a.j.enqueue({stroke:'one'},1);
 let release;api.hold=new Promise(r=>release=r);const writing=flush(a.j);
 while(![...api.files.values()].some(f=>f.raw))await new Promise(r=>setTimeout(r,1));
 await a.j.enqueue({stroke:'two'},2);release();await writing;api.hold=null;
 assert.equal(a.j.status().pending,1);assert.equal(a.receipts[0].version,1);
 a.j.stop();const recovery=replica(api,'new-process',a.store,'');await flush(recovery.j);
 const persisted=[...a.store.data.values()].find(v=>v.kind);assert.equal(persisted.ack,2);
 assert.deepEqual(JSON.parse(api.files.get(persisted.fileId).raw).ops,[{stroke:'one'},{stroke:'two'}]);
});
test('lost create response retries the reserved identity instead of duplicating a journal',async()=>{
 const api=fakeInkDrive(),a=replica(api,'a');api.loseCreate=true;
 await a.j.enqueue({stroke:'one'},1);await flush(a.j);assert.equal(a.j.status().pending,1);
 await flush(a.j);assert.equal(a.j.status().pending,0);
 assert.equal([...api.files.values()].filter(f=>f.raw).length,1);
});
test('rotation preserves a predecessor chain and two writers never overwrite one another',async()=>{
 const api=fakeInkDrive(),a=replica(api,'a'),b=replica(api,'b');
 for(let n=0;n<130;n++)await a.j.enqueue({n},n);
 await flush(a.j);await flush(a.j);
 await b.j.enqueue({b:1},1);await flush(b.j);
 const remote=[...api.files.values()].filter(f=>f.raw);assert.equal(remote.length,3);
 const tail=remote.find(f=>f.appProperties.ihnActor==='a'&&f.appProperties.ihnPart==='1');assert.ok(JSON.parse(tail.raw).previous);
 const reader=replica(api,'reader');api.hideSearch=true;reader.j.hint(tail.id);
 await poll(reader.j);await poll(reader.j);
 assert.equal(reader.applied.length,130);assert.equal(new Set(reader.applied.map(op=>op.n)).size,130);
});
test('wrong document and failed merges cannot advance the applied revision',async()=>{
 const api=fakeInkDrive(),a=replica(api,'a'),b=replica(api,'b',undefined,'other');
 await a.j.enqueue({stroke:'one'},1);await flush(a.j);b.j.hint(a.j.current.fileId);await poll(b.j);
 assert.equal(b.applied.length,0);assert.equal(b.j.seen.size,0);
 b.j.docId='doc';b.j.host.apply=async()=>{throw Error('disk full');};await poll(b.j);assert.equal(b.j.seen.size,0);
 b.j.host.apply=async ops=>b.applied.push(...ops);await poll(b.j);assert.equal(b.applied.length,1);
});

test('a stale writer cannot roll back an acknowledged newer file revision',async()=>{
 const api=fakeInkDrive(),a=replica(api,'a');
 await a.j.enqueue({stroke:1},1);await flush(a.j);
 const stale=await a.j.read(a.j.key(a.j.current));
 await a.j.enqueue({stroke:2},2);await flush(a.j);
 await assert.rejects(a.j.upload(stale,a.j.key(stale)),error=>error.status===412);
 assert.equal(JSON.parse(api.files.get(stale.fileId).raw).rev,2);
 // Recovery with an invalidated precondition recognizes the newer prefix;
 // it acknowledges only the requested revision without replacing remote bytes.
 stale.etag=null;await a.j.upload(stale,a.j.key(stale));
 assert.equal(JSON.parse(api.files.get(stale.fileId).raw).rev,2);
});
test('an unavailable discovery request never delays an already known live file',async()=>{
 const api=fakeInkDrive(),a=replica(api,'a'),b=replica(api,'b');
 await a.j.enqueue({stroke:1},1);await flush(a.j);b.j.hint(a.j.current.fileId);
 let release;const real=b.j.host.fetch;
 b.j.host.fetch=(url,options)=>url.includes('ihnKind')?new Promise(resolve=>release=resolve):real(url,options);
 await b.j.poll();assert.equal(b.applied.length,1);
 release(new Response('{"files":[]}'));await b.j.discovery;
});

test('a successor cannot seal a stale partial read of its predecessor',async()=>{
 const api=fakeInkDrive(),a=replica(api,'a'),reader=replica(api,'reader');
 await a.j.enqueue({n:0},0);await flush(a.j);
 const headId=a.j.current.fileId,early=api.files.get(headId).raw;
 for(let n=1;n<129;n++)await a.j.enqueue({n},n);
 await flush(a.j);await flush(a.j);const full=api.files.get(headId).raw;
 api.files.get(headId).raw=early;
 await poll(reader.j);assert.equal(reader.applied.length,2);
 assert.notEqual(reader.j.sources.get(headId).next,Infinity);
 api.files.get(headId).raw=full;await poll(reader.j);
 assert.equal(reader.applied.length,129);assert.equal(new Set(reader.applied.map(v=>v.n)).size,129);
});

test('slow discovery hints cannot hold the write lease or delay the next durable revision',async()=>{
 const api=fakeInkDrive(),a=replica(api,'a');let release;const gate=new Promise(r=>release=r),notices=[];
 a.j.host.announce=async(doc,id)=>{notices.push(id);await gate;};
 await a.j.enqueue({stroke:'first'},1);
 await Promise.race([flush(a.j),new Promise((_,reject)=>{const t=setTimeout(()=>reject(Error('hint blocked upload')),500);t.unref();})]);
 assert.equal(a.j.writing,false);assert.equal(a.receipts.length,1);
 await a.j.enqueue({stroke:'second'},2);await flush(a.j);
 assert.equal(a.j.status().pending,0);assert.equal(a.receipts[1].version,2);
 assert.equal(notices.length,1);assert.equal(a.j.announcements.size,1);
 release();await new Promise(r=>setTimeout(r,0));assert.equal(notices.length,2);a.j.stop();
});
test('an idle journal avoids storage churn but wakes immediately for new durable work',async()=>{
 const api=fakeInkDrive(),store=memoryInkStore();let leases=0;
 const lease=store.withLease;store.withLease=(...args)=>{leases++;return lease(...args);};
 const j=new Journal({store,account:'a',actor:'writer',docId:'doc',fetch:api.fetch});
 j.poll=async()=>{};j.start();
 try{
  await new Promise(r=>setTimeout(r,300));assert.equal(leases,1);
  const start=performance.now();await j.enqueue({stroke:'wake'},1);
  const deadline=Date.now()+1000;
  while(j.status().pending&&Date.now()<deadline)await new Promise(r=>setTimeout(r,5));
  assert.equal(j.status().pending,0);assert.ok(performance.now()-start<300);
 }finally{j.stop();}
});
test('failed advisory hints never turn a confirmed save into an upload failure',async()=>{
 const api=fakeInkDrive(),a=replica(api,'a');const warnings=[];
 a.j.host.announce=async()=>{throw Error('hint unavailable');};a.j.host.warning=e=>warnings.push(e.message);
 await a.j.enqueue({stroke:'safe'},1);await flush(a.j);await new Promise(r=>setTimeout(r,0));
 assert.equal(a.j.status().pending,0);assert.equal(a.j.status().error,'');assert.equal(a.receipts.length,1);
 assert.deepEqual(warnings,['hint unavailable']);a.j.stop();
});

test('a stalled writer cannot block consecutive edits from a responsive writer',async()=>{
 const api=fakeInkDrive(),slow=replica(api,'slow'),fast=replica(api,'fast'),reader=replica(api,'reader');
 await slow.j.enqueue({writer:'slow',n:1},1);await flush(slow.j);
 await fast.j.enqueue({writer:'fast',n:1},1);await flush(fast.j);
 reader.j.nextDiscovery=Infinity;reader.j.hint(slow.j.current.fileId);reader.j.hint(fast.j.current.fileId);
 let release;const gate=new Promise(r=>release=r),fetch=reader.j.host.fetch;
 reader.j.host.fetch=async(url,options)=>{if(url.includes(`/${slow.j.current.fileId}?alt=media`))await gate;return fetch(url,options);};
 const first=reader.j.poll();
 try{
  const deadline=Date.now()+500;
  while(reader.applied.length===0&&Date.now()<deadline)await new Promise(r=>setTimeout(r,2));
  assert.deepEqual(reader.applied,[{writer:'fast',n:1}]);assert.equal(reader.j.readTasks.size,1);
  await fast.j.enqueue({writer:'fast',n:2},2);await flush(fast.j);
  reader.j.sources.get(fast.j.current.fileId).next=0;await reader.j.poll();
  assert.deepEqual(reader.applied,[{writer:'fast',n:1},{writer:'fast',n:2}]);
  assert.equal(reader.j.readTasks.size,1);assert.equal(reader.j.seen.has(slow.j.current.fileId),false);
 }finally{release();await first;reader.j.stop();}
 assert.deepEqual(reader.applied.at(-1),{writer:'slow',n:1});
});

test('journal reads have four global lanes and one request per source',async()=>{
 const api=fakeInkDrive(),reader=replica(api,'reader');reader.j.nextDiscovery=Infinity;
 const releases=[];let active=0,peak=0,requests=0;
 reader.j.host.fetch=(url,{signal})=>new Promise((resolve,reject)=>{
  const id=new URL(url).pathname.split('/').at(-1);requests++;active++;peak=Math.max(peak,active);
  let done=false;const end=()=>{if(done)return false;done=true;active--;return true;};
  signal.addEventListener('abort',()=>{if(end())reject(new DOMException('Closed','AbortError'));},{once:true});
  releases.push(()=>{if(end())resolve(new Response(JSON.stringify({v:1,docId:'doc',actor:id,part:0,rev:1,ops:[{id}]})));});
 });
 for(let n=0;n<8;n++)reader.j.hint(`source-${n}`);
 const first=reader.j.poll();await reader.j.poll();await reader.j.poll();
 assert.equal(requests,4);assert.equal(peak,4);assert.equal(reader.j.readTasks.size,4);
 releases[0]();await new Promise(r=>setTimeout(r,0));
 const next=reader.j.poll();assert.equal(requests,5);assert.equal(peak,4);
 reader.j.stop();await Promise.all([first,next]);
 for(const release of releases)release();
 assert.equal(reader.applied.length,1);assert.equal(reader.j.readTasks.size,0);
});

test('parallel responses serialize merges and a stopped journal never applies a queued response',async()=>{
 const api=fakeInkDrive(),a=replica(api,'a'),b=replica(api,'b'),reader=replica(api,'reader');
 await a.j.enqueue({writer:'a'},1);await flush(a.j);await b.j.enqueue({writer:'b'},1);await flush(b.j);
 reader.j.nextDiscovery=Infinity;reader.j.hint(a.j.current.fileId);reader.j.hint(b.j.current.fileId);
 let release,entered=0;const gate=new Promise(r=>release=r);
 reader.j.host.idle=async()=>{entered++;await gate;};
 const reading=reader.j.poll();await new Promise(r=>setTimeout(r,0));
 assert.equal(reader.j.readTasks.size,2);assert.equal(entered,1);assert.equal(reader.applied.length,0);
 reader.j.stop();release();await reading;
 assert.equal(reader.applied.length,0);assert.equal(reader.j.seen.size,0);assert.equal(reader.j.readTasks.size,0);
});
