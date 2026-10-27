import test from 'node:test';import assert from 'node:assert/strict';import {createRequire} from 'node:module';
import {fakeInkDrive,memoryInkStore} from './helpers/fake-ink-drive.mjs';
const {Journal}=createRequire(import.meta.url)('../drive-ink-journal-v5.js');
function replica(api,actor,store=memoryInkStore(),docId='doc'){
 const applied=[],receipts=[];
 const j=new Journal({store,account:'a',actor,docId,fetch:api.fetch,apply:async ops=>applied.push(...ops),receipt:r=>receipts.push(r)});
 j.wake=()=>{};return {j,store,applied,receipts};
}
async function flush(j){j.lastWrite=0;j.nextOutboxCheck=0;for(const record of await j.store.list('settings',j.account))if(record.retryAt)await j.patch(record.id,old=>({...old,retryAt:0}));await j.flush();}
async function poll(j){for(const s of j.sources.values()){s.next=0;s.retryAt=0;}await j.poll();}
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

test('cold uploads reserve their identity while folder lookup is still pending',async()=>{
 const api=fakeInkDrive(),a=replica(api,'a');const fetch=a.j.host.fetch;let folderStarted=false,idStarted=false;
 let releaseFolder,releaseId;const folderGate=new Promise(r=>releaseFolder=r),idGate=new Promise(r=>releaseId=r);
 a.j.host.fetch=async(url,options)=>{
  const parsed=new URL(url),q=parsed.searchParams.get('q')||'';
  if(parsed.pathname.endsWith('/generateIds')){idStarted=true;await idGate;}
  if(q.includes("mimeType='application/vnd.google-apps.folder'")){folderStarted=true;await folderGate;}
  return fetch(url,options);
 };
 await a.j.enqueue({stroke:'first'},1);const writing=flush(a.j);
 try{
  const deadline=Date.now()+500;while((!folderStarted||!idStarted)&&Date.now()<deadline)await new Promise(r=>setTimeout(r,2));
  assert.equal(folderStarted,true);assert.equal(idStarted,true);assert.equal(a.receipts.length,0);
 }finally{releaseFolder();releaseId();await writing;}
 assert.equal(a.j.status().pending,0);assert.equal(a.receipts.length,1);a.j.stop();
});

test('failed folder setup retains the write lease until identity reservation has settled',async()=>{
 const api=fakeInkDrive(),a=replica(api,'a');const fetch=a.j.host.fetch,ensureFolder=a.j.ensureFolder;
 let release,started=false,finished=false;const gate=new Promise(r=>release=r);
 a.j.ensureFolder=async()=>{throw Error('Folder temporarily unavailable');};
 a.j.host.fetch=async(url,options)=>{if(url.includes('/generateIds')){started=true;await gate;}return fetch(url,options);};
 await a.j.enqueue({stroke:'first'},1);const writing=flush(a.j).then(()=>{finished=true;});
 await new Promise(r=>setTimeout(r,0));assert.equal(started,true);assert.equal(finished,false);assert.equal(a.j.writing,true);
 release();await writing;
 const reserved=await a.j.read(a.j.key(a.j.current));assert.ok(reserved.fileId);assert.equal(reserved.ack,0);
 a.j.ensureFolder=ensureFolder;await flush(a.j);
 assert.equal(a.j.status().pending,0);assert.equal(a.j.current.fileId,reserved.fileId);
 assert.equal(api.calls.filter(c=>c.url.includes('/generateIds')).length,1);a.j.stop();
});

