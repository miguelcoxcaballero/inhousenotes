import test from 'node:test';
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
const {CoalescingQueue, WorkerRpcClient, runWithRetries} = createRequire(import.meta.url)('../runtime-core-v5.js');
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((a,b) => {resolve=a;reject=b;}); return {promise,resolve,reject}; };

test('save bursts share the next checkpoint, never the in-flight snapshot', async () => {
 const gate=deferred(),calls=[];
 const queue=new CoalescingQueue(async input=>{calls.push(input);if(calls.length===1)await gate.promise;return input;},(a,b)=>({...b,force:a.force||b.force}));
 const first=queue.request('doc',{version:1,force:false});
 const waiting=Array.from({length:20},(_,i)=>queue.request('doc',{version:i+2,force:i===0}));
 let completed=false;waiting[0].then(()=>completed=true);
 await tick();assert.equal(completed,false);assert.equal(calls.length,1);
 gate.resolve();assert.equal((await first).version,1);
 const results=await Promise.all(waiting);
 assert.equal(calls.length,2);assert.ok(results.every(r=>r.version===21&&r.force));
});
test('a failed writer releases its lane for a different document', async () => {
 const gate=deferred();const queue=new CoalescingQueue(async name=>{if(name==='old'){await gate.promise;throw Error('disk full');}return name;});
 const old=queue.request('old','old');const rejected=assert.rejects(old,/disk full/);
 const next=queue.request('new','new');gate.resolve();await rejected;assert.equal(await next,'new');
});
function processor(options={}) {
 const workers=[];
 const client=new WorkerRpcClient(()=>{const worker={sent:[],postMessage(m){this.sent.push(m);},terminate(){this.stopped=true;},reply(value){this.onmessage({data:{id:this.sent.at(-1).id,value}});}};workers.push(worker);return worker;},options);
 return {client,workers};
}
test('PDF mutations run in order; queued jobs do not consume their execution deadline',async()=>{
 const {client,workers}=processor({timeoutMs:500});
 const first=client.run('load',{}),second=client.run('save',{}, {timeoutMs:10});
 await new Promise(r=>setTimeout(r,20));assert.equal(workers[0].sent.length,1);
 workers[0].reply('loaded');assert.equal(await first,'loaded');assert.equal(workers[0].sent.length,2);
 workers[0].reply('saved');assert.equal(await second,'saved');client.reset();
});
test('worker crash rejects all pending work, expires handles and ignores late replies',async()=>{
 const {client,workers}=processor();const epoch=client.epoch;
 const a=client.run('load',{}),b=client.run('save',{});
 const rejected=Promise.all([assert.rejects(a,/crashed/),assert.rejects(b,/crashed/)]);
 workers[0].onerror({message:'crashed'});await rejected;assert.throws(()=>client.assertEpoch(epoch),/expired/);
 const c=client.run('create',{});workers[0].reply('stale');assert.equal(client.active.type,'create');
 workers[1].reply('new');assert.equal(await c,'new');client.reset();
});
test('cancelling a queued job leaves the active PDF intact',async()=>{
 const {client,workers}=processor();const abort=new AbortController();
 const a=client.run('load',{}),b=client.run('save',{}, {signal:abort.signal});
 const rejected=assert.rejects(b,e=>e.name==='AbortError');abort.abort();await rejected;
 assert.equal(workers[0].stopped,undefined);workers[0].reply(true);await a;assert.equal(workers[0].sent.length,1);client.reset();
});
test('active timeout invalidates resources and permits a fresh worker',async()=>{
 const {client,workers}=processor({timeoutMs:5});
 await assert.rejects(client.run('stalled',{}),e=>e.name==='TimeoutError');assert.equal(workers[0].stopped,true);
 const next=client.run('create',{});workers[1].reply(true);assert.equal(await next,true);client.reset();
});
test('worker queue rejects overload without evicting accepted work',async()=>{
 const {client,workers}=processor({maxPending:1});const a=client.run('a',{}),b=client.run('b',{});
 await assert.rejects(client.run('c',{}),/queue is full/);workers[0].reply(1);await a;workers[0].reply(2);await b;client.reset();
});
test('retry cancellation never executes an already-cancelled operation',async()=>{
 const controller=new AbortController();controller.abort();let calls=0;
 await assert.rejects(runWithRetries(()=>++calls,{signal:controller.signal}),e=>e.name==='AbortError');assert.equal(calls,0);
});
test('permanent errors are not retried',async()=>{
 let calls=0;await assert.rejects(runWithRetries(()=>{calls++;throw Error('permission denied');},{shouldRetry:()=>false,baseDelayMs:0}),/permission denied/);assert.equal(calls,1);
});

test('retry delays remove cancellation listeners after success and abort',async()=>{
 const {getEventListeners}=await import('node:events');const signal=new AbortController().signal;let calls=0;
 await runWithRetries(()=>{if(++calls<4)throw Error('temporary');},{attempts:4,baseDelayMs:1,signal});
 assert.equal(getEventListeners(signal,'abort').length,0);
});
