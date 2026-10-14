import { expect, test } from '@playwright/test';

async function fixture(page) {
  await page.goto('/scanner/index.html?embed=1&session=vector-test', {waitUntil:'domcontentloaded'});
  return page.evaluate(async()=>{
    const canvas=document.createElement('canvas');canvas.width=794;canvas.height=1123;
    const ctx=canvas.getContext('2d');ctx.lineCap=ctx.lineJoin='round';
    ctx.strokeStyle='rgba(170,23,40,.65)';ctx.lineWidth=4;
    ctx.beginPath();ctx.moveTo(110,150);ctx.lineTo(400,150);ctx.stroke();
    ctx.strokeStyle='rgba(65,85,190,.72)';ctx.lineWidth=3;
    ctx.beginPath();ctx.arc(240,270,50,0,Math.PI*2);ctx.stroke();
    ctx.strokeStyle='rgba(24,24,24,.28)';ctx.lineWidth=2;
    ctx.beginPath();ctx.moveTo(100,400);ctx.bezierCurveTo(400,320,100,540,400,470);ctx.stroke();
    ctx.fillStyle='rgba(60,60,60,.4)';ctx.beginPath();ctx.arc(440,470,2,0,Math.PI*2);ctx.fill();
    ctx.fillStyle='rgba(0,0,0,.05)';ctx.fillRect(500,500,30,15);
    const pixels=ctx.getImageData(0,0,canvas.width,canvas.height);
    const start=performance.now();
    const ink=await ScannerPro.InkVectorizer.fromPixels(pixels);
    const elapsed=performance.now()-start;
    const fallback=await ScannerPro.InkVectorizer.fromPixels(pixels,{forceJavaScript:true});
    const output=document.createElement('canvas');output.width=canvas.width;output.height=canvas.height;
    ScannerPro.InkVectorizer.render(output.getContext('2d'),ink);
    const raster=output.getContext('2d').getImageData(0,0,output.width,output.height).data;
    let sourceInk=0,missing=0,outputInk=0,extra=0;
    const nearby=(data,x,y)=>{for(let dy=-2;dy<=2;dy++)for(let dx=-2;dx<=2;dx++){
      const xx=x+dx,yy=y+dy;if(xx>=0&&xx<794&&yy>=0&&yy<1123&&data[(yy*794+xx)*4+3]>=30)return true;}return false;};
    for(let y=1;y<1122;y++)for(let x=1;x<793;x++){
      const i=(y*794+x)*4;
      if(pixels.data[i+3]>=30){sourceInk++;if(!nearby(raster,x,y))missing++;}
      if(raster[i+3]>=30){outputInk++;if(!nearby(pixels.data,x,y))extra++;}
    }
    window.editableFixture={ink,png:output.toDataURL(),source:canvas.toDataURL()};
    return {ink,elapsed,sourceInk,missing,extra,outputInk,
      parity:JSON.stringify(ink.strokes)===JSON.stringify(fallback.strokes),
      widths:ink.strokes.map(s=>s.width),bytes:JSON.stringify(ink).length};
  });
}

test('Rust makes opaque palette pens while preserving curves, closed loops, pencil and punctuation',async({page})=>{
  const report=await fixture(page);
  console.log(JSON.stringify({vectorMs:report.elapsed,strokes:report.ink.strokes.length,bytes:report.bytes,missing:report.missing,extra:report.extra}));
  expect(report.ink.backend).toBe('rust');expect(report.parity).toBe(true);
  expect([...new Set(report.ink.strokes.map(s=>s.color))].sort()).toEqual(['#002FD9','#4D4D4D','#E81010']);
  expect(report.missing/report.sourceInk).toBeLessThan(.015);
  expect(report.extra/report.outputInk).toBeLessThan(.015);
  expect(report.ink.strokes.length).toBeLessThan(40);
  expect(report.ink.strokes.some(s=>s.points.every(p=>p.x>435&&p.x<445&&p.y>465&&p.y<475))).toBe(true);
  expect(report.ink.strokes.some(s=>s.points.some(p=>p.x>=500&&p.y>=500))).toBe(false);
  for(const stroke of report.ink.strokes){
    expect(stroke.tool).toBe('pen');expect(stroke.width).toBeGreaterThan(0);
    expect(stroke.opacity).toBeUndefined();
    for(let i=1;i<stroke.points.length;i++)expect(Math.hypot(stroke.points[i].x-stroke.points[i-1].x,stroke.points[i].y-stroke.points[i-1].y)).toBeLessThanOrEqual(2.001);
  }
});