test('durable and preview transports share a cold folder lookup only within the same account',async()=>{
 const {LivePreview}=createRequire(import.meta.url)('../drive-live-preview-v5.js');
 const api=fakeInkDrive(),store=memoryInkStore(),a=replica(api,'a',store);let searches=0,release;
 const gate=new Promise(r=>release=r),fetch=api.fetch;
 const network=async(url,options)=>{if((new URL(url).searchParams.get('q')||'').includes("mimeType='application/vnd.google-apps.folder'")){searches++;await gate;}return fetch(url,options);};
 a.j.host.fetch=network;const preview=new LivePreview({store,account:'a',docId:'doc',fetch:network});
 const first=a.j.ensureFolder(),second=preview.ensureFolder();await new Promise(r=>setTimeout(r,0));
 assert.equal(searches,1);release();const ids=await Promise.all([first,second]);
 assert.equal(ids[0],ids[1]);assert.equal(api.calls.filter(c=>c.method==='POST'&&!c.url.includes('uploadType')).length,1);
 const other=new Journal({store,account:'other',docId:'doc',fetch:network});await other.ensureFolder();
 assert.equal(searches,2);a.j.stop();preview.stop();other.stop();
});

test('a cancelled shared folder lookup can be retried by the surviving transport',async()=>{
 const api=fakeInkDrive(),store=memoryInkStore(),a=replica(api,'a',store),b=replica(api,'b',store);
 const fetch=api.fetch;let entered=false;
 a.j.host.fetch=(url,options)=>{
  if((new URL(url).searchParams.get('q')||'').includes('mimeType')){
   entered=true;return new Promise((_,reject)=>options.signal.addEventListener('abort',()=>reject(new DOMException('Closed','AbortError')),{once:true}));
  }
  return fetch(url,options);
 };
 const first=a.j.ensureFolder(),second=b.j.ensureFolder();
 const results=Promise.allSettled([first,second]);while(!entered)await new Promise(r=>setTimeout(r,0));a.j.stop();
 assert.deepEqual((await results).map(r=>r.status),['rejected','rejected']);
 const id=await b.j.ensureFolder();assert.ok(id);assert.equal(b.j.stopped,false);b.j.stop();
});

test('a direct hint arriving during first discovery bypasses the stalled search index',async()=>{
 const api=fakeInkDrive(),writer=replica(api,'writer'),reader=replica(api,'reader');
 await writer.j.enqueue({stroke:'direct'},1);await flush(writer.j);
 let release,searches=0;const gate=new Promise(r=>release=r),fetch=reader.j.host.fetch;
 reader.j.host.fetch=async(url,options)=>{if(url.includes('ihnKind')){searches++;await gate;}return fetch(url,options);};
 const bootstrap=reader.j.poll();await new Promise(r=>setTimeout(r,0));
 try{
  await reader.j.poll();await reader.j.poll();assert.equal(searches,1);assert.equal(reader.j.reading,false);
  reader.j.hint(writer.j.current.fileId);await reader.j.poll();
  assert.deepEqual(reader.applied,[{stroke:'direct'}]);assert.ok(reader.j.bootstrapWait);
 }finally{release();await bootstrap;reader.j.stop();}
 assert.equal(reader.applied.length,1);assert.equal(reader.j.bootstrapWait,null);
});

test('updating an existing journal does not require folder metadata',async()=>{
 const api=fakeInkDrive(),a=replica(api,'a');await a.j.enqueue({stroke:1},1);await flush(a.j);
 a.j.folder=null;a.j.ensureFolder=async()=>{throw Error('Folder metadata should not gate an existing media update');};
 await a.j.enqueue({stroke:2},2);await flush(a.j);
 assert.equal(a.j.status().pending,0);assert.equal(a.receipts.at(-1).version,2);a.j.stop();
});

test('preview creation invalidates a deleted folder cache and recovers on retry',async()=>{
 const {LivePreview}=createRequire(import.meta.url)('../drive-live-preview-v5.js');
 const api=fakeInkDrive(),store=memoryInkStore();await store.put('settings','a','ink-journal-folder',{fileId:'deleted'});
 const fetch=async(url,options)=>{
  if(url.includes('uploadType=multipart')){
   const body=typeof options.body==='string'?options.body:await options.body.text();
   if(body.includes('"parents":["deleted"]'))return new Response('{}',{status:404});
  }
  return api.fetch(url,options);
 };
 const live=new LivePreview({store,account:'a',docId:'doc',actor:'writer',fetch});live.scheduleFrame=()=>{};live.schedulePoll=()=>{};
 live.frame=1;await live.flushFrame();assert.equal(live.exists,false);assert.equal(await store.read('settings','a','ink-journal-folder'),null);
 await live.flushFrame();assert.equal(live.exists,true);assert.notEqual(live.folder,'deleted');live.stop();
});

