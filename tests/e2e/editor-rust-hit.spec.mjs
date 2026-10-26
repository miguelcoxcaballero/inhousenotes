import {test,expect} from '@playwright/test';
// Reference geometry: the original JavaScript formulas (Math.hypot), independent of the Rust store.
const REFERENCE=`(()=>{const dist=(px,py,x1,y1,x2,y2)=>{const dx=x2-x1,dy=y2-y1;if(dx===0&&dy===0)return Math.hypot(px-x1,py-y1);
  const t=((px-x1)*dx+(py-y1)*dy)/(dx*dx+dy*dy),c=Math.max(0,Math.min(1,t));return Math.hypot(px-(x1+c*dx),py-(y1+c*dy));};
  const hit=(pts,x,y,r)=>pts.length===1?Math.hypot(pts[0].x-x,pts[0].y-y)<=r:pts.slice(1).some((p,i)=>dist(x,y,pts[i].x,pts[i].y,p.x,p.y)<=r);return {dist,hit};})()`;
test('Rust long-stroke hit tests match the existing geometry and invalidate edited coordinates',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async(reference)=>{
  const {dist,hit}=eval(reference);
  if(!await InhouseRust.ready)throw new Error('Rust not loaded');
  let seed=12345;const random=()=>((seed=(Math.imul(seed,1664525)+1013904223)>>>0)/4294967296);
  const points=Array.from({length:1000},(_,i)=>({x:i*.5,y:Math.sin(i*.1)*50+100}));
  const stroke={points,tool:'pen',width:2};
  let mismatches=0;
  for(let i=0;i<600;i++){const x=random()*600,y=random()*220,r=random()*5;if((InhouseRust.strokesHitCircle([stroke],x,y,r,1).length===1)!==hit(points,x,y,r))mismatches++;}
  // Exact segment distance equals the browser's own Math.hypot formula bit for bit.
  let distanceMismatches=0;
  for(let i=0;i<20000;i++){const a=[random()*40,random()*40,random()*40,random()*40,random()*40,random()*40];if(!Object.is(InhouseRust.segmentDistance(...a),dist(...a)))distanceMismatches++;}
  const straight={tool:'pen',width:2,points:Array.from({length:300},(_,i)=>({x:i,y:0}))};
  const old=InhouseRust.strokesHitCircle([straight],150,3,1,2).length===1;
  straight.points[150].y=3;
  const edited=InhouseRust.strokesHitCircle([straight],150,3,1,3).length===1;
  for(let i=0;i<70;i++)InhouseRust.strokesHitCircle([{tool:'pen',width:2,points:points.map(p=>({...p}))}],0,0,2,4);
  const afterEviction=(InhouseRust.strokesHitCircle([stroke],20,100,1,4).length===1)===hit(points,20,100,1);
  return {mismatches,distanceMismatches,old,edited,afterEviction};
 },REFERENCE);expect(result).toEqual({mismatches:0,distanceMismatches:0,old:false,edited:true,afterEviction:true});
});
test('measure repeated long-stroke eraser queries including adapter overhead',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async(reference)=>{
  const {dist,hit}=eval(reference);
  await InhouseRust.ready;
  const points=Array.from({length:3000},(_,i)=>({x:i*.1,y:Math.sin(i*.1)*50+100}));
  const stroke={tool:'pen',width:2,points};
  const reference_=(x,y)=>{for(let i=1;i<points.length;i++){const a=points[i-1],b=points[i];if(dist(x,y,a.x,a.y,b.x,b.y)<=.1)return true;}return false;};
  const queries=Array.from({length:1200},(_,i)=>({x:i%300,y:80+(i%7)}));
  const first=performance.now();InhouseRust.strokesHitCircle([stroke],0,25,.1,'bench');const coldMs=performance.now()-first;
  for(const q of queries.slice(0,100)){reference_(q.x,q.y);InhouseRust.strokesHitCircle([stroke],q.x,q.y,.1,'bench');}
  let start=performance.now(),jsHits=0;for(const q of queries)jsHits+=reference_(q.x,q.y);const jsMs=performance.now()-start;
  start=performance.now();let rustHits=0;for(const q of queries)rustHits+=InhouseRust.strokesHitCircle([stroke],q.x,q.y,.1,'bench').length;const rustMs=performance.now()-start;
  return {jsMs,rustMs,coldMs,jsHits,rustHits};
 },REFERENCE);console.log('EDITOR_HIT_BENCH',JSON.stringify(result));expect(result.rustHits).toBe(result.jsHits);expect(result.jsHits).toBeGreaterThan(0);expect(result.jsHits).toBeLessThan(1200);
});
test('the editor eraser uses Rust on long ink and preserves neighbouring strokes',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  await InhouseRust.ready;await __IHN_TEST_API__.resetLocalDocument(1,'Rust eraser');
  const line=(id,y)=>({id,tool:'pen',width:2,color:'#000000',points:Array.from({length:600},(_,i)=>({x:i,y}))});
  state.pages[0].strokes=[line('remove',100),line('keep',200)];markPageDirty(0,'full');
  const old=InhouseRust;let calls=0;
  // The facade is frozen; shadow it with a counting wrapper for the duration of the call.
  window.InhouseRust={...old,strokesHitCircle(...args){const result=old.strokesHitCircle(...args);calls++;return result;}};
  const erased=[];try{eraseStrokeAtPoint(0,200,100,erased,2,{deferRedraw:true});}finally{window.InhouseRust=old;}
  return {calls,ids:state.pages[0].strokes.map(s=>s.id),removed:erased.map(e=>e.stroke.id)};
 });expect(result).toEqual({calls:1,ids:['keep'],removed:['remove']});
});
