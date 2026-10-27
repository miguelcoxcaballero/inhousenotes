import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFileSync} from 'node:fs';

const app=readFileSync(new URL('../app-v5.js',import.meta.url),'utf8');
const start=app.indexOf('        const MAX_HISTORY_MEMORY =');
const end=app.indexOf('        // Run memory check every 15 seconds',start);
assert.ok(start>=0&&end>start);
const source=app.slice(start,end);
function harness(history,index){
 const context=vm.createContext({state:{history,historyIndex:index,pages:[]},
  imageCache:new Map(),imageCacheUsage:new Map(),MAX_IMAGE_CACHE:30,
  pdfBackgroundCache:new Map(),activePageIndices:new Set(),
  performance:{memory:{usedJSHeapSize:90,jsHeapSizeLimit:100}},
  console:{warn(){}},URL:{revokeObjectURL(){}}});
 vm.runInContext(source,context);return context;
}
// Independent model of the former estimator/shift loop, including its exact
// field coverage. This change deliberately does not redefine memory policy.
function legacyWeight(action){
 let total=0;
 if(action.stroke?.points)total+=action.stroke.points.length;
 if(action.strokes)for(const item of action.strokes)if(item.stroke?.points)total+=item.stroke.points.length;
 if(action.before){
  if(Array.isArray(action.before)){for(const item of action.before)if(item.stroke?.points)total+=item.stroke.points.length;}
  else if(action.before.strokes){for(const item of action.before.strokes)if(item.stroke?.points)total+=item.stroke.points.length;}
 }
 if(Array.isArray(action.after))for(const item of action.after)if(item.stroke?.points)total+=item.stroke.points.length;
 return total;
}
function legacyTrim(history,index,target){
 const remaining=history.slice();
 while(remaining.length>1&&remaining.reduce((sum,action)=>sum+legacyWeight(action),0)>target){remaining.shift();index=Math.max(-1,index-1);}
 return {history:remaining,index};
}
const item=length=>({stroke:{points:new Array(length)}});

test('history compaction retains the exact legacy actions and undo index across entry shapes',()=>{
 let seed=123456789;const random=()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed;};
 for(let round=0;round<160;round++){
  const history=Array.from({length:random()%15},(_,id)=>{
   const length=random()%14000,shape=random()%6;
   return {id,...(shape===0?{stroke:item(length).stroke}:shape===1?{strokes:[item(length),item(length>>2)]}:
    shape===2?{before:[item(length)],after:[item(length>>1)]}:shape===3?{before:{strokes:[item(length)]}}:
    shape===4?{before:{images:[]},after:{strokes:[item(length)]}}:{})};
  });
  const index=history.length?random()%(history.length+1)-1:-1;
  for(const [method,target]of [['trimHistoryByMemory',25000],['releaseMemoryOnHide',12500],['checkMemoryPressure',10000]]){
   const copy=history.slice(),expected=legacyTrim(history,index,target),context=harness(copy,index);
   context[method]();
   assert.equal(context.state.history,copy,'preserve the history array identity');
   assert.deepEqual(copy,expected.history,`${method}, fixture ${round}`);
   assert.equal(context.state.historyIndex,expected.index,`${method}, fixture ${round}`);
   for(let i=0;i<copy.length;i++)assert.equal(copy[i],expected.history[i],'preserve action identities');
  }
 }
});

test('memory limits remain strict and the newest action is retained even if it alone exceeds the limit',()=>{
 for(const lengths of [[],[26000],[12500,12500],[12500,12501],[0,30000]]){
  const history=lengths.map((n,id)=>({id,stroke:item(n).stroke})),index=history.length-1;
  const expected=legacyTrim(history,index,25000),context=harness(history.slice(),index);
  context.trimHistoryByMemory();assert.deepEqual(context.state.history,expected.history);assert.equal(context.state.historyIndex,expected.index);
 }
});

test('large grouped history entries are scored once rather than rescanned after every removal',()=>{
 function fixture(){
  let visits=0;
  const history=Array.from({length:10},(_,id)=>({id,strokes:Array.from({length:1000},()=>{
   const stroke={points:{length:100}};
   return Object.defineProperty({},'stroke',{get(){visits++;return stroke;},enumerable:true});
  })}));
  return {history,visits:()=>visits};
 }
 const old=fixture();legacyTrim(old.history,9,25000);const previousVisits=old.visits();
 const current=fixture(),context=harness(current.history,9);context.trimHistoryByMemory();
 assert.equal(context.state.history.length,1);assert.equal(context.state.historyIndex,0);
 assert.equal(current.visits(),10*1000*2,'each stored stroke is measured once');
 assert.ok(previousVisits>current.visits()*5);
 console.log('HISTORY_WEIGHT_READS',{previous:previousVisits,current:current.visits()});
});
