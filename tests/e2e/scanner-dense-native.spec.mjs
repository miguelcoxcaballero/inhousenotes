import {expect,test} from '@playwright/test';
import {isDeepStrictEqual} from 'node:util';

// The short line/circle bridge fixture misses the thousands of branches in a
// handwritten page. Generate a camera-like raster, then let the real scanner
// vectorizer choose the ordinary pens sent through the editor bridge.
async function denseScan(page) {
  await page.goto('/scanner/index.html?embed=1&session=dense-ink', {waitUntil:'domcontentloaded'});
  return page.evaluate(async()=>{
    const cameraWidth=794,cameraHeight=1123,width=1800,height=Math.round(width*1123/794),rows=30,columns=30;
    const source=document.createElement('canvas');source.width=cameraWidth;source.height=cameraHeight;
    const ctx=source.getContext('2d');ctx.fillStyle='#fff';ctx.fillRect(0,0,cameraWidth,cameraHeight);
    ctx.lineCap=ctx.lineJoin='round';
    for(let row=0;row<rows;row++)for(let column=0;column<columns;column++){
      const x=68+column*22,y=150+row*26;
      ctx.strokeStyle=['#294e9b','#444444','#a52e3c'][row%3];
      for(let i=1;i<=96;i++){
        const a=(i-1)/96*Math.PI*2,b=i/96*Math.PI*2;
        ctx.lineWidth=.8+2*((1+Math.sin((a+b)/2))/2)**1.5;
        ctx.beginPath();ctx.moveTo(x+4+3.1*Math.cos(a),y+8+4.1*Math.sin(a));
        ctx.lineTo(x+4+3.1*Math.cos(b),y+8+4.1*Math.sin(b));ctx.stroke();
      }
      ctx.lineWidth=1.8+(column%3)*.1;
      ctx.beginPath();ctx.moveTo(x+7,y);ctx.lineTo(x+7,y+11);
      ctx.bezierCurveTo(x+8,y+15,x+11,y+8,x+12,y+6);
      ctx.bezierCurveTo(x+13,y+3,x+16,y+5,x+16,y+8);
      ctx.quadraticCurveTo(x+17,y+14,x+20,y+10);ctx.stroke();
      if(column%4===0){ctx.beginPath();ctx.moveTo(x+3,y+3);ctx.lineTo(x+10,y+3);ctx.stroke();}
    }
    const camera=document.createElement('canvas');camera.width=cameraWidth;camera.height=cameraHeight;
    const lens=camera.getContext('2d');lens.filter='blur(.35px)';lens.drawImage(source,0,0);lens.filter='none';
    const blob=await new Promise(resolve=>camera.toBlob(resolve,'image/jpeg',.9));
    const output=document.createElement('canvas');output.width=width;output.height=height;
    const cc=output.getContext('2d');
    const bitmap=await createImageBitmap(blob);cc.drawImage(bitmap,0,0,width,height);bitmap.close();
    const pixels=cc.getImageData(0,0,width,height),coverage=new Uint8Array(width*height);
    for(let i=0;i<coverage.length;i++)coverage[i]=Math.round(255-(pixels.data[i*4]*.2126+pixels.data[i*4+1]*.7152+pixels.data[i*4+2]*.0722));
    InhouseRust.separateInk(pixels.data);
    const started=performance.now();
    const ink=await ScannerPro.InkVectorizer.fromPixels(pixels,{coverage});
    const vectorMs=performance.now()-started;
    cc.clearRect(0,0,width,height);ScannerPro.InkVectorizer.render(cc,ink);
    return {ink,png:output.toDataURL(),glyphs:rows*columns,vectorMs,
      points:ink.strokes.reduce((sum,s)=>sum+s.points.length,0),bytes:JSON.stringify(ink).length};
  });
}

