import {test,expect} from '@playwright/test';
test('repeated PDF saves reuse unchanged strokes and invalidate the edited page',async({page})=>{
 test.setTimeout(120000);
 await page.goto('/?e2e=1');await page.evaluate(()=>window.__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  await __IHN_TEST_API__.seedDensePageForTest(800,60);
  async function build(){const start=performance.now(),blob=await buildPdfBlob({status:false,optimizeForUpload:true});return {blob,ms:performance.now()-start};}
  const first=await build();
  const firstCompact=pdfCompactStrokeCache.get(state.pages[0]);
  const second=await build();
  const reused=pdfCompactStrokeCache.get(state.pages[0])===firstCompact;
  state.pages[0].strokes.push({id:'cache-new-ink',tool:'pen',color:'#000000',width:3,points:[{x:10,y:20},{x:30,y:50}]});
  markPageDirty(0,'full');bumpDriveContentVersion();
  const third=await build();
  const updated=pdfCompactStrokeCache.get(state.pages[0]);
  const doc=await pdfjsLib.getDocument({data:await third.blob.arrayBuffer()}).promise;
  const keywords=(await doc.getMetadata()).info.Keywords;
  await doc.destroy();
  const encoded=keywords.match(/STROKES_Z:([^;]+)/)[1];
  const bytes=Uint8Array.from(atob(encoded),c=>c.charCodeAt(0));
  const text=await new Response(new Blob([bytes]).stream().pipeThrough(new DecompressionStream('deflate'))).text();
  return {reused,invalidated:updated!==firstCompact,newInk:JSON.stringify(JSON.parse(text)).includes('cache-new-ink'),
   timings:{first:first.ms,repeat:second.ms,edit:third.ms},sizes:[first.blob.size,second.blob.size,third.blob.size]};
 });
 console.log('PDF_SAVE_BENCHMARK',JSON.stringify(result));
 expect(result.reused).toBe(true);expect(result.invalidated).toBe(true);expect(result.newInk).toBe(true);
});
test('cached PNG processing preserves exact pixels across documents and changes',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>window.__IHN_TEST_API__.ready());
 const result=await page.evaluate(async()=>{
  const processor=InhouseDocumentProcessing,lib=await processor.pdfLib(PDFLib);
  const canvas=new OffscreenCanvas(1024,1024),ctx=canvas.getContext('2d');
  const bytes=async()=>new Uint8Array(await (await canvas.convertToBlob({type:'image/png'})).arrayBuffer());
  async function render(png){
   const doc=await lib.PDFDocument.create(),start=performance.now();
   const img=await doc.embedPng(png),ms=performance.now()-start,sheet=await doc.addPage([1024,1024]);
   await sheet.drawImage(img,{x:0,y:0,width:1024,height:1024});
   const saved=await doc.save();await doc.dispose();
   const pdf=await pdfjsLib.getDocument({data:saved}).promise,pg=await pdf.getPage(1);
   const target=new OffscreenCanvas(1024,1024);
   await pg.render({canvasContext:target.getContext('2d'),viewport:pg.getViewport({scale:1})}).promise;
   const pixel=[...target.getContext('2d').getImageData(40,40,1,1).data];await pdf.destroy();return {pixel,ms};
  }
  ctx.fillStyle='#123456';ctx.fillRect(0,0,1024,1024);
  const original=await bytes(),first=await render(original),repeat=await render(original);
  ctx.fillStyle='#fedcba';ctx.fillRect(0,0,1024,1024);const changed=await render(await bytes());
  return {first,repeat,changed};
 });
 console.log('PNG_CACHE_BENCHMARK',JSON.stringify(result));
 expect(result.first.pixel).toEqual([18,52,86,255]);expect(result.repeat.pixel).toEqual(result.first.pixel);expect(result.changed.pixel).toEqual([254,220,186,255]);
});