test('a backing-off outbox avoids lease and status churn until its next useful check',async()=>{
 const api=fakeInkDrive(),a=replica(api,'writer');let leases=0,changes=0;
 const lease=a.store.withLease;a.store.withLease=(...args)=>{leases++;return lease(...args);};a.j.host.changed=()=>changes++;
 await a.j.enqueue({stroke:'pending'},1);
 const retryAt=Date.now()+10000;await a.j.patch(a.j.key(a.j.current),old=>({...old,retryAt}));
 await a.j.flush();const next=a.j.nextOutboxCheck;
 for(let i=0;i<20;i++)await a.j.flush();
 assert.equal(leases,1);assert.equal(changes,1);assert.equal(a.receipts.length,0);
 assert.ok(next>Date.now()+1500&&next<=Date.now()+2000);
 // An earlier retry deadline remains eligible; the storage throttle must
 // never stretch a short network backoff to the full two-second idle delay.
 const sooner=Date.now()+600;await a.j.patch(a.j.key(a.j.current),old=>({...old,retryAt:sooner}));
 a.j.nextOutboxCheck=0;await a.j.flush();assert.equal(a.j.nextOutboxCheck,sooner);a.j.stop();
});

test('a fresh persisted edit arriving during an empty outbox scan cannot be put back to sleep',async()=>{
 const api=fakeInkDrive(),a=replica(api,'writer');a.j.outboxReady=true;
 // Keep wake's generation/deadline behavior, but explicitly drive the ticks.
 a.j.wake=function(){Journal.prototype.wake.call(this);clearTimeout(this.timer);this.timer=null;};
 const read=a.j.read;let release,entered=false,held=false;
 const gate=new Promise(r=>release=r);
 a.j.read=async function(key){const result=await read.call(this,key);if(key==='ink-journal-outbox'&&!held){held=true;entered=true;await gate;}return result;};
 const scanning=a.j.flush();while(!entered)await new Promise(r=>setTimeout(r,0));
 await a.j.enqueue({stroke:'arrived-during-scan'},1);release();await scanning;
 assert.equal(a.j.nextOutboxCheck,0);
 await a.j.flush();assert.equal(a.j.status().pending,0);assert.equal(a.receipts[0].version,1);a.j.stop();
});

test('ordinary presence hints cannot bypass a failed source read backoff, but explicit retry can',async()=>{
 const api=fakeInkDrive(),writer=replica(api,'writer'),reader=replica(api,'reader');
 await writer.j.enqueue({stroke:'remote'},1);await flush(writer.j);
 const id=writer.j.current.fileId;reader.j.nextDiscovery=Infinity;reader.j.hint(id);
 const fetch=reader.j.host.fetch;let reads=0,fail=true;
 reader.j.host.fetch=async(url,options)=>{if(url.includes(`/${id}?alt=media`)){reads++;if(fail)return new Response('{}',{status:503});}return fetch(url,options);};
 await reader.j.poll();const source=reader.j.sources.get(id),retryAt=source.retryAt;
 assert.ok(retryAt>Date.now());assert.equal(reads,1);
 for(let i=0;i<10;i++){reader.j.hint(id);await reader.j.poll();}
 assert.equal(source.next,retryAt);assert.equal(reads,1);
 // A predecessor hint can force next=0, but must not override transport backoff.
 source.next=0;await reader.j.poll();assert.equal(reads,1);
 reader.j.hint('sealed');reader.j.sources.get('sealed').next=Infinity;
 fail=false;await reader.j.retry();assert.equal(source.retryAt,0);assert.equal(reader.j.sources.get('sealed').next,Infinity);
 await reader.j.poll();assert.equal(reads,2);assert.deepEqual(reader.applied,[{stroke:'remote'}]);
 writer.j.stop();reader.j.stop();
});