// Collaborative undo intentionally advances syncStamp; the stored pen ID,
// colour, width, pressure and every coordinate must still be lossless.
// Compare every coordinate, but report only the first changed stroke. A
// whole-page deep diff can exhaust the runner while formatting 180k points.
function equalStrokes(actual,expected){
  expect(actual.length).toBe(expected.length);
  for(let i=0;i<actual.length;i++)if(!isDeepStrictEqual(actual[i],expected[i])){
    expect(actual[i],`stroke ${i}: ${actual[i].id||'scan'}`).toEqual(expected[i]);
  }
}
const ordered=strokes=>strokes.map(({syncStamp,...stroke})=>stroke).sort((a,b)=>a.id.localeCompare(b.id));

test('a dense scanned page stays editable and preserves native pens through input, undo and reload',async({page,context})=>{
  test.setTimeout(120000);
  await page.setViewportSize({width:1440,height:1800});
  const scan=await denseScan(page);
  console.log('DENSE_SCAN_RASTER',JSON.stringify({glyphs:scan.glyphs,pens:scan.ink.strokes.length,points:scan.points,
    vectorMs:scan.vectorMs,bytes:scan.bytes,backend:scan.ink.backend,repairCount:scan.ink.repairCount,repairBackend:scan.ink.repairBackend}));
  // Bound data growth, rather than imposing a machine-dependent CPU deadline.
  expect(scan.ink.backend).toBe('rust');
  expect(scan.ink.reconstruction).toBe('measured-envelope');expect(scan.ink.repairBackend).toBe('javascript');expect(scan.ink.repairCount).toBeGreaterThan(0);
  expect(scan.ink.strokes.length).toBeGreaterThan(scan.glyphs);
  expect(scan.ink.strokes.length).toBeLessThan(scan.glyphs*12);
  // Full editable envelopes replace isolated repair discs. Keep the same
  // pen and serialized-byte budgets; allow their longer, continuous paths.
  expect(scan.points).toBeLessThan(scan.glyphs*220);
  expect(scan.bytes).toBeLessThan(scan.glyphs*6000);
  await page.goto('/?e2e=1',{waitUntil:'domcontentloaded'});
  await page.waitForFunction(()=>window.__IHN_TEST_API__);
  await page.evaluate(async()=>{
    await __IHN_TEST_API__.ready();await __IHN_TEST_API__.resetLocalDocument(1,'Dense scanned handwriting');
    await __IHN_TEST_API__.showEditorForTest();await __IHN_TEST_API__.openScannerForTest();
  });
  const frame=page.frameLocator('#scanner-editor-frame');
  await expect(frame.locator('#liveScanner')).toBeVisible();
  const insertionStarted=Date.now();
  await frame.locator('body').evaluate(async(_,{png,ink})=>{
    const blob=await(await fetch(png)).blob();
    parent.postMessage({source:'inhouse-scanner',type:'ihn-scanner-pages',
      sessionId:new URLSearchParams(location.search).get('session'),
      pages:[{blob,width:ink.width,height:ink.height,inkLayer:true,templateKind:'default',editableInk:ink}]},location.origin);
  },scan);
  await expect(page.locator('#scanner-editor-overlay')).not.toHaveClass(/visible/);
  const insertionMs=Date.now()-insertionStarted;
  const snapshot=()=>page.evaluate(()=>__IHN_TEST_API__.snapshot().pages[1]);
  const initial=await snapshot();
  expect(initial.strokes.length).toBe(scan.ink.strokes.length);
  expect(initial.images).toEqual([]);expect(initial.backgroundImage).toBeNull();
  expect(initial.backgroundSource).toBe('template');
  expect(new Set(initial.strokes.map(s=>s.id)).size).toBe(initial.strokes.length);
  const sx=794/scan.ink.width,sy=1123/scan.ink.height;
  const expected=scan.ink.strokes.map(stroke=>({...stroke,width:stroke.width*Math.sqrt(sx*sy),
    points:stroke.points.map(p=>({x:p.x*sx,y:p.y*sy}))}));
  equalStrokes(initial.strokes.map(({tool,color,width,points})=>({tool,color,width,points:points.map(({x,y})=>({x,y}))})),expected);
  const repairIds=new Set(initial.strokes.slice(-scan.ink.repairCount).map(s=>s.id));

  const canvas=page.locator('canvas.page-canvas[data-page="1"]');
  await page.mouse.move(1200,600);
  for(let i=0;i<3;i++)await page.mouse.wheel(0,-100);
  await expect.poll(()=>canvas.evaluate(c=>c.getBoundingClientRect().top)).toBeGreaterThan(60);
  const cdp=await context.newCDPSession(page);
  const position=(x,y)=>canvas.evaluate((c,{x,y})=>{
    const r=c.getBoundingClientRect();return {x:r.left+x*r.width/794,y:r.top+y*r.height/1123};
  },{x,y});
  async function gesture(points){
    await cdp.send('Input.dispatchMouseEvent',{type:'mousePressed',...await position(...points[0]),button:'left',buttons:1,clickCount:1,pointerType:'pen'});
    for(const p of points.slice(1))await cdp.send('Input.dispatchMouseEvent',{type:'mouseMoved',...await position(...p),buttons:1,pointerType:'pen'});
    await cdp.send('Input.dispatchMouseEvent',{type:'mouseReleased',...await position(...points.at(-1)),button:'left',buttons:0,clickCount:1,pointerType:'pen'});
  }
  const firstWord=[[60,143],[225,143],[225,169],[60,169],[60,143]];
  await page.locator('[data-tool="lasso"]').click();await gesture(firstWord);
  const selection=page.locator('.page-wrapper[data-page="1"] .selection-box');
  await expect(selection).toBeVisible();
  await selection.locator('.selection-color-btn').click();await selection.locator('[data-swatch-color="#E81010"]').click();
  const recolored=await snapshot();
  const changed=recolored.strokes.filter((s,i)=>s.color!==initial.strokes[i].color);
  expect(changed.length).toBeGreaterThan(10);
  expect(changed.every(s=>s.color==='#E81010'&&s.points.every(p=>p.y<170))).toBe(true);
  await page.locator('#btn-undo').click();
  equalStrokes(ordered((await snapshot()).strokes),ordered(initial.strokes));

  await page.locator('#btn-eraser').click();
  await page.evaluate(()=>{const toggle=document.getElementById('eraser-stroke-toggle');
    if(toggle.checked){toggle.checked=false;toggle.dispatchEvent(new Event('change',{bubbles:true}));}});
  await gesture([[183,149],[183,157],[183,165]]);
  const erased=await snapshot();
  expect(JSON.stringify(ordered(erased.strokes))===JSON.stringify(ordered(initial.strokes))).toBe(false);
  const untouched=strokes=>ordered(strokes.filter(s=>s.points.every(p=>p.y>190)));
  equalStrokes(untouched(erased.strokes),untouched(initial.strokes));
  await page.locator('#btn-undo').click();
  equalStrokes(ordered((await snapshot()).strokes),ordered(initial.strokes));

  // Dispatch the burst captured by a real stylus: rendering must batch it,
  // while retaining its curves and release tail on this scanned page.
  await page.locator('[data-tool="pen"][data-color="#E81010"]').click();
  await canvas.evaluate(c=>c.addEventListener('pointerdown',e=>window.densePenId=e.pointerId,{once:true}));
  await cdp.send('Input.dispatchMouseEvent',{type:'mousePressed',...await position(100,100),button:'left',buttons:1,clickCount:1,pointerType:'pen'});
  const input=await canvas.evaluate(async c=>{
    const r=c.getBoundingClientRect(),id=window.densePenId,native=CanvasRenderingContext2D.prototype.stroke;
    let calls=0,clock=performance.now();const started=clock;
    CanvasRenderingContext2D.prototype.stroke=function(...args){calls++;return native.apply(this,args);};
    const curve=x=>100+12*Math.sin((x-100)/15);
    const sample=(type,x,time)=>{
      const e=new PointerEvent(type,{clientX:r.left+x*r.width/794,clientY:r.top+curve(x)*r.height/1123,
        pointerId:id,pointerType:'pen',pressure:.5,buttons:1,bubbles:true});
      Object.defineProperty(e,'timeStamp',{value:time});return e;
    };
    for(let x=102;x<=400;x+=2){
      const raw=sample('pointerrawupdate',x,++clock);Object.defineProperty(raw,'getCoalescedEvents',{value:()=>[]});c.dispatchEvent(raw);
      const move=sample('pointermove',x,clock);Object.defineProperty(move,'getCoalescedEvents',{value:()=>[raw]});c.dispatchEvent(move);
    }
    const beforeFrame=calls,burstMs=performance.now()-started;await new Promise(requestAnimationFrame);const frameCalls=calls;
    const a=sample('pointermove',410,++clock),up=sample('pointerup',420,++clock);
    Object.defineProperty(up,'getCoalescedEvents',{value:()=>[a]});c.dispatchEvent(up);
    CanvasRenderingContext2D.prototype.stroke=native;
    return {beforeFrame,frameCalls,burstMs};
  });
  await cdp.send('Input.dispatchMouseEvent',{type:'mouseReleased',...await position(420,100),button:'left',buttons:0,clickCount:1,pointerType:'pen'});
  expect(input.beforeFrame).toBe(0);expect(input.frameCalls).toBeGreaterThan(0);expect(input.frameCalls).toBeLessThanOrEqual(3);
  const edited=await snapshot(),originalIds=new Set(initial.strokes.map(s=>s.id));
  const added=edited.strokes.filter(s=>!originalIds.has(s.id));expect(added).toHaveLength(1);
  equalStrokes(ordered(edited.strokes.filter(s=>originalIds.has(s.id))),ordered(initial.strokes));
  expect(added[0].points.length).toBeGreaterThan(120);
  expect(added[0].points.at(-1).x).toBeCloseTo(420,3);
  expect(Math.min(...added[0].points.map(p=>p.y))).toBeLessThan(89);
  expect(Math.max(...added[0].points.map(p=>p.y))).toBeGreaterThan(111);
  await page.evaluate(()=>__IHN_TEST_API__.checkpointBeforeLeaving());
  await page.reload({waitUntil:'domcontentloaded'});await page.waitForFunction(()=>window.__IHN_TEST_API__);
  await page.evaluate(async()=>{await __IHN_TEST_API__.ready();await __IHN_TEST_API__.loadPage(1);});
  equalStrokes(ordered((await snapshot()).strokes),ordered(edited.strokes));
  const pdf=await page.evaluate(()=>__IHN_TEST_API__.buildPdfBlobForTest({includeBytes:true}));
  expect(pdf.ok).toBe(true);
  await page.evaluate(bytes=>__IHN_TEST_API__.importPdfBlobForTest(bytes),pdf.bytes);
  await page.evaluate(()=>__IHN_TEST_API__.loadPage(1));
  const reopened=await snapshot();
  // PDF metadata stores coordinates at 0.01 editor pixels; that must not
  // discard the native contours which recover photographed letter lobes.
  const rounded=strokes=>ordered(strokes).map(s=>({...s,points:s.points.map(p=>({...p,
    x:Math.round(p.x*100)/100,y:Math.round(p.y*100)/100}))}));
  equalStrokes(rounded(reopened.strokes),rounded(edited.strokes));
  const reopenedRepairs=reopened.strokes.filter(s=>repairIds.has(s.id));
  expect(reopenedRepairs).toHaveLength(scan.ink.repairCount);
  expect(reopenedRepairs.every(s=>s.points.length>=2&&s.points.some(p=>Math.hypot(p.x-s.points[0].x,p.y-s.points[0].y)>.02))).toBe(true);
  expect(reopened.images).toEqual([]);expect(reopened.backgroundImage).toBeNull();
  console.log('DENSE_SCAN_NATIVE',JSON.stringify({glyphs:scan.glyphs,pens:scan.ink.strokes.length,points:scan.points,
    vectorMs:scan.vectorMs,insertionMs,bytes:scan.bytes,backend:scan.ink.backend,repairCount:scan.ink.repairCount,
    repairBackend:scan.ink.repairBackend,input}));
});
