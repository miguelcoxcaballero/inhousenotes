import { expect, test } from '@playwright/test';

async function openEditor(page) {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(async () => {
    await window.__IHN_TEST_API__.resetLocalDocument(2, 'Pages panel test');
    await window.__IHN_TEST_API__.showEditorForTest();
  });
}

test('hidden page edits defer thumbnail work until the panel opens', async ({ page }) => {
  await openEditor(page);
  await page.locator('#btn-edit-pages').click();
  await expect(page.locator('#pages-list .page-item')).toHaveCount(2);
  await page.locator('#panel-close').click();

  const result = await page.evaluate(async () => {
    const list = document.getElementById('pages-list');
    let mutations = 0;
    const observer = new MutationObserver(records => { mutations += records.length; });
    observer.observe(list, { childList: true, subtree: true });
    const insertion = await window.__IHN_TEST_API__.addScannedPagesForTest(1);
    mutations += observer.takeRecords().length;
    observer.disconnect();
    return { inserted: insertion.inserted, pageCount: insertion.snapshot.pages.length, mutations };
  });
  expect(result).toEqual({ inserted: true, pageCount: 3, mutations: 0 });

  await page.locator('#btn-edit-pages').click();
  await expect(page.locator('#pages-list .page-item')).toHaveCount(3);
  await expect(page.locator('#thumb-2')).toBeVisible();
});

test('reopening the pages panel starts only one gesture per drag', async ({ page }) => {
  await openEditor(page);
  for (let index = 0; index < 4; index++) {
    await page.locator('#btn-edit-pages').click();
    if (index < 3) await page.locator('#panel-close').click();
  }

  const result = await page.evaluate(() => {
    const original = document.addEventListener;
    let moves = 0;
    document.addEventListener = function(type, ...args) {
      if (type === 'pointermove') moves++;
      return original.call(this, type, ...args);
    };
    try {
      const handle = document.querySelector('.page-drag-handle');
      handle.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerId: 7 }));
      const dragging = document.querySelectorAll('.page-item.is-dragging').length;
      document.dispatchEvent(new PointerEvent('pointercancel', { pointerId: 7 }));
      return { moves, dragging, remaining: document.querySelectorAll('.page-item.is-dragging').length };
    } finally {
      document.addEventListener = original;
    }
  });
  expect(result).toEqual({ moves: 1, dragging: 1, remaining: 0 });
});