test('an expired source backoff clears on successful read without clearing an unrelated write error',async()=>{
 const api=fakeInkDrive(),writer=replica(api,'writer'),reader=replica(api,'reader');
 await writer.j.enqueue({stroke:'remote'},1);await flush(writer.j);reader.j.nextDiscovery=Infinity;reader.j.hint(writer.j.current.fileId);
 const source=reader.j.sources.get(writer.j.current.fileId);source.retryAt=Date.now()-1;source.next=0;
 reader.j.lastError='Upload still needs retry';await reader.j.poll();
 assert.equal(source.retryAt,0);assert.equal(reader.j.lastError,'Upload still needs retry');assert.equal(reader.applied.length,1);
 writer.j.stop();reader.j.stop();
});

test('new samples wake the scan without bypassing a failed file write backoff',async()=>{
 const api=fakeInkDrive(),a=replica(api,'writer');
 a.j.wake=function(){Journal.prototype.wake.call(this);clearTimeout(this.timer);this.timer=null;};
 await a.j.enqueue({stroke:'first'},1);
 const retryAt=Date.now()+10000;await a.j.patch(a.j.key(a.j.current),old=>({...old,retryAt}));
 await a.j.flush();assert.ok(a.j.nextOutboxCheck>Date.now());
 await a.j.enqueue({stroke:'second'},2);assert.equal(a.j.nextOutboxCheck,0);
 await a.j.flush();assert.equal(a.receipts.length,0);assert.equal(api.calls.length,0);
 const stored=await a.j.read(a.j.key(a.j.current));assert.equal(stored.rev,2);assert.equal(stored.retryAt,retryAt);
 await a.j.retry();await a.j.flush();assert.equal(a.receipts.at(-1).version,2);assert.equal(a.j.status().pending,0);a.j.stop();
});

for(const result of ['changed','unchanged','failed'])test(`an in-flight ${result} self-read cannot reactivate an acknowledged journal`,async()=>{
 const api=fakeInkDrive(),writer=replica(api,'writer');writer.j.nextDiscovery=Infinity;
 let releaseWrite,releaseRead,readStarted=false,reads=0;
 api.hold=new Promise(r=>releaseWrite=r);const readGate=new Promise(r=>releaseRead=r),fetch=writer.j.host.fetch;
 writer.j.host.fetch=async(url,options)=>{
  const response=await fetch(url,options);
  if(url.includes('alt=media')){reads++;readStarted=true;await readGate;if(result==='failed')throw Error('Obsolete read failed');}
  return response;
 };
 await writer.j.enqueue({stroke:'first'},1);const writing=flush(writer.j);
 while(![...api.files.values()].some(f=>f.raw))await new Promise(r=>setTimeout(r,0));
 const file=[...api.files.values()].find(f=>f.raw);writer.j.hint(file.id);
 if(result==='unchanged')writer.j.sources.get(file.id).raw=file.raw;
 const reading=writer.j.poll();while(!readStarted)await new Promise(r=>setTimeout(r,0));
 try{
  releaseWrite();await writing;api.hold=null;
  const source=writer.j.sources.get(file.id);assert.equal(source.next,Infinity);assert.equal(source.local,true);
  await writer.j.enqueue({stroke:'second'},2);
  releaseRead();await reading;
  assert.equal(source.next,Infinity);assert.equal(source.retryAt||0,0);assert.equal(writer.j.status().error,'');
  assert.equal(writer.j.status().pending,1);assert.equal(writer.receipts.at(-1).version,1);
  writer.j.hint(file.id);await writer.j.retry();await writer.j.poll();assert.equal(reads,1);
  await flush(writer.j);assert.equal(writer.j.status().pending,0);assert.equal(writer.receipts.at(-1).version,2);
 }finally{releaseWrite();releaseRead();await Promise.allSettled([writing,reading]);writer.j.stop();}
});

