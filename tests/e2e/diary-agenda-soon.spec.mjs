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

test('Pages-panel Calendar stays as a Soon button only for agenda documents', async ({ page }) => {
  await page.goto('/?e2e=1');
  await page.evaluate(() => __IHN_TEST_API__.ready());
  await page.evaluate(async () => {
    await __IHN_TEST_API__.resetLocalDocument(2, 'Plain');
    await __IHN_TEST_API__.showEditorForTest();
    openPagesPanel();
  });
  const calendar = page.locator('#btn-open-calendar');
  await expect(calendar).toBeHidden();
  await page.evaluate(() => {
    closePagesPanel();
    state.pages[1].templateKind = 'agenda';
    openPagesPanel();
  });
  await expect(calendar).toBeVisible();
  await expect(calendar.locator('.soon-badge')).toHaveText('Soon');
  await calendar.click();
  await expect(page.locator('#soon-toast')).toHaveText('Soon');
  await expect(page.locator('#soon-toast')).toHaveCount(0);
});
