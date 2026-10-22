import {test,expect} from '@playwright/test';
test.beforeEach(async({page})=>{
 await page.route('https://accounts.google.com/**',r=>r.fulfill({body:''}));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
});
test('offscreen PDF pages get real low-resolution previews before their full canvases load',async({page})=>{
 await page.evaluate(async()=>{
  const doc=await PDFLib.PDFDocument.create();
  for(let i=0;i<6;i++){const p=doc.addPage([400,600]);p.drawRectangle({x:50,y:80,width:300,height:400,color:PDFLib.rgb(1,0,0)});p.drawText('Page '+(i+1),{x:80,y:520,size:30});}
  await __IHN_TEST_API__.importPdfBlobForTest([...await doc.save()]);
  await __IHN_TEST_API__.showEditorForTest();
  queueScrollPagePreviews(true);
 });
 await page.waitForFunction(()=>state.pages[5]?.preview);
 const result=await page.evaluate(async()=>{
  const img=new Image();img.src=state.pages[5].preview;await img.decode();
  const c=document.createElement('canvas');c.width=img.naturalWidth;c.height=img.naturalHeight;const ctx=c.getContext('2d');ctx.drawImage(img,0,0);
  const pixel=[...ctx.getImageData(c.width/2,c.height/2,1,1).data];
  return {width:c.width,height:c.height,pixel,full:!!document.querySelector('canvas.page-canvas[data-page="5"]'),css:document.querySelector('.page-wrapper[data-page="5"]').style.backgroundImage};
 });
 expect(result.width).toBeLessThanOrEqual(320);expect(result.height).toBeLessThanOrEqual(440);
 expect(result.pixel[0]).toBeGreaterThan(200);expect(result.pixel[1]).toBeLessThan(40);expect(result.full).toBe(false);expect(result.css).toContain('data:image/jpeg');
 // Scrub immediately: the page already has content while detailed rendering waits.
 await page.locator('#document-fast-scroll').focus();await page.keyboard.press('End');
 await expect(page.locator('.page-wrapper[data-page="5"]')).toHaveCSS('background-image',/url/);
});
test('unloaded ink pages are previewed and cached without materializing full editor canvases',async({page})=>{
 await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(6,'Ink previews');
  const p=state.pages[5];p.strokes=[{id:'preview-ink',tool:'pen',color:'#ff0000',width:30,points:[{x:50,y:50},{x:300,y:300}]}];
  await savePageToIndexedDb(5,p);p.strokes=null;p.images=null;p.unloaded=true;p.preview=null;
  await __IHN_TEST_API__.showEditorForTest();queueScrollPagePreviews(true);
 });
 await page.waitForFunction(()=>state.pages[5]?.preview);
 await expect.poll(()=>page.evaluate(async()=>!!(await loadPageFromIndexedDb(5))?.preview)).toBe(true);
 const result=await page.evaluate(async()=>{
  const p=state.pages[5],img=new Image();img.src=p.preview;await img.decode();const c=document.createElement('canvas');c.width=img.width;c.height=img.height;const ctx=c.getContext('2d');ctx.drawImage(img,0,0);
  const pixels=ctx.getImageData(0,0,c.width,c.height).data;let red=0;for(let i=0;i<pixels.length;i+=4)if(pixels[i]>180&&pixels[i+1]<80&&pixels[i+2]<80)red++;
  return {red,unloaded:p.unloaded,strokes:p.strokes,full:!!document.querySelector('canvas.page-canvas[data-page="5"]')};
 });
 expect(result.red).toBeGreaterThan(5);expect(result.unloaded).toBe(true);expect(result.strokes).toBeNull();expect(result.full).toBe(false);
});
test('an old asynchronous preview cannot attach to a different document',async({page})=>{
 const result=await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(1,'Old');resetScrollPagePreviews();
  const p=state.pages[0];p.backgroundSource='pdf';p.pdfPageIndex=1;p.preview=null;
  let release;const waiting=new Promise(resolve=>release=resolve);const saved=activePdfDocument;
  activePdfDocument={getPage:()=>waiting};scrollPreviewQueue.set(0,0);const job=processScrollPagePreview();
  await __IHN_TEST_API__.resetLocalDocument(1,'New');resetScrollPagePreviews();
  release({getViewport(){throw Error('Old PDF must not render');}});await job;
  activePdfDocument=saved;return {name:state.exportName,preview:state.pages[0].preview||null};
 });
 expect(result).toEqual({name:'New',preview:null});
});