test('a sealed predecessor ignores an old refresh but reactivates for a genuinely newer required revision',async()=>{
 const api=fakeInkDrive(),reader=replica(api,'reader');reader.j.nextDiscovery=Infinity;
 const initial={v:1,docId:'doc',actor:'remote',part:0,rev:1,ops:[{n:1}]};
 api.files.set('head',{id:'head',raw:JSON.stringify(initial)});
 api.files.set('tail',{id:'tail',raw:JSON.stringify({v:1,docId:'doc',actor:'remote',part:1,rev:1,previous:'head',previousRev:1,ops:[{n:3}]})});
 reader.j.hint('head');await reader.j.poll();assert.equal(reader.applied.length,1);
 let release,started=false;const gate=new Promise(r=>release=r),fetch=reader.j.host.fetch;
 reader.j.host.fetch=async(url,options)=>{
  const response=await fetch(url,options);
  if(url.includes('/head?alt=media')){started=true;await gate;}
  return response;
 };
 reader.j.sources.get('head').next=0;const oldRead=reader.j.poll();while(!started)await new Promise(r=>setTimeout(r,0));
 try{
  reader.j.hint('tail');await reader.j.poll();const source=reader.j.sources.get('head');
  assert.equal(source.next,Infinity);assert.equal(source.retired,true);
  release();await oldRead;assert.equal(source.next,Infinity);
  // A later successor can prove the predecessor was not complete after all.
  // This explicitly starts a new read generation, rather than trusting the old response.
  api.files.get('head').raw=JSON.stringify({...initial,rev:2,ops:[{n:1},{n:2}]});
  api.files.get('tail').raw=JSON.stringify({v:1,docId:'doc',actor:'remote',part:1,rev:2,previous:'head',previousRev:2,ops:[{n:3},{n:4}]});
  reader.j.sources.get('tail').next=0;await reader.j.poll();
  assert.equal(source.retired,false);assert.equal(source.next,0);
  await reader.j.poll();assert.equal(source.rev,2);assert.equal(source.next,Infinity);
  assert.deepEqual(reader.applied.map(op=>op.n).sort(),[1,2,3,4]);
 }finally{release();await oldRead;reader.j.stop();}
});

test('recovering another tab journal does not suppress its current or future edits',async()=>{
 const api=fakeInkDrive(),store=memoryInkStore(),a=replica(api,'tab-a',store),b=replica(api,'tab-b',store);
 await a.j.enqueue({writer:'a',n:1},1);await flush(b.j);
 const record=await a.j.read(a.j.key(a.j.current)),source=b.j.sources.get(record.fileId);
 assert.ok(source);assert.notEqual(source.local,true);assert.notEqual(source.next,Infinity);assert.equal(source.rev,0);
 await poll(b.j);assert.deepEqual(b.applied,[{writer:'a',n:1}]);
 await a.j.enqueue({writer:'a',n:2},2);await flush(a.j);await poll(b.j);
 assert.deepEqual(b.applied,[{writer:'a',n:1},{writer:'a',n:2}]);
 assert.equal(a.j.sources.get(record.fileId).local,true);assert.equal(a.j.sources.get(record.fileId).next,Infinity);
 a.j.stop();b.j.stop();
});

test('outbox recovery of another document does not add a wrong-document read source',async()=>{
 const api=fakeInkDrive(),store=memoryInkStore(),a=replica(api,'tab-a',store,'other-doc'),b=replica(api,'tab-b',store,'doc');
 await a.j.enqueue({writer:'a',n:1},1);await flush(b.j);
 assert.equal(b.j.sources.size,0);assert.equal(b.j.status().error,'');
 assert.equal((await a.j.read(a.j.key(a.j.current))).ack,1);a.j.stop();b.j.stop();
});

test('a queued self-read retired before merge cannot reactivate the source',async()=>{
 const api=fakeInkDrive(),writer=replica(api,'writer');writer.j.nextDiscovery=Infinity;
 let releaseWrite,releaseApply;api.hold=new Promise(r=>releaseWrite=r);
 writer.j.applyChain=new Promise(r=>releaseApply=r);
 await writer.j.enqueue({stroke:'first'},1);const writing=flush(writer.j);
 while(![...api.files.values()].some(f=>f.raw))await new Promise(r=>setTimeout(r,0));
 const file=[...api.files.values()].find(f=>f.raw);writer.j.hint(file.id);
 const previousApply=writer.j.applyChain,reading=writer.j.poll();
 while(writer.j.applyChain===previousApply)await new Promise(r=>setTimeout(r,0));
 try{
  releaseWrite();await writing;assert.equal(writer.j.sources.get(file.id).next,Infinity);
  releaseApply();await reading;
  assert.equal(writer.j.sources.get(file.id).next,Infinity);assert.equal(writer.j.seen.has(file.id),false);
  assert.equal(writer.applied.length,0);assert.equal(writer.j.status().error,'');
 }finally{releaseWrite();releaseApply();await Promise.allSettled([writing,reading]);writer.j.stop();}
});

