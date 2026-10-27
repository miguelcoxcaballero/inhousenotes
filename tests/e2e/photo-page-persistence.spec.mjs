import { test, expect } from '@playwright/test';
for (const original of [false, true]) test(`photo page survives save, local reload and PDF reopen (original PDF=${original})`, async ({ page }) => {
 test.setTimeout(90_000);
 await page.goto('/?e2e=1'); await page.evaluate(() => __IHN_TEST_API__.ready());
 const local = await page.evaluate(async original => {
  await __IHN_TEST_API__.resetLocalDocument(1, 'Photo persistence');
  if (original) { const pdf = new jspdf.jsPDF(); pdf.text('Original', 20, 20); await __IHN_TEST_API__.importPdfBlobForTest([...new Uint8Array(pdf.output('arraybuffer'))]); }
  const canvas = document.createElement('canvas'); canvas.width=160;canvas.height=200;
  const ctx=canvas.getContext('2d');ctx.fillStyle='#19bb42';ctx.fillRect(0,0,160,200);
  const file=new File([await new Promise(r=>canvas.toBlob(r,'image/png'))],'photo.png',{type:'image/png'});
  const asset=await buildCoverImageAsset(file,'original');
  if(!await insertPhotoPageAt(asset,0))throw new Error('Photo insertion failed');
  await __IHN_TEST_API__.checkpointBeforeLeaving();
  const saved=await loadPageFromIndexedDb(1);
  return { source:saved.backgroundSource, image:!!saved.backgroundImage };
 }, original);
 expect(local).toEqual({source:'custom',image:true});
 await page.reload();await page.evaluate(()=>__IHN_TEST_API__.ready());
 expect(await page.evaluate(async()=>{await ensurePageDataLoaded(1);return !!state.pages[1].backgroundImage;})).toBe(true);
 const pixels=await page.evaluate(async()=>{
  const blob=await buildPdfBlob({status:false});
  const bytes=new Uint8Array(await blob.arrayBuffer());
  async function pixel(doc){const p=await doc.getPage(2),v=p.getViewport({scale:.2}),c=document.createElement('canvas');c.width=v.width;c.height=v.height;const ctx=c.getContext('2d');await p.render({canvasContext:ctx,viewport:v,intent:'print'}).promise;return [...ctx.getImageData(Math.floor(c.width/2),Math.floor(c.height/2),1,1).data];}
  const pdf=await pdfjsLib.getDocument({data:bytes.slice()}).promise;
  const exported=await pixel(pdf);await pdf.destroy();
  await __IHN_TEST_API__.importPdfBlobForTest([...bytes]);
  if(cleanOriginalRecoveryPromise)await cleanOriginalRecoveryPromise;
  const reopened=await pixel(activePdfDocument);
  const second=await buildPdfBlob({status:false});
  const pdf2=await pdfjsLib.getDocument({data:new Uint8Array(await second.arrayBuffer())}).promise;
  const resaved=await pixel(pdf2);await pdf2.destroy();
  return {exported,reopened,resaved};
 });
 for(const [stage,rgba] of Object.entries(pixels)){expect(rgba[1],stage).toBeGreaterThan(130);expect(rgba[0],stage).toBeLessThan(80);expect(rgba[2],stage).toBeLessThan(110);}
});

test('raster fallback includes the photo even when the live ink canvas is transparent',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(1,'Raster photo');
  await __IHN_TEST_API__.showEditorForTest();
  await new Promise(requestAnimationFrame);
  const photo=document.createElement('canvas');photo.width=100;photo.height=100;
  const ctx=photo.getContext('2d');ctx.fillStyle='#19bb42';ctx.fillRect(0,0,100,100);
  const data=photo.toDataURL();
  state.pages[0].backgroundSource='custom';state.pages[0].backgroundImage=data;
  state.pages[0].needsRedraw=false;
  const live=document.querySelector('canvas.page-canvas[data-page="0"]');
  live.style.backgroundImage=`url("${data}")`;
  const reused=!!getPageCanvasForExport(0,state.pages[0]);
  canBuildCurrentPdfWithPdfLib=()=>false;
  const blob=await buildPdfBlob({status:false});
  const pdf=await pdfjsLib.getDocument({data:new Uint8Array(await blob.arrayBuffer())}).promise;
  const p=await pdf.getPage(1),v=p.getViewport({scale:.2}),c=document.createElement('canvas');c.width=v.width;c.height=v.height;
  const out=c.getContext('2d');await p.render({canvasContext:out,viewport:v,intent:'print'}).promise;
  const rgba=[...out.getImageData(Math.floor(c.width/2),Math.floor(c.height/2),1,1).data];
  await pdf.destroy();return {reused,rgba};
 });
 expect(result.reused).toBe(false);expect(result.rgba[0]).toBeLessThan(80);expect(result.rgba[1]).toBeGreaterThan(130);
});

