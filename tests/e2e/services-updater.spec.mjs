import { expect, test } from '@playwright/test';

test('Android settings use the common optional popup and preserve channel/reminder', async ({ page }) => {
  const releases = [{ version: '1.0.12', versionCode: 13, apkUrl: 'https://github.com/miguelcoxcaballero/inhousenotes/releases/download/test/app.apk', apkSha256: 'a'.repeat(64), apkSizeBytes: 400, changelog: 'Aviso mejorado.' }];
  await page.addInitScript(() => {
    window.InhouseNative = {
      getAppVersion: () => '1.0.11', getAppVersionCode: () => 12,
      getUpdateState: () => localStorage.getItem('native-update-state') || '{"channel":"A","snooze":{}}',
      saveUpdateState: value => localStorage.setItem('native-update-state', value),
    };
  });
  const fulfill = async route => {
    const channel = route.request().url().includes('B.json') ? 'B' : 'A';
    const catalog = { schemaVersion: 1, channel, latest: channel === 'A' ? releases[0] : null, releases: channel === 'A' ? releases : [] };
    const json = JSON.stringify(catalog);
    await route.fulfill({ contentType: 'application/json', body: route.request().url().includes('api.github.com') ? JSON.stringify({ content: Buffer.from(json).toString('base64') }) : json });
  };
  await page.route('https://raw.githubusercontent.com/**', fulfill);
  await page.route('https://api.github.com/**', fulfill);
  await page.goto('/?e2e=1&inhouse_app=1');
  await page.waitForFunction(() => !!window.InhouseUpdater);
  expect(await page.evaluate(() => InhouseUpdater.getState().channel)).toBe('B');
  await expect(page.locator('#inhouse-services-update')).toHaveCount(0);
  await page.locator('[data-inhouse-update-settings]').first().dispatchEvent('click');
  await page.getByRole('button', { name: 'Version A', exact: true }).click();
  await page.getByRole('button', { name: 'I understand', exact: true }).click();
  await page.locator('input[type=range]').focus();await page.keyboard.press('End');await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Later', exact: true })).toBeVisible();
  await expect(page.locator('.isu-versions')).toHaveText('1.0.11 → 1.0.12');
  await page.getByRole('button', { name: 'Later', exact: true }).click();
  const deadline = await page.evaluate(() => JSON.parse(localStorage.getItem('native-update-state')).snooze.A);
  expect(deadline - Date.now()).toBeGreaterThan(13 * 86400000);
  await page.reload();
  await page.waitForFunction(() => !!window.InhouseUpdater);
  await expect(page.locator('#inhouse-services-update')).toHaveCount(0);
  await expect(page.locator('[data-inhouse-update-settings]')).toHaveCount(2);
  await page.locator('[data-inhouse-update-settings]').first().dispatchEvent('click');
  await page.getByRole('button', { name: 'Version B', exact: true }).click();
  await expect(page.getByText('No releases on version B.')).toBeVisible();
  await page.reload();
  await page.waitForFunction(() => !!window.InhouseUpdater);
  await page.locator('[data-inhouse-update-settings]').last().dispatchEvent('click');
  await expect(page.getByRole('button', { name: 'Version B', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('button', { name: 'Version A', exact: true }).click();
  await page.getByRole('button', { name: 'I understand', exact: true }).click();
  await page.locator('input[type=range]').focus();await page.keyboard.press('End');await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Update', exact: true })).toBeVisible();
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('native-update-state')).snooze.A)).toBe(deadline);
});
