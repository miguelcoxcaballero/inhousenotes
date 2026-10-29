import { test, expect } from '@playwright/test';

test('Diary and Agenda keep their menu buttons with a Soon badge and only show "Soon"', async ({ page }) => {
  await page.goto('/?e2e=1');
  await page.evaluate(() => __IHN_TEST_API__.ready());
  await page.evaluate(() => {
    showDriveHome(); stopDriveHomeAutoRefresh();
    document.getElementById('drive-profile').classList.remove('hidden');
    toggleDriveProfileMenu(driveProfileMenu, driveProfileBtn, true);
  });
  for (const [id, label] of [['btn-drive-menu-diary', 'Diary'], ['btn-drive-menu-agenda', 'Agenda']]) {
    const button = page.locator(`#${id}`);
    await expect(button).toBeVisible();
    await expect(button).toBeEnabled();
    await expect(button.locator('.soon-badge')).toHaveText('Soon');
    await expect(button).toContainText(label);
    await page.evaluate(() => showStatus('Saved', { preserveState: true }));
    await button.click();
    await expect(page.locator('#save-indicator')).toHaveAttribute('aria-label', 'Soon');
    await expect(page.locator('#loading-overlay')).not.toHaveClass(/visible/);
    await expect(page.locator('#drive-home')).toBeVisible();
  }
});