async function insertVectors(page, corrupt=false) {
  const report=await fixture(page),ink=report.ink;
  const png=await page.evaluate(()=>editableFixture.png);
  await page.goto('/?e2e=1',{waitUntil:'domcontentloaded'});
  await page.waitForFunction(()=>window.__IHN_TEST_API__);
  await page.evaluate(async()=>{await __IHN_TEST_API__.ready();await __IHN_TEST_API__.resetLocalDocument(1,'Editable scan');
    await __IHN_TEST_API__.showEditorForTest();await __IHN_TEST_API__.openScannerForTest();});
  const frame=page.frameLocator('#scanner-editor-frame');
  await expect(frame.locator('#liveScanner')).toBeVisible();
  if(corrupt)ink.strokes[0].color='#123456';
  await frame.locator('body').evaluate(async(_,{png,ink})=>{
    window.scanReply=null;addEventListener('message',e=>{if(e.data?.type==='ihn-scanner-result')window.scanReply=e.data;});
    const blob=await(await fetch(png)).blob();
    parent.postMessage({source:'inhouse-scanner',type:'ihn-scanner-pages',sessionId:new URLSearchParams(location.search).get('session'),
      pages:[{blob,width:ink.width,height:ink.height,inkLayer:true,templateKind:'default',editableInk:ink}]},location.origin);
  },{png,ink});
  return {frame,ink};
}

test('scan pens survive local reload and PDF reopening with no photograph underneath',async({page})=>{
  test.setTimeout(120000);
  const {ink}=await insertVectors(page);
  await expect(page.locator('#scanner-editor-overlay')).not.toHaveClass(/visible/);
  const snapshot=()=>page.evaluate(async()=>{await __IHN_TEST_API__.loadPage(1);return __IHN_TEST_API__.snapshot().pages[1];});
  const inserted=await snapshot();
  expect(inserted.images).toEqual([]);expect(inserted.backgroundImage).toBeNull();expect(inserted.backgroundSource).toBe('template');
  expect(inserted.strokes.length).toBe(ink.strokes.length);
  expect(inserted.strokes.map(({tool,color,width,points})=>({tool,color,width,points:points.map(({x,y})=>({x,y}))}))).toEqual(ink.strokes);
  expect(new Set(inserted.strokes.map(s=>s.id)).size).toBe(inserted.strokes.length);
  await page.evaluate(()=>__IHN_TEST_API__.checkpointBeforeLeaving());
  await page.reload({waitUntil:'domcontentloaded'});await page.waitForFunction(()=>window.__IHN_TEST_API__);
  await page.evaluate(()=>__IHN_TEST_API__.ready());
  expect((await snapshot()).strokes).toEqual(inserted.strokes);
  const pdf=await page.evaluate(()=>__IHN_TEST_API__.buildPdfBlobForTest({includeBytes:true}));expect(pdf.ok).toBe(true);
  await page.evaluate(bytes=>__IHN_TEST_API__.importPdfBlobForTest(bytes),pdf.bytes);
  const reopened=await snapshot();
  // The existing PDF format stores native coordinates to 0.01 logical pixel.
  const rounded=strokes=>strokes.map(s=>({...s,points:s.points.map(p=>({...p,x:Math.round(p.x*100)/100,y:Math.round(p.y*100)/100}))}));
  expect(rounded(reopened.strokes)).toEqual(rounded(inserted.strokes));
  expect(reopened.images).toEqual([]);expect(reopened.backgroundImage).toBeNull();
});

