import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {createRequire} from 'node:module';
const {DocumentWorkQueue}=createRequire(import.meta.url)('../runtime-core-v5.js');
const bytes=await readFile(new URL('../wasm/inhouse_notes_core.wasm',import.meta.url));
const scope={WebAssembly,Uint8Array,Float64Array,URL,location:new URL('https://notes.test/'),fetch:async()=>new Response(bytes),AbortSignal,setTimeout,clearTimeout,console};
vm.createContext(scope);vm.runInContext(await readFile(new URL('../rust-core-v5.js',import.meta.url),'utf8'),scope);
assert.equal(await scope.InhouseRust.ready,true);
function clock(){
 let now=0,next=0,callbacks=0;const timers=new Map();
 return {get now(){return now;},get callbacks(){return callbacks;},get size(){return timers.size;},
  set(fn,delay){const id=++next;timers.set(id,{fn,at:now+delay});return id;},clear(id){timers.delete(id);},
  async advance(ms){const end=now+ms;for(;;){const first=[...timers].sort((a,b)=>a[1].at-b[1].at)[0];if(!first||first[1].at>end)break;
   timers.delete(first[0]);now=first[1].at;callbacks++;first[1].fn();await Promise.resolve();}now=end;await Promise.resolve();}
 };
}
function setup(rust=true){
 const timer=clock();const state={lastActivity:0,session:1,busy:true,bypass:false};
 const queue=new DocumentWorkQueue(()=>({...state,now:timer.now}),{makePlanner:()=>rust?scope.InhouseRust.documentWorkQueue():null,
  now:()=>timer.now,setTimer:(fn,ms)=>timer.set(fn,ms),clearTimer:id=>timer.clear(id)});
 return {queue,timer,state};
}
for(const rust of [true,false]){
 test(`one alarm pauses 64 tasks and resumes in bounded batches (${rust?'Rust':'fallback'})`,async()=>{
  const {queue,timer,state}=setup(rust);let finished=0;
  const jobs=Array.from({length:64},()=>queue.wait({sessionToken:1,windowMs:1200}).then(ok=>{assert.equal(ok,true);finished++;}));
  assert.equal(timer.size,1);await timer.advance(1200);assert.equal(finished,0);
  assert.ok(timer.callbacks<=14,`unexpected wake count ${timer.callbacks}`);
  state.busy=false;queue.wake();await timer.advance(0);assert.equal(finished,8);
  state.busy=true;await timer.advance(300);assert.equal(finished,8,'new contact must stop the next batch');
  state.busy=false;queue.wake();await timer.advance(20);await Promise.all(jobs);
  assert.equal(queue.diagnostics.maxBatch,8);assert.equal(timer.size,0);
  assert.equal(queue.diagnostics.backend,rust?'rust':'javascript');queue.dispose();
 });
 test(`abort and document replacement reject waits even while the pen is held (${rust?'Rust':'fallback'})`,async()=>{
  const {queue,timer,state}=setup(rust);const controller=new AbortController();
  const cancelled=queue.wait({abortSignal:controller.signal,sessionToken:1});controller.abort();assert.equal(await cancelled,false);
  const stale=queue.wait({sessionToken:1});state.session=2;queue.wake();await timer.advance(0);assert.equal(await stale,false);
  const plain={aborted:false};const stopped=queue.wait({abortSignal:plain});plain.aborted=true;await timer.advance(100);assert.equal(await stopped,false);
  const exit=queue.wait({sessionToken:2,windowMs:1200});state.bypass=true;queue.wake();await timer.advance(0);assert.equal(await exit,true);
  assert.equal(queue.diagnostics.pending,0);assert.equal(timer.size,0);queue.dispose();
 });
}
test('Rust deadlines match fallback across varied windows, session changes, bypass and cancellation',()=>{
 const rust=scope.InhouseRust.documentWorkQueue();const {queue}=setup(false);
 let seed=41;const rand=()=>((seed=Math.imul(seed,1664525)+1013904223>>>0)/2**32);
 for(let id=1;id<=300;id++){
  const session=id%5===0?-1:1+id%3,quietMs=Math.floor(rand()*2400);
  assert.equal(rust.add(id,session,quietMs),true);
  queue.waiters.set(id,{id,session,quietMs});
 }
 for(let i=0;i<150;i++){
  const state={now:i*40,lastActivity:Math.floor(rand()*3000),session:1+i%3,busy:rand()<.6,bypass:rand()<.1};
  if(i%4===0){const id=1+Math.floor(rand()*300);rust.cancel(id);queue.waiters.delete(id);}
  const expected=queue.fallback(state),actual=rust.poll(state);
  assert.deepEqual(JSON.parse(JSON.stringify(actual)),expected);
  for(const [id] of expected.ready)queue.waiters.delete(id);
 }
 rust.dispose();rust.dispose();queue.waiters.clear();queue.dispose();
 assert.throws(()=>rust.poll({}),/disposed/);assert.equal(rust.add(1,1,0),false);
});
test('Rust queue survives WASM memory growth and rejects malformed or duplicate jobs',async()=>{
 const planner=scope.InhouseRust.documentWorkQueue();
 assert.equal(planner.add(0,1,1200),false);assert.equal(planner.add(1,NaN,1200),false);
 assert.equal(planner.add(1,1,1200),true);assert.equal(planner.add(1,1,1200),false);
 assert.equal(planner.add(2,1,Infinity),false);
 // A different numeric kernel allocates enough memory to detach prior views.
 scope.InhouseRust.splitRgba(new Uint8Array(4*1024*1024));
 assert.deepEqual(JSON.parse(JSON.stringify(planner.poll({now:1200,lastActivity:0,session:1,busy:false,bypass:false}))),{ready:[[1,true]],delay:-1});
 planner.dispose();
});
test('pending fallback jobs migrate to Rust after delayed WASM loading without loss',async()=>{
 const {queue,timer,state}=setup(false);let loaded=false;
 queue.makePlanner=()=>loaded?scope.InhouseRust.documentWorkQueue():null;
 const jobs=Array.from({length:24},()=>queue.wait({sessionToken:1,windowMs:1200}));
 assert.equal(queue.diagnostics.backend,'javascript');
 loaded=true;queue.wake();await timer.advance(0);
 assert.equal(queue.diagnostics.backend,'rust');assert.equal(queue.diagnostics.pending,24);
 await timer.advance(1200);state.busy=false;queue.wake();await timer.advance(10);
 assert.deepEqual(await Promise.all(jobs),Array(24).fill(true));queue.dispose();
});
test('a full Rust queue falls back without dropping waiters or cancellation',async()=>{
 const {queue,timer,state}=setup();
 const controller=new AbortController();
 const jobs=Array.from({length:4100},()=>queue.wait({sessionToken:1,abortSignal:controller.signal}));
 // Cancellation delivers false for every job even beyond the native capacity.
 controller.abort();assert.equal((await Promise.all(jobs)).every(value=>value===false),true);
 assert.equal(queue.diagnostics.pending,0);assert.equal(timer.size,0);queue.dispose();
});
