import {test,expect} from '@playwright/test';
test.beforeEach(async({page})=>{await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());await page.evaluate(()=>__IHN_TEST_API__.seedDensePageForTest(400,80));await page.waitForFunction(()=>!__IHN_TEST_API__.densePaintStatusForTest().pending);});
test('zoom resize retains ink immediately and throughout a delayed repaint',async({page})=>{
 const result=await page.evaluate(async()=>{
  const canvas=document.querySelector('canvas.page-canvas[data-page="0"]');
  function ink(){const scale=canvas.width/getPageDimensions(0).width;const ctx=canvas.getContext('2d');const data=ctx.getImageData(Math.floor(25*scale),Math.floor(24*scale),Math.ceil(25*scale),Math.ceil(14*scale)).data;let count=0;for(let i=0;i<data.length;i+=4)if(data[i]<70&&data[i+1]<110&&data[i+2]<160&&data[i+3]>100)count++;return count;}
  const before=ink();const size=getPageDimensions(0);resizePageCanvas(canvas,size.width,size.height,2);const resized=ink();
  state.activePointers.set(999,{pointerType:'touch'});redrawPage(0);await new Promise(r=>setTimeout(r,80));const waiting=ink();state.activePointers.delete(999);return {before,resized,waiting};
 });
 expect(result.before).toBeGreaterThan(10);expect(result.resized).toBeGreaterThan(10);expect(result.waiting).toBeGreaterThan(10);
 await page.waitForFunction(()=>!__IHN_TEST_API__.densePaintStatusForTest().pending);
});
test('duplicate repaint requests reuse work but new ink and image readiness invalidate it',async({page})=>{
 const result=await page.evaluate(()=>{
  const canvas=document.querySelector('canvas.page-canvas[data-page="0"]');redrawPage(0);const first=pendingPageRepaints.get(canvas);
  for(let i=0;i<30;i++)redrawPage(0);
  const same=pendingPageRepaints.get(canvas)===first&&!first.cancelled;
  redrawPage(0,{assetsReady:true});const image=pendingPageRepaints.get(canvas);
  markPageDirty(0,'full');redrawPage(0);const next=pendingPageRepaints.get(canvas);
  return {same,imageInvalidated:first.cancelled&&image!==first,inkInvalidated:image.cancelled&&next!==image};
 });expect(result).toEqual({same:true,imageInvalidated:true,inkInvalidated:true});
 await page.waitForFunction(()=>!__IHN_TEST_API__.densePaintStatusForTest().pending);
});