test('native lasso resize, recolour, partial erase and undo work on scanned pens',async({page,context})=>{
  test.setTimeout(90000);
  await insertVectors(page);
  await expect(page.locator('#scanner-editor-overlay')).not.toHaveClass(/visible/);
  const canvas=page.locator('canvas.page-canvas[data-page="1"]');
  const snapshot=()=>page.evaluate(()=>__IHN_TEST_API__.snapshot().pages[1]);
  const initial=await snapshot();
  // The editor centres tall pages by transforming its canvas container; it
  // deliberately does not use a browser scrolling element.
  await page.mouse.move(900,350);
  for(let i=0;i<3;i++)await page.mouse.wheel(0,-100);
  await expect.poll(()=>canvas.evaluate(c=>c.getBoundingClientRect().top)).toBeGreaterThan(60);
  const cdp=await context.newCDPSession(page);
  async function position(x,y){return canvas.evaluate((canvas,{x,y})=>{
    const r=canvas.getBoundingClientRect();return {x:r.left+x*r.width/794,y:r.top+y*r.height/1123};},{x,y});}
  async function gesture(points){
    const first=await position(...points[0]);
    await cdp.send('Input.dispatchMouseEvent',{type:'mousePressed',...first,button:'left',buttons:1,clickCount:1,pointerType:'pen'});
    for(const p of points.slice(1)){const xy=await position(...p);await cdp.send('Input.dispatchMouseEvent',{type:'mouseMoved',...xy,buttons:1,pointerType:'pen'});}
    const last=await position(...points.at(-1));
    await cdp.send('Input.dispatchMouseEvent',{type:'mouseReleased',...last,button:'left',buttons:0,clickCount:1,pointerType:'pen'});
  }
  await page.locator('[data-tool="lasso"]').click();
  await gesture([[90,125],[420,125],[420,175],[90,175],[90,125]]);
  const selection=page.locator('.page-wrapper[data-page="1"] .selection-box');
  await expect(selection).toBeVisible();
  await selection.locator('.selection-color-btn').click();
  await selection.locator('[data-swatch-color="#002FD9"]').click();
  await expect.poll(async()=> (await snapshot()).strokes.filter(s=>s.color==='#E81010').length).toBe(0);
  await page.locator('#btn-undo').click();
  expect((await snapshot()).strokes.map(s=>s.color)).toEqual(initial.strokes.map(s=>s.color));
  // Undo clears selection; select the horizontal pen again before resizing.
  await gesture([[90,125],[420,125],[420,175],[90,175],[90,125]]);
  const handle=selection.locator('[data-handle="e"]');const box=await handle.boundingBox();
  await page.mouse.move(box.x+box.width/2,box.y+box.height/2);await page.mouse.down();
  await page.mouse.move(box.x+box.width/2+60,box.y+box.height/2,{steps:8});await page.mouse.up();
  const right=strokes=>Math.max(...strokes.filter(s=>s.color==='#E81010').flatMap(s=>s.points.map(p=>p.x)));
  await expect.poll(async()=>right((await snapshot()).strokes)).toBeGreaterThan(right(initial.strokes)+20);
  await page.locator('#btn-undo').click();
  expect(right((await snapshot()).strokes)).toBeCloseTo(right(initial.strokes),2);
  await page.locator('#btn-eraser').click();
  // Default eraser is the partial (area) mode; explicitly select it if needed.
  await page.evaluate(()=>{const toggle=document.getElementById('eraser-stroke-toggle');
    if(toggle.checked){toggle.checked=false;toggle.dispatchEvent(new Event('change',{bubbles:true}));}});
  await gesture([[250,145],[250,150],[250,155]]);
  const erased=await snapshot();
  expect(erased.images).toEqual([]);
  const red=erased.strokes.filter(s=>s.color==='#E81010');
  expect(red.length).toBeGreaterThan(initial.strokes.filter(s=>s.color==='#E81010').length);
  expect(red.some(s=>s.points.some(p=>p.x<150))).toBe(true);expect(red.some(s=>s.points.some(p=>p.x>350))).toBe(true);
  expect(red.some(s=>s.points.some(p=>Math.abs(p.x-250)<3))).toBe(false);
  await page.locator('#btn-undo').click();
  // Collaborative undo reinserts restored IDs after unrelated surviving pens.
  expect((await snapshot()).strokes.map(s=>s.id).sort()).toEqual(initial.strokes.map(s=>s.id).sort());
  await page.locator('[data-tool="pen"][data-color="#E81010"]').click();
  await gesture([[100,550],[120,555],[140,550],[160,555]]);
  await expect.poll(async()=>(await snapshot()).strokes.length).toBeGreaterThan(initial.strokes.length);
});

test('the scanner bridge rejects intermediate colours instead of storing them',async({page})=>{
  const {frame}=await insertVectors(page,true);
  await expect.poll(()=>frame.locator('body').evaluate(()=>window.scanReply?.ok)).toBe(false);
  expect(await page.evaluate(()=>__IHN_TEST_API__.snapshot().pages.length)).toBe(1);
});

