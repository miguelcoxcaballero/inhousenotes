import {test,expect} from '@playwright/test';
test.beforeEach(async({page})=>{
 await page.route('https://accounts.google.com/**',r=>r.fulfill({body:''}));
 await page.goto('/?e2e=1');await page.evaluate(()=>__IHN_TEST_API__.ready());
});
test('Photos-style thumb drags the editor and supports keyboard navigation',async({page})=>{
 await page.evaluate(async()=>{await __IHN_TEST_API__.resetLocalDocument(8,'Scroll');await __IHN_TEST_API__.showEditorForTest();updateTransform();});
 const thumb=page.locator('#document-fast-scroll');await expect(thumb).toBeVisible();
 await thumb.focus();await page.keyboard.press('End');
 await expect(thumb).toHaveAttribute('aria-valuenow','100');
 expect(await page.evaluate(()=>state.panY)).toBeLessThan(-500);
 await page.keyboard.press('Home');await expect(thumb).toHaveAttribute('aria-valuenow','0');
 const box=await thumb.boundingBox();
 await page.mouse.move(box.x+box.width/2,box.y+box.height/2);await page.mouse.down();
 await page.mouse.move(box.x+box.width/2,box.y+box.height/2+180,{steps:8});await page.mouse.up();
 await expect.poll(async()=>Number(await thumb.getAttribute('aria-valuenow'))).toBeGreaterThan(10);
 await expect(thumb).not.toHaveClass(/is-dragging/);
});
test('thumb scrolls Home and folder containers and hides when content fits',async({page})=>{
 await page.evaluate(()=>{
  showDriveHome();driveMain.style.height='300px';driveMain.style.flex='none';
  const spacer=document.createElement('div');spacer.id='scroll-fixture';spacer.style.height='6000px';driveMain.append(spacer);
 });
 const thumb=page.locator('#document-fast-scroll');await expect(thumb).toBeVisible();
 await thumb.focus();await page.keyboard.press('End');
 expect(await page.evaluate(()=>driveMain.scrollTop)).toBeGreaterThan(1000);
 await page.evaluate(()=>{
  driveFolderViewEl.classList.remove('hidden');driveFolderViewScrollEl.style.height='300px';driveFolderViewScrollEl.style.flex='none';
  const spacer=document.createElement('div');spacer.style.height='5000px';driveFolderViewScrollEl.append(spacer);
 });
 await expect(thumb).toHaveAttribute('aria-controls','drive-folder-view-scroll');
 await thumb.focus();await page.keyboard.press('End');
 expect(await page.evaluate(()=>driveFolderViewScrollEl.scrollTop)).toBeGreaterThan(1000);
 await page.evaluate(()=>{driveFolderViewScrollEl.replaceChildren();});
 await expect(thumb).toBeHidden();
});
