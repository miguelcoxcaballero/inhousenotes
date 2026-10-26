import {test,expect} from '@playwright/test';
test('pen preserves a sharp measured corner in live ink and page repaint without discarding samples',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(()=>{
  const points=[{x:30,y:30,p:.3},{x:70,y:130,p:.7},{x:110,y:30,p:.4},...Array.from({length:20},(_,i)=>({x:111+i*.03,y:30+(i%2)*.08,p:.5}))];
  const original=JSON.stringify(points);
  const stroke={tool:'pen',color:'#E81010',width:2,points,finalizedIndex:-1};
  const live=document.createElement('canvas'),stored=document.createElement('canvas');
  live.width=stored.width=160;live.height=stored.height=160;
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