test('unreadable photo rejects export rather than silently saving a blank page',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(1,'Bad photo');
  state.pages[0].backgroundSource='custom';state.pages[0].backgroundImage='data:image/jpeg;base64,aGVsbG8=';
  try {await buildPdfBlob({status:false});return 'saved';}catch(error){return error.message;}
 });
 expect(result).not.toBe('saved');expect(result).toMatch(/image|decode|load/i);
});

test('full-page photos retain high-resolution detail and bounded PNG originals are lossless',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(1,'Photo detail');
  const canvas=document.createElement('canvas');canvas.width=2400;canvas.height=3200;
  const ctx=canvas.getContext('2d');ctx.fillStyle='white';ctx.fillRect(0,0,2400,3200);
  ctx.fillStyle='#17283f';for(let x=10;x<2400;x+=8)ctx.fillRect(x,0,2,3200);
  const png=await new Promise(r=>canvas.toBlob(r,'image/png'));
  const original=await dataUrlFromBlob(png);
  const photo=await buildCoverImageAsset(png,'photo');
  const inserted=await compressImageForDocument(png);
  const jpeg=await new Promise(r=>canvas.toBlob(r,'image/jpeg',.98));
  const asset=await buildCoverImageAsset(jpeg,'fit');
  const decoded=await decodeImageSource(new Blob([Uint8Array.from(atob(asset.dataUrl.split(',')[1]),c=>c.charCodeAt(0))],{type:'image/jpeg'}));
  const size={width:decoded.width,height:decoded.height};decoded.cleanup();
  return {photoUnchanged:photo.dataUrl===original,insertedUnchanged:inserted===original,size,ratio:asset.pageWidth/asset.pageHeight};
 });
 expect(result.photoUnchanged).toBe(true);expect(result.insertedUnchanged).toBe(true);
 expect(result.size.height).toBeGreaterThan(3000);expect(result.size.height).toBeLessThanOrEqual(3840);
 expect(result.size.width/result.size.height).toBeCloseTo(result.ratio,2);
});

test('photo imports use one encode, preserve transparency and avoid upscaling small sources',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  await __IHN_TEST_API__.resetLocalDocument(1,'Bounded photo');
  const canvas=document.createElement('canvas');canvas.width=4000;canvas.height=3000;
  const ctx=canvas.getContext('2d');ctx.fillStyle='#209abc';ctx.fillRect(0,0,4000,3000);
  const large=await new Promise(r=>canvas.toBlob(r,'image/jpeg',.96));
  const originalEncoder=canvasToBlobSafe;let encodes=0;
  canvasToBlobSafe=async(...args)=>{encodes++;return originalEncoder(...args);};
  const url=await compressImageForDocument(large);
  const decoded=await decodeImageSource(new Blob([Uint8Array.from(atob(url.split(',')[1]),c=>c.charCodeAt(0))],{type:'image/jpeg'}));
  const largeSize=[decoded.width,decoded.height];decoded.cleanup();const largeEncodes=encodes;
  canvas.width=80;canvas.height=100;ctx.fillStyle='red';ctx.fillRect(20,20,40,60);
  const small=await new Promise(r=>canvas.toBlob(r,'image/png'));
  const preserved=await compressImageForDocument(small);const original=await dataUrlFromBlob(small);
  const smallJpeg=await new Promise(r=>canvas.toBlob(r,'image/jpeg',.95));
  const asset=await buildCoverImageAsset(smallJpeg,'photo');
  const image=await decodeImageSource(new Blob([Uint8Array.from(atob(asset.dataUrl.split(',')[1]),c=>c.charCodeAt(0))],{type:'image/jpeg'}));
  const smallSize=[image.width,image.height];image.cleanup();canvasToBlobSafe=originalEncoder;
  return {largeSize,largeEncodes,alphaUnchanged:preserved===original,smallSize};
 });
 expect(result.largeEncodes).toBe(1);expect(result.largeSize).toEqual([3200,2400]);
 expect(result.alphaUnchanged).toBe(true);expect(result.smallSize[0]).toBeLessThanOrEqual(81);expect(result.smallSize[1]).toBeLessThanOrEqual(101);
});