test('blank ink and cancellation cannot introduce template dots or a partial vector result',async({page})=>{
  await page.goto('/scanner/index.html?embed=1&session=vector-empty',{waitUntil:'domcontentloaded'});
  const result=await page.evaluate(async()=>{
    const pixels=new ImageData(320,450),controller=new AbortController();
    const blank=await ScannerPro.InkVectorizer.fromPixels(pixels);
    controller.abort();let aborted;
    try{await ScannerPro.InkVectorizer.fromPixels(pixels,{signal:controller.signal});}catch(error){aborted=error.name;}
    return {count:blank.strokes.length,aborted};
  });
  expect(result).toEqual({count:0,aborted:'AbortError'});
});

test('weak JPEG fringes do not turn coloured handwriting into outlined black pens',async({page})=>{
  await page.goto('/scanner/index.html?embed=1&session=vector-fringe',{waitUntil:'domcontentloaded'});
  const result=await page.evaluate(async()=>{
    const canvas=document.createElement('canvas');canvas.width=320;canvas.height=180;
    const ctx=canvas.getContext('2d');
    ctx.fillStyle='rgba(25,25,25,.18)';ctx.fillRect(30,66,250,9);
    ctx.fillStyle='#164fbb';ctx.fillRect(30,69,250,3);
    // A real neutral crossing and faint pencil must remain black.
    ctx.fillStyle='rgb(35,35,35)';ctx.fillRect(159,35,3,85);
    ctx.fillStyle='rgba(0,0,0,.1)';ctx.fillRect(30,140,240,2);
    const pixels=ctx.getImageData(0,0,320,180);
    const ink=await ScannerPro.InkVectorizer.fromPixels(pixels),js=await ScannerPro.InkVectorizer.fromPixels(pixels,{forceJavaScript:true});
    return {parity:JSON.stringify(ink.strokes)===JSON.stringify(js.strokes),strokes:ink.strokes};
  });
  expect(result.parity).toBe(true);
  expect(result.strokes.some(s=>s.color==='#4D4D4D'&&s.points.some(p=>p.y>138))).toBe(true);
  expect(result.strokes.some(s=>s.color==='#4D4D4D'&&s.points.some(p=>p.y<50))).toBe(true);
  expect(result.strokes.some(s=>s.color==='#4D4D4D'&&s.points.some(p=>p.y>=66&&p.y<=75&&Math.abs(p.x-160)>8))).toBe(false);
});

test('a photographed page reaches the document as native pens through the actual Add button',async({page})=>{
  test.setTimeout(60000);
  await page.goto('/?e2e=1',{waitUntil:'domcontentloaded'});await page.waitForFunction(()=>window.__IHN_TEST_API__);
  await page.evaluate(async()=>{await __IHN_TEST_API__.ready();await __IHN_TEST_API__.resetLocalDocument(1,'Photographed ink');
    await __IHN_TEST_API__.showEditorForTest();await __IHN_TEST_API__.openScannerForTest();});
  const iframe=page.frameLocator('#scanner-editor-frame');await expect(iframe.locator('#liveScanner')).toBeVisible();
  const frame=page.frames().find(frame=>frame.url().includes('/scanner/index.html'));
  await frame.addScriptTag({url:'/tests/e2e/fixtures/scanner-developable.js'});
  await frame.evaluate(async()=>{
    const source=ScannerDevelopableFixtures.render(ScannerDevelopableFixtures.CASES[0]).source;
    const blob=await new Promise(resolve=>source.toBlob(resolve,'image/jpeg',.94));source.width=source.height=0;
    const transfer=new DataTransfer();transfer.items.add(new File([blob],'scan.jpg',{type:'image/jpeg'}));
    const input=document.getElementById('liveFileInput');input.files=transfer.files;input.dispatchEvent(new Event('change',{bubbles:true}));
  });
  await expect(iframe.locator('#liveAddBtn')).toBeEnabled({timeout:45000});
  await iframe.locator('#liveAddBtn').click();await expect(page.locator('#scanner-editor-overlay')).not.toHaveClass(/visible/);
  const inserted=await page.evaluate(()=>__IHN_TEST_API__.snapshot().pages[1]);
  expect(inserted.images).toEqual([]);expect(inserted.backgroundImage).toBeNull();
  expect(inserted.backgroundSource).toBe('template');expect(inserted.strokes.length).toBeGreaterThan(0);
  expect(inserted.strokes.every(s=>s.tool==='pen'&&['#4D4D4D','#002FD9','#E81010'].includes(s.color))).toBe(true);
  // Junction tangents must not mistake a line's length for its width.
  expect(Math.max(...inserted.strokes.map(s=>s.width))).toBeLessThan(8);
});

