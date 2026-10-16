import {test,expect} from '@playwright/test';

test('rotating Drive thumbnail links preserve the card, image and decoded pixels',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(()=>{
  const canvas=document.createElement('canvas');canvas.width=30;canvas.height=40;
  const ctx=canvas.getContext('2d');ctx.fillStyle='#abc';ctx.fillRect(0,0,30,40);
  window.previewFile={id:'steady-preview',name:'Steady.pdf',mimeType:'application/pdf',localPreview:canvas.toDataURL(),modifiedTime:'2026-10-16T10:00:00Z'};
  showDriveHome();renderDriveRecents([previewFile]);
 });
 await expect(page.locator('#drive-recents .drive-card-preview')).toHaveClass(/is-loaded/);
 const result=await page.evaluate(async()=>{
  const card=document.querySelector('#drive-recents [data-drive-item-id]'),img=card.querySelector('img'),src=img.src;
  let mutations=0;const observer=new MutationObserver(records=>{mutations+=records.filter(r=>r.type==='childList'||r.attributeName==='src'||r.attributeName==='class').length;});
  observer.observe(card,{subtree:true,childList:true,attributes:true});
  for(let i=0;i<50;i++)renderDriveRecents([{...previewFile,thumbnailLink:`https://lh3.googleusercontent.com/rotating-token-${i}`}]);
  await Promise.resolve();observer.disconnect();
  return {sameCard:card===document.querySelector('#drive-recents [data-drive-item-id]'),sameImage:img===card.querySelector('img'),samePixels:img.src===src,mutations};
 });
 expect(result).toEqual({sameCard:true,sameImage:true,samePixels:true,mutations:0});
});

test('a changed preview decodes before replacing pixels and a failed refresh preserves the good image',async({page})=>{
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
 await page.evaluate(()=>{
  const canvas=document.createElement('canvas');canvas.width=30;canvas.height=40;
  const ctx=canvas.getContext('2d');ctx.fillStyle='#abc';ctx.fillRect(0,0,30,40);
  window.previewFile={id:'changing-preview',name:'Changing.pdf',mimeType:'application/pdf',localPreview:canvas.toDataURL(),modifiedTime:'2026-10-16T10:00:00Z'};
  showDriveHome();renderDriveRecents([previewFile]);
 });
 await expect(page.locator('#drive-recents .drive-card-preview')).toHaveClass(/is-loaded/);
 const result=await page.evaluate(async()=>{
  const card=document.querySelector('#drive-recents [data-drive-item-id]'),img=card.querySelector('img'),before=img.src;
  const canvas=document.createElement('canvas');canvas.width=30;canvas.height=40;canvas.getContext('2d').fillRect(0,0,30,40);
  const preview=card.querySelector('.drive-card-preview');
  const change=preview.refreshFile({...previewFile,modifiedTime:'2026-10-16T11:00:00Z',localPreview:canvas.toDataURL()});
  const retained=img.src===before&&preview.classList.contains('is-loaded')&&!preview.classList.contains('is-loading');
  await change;const changed=img.src!==before;
  const good=img.src;await preview.refreshFile({...previewFile,modifiedTime:'2026-10-16T12:00:00Z',localPreview:'data:image/png;base64,invalid'});
  return {retained,changed,sameImage:img===card.querySelector('img'),failedKept:img.src===good,loaded:preview.classList.contains('is-loaded')};
 });
 expect(result).toEqual({retained:true,changed:true,sameImage:true,failedKept:true,loaded:true});
});
