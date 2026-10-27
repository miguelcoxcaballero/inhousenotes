import {test,expect} from '@playwright/test';
test('pen preserves a sharp measured corner in live ink and page repaint without discarding samples',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(()=>{
  const points=[{x:30,y:30,p:.3},{x:70,y:130,p:.7},{x:110,y:30,p:.4},...Array.from({length:20},(_,i)=>({x:111+i*.03,y:30+(i%2)*.08,p:.5}))];
  const original=JSON.stringify(points);
  const stroke={tool:'pen',color:'#E81010',width:2,points,finalizedIndex:-1};
  const live=document.createElement('canvas'),stored=document.createElement('canvas');
  live.width=stored.width=794;live.height=stored.height=1123;
  finalizeStrokeSegments(0,live,stroke);
  simplifyStrokePoints(stroke);
  drawSmoothStroke(stored.getContext('2d'),stroke);
  const pixel=c=>Array.from(c.getContext('2d').getImageData(70,129,1,1).data);
  const before=live.toDataURL();
  finalizeStrokeSegments(0,live,stroke);
  return {unchanged:JSON.stringify(stroke.points)===original,live:pixel(live),stored:pixel(stored),end:stroke.finalizedIndex,count:points.length,repeatedUnchanged:before===live.toDataURL()};
 });
 expect(result.unchanged).toBe(true);expect(result.end).toBe(result.count-1);expect(result.repeatedUnchanged).toBe(true);
 for(const color of [result.live,result.stored]){expect(color[0]).toBeGreaterThan(180);expect(color[1]).toBeLessThan(50);expect(color[3]).toBeGreaterThan(100);}
});
for(const scale of [1.25,2.25,3.5])test(`live pen position matches backing dimensions at scale ${scale}`,async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(scale=>{
  const c=document.createElement('canvas');c.width=Math.round(794*scale);c.height=Math.round(1123*scale);
  // Deliberately stale transform: drawing must establish its own current mapping.
  const ctx=c.getContext('2d');ctx.setTransform(1,0,0,1,0,-8);
  const stroke={tool:'pen',color:'#E81010',width:1,points:[{x:20,y:1000},{x:200,y:1000}],finalizedIndex:-1};
  commitLiveStrokeToCanvas(0,c,stroke);
  const x=Math.round(100*c.width/794),expected=1000*c.height/1123;
  const y=Math.floor(expected)-8,p=ctx.getImageData(x,y,1,17).data;
  let total=0,weighted=0;for(let i=0;i<17;i++){const alpha=p[i*4+3];total+=alpha;weighted+=alpha*(y+i+.5);}
  return {error:Math.abs(weighted/total-expected),total};
 },scale);
 expect(result.total).toBeGreaterThan(100);expect(result.error).toBeLessThan(.6);
});

test('live pen ink is a smooth curve through every sample and repaints without shifting',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(()=>{
  // Sparse samples around a circle: a polyline shows visible facets here.
  const points=Array.from({length:13},(_,i)=>({x:300+120*Math.cos(i*Math.PI/6),y:400+120*Math.sin(i*Math.PI/6),p:.5}));
  const stroke={tool:'pen',color:'#E81010',width:3,points:[points[0]],finalizedIndex:-1,lastMid:null};
  const live=document.createElement('canvas'),stored=document.createElement('canvas');
  live.width=stored.width=794;live.height=stored.height=1123;
  // Samples arrive one by one, as during writing, then the pen lifts.
  for(let i=1;i<points.length;i++){stroke.points.push(points[i]);finalizeStrokeSegments(0,live,stroke);}
  commitLiveStrokeToCanvas(0,live,stroke);
  drawSmoothStroke(stored.getContext('2d'),{tool:'pen',color:'#E81010',width:3,points});
  const a=live.getContext('2d').getImageData(0,0,794,1123).data,b=stored.getContext('2d').getImageData(0,0,794,1123).data;
  // Incremental drawing antialiases segment joins twice, so compare where
  // the ink is (coverage masks), not exact edge alpha.
  let different=0,inked=0;for(let i=3;i<a.length;i+=4){if(b[i]>64)inked++;if((a[i]>64)!==(b[i]>64))different++;}
  // Midway between two samples the curve bulges outward to the circle;
  // the straight chord between them lies 4 px inside it.
  const alpha=(x,y)=>stored.getContext('2d').getImageData(Math.round(x),Math.round(y),1,1).data[3];
  const angle=3.5*Math.PI/6; // an interior segment, between samples 3 and 4
  return {different,inked,onCurve:alpha(300+120*Math.cos(angle),400+120*Math.sin(angle)),onChord:alpha(300+120*Math.cos(angle)*Math.cos(Math.PI/12)-0,400+120*Math.sin(angle)*Math.cos(Math.PI/12)),
   throughSample:alpha(points[3].x,points[3].y)};
 });
 expect(result.inked).toBeGreaterThan(1500);
 expect(result.different/result.inked).toBeLessThan(.01);
 expect(result.throughSample).toBeGreaterThan(100);
 expect(result.onCurve).toBeGreaterThan(100);
 expect(result.onChord).toBeLessThan(30);
});
