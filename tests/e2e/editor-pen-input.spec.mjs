import {test,expect} from '@playwright/test';

async function startPen(page,context,count=3000) {
  await page.goto('/?e2e=1');await page.waitForFunction(()=>window.__IHN_TEST_API__);
  await page.evaluate(count=>__IHN_TEST_API__.seedDensePageForTest(count,100),count);
  await page.waitForFunction(()=>!__IHN_TEST_API__.densePaintStatusForTest().pending);
  await page.locator('[data-tool="pen"][data-color="#E81010"]').click();
  const target=await page.evaluate(()=>{
    const c=document.querySelector('canvas.page-canvas[data-page="0"]'),r=c.getBoundingClientRect();
    c.addEventListener('pointerdown',e=>window.penInputId=e.pointerId,{once:true});
    return {x:r.left+100*r.width/794,y:r.top+400*r.height/1123};
  });
  const cdp=await context.newCDPSession(page);
  await cdp.send('Input.dispatchMouseEvent',{type:'mousePressed',...target,button:'left',buttons:1,clickCount:1,pointerType:'pen'});
  return {cdp,target};
}

test('empty coalesced events, raw samples and the release tail retain a curved pen without duplicate paints or palm jumps',async({page,context})=>{
  const {cdp,target}=await startPen(page,context);
  const result=await page.evaluate(async()=>{
    const c=document.querySelector('canvas.page-canvas[data-page="0"]'),r=c.getBoundingClientRect(),id=penInputId;
    let clock=performance.now(),calls=0;
    const native=CanvasRenderingContext2D.prototype.stroke;
    CanvasRenderingContext2D.prototype.stroke=function(...args){calls++;return native.apply(this,args);};
    const sample=(type,x,y,time,pointerId=id)=>{
      const e=new PointerEvent(type,{clientX:r.left+x*r.width/794,clientY:r.top+y*r.height/1123,pointerId,pointerType:'pen',pressure:.5,buttons:1,bubbles:true});
      Object.defineProperty(e,'timeStamp',{value:time});return e;
    };
    const curve=x=>400+40*Math.sin((x-100)/25);
    for(let x=102;x<=500;x+=2){
      const time=++clock;
      const raw=sample('pointerrawupdate',x,curve(x),time);
      Object.defineProperty(raw,'getCoalescedEvents',{value:()=>[]});c.dispatchEvent(raw);
      const move=sample('pointermove',x,curve(x),time);
      Object.defineProperty(move,'getCoalescedEvents',{value:()=>[raw]});c.dispatchEvent(move);
      if(x===300){c.dispatchEvent(sample('pointerleave',x,curve(x),time));
        c.dispatchEvent(sample('pointermove',700,900,time,id+9));c.dispatchEvent(sample('pointerup',700,900,time,id+9));}
    }
    const beforeFrame=calls;
    await new Promise(requestAnimationFrame);
    const frameCalls=calls;
    // No rawupdate is delivered on the last part; ordinary coalesced input
    // and pointerup still need to supply its entire curved tail.
    const a=sample('pointermove',510,curve(510),++clock),b=sample('pointermove',520,curve(520),++clock);
    const up=sample('pointerup',530,curve(530),++clock);
    Object.defineProperty(up,'getCoalescedEvents',{value:()=>[a,b]});c.dispatchEvent(up);
    CanvasRenderingContext2D.prototype.stroke=native;
    return {beforeFrame,frameCalls,strokes:__IHN_TEST_API__.snapshot().pages[0].strokes.filter(s=>s.color==='#E81010')};
  });
  await cdp.send('Input.dispatchMouseEvent',{type:'mouseReleased',...target,button:'left',buttons:0,clickCount:1,pointerType:'pen'});
  expect(result.beforeFrame).toBe(0);expect(result.frameCalls).toBeGreaterThan(0);expect(result.frameCalls).toBeLessThanOrEqual(3);
  expect(result.strokes).toHaveLength(1);
  const points=result.strokes[0].points;
  expect(points.length).toBeGreaterThan(140);expect(points.length).toBeLessThan(210);
  expect(points.at(-1).x).toBeCloseTo(530,3);
  expect(Math.min(...points.map(p=>p.y))).toBeLessThan(361);
  expect(Math.max(...points.map(p=>p.y))).toBeLessThan(441);
  expect(points.filter(p=>p.x>=510)).toHaveLength(3);
  await page.evaluate(()=>__IHN_TEST_API__.checkpointBeforeLeaving());
  await page.reload();await page.waitForFunction(()=>window.__IHN_TEST_API__);
  const saved=await page.evaluate(async()=>{await __IHN_TEST_API__.ready();await __IHN_TEST_API__.loadPage(0);return __IHN_TEST_API__.snapshot().pages[0].strokes.filter(s=>s.color==='#E81010');});
  expect(saved).toEqual(result.strokes);
});

for(const count of [40,3000])test(`a stationary pen blocks full repaint and page snapshot work on ${count} pens until lift`,async({page,context})=>{
  const {cdp,target}=await startPen(page,context,count);
  await page.evaluate(()=>{
    window.penStoredPaints=0;window.penPageWrites=0;window.penSnapshotDone=false;
    const stroke=CanvasRenderingContext2D.prototype.stroke,put=IDBObjectStore.prototype.put;
    CanvasRenderingContext2D.prototype.stroke=function(...args){if(args[0] instanceof Path2D)penStoredPaints++;return stroke.apply(this,args);};
    IDBObjectStore.prototype.put=function(...args){if(this.name==='pages')penPageWrites++;return put.apply(this,args);};
    __IHN_TEST_API__.redrawDensePageForTest();
    window.penSnapshot=__IHN_TEST_API__.saveDensePageForTest().then(()=>{penSnapshotDone=true;});
  });
  await page.waitForTimeout(650); // Longer than both desktop and Android idle timers.
  expect(await page.evaluate(()=>({active:__IHN_TEST_API__.eraserPixelsForTest().penActive,paints:penStoredPaints,writes:penPageWrites,done:penSnapshotDone})))
    .toEqual({active:true,paints:0,writes:0,done:false});
  await cdp.send('Input.dispatchMouseEvent',{type:'mouseMoved',x:target.x+30,y:target.y+30,buttons:1,pointerType:'pen'});
  await cdp.send('Input.dispatchMouseEvent',{type:'mouseReleased',x:target.x+30,y:target.y+30,button:'left',buttons:0,clickCount:1,pointerType:'pen'});
  await page.evaluate(()=>penSnapshot);
  await page.waitForFunction(()=>!__IHN_TEST_API__.densePaintStatusForTest().pending);
  await page.evaluate(()=>__IHN_TEST_API__.checkpointBeforeLeaving());
  expect(await page.evaluate(()=>penPageWrites)).toBeGreaterThan(0);
  expect((await page.evaluate(()=>__IHN_TEST_API__.snapshot().pages[0].strokes)).filter(s=>s.color==='#E81010')).toHaveLength(1);
});