async function settleUntil(predicate){
 for(let i=0;i<200;i++){if(predicate())return;await new Promise(setImmediate);}
 assert.ok(predicate(),'asynchronous journal work did not settle');
}

test('a successful slow upload immediately drains queued edits after releasing its lease',async t=>{
 t.mock.timers.enable({apis:['Date','setTimeout'],now:1_000_000});
 const api=fakeInkDrive(),store=memoryInkStore(),receipts=[],starts=[];let active=0,peak=0,leases=0,release;
 const lease=store.withLease;store.withLease=(...args)=>{leases++;return lease(...args);};
 api.hold=new Promise(r=>release=r);
 const journal=new Journal({store,account:'a',docId:'doc',actor:'writer',receipt:r=>receipts.push(r),fetch:async(url,options)=>{
  if(!url.includes('uploadType'))return api.fetch(url,options);
  starts.push(Date.now());active++;peak=Math.max(peak,active);
  try{return await api.fetch(url,options);}finally{active--;}
 }});journal.poll=async()=>{};
 try{
  await journal.enqueue({stroke:1},1);t.mock.timers.tick(0);
  await settleUntil(()=>[...api.files.values()].some(f=>f.raw));
  await journal.enqueue({stroke:2},2);
  t.mock.timers.tick(120);t.mock.timers.tick(10); // The periodic timer now waits until 240 ms.
  api.hold=null;release();await settleUntil(()=>!journal.writing&&receipts.length===1);
  t.mock.timers.tick(0);await settleUntil(()=>!journal.writing&&journal.status().pending===0);
  assert.deepEqual(starts,[1_000_000,1_000_130]);assert.equal(peak,1);assert.equal(receipts.at(-1).version,2);
  t.mock.timers.tick(120);await settleUntil(()=>!journal.writing);
  const idleLeases=leases;assert.equal(idleLeases,3);
  t.mock.timers.tick(1500);await settleUntil(()=>!journal.writing);
  assert.equal(leases,idleLeases);assert.equal(starts.length,2);
 }finally{release();journal.stop();t.mock.timers.reset();}
});

test('success follow-up enforces the write rate limit and failures never schedule a retry burst',async t=>{
 t.mock.timers.enable({apis:['Date','setTimeout'],now:2_000_000});
 const api=fakeInkDrive(),starts=[];let fail=false;
 const journal=new Journal({store:memoryInkStore(),account:'a',docId:'doc',actor:'writer',fetch:async(url,options)=>{
  if(url.includes('uploadType')){starts.push(Date.now());if(fail)return new Response('{}',{status:503});}
  return api.fetch(url,options);
 }});journal.poll=async()=>{};
 try{
  await journal.enqueue({stroke:1},1);t.mock.timers.tick(0);await settleUntil(()=>!journal.writing&&journal.status().pending===0);
  await journal.enqueue({stroke:2},2);t.mock.timers.tick(119);await new Promise(setImmediate);assert.equal(starts.length,1);
  t.mock.timers.tick(1);await settleUntil(()=>!journal.writing&&journal.status().pending===0);
  assert.deepEqual(starts,[2_000_000,2_000_120]);
  fail=true;await journal.enqueue({stroke:3},3);t.mock.timers.tick(120);
  await settleUntil(()=>!journal.writing&&!!journal.lastError);assert.equal(starts.length,3);
  t.mock.timers.tick(0);await new Promise(setImmediate);assert.equal(starts.length,3);
  t.mock.timers.tick(120);await settleUntil(()=>!journal.writing);
  t.mock.timers.tick(479);await new Promise(setImmediate);assert.equal(starts.length,3);
  const saved=await journal.read(journal.key(journal.current));assert.equal(saved.ack,2);assert.equal(saved.rev,3);
 }finally{journal.stop();t.mock.timers.reset();}
});

