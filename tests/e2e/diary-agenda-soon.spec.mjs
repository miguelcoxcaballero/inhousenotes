import { test, expect } from '@playwright/test';

async function openProfileMenu(page) {
  await page.evaluate(() => {
    showDriveHome(); stopDriveHomeAutoRefresh();
    document.getElementById('drive-profile').classList.remove('hidden');
    toggleDriveProfileMenu(driveProfileMenu, driveProfileBtn, true);
  });
}

for (const width of [390, 1280]) {
  test(`Diary and Agenda keep their menu buttons with a Soon badge and only show "Soon" (${width}px)`, async ({ page }) => {
    await page.setViewportSize({ width, height: 844 });
    await page.goto('/?e2e=1');
    await page.evaluate(() => __IHN_TEST_API__.ready());
    for (const [id, label] of [['btn-drive-menu-diary', 'Diary'], ['btn-drive-menu-agenda', 'Agenda']]) {
      await openProfileMenu(page);
      const button = page.locator(`#${id}`);
      await expect(button).toBeVisible();
      await expect(button).toBeEnabled();
      await expect(button.locator('.soon-badge')).toHaveText('Soon');
      await expect(button).toContainText(label);
      // The badged row stays inside the menu, aligned with the rows below.
      const edges = await page.evaluate(() => ({
        agenda: document.getElementById('btn-drive-menu-agenda').getBoundingClientRect().right,
        signOut: document.getElementById('btn-drive-menu-logout').getBoundingClientRect().right
      }));
      expect(Math.abs(edges.agenda - edges.signOut)).toBeLessThan(1);
      await button.click();
      const toast = page.locator('#soon-toast');
      await expect(toast).toBeVisible();
      await expect(toast).toHaveText('Soon');
      await expect(toast).toHaveCSS('opacity', '1');
      await expect(page.locator('#loading-overlay')).not.toHaveClass(/visible/);
      await expect(page.locator('#drive-home')).toBeVisible();
    }
  });
}

test('stored agenda side panels are kept as opaque page data', async ({ page }) => {
  await page.goto('/?e2e=1');
  await page.evaluate(() => __IHN_TEST_API__.ready());
  const result = await page.evaluate(async () => {
    await __IHN_TEST_API__.resetLocalDocument(1, 'Agenda');
    const panel = { title: 'Week', dateKeys: ['2026-10-26'], html: '<p>my notes</p>' };
    state.pages[0].sidePanel = panel;
    return {
      stored: sanitizePageForStorage(state.pages[0]).sidePanel,
      meta: buildPageMeta(state.pages[0], 0).sidePanel,
      snapshot: buildTimelineSnapshotFromPages(state.pages)[0].sidePanel,
      field: getCollabPageFieldValue(state.pages[0], 'sidePanel')
    };
  });
  const panel = { title: 'Week', dateKeys: ['2026-10-26'], html: '<p>my notes</p>' };
  expect(result.stored).toEqual(panel);
  expect(result.meta).toEqual(panel);
  expect(result.snapshot).toEqual(panel);
  expect(result.field).toEqual({ title: 'Week', dateKeys: ['2026-10-26'] });
});
