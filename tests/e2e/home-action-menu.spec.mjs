import { test, expect } from '@playwright/test';

for (const folder of [false, true]) {
  test(`${folder ? 'folder' : 'document'} actions only appear in the three-dot menu with icons`, async ({ page }) => {
    await page.goto('/?e2e=1');
    await page.evaluate(() => __IHN_TEST_API__.ready());
    await page.evaluate(folder => {
      showDriveHome(); stopDriveHomeAutoRefresh();
      const item = { id: 'menu-test', name: 'Menu test', mimeType: folder ? 'application/vnd.google-apps.folder' : 'application/pdf' };
      setDriveHomeTab('files');
      renderDriveFolderContents(folder ? [item] : [], folder ? [] : [item]);
    }, folder);
    const card = page.locator(folder ? '#drive-folders .drive-folder-card' : '#drive-folder-files .drive-card');
    await card.hover();
    await expect(page.locator('.drive-card-tools')).toHaveCount(0);
    await expect(page.locator('.drive-card-dropdown:not(.hidden)')).toHaveCount(0);
    await expect(card.locator('.drive-card-dropdown [data-action]')).toHaveCount(0);
    await expect(card.locator('.drive-card-dropdown svg')).toHaveCount(0);
    await card.locator('.drive-card-more').click();
    const menu = page.locator('.drive-card-dropdown:not(.hidden)');
    await expect(menu).toBeVisible();
    await expect(menu.locator('button')).toHaveCount(4);
    await expect(menu.locator('button svg')).toHaveCount(4);
    await expect(menu.locator(`[data-action="${folder ? 'color' : 'offline'}"]`)).toBeVisible();
    await expect(page.locator('#drive-home')).toBeVisible();
    await menu.locator('[data-action="trash"]').click();
    await expect(page.locator('#modal-trash-file')).toHaveClass(/visible/);
  });
}