test('oversized photo payloads reject before serialization without mutating the durable queue',async()=>{
 const api=fakeInkDrive(),writer=replica(api,'writer');const photo='data:image/png;base64,'+'A'.repeat(4*1024*1024);
 const operation={v:1,pages:[{pageId:'photo',backgroundImage:photo,images:[]}]};
 const stringify=JSON.stringify;let encoded=0;
 JSON.stringify=(value,...args)=>{if(value===operation)encoded++;return stringify(value,...args);};
 try{
  for(let i=0;i<3;i++)await assert.rejects(writer.j.enqueue(operation,1),error=>error.code==='INK_REQUIRES_PDF');
 }finally{JSON.stringify=stringify;writer.j.stop();}
 assert.equal(encoded,0);assert.equal(operation.pages[0].backgroundImage,photo);
 assert.equal(writer.j.records.size,0);assert.equal(writer.store.data.size,0);assert.equal(writer.receipts.length,0);
});

test('photo preflight sums all page and embedded image payloads conservatively',async()=>{
 const api=fakeInkDrive(),writer=replica(api,'writer');const image='A'.repeat(1536*1024);
 const operation={v:1,pages:[{backgroundImage:image,images:[{src:image}]},{images:[{src:image}]}]};
 const stringify=JSON.stringify;let encoded=0;
 JSON.stringify=(value,...args)=>{if(value===operation)encoded++;return stringify(value,...args);};
 try{await assert.rejects(writer.j.enqueue(operation,1),error=>error.code==='INK_REQUIRES_PDF');}
 finally{JSON.stringify=stringify;writer.j.stop();}
 assert.equal(encoded,0);assert.equal(writer.j.records.size,0);
});

test('fitting photos retain identical wire bytes and exact serialization checks escaped overflow',async()=>{
 const api=fakeInkDrive(),writer=replica(api,'writer');
 const fitting={v:1,pages:[{backgroundImage:'data:image/png;base64,'+'A'.repeat(1024*1024),images:[{src:'small'}]}]};
 const expected=JSON.stringify(fitting);await writer.j.enqueue(fitting,1);assert.equal(writer.j.current.ops[0],expected);
 const escaped={v:1,pages:[{backgroundImage:'"'.repeat(3*1024*1024)}]};
 await assert.rejects(writer.j.enqueue(escaped,2),error=>error.code==='INK_REQUIRES_PDF');
 const envelope={v:1,pages:[{backgroundImage:'A'.repeat(4*1024*1024-8)}]};
 await assert.rejects(writer.j.enqueue(envelope,3),error=>error.code==='INK_REQUIRES_PDF');
 assert.equal(writer.j.current.rev,1);assert.equal(writer.j.current.version,1);writer.j.stop();
});

test('size preflight does not reject omitted fields or custom JSON serialization',async()=>{
 const api=fakeInkDrive(),writer=replica(api,'writer');const huge='A'.repeat(5*1024*1024);
 const hidden={pageId:'hidden'};Object.defineProperty(hidden,'backgroundImage',{value:huge,enumerable:false});
 const custom={backgroundImage:huge,toJSON(){return {pageId:'small'};}};
 const operation={v:1,pages:[hidden,custom]};const expected=JSON.stringify(operation);
 await writer.j.enqueue(operation,1);assert.equal(writer.j.current.ops[0],expected);writer.j.stop();
});

test('size classification never masks a real local persistence failure',async()=>{
 const api=fakeInkDrive(),writer=replica(api,'writer');const failure=Object.assign(Error('Device storage full'),{code:'QUOTA'});
 writer.store.patch=async()=>{throw failure;};
 await assert.rejects(writer.j.enqueue({v:1,pages:[{backgroundImage:'small'}]},1),error=>error===failure);
 assert.equal(writer.j.status().pending,1);assert.equal(writer.receipts.length,0);writer.j.stop();
});
