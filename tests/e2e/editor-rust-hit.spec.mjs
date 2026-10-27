import {test,expect} from '@playwright/test';
test('Rust long-stroke hit tests match the existing geometry and invalidate edited coordinates',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  if(!await InhouseRust.ready)throw new Error('Rust not loaded');
  let seed=12345;const random=()=>((seed=(Math.imul(seed,1664525)+1013904223)>>>0)/4294967296);
  const points=Array.from({length:1000},(_,i)=>({x:i*.5,y:Math.sin(i*.1)*50+100}));
  const reference=(pts,x,y,r)=>pts.slice(1).some((p,i)=>distancePointToSegment(x,y,pts[i].x,pts[i].y,p.x,p.y)<=r);
  let mismatches=0;
  for(let i=0;i<600;i++){const x=random()*600,y=random()*220,r=random()*5;if(InhouseRust.strokeHitTest(points,x,y,r,1)!==reference(points,x,y,r))mismatches++;}
  const straight=Array.from({length:300},(_,i)=>({x:i,y:0}));
  const old=InhouseRust.strokeHitTest(straight,150,100,1,2);
  straight[150].y=100;
  const edited=InhouseRust.strokeHitTest(straight,150,100,1,3);
  const stroke={points,tool:'pen',width:2};
  const expected=strokeHitsPoint(stroke,50,100,2);
  const original=InhouseRust;window.InhouseRust={...original,strokeHitTest:()=>null};
  const fallback=strokeHitsPoint(stroke,50,100,2);window.InhouseRust=original;
  for(let i=0;i<70;i++)InhouseRust.strokeHitTest(points.map(p=>({...p})),0,0,2,4);
  const afterEviction=InhouseRust.strokeHitTest(points,20,100,1,4)===reference(points,20,100,1);
  return {mismatches,old,edited,fallback:expected===fallback,afterEviction};
 });expect(result).toEqual({mismatches:0,old:false,edited:true,fallback:true,afterEviction:true});
});
test('measure repeated long-stroke eraser queries including adapter overhead',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  await InhouseRust.ready;
  const points=Array.from({length:3000},(_,i)=>({x:i*.1,y:Math.sin(i*.1)*50+100}));
  const reference=(x,y)=>{for(let i=1;i<points.length;i++){const a=points[i-1],b=points[i];if(distancePointToSegment(x,y,a.x,a.y,b.x,b.y)<=.1)return true;}return false;};
  const queries=Array.from({length:1200},(_,i)=>({x:i%300,y:80+(i%7)}));
  const first=performance.now();if(InhouseRust.strokeHitTest(points,0,25,.1,'bench')===null)throw new Error('Rust hit kernel unavailable');const coldMs=performance.now()-first;
  for(const q of queries.slice(0,100)){reference(q.x,q.y);InhouseRust.strokeHitTest(points,q.x,q.y,.1,'bench');}
  let start=performance.now(),jsHits=0;for(const q of queries)jsHits+=reference(q.x,q.y);const jsMs=performance.now()-start;
  start=performance.now();let rustHits=0;for(const q of queries)rustHits+=InhouseRust.strokeHitTest(points,q.x,q.y,.1,'bench');const rustMs=performance.now()-start;
  return {jsMs,rustMs,coldMs,jsHits,rustHits};
 });console.log('EDITOR_HIT_BENCH',JSON.stringify(result));expect(result.rustHits).toBe(result.jsHits);expect(result.jsHits).toBeGreaterThan(0);expect(result.jsHits).toBeLessThan(1200);
});
test('the editor eraser uses Rust on long ink and preserves neighbouring strokes',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  await InhouseRust.ready;await __IHN_TEST_API__.resetLocalDocument(1,'Rust eraser');
  const line=(id,y)=>({id,tool:'pen',width:2,color:'#000000',points:Array.from({length:600},(_,i)=>({x:i,y}))});
  state.pages[0].strokes=[line('remove',100),line('keep',200)];markPageDirty(0,'full');
  const old=InhouseRust;let calls=0;
  window.InhouseRust={...old,strokeHitTest(...args){const result=old.strokeHitTest(...args);if(result!==null)calls++;return result;}};
  const erased=[];eraseStrokeAtPoint(0,200,100,erased,2,{deferRedraw:true});window.InhouseRust=old;
  return {calls,ids:state.pages[0].strokes.map(s=>s.id),removed:erased.map(e=>e.stroke.id)};
 });expect(result).toEqual({calls:1,ids:['keep'],removed:['remove']});
});