test('mixed camera fringes produce one red axis, while real blue and black crossings keep their colour',async({page})=>{
  await page.goto('/scanner/index.html?embed=1&session=colours');
  const result=await page.evaluate(async()=>{
    const c=document.createElement('canvas');c.width=794;c.height=1123;
    const x=c.getContext('2d');x.lineCap='round';
    const curve=()=>{x.beginPath();x.moveTo(90,160);x.bezierCurveTo(160,70,220,250,370,150);};
    // The photograph's dark centre and blue JPEG edge are all the SAME red pen.
    x.strokeStyle='rgba(20,50,160,.45)';x.lineWidth=7;curve();x.stroke();
    x.strokeStyle='#ba1722';x.lineWidth=6;curve();x.stroke();
    x.strokeStyle='#343434';x.lineWidth=1.5;curve();x.stroke();
    x.fillStyle='#164fbb';x.fillRect(100,400,300,3);
    x.fillStyle='#292929';x.fillRect(249,365,3,80);
    // A pale neutral dot with sensor tint must not become a red pen.
    x.fillStyle='rgba(90,0,0,.1)';x.fillRect(500,500,3,3);
    const pixels=x.getImageData(0,0,c.width,c.height);
    const ink=await ScannerPro.InkVectorizer.fromPixels(pixels);
    const js=await ScannerPro.InkVectorizer.fromPixels(pixels,{forceJavaScript:true});
    return {strokes:ink.strokes,parity:JSON.stringify(ink.strokes)===JSON.stringify(js.strokes)};
  });
  expect(result.parity).toBe(true);
  const red=result.strokes.filter(s=>s.points.some(p=>p.y<300));
  expect(red.length).toBeLessThanOrEqual(3);
  expect(red.every(s=>s.color==='#E81010')).toBe(true);
  expect(result.strokes.some(s=>s.color==='#002FD9'&&s.points.some(p=>p.x<150&&p.y>395))).toBe(true);
  expect(result.strokes.some(s=>s.color==='#4D4D4D'&&s.points.some(p=>p.y>360&&p.y<380))).toBe(true);
  const dust=result.strokes.filter(s=>s.points.some(p=>p.x>490));
  expect(dust).toHaveLength(1);expect(dust[0].color).toBe('#4D4D4D');
});

test('near-standard pen widths snap to the editor width at either scan resolution, preserving thin and thick pens',async({page})=>{
  await page.goto('/scanner/index.html?embed=1&session=widths');
  const result=await page.evaluate(async()=>{
    const reports=[];
    for(const scale of [1,2]){
      const c=document.createElement('canvas');c.width=794*scale;c.height=1123*scale;
      const ctx=c.getContext('2d');ctx.scale(scale,scale);ctx.strokeStyle='#ba1722';ctx.lineCap='round';
      [1,2,2.5,6].forEach((w,k)=>{ctx.lineWidth=w;ctx.beginPath();ctx.moveTo(100,100+k*100+(k===0?.5:0));ctx.lineTo(400,100+k*100+(k===0?.5:0));ctx.stroke();});
      const ink=await ScannerPro.InkVectorizer.fromPixels(ctx.getImageData(0,0,c.width,c.height));
      reports.push(ink.strokes.map(s=>({width:s.width/scale,y:s.points[0].y/scale})).sort((a,b)=>a.y-b.y));
    }
    return {reports,standard:InhouseInkDefaults.width};
  });
  for(const report of result.reports){
    expect(report).toHaveLength(4);
    expect(report[0].width).toBeLessThan(1.5);
    expect(report[1].width).toBeCloseTo(result.standard,6);
    expect(report[2].width).toBeCloseTo(result.standard,6);
    expect(report[3].width).toBeGreaterThan(4.5);
  }
  await page.goto('/?e2e=1');
  for(const pen of await page.locator('[data-tool="pen"][data-width]').all())expect(Number(await pen.getAttribute('data-width'))).toBe(result.standard);
});
