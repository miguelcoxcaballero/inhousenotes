import { test, expect } from '@playwright/test';

async function setup(page) {
    await page.goto('/?e2e=1');
    await page.evaluate(async () => {
        await __IHN_TEST_API__.ready();
        await __IHN_TEST_API__.resetLocalDocument(12, 'Reorder');
        await __IHN_TEST_API__.showEditorForTest();
        state.pages.forEach((p, i) => {
            p.strokes = [{ id: `ink-${i}`, tool: 'pen', color: '#000000', width: 2, points: [{ x: i + 10, y: 20 }, { x: i + 15, y: 30 }] }];
            markPageDirty(i, 'full');
        });
        await __IHN_TEST_API__.checkpointBeforeLeaving();
    });
    await page.locator('#btn-edit-pages').click();
    await expect(page.locator('#pages-list .page-item')).toHaveCount(12);
}

test('stationary pointer scrolls down continuously and drops into the visible position without losing ink', async ({ page }) => {
    await setup(page);
    const ids = await page.evaluate(() => state.pages.map(p => p.pageId));
    const handle = page.locator('.page-drag-handle').first();
    await handle.hover();
    const box = await handle.boundingBox();
    const list = await page.locator('#pages-list').boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(list.x + list.width / 2, list.y + list.height - 2);
    await expect(page.locator('.page-reorder-preview')).toHaveCount(1);
    await expect.poll(() => page.locator('#pages-list').evaluate(el => el.scrollTop)).toBeGreaterThan(250);
    const firstScroll = await page.locator('#pages-list').evaluate(el => el.scrollTop);
    await expect.poll(() => page.locator('#pages-list').evaluate(el => el.scrollTop)).toBeGreaterThan(firstScroll + 200);
    await page.mouse.up();
    await expect(page.locator('.page-reorder-preview')).toHaveCount(0);
    await expect.poll(() => page.evaluate(id => state.pages.findIndex(p => p.pageId === id), ids[0])).toBeGreaterThan(1);
    await page.waitForFunction(() => !localPageStructureMutationToken);
    const result = await page.evaluate(async () => {
        const ink = [];
        for (let i = 0; i < state.pages.length; i++) {
            await ensurePageDataLoaded(i);
            const saved = await loadPageFromIndexedDb(i);
            ink.push([state.pages[i].pageId, saved?.strokes[0]?.id]);
        }
        return { ink, scroll: document.getElementById('pages-list').scrollTop };
    });
    for (const [id, ink] of result.ink) expect(ink).toBe(`ink-${ids.indexOf(id)}`);
    expect(result.scroll).toBeGreaterThan(200);
});

test('dragging a lower page upwards scrolls without more pointer moves and Escape cancels', async ({ page }) => {
    await setup(page);
    const before = await page.evaluate(() => {
        const list = document.getElementById('pages-list');
        list.scrollTop = list.scrollHeight;
        return state.pages.map(p => p.pageId);
    });
    await page.locator('.page-drag-handle').last().hover();
    const box = await page.locator('.page-drag-handle').last().boundingBox();
    const list = await page.locator('#pages-list').boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    const start = await page.locator('#pages-list').evaluate(el => el.scrollTop);
    await page.mouse.move(list.x + list.width / 2, list.y + 2);
    await expect.poll(() => page.locator('#pages-list').evaluate(el => el.scrollTop)).toBeLessThan(start - 250);
    await page.keyboard.press('Escape');
    await page.mouse.up();
    expect(await page.evaluate(() => state.pages.map(p => p.pageId))).toEqual(before);
    await expect(page.locator('.page-reorder-preview')).toHaveCount(0);
    expect(await page.evaluate(() => pageStructureDragInProgress)).toBe(false);
});

test('touch drag ignores other pointers and closing the panel stops scrolling and releases the gesture', async ({ page }) => {
    await page.setViewportSize({ width: 412, height: 850 });
    await setup(page);
    const start = await page.evaluate(() => {
        const handle = document.querySelector('.page-drag-handle');
        const b = handle.getBoundingClientRect(), list = document.getElementById('pages-list').getBoundingClientRect();
        handle.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'touch', pointerId: 5, isPrimary: true, clientX: b.x + 8, clientY: b.y + 8 }));
        document.dispatchEvent(new PointerEvent('pointermove', { bubbles: true, pointerType: 'touch', pointerId: 5, clientX: list.x + 40, clientY: list.bottom - 2 }));
        document.dispatchEvent(new PointerEvent('pointerup', { pointerId: 6 }));
        return pageStructureDragInProgress;
    });
    expect(start).toBe(true);
    await expect.poll(() => page.locator('#pages-list').evaluate(el => el.scrollTop)).toBeGreaterThan(200);
    await page.evaluate(() => closePagesPanel());
    expect(await page.evaluate(() => pageStructureDragInProgress)).toBe(false);
    await expect(page.locator('.page-reorder-preview')).toHaveCount(0);
    const scroll = await page.locator('#pages-list').evaluate(el => el.scrollTop);
    await page.waitForTimeout(120);
    expect(await page.locator('#pages-list').evaluate(el => el.scrollTop)).toBe(scroll);
});
