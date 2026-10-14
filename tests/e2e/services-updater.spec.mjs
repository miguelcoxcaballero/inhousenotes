import { expect, test } from '@playwright/test';

test('B release opens the shared download manager and forwards native progress', async ({ page }) => {
  const release = { version: '1.0.19', versionCode: 20, apkUrl: 'https://github.com/miguelcoxcaballero/inhousenotes/releases/download/android-v1.0.19/inhouse-notes-release-v1.0.19.apk', apkSha256: 'b'.repeat(64), apkSizeBytes: 400, changelog: 'Updated Inhouse services updater.' };
  await page.addInitScript(() => {
    window.InhouseNative = {
      getAppVersion: () => '1.0.18', getAppVersionCode: () => 19,
      getUpdateState: () => '{}', saveUpdateState: () => {},
      installInhouseUpdate: (url, sha256) => {
        window.requestedUpdate = { url, sha256 };
        window.handleInhouseUpdateResult({ status: 'downloading', percent: 50, downloadedBytes: 200, totalBytes: 400 });
      },
    };
  });
  await page.route('https://raw.githubusercontent.com/**', async route => {
    expect(route.request().url()).toContain('/updates/B.json');
    await route.fulfill({ json: { schemaVersion: 1, channel: 'B', latest: release, releases: [release] } });
  });
  await page.goto('/?e2e=1&inhouse_app=1');
  await expect(page.getByRole('heading', { name: 'Update available', exact: true })).toBeVisible();
  await expect(page.locator('.isu-versions')).toHaveText('1.0.18 → 1.0.19');
  await page.getByRole('button', { name: 'Update', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.requestedUpdate)).toEqual({ url: release.apkUrl, sha256: release.apkSha256 });
  await expect(page.getByRole('progressbar', { name: 'Download' })).toHaveAttribute('value', '50');
  await expect(page.locator('.isu-progress-label')).toHaveText('Downloading… 50%');
});

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

// Keep Notes and its shared updater real; replace only the Android bridge and
// remote catalogs. Native state survives reload independently of updater state.
async function installResultFixture(page) {
  const release = {
    version: '1.0.15', versionCode: 16,
    apkUrl: 'https://github.com/miguelcoxcaballero/inhousenotes/releases/download/android-v1.0.15/inhouse-notes-release-v1.0.15.apk',
    apkSha256: 'c'.repeat(64), apkSizeBytes: 400, changelog: 'Show installation results.'
  };
  await page.addInitScript(release => {
    window.nativeInstallRequests = [];
    window.nativeInstallStateReads = [];
    window.InhouseNative = {
      getAppVersion: () => localStorage.getItem('e2e-native-version') || '1.0.14',
      getAppVersionCode: () => Number(localStorage.getItem('e2e-native-version-code') || 15),
      getUpdateState: () => localStorage.getItem('native-update-state') || '{}',
      saveUpdateState: value => localStorage.setItem('native-update-state', value),
      getInstallState: () => {
        const value = localStorage.getItem('e2e-native-install-state') || '{}';
        window.nativeInstallStateReads.push(JSON.parse(value));
        return value;
      },
      installInhouseUpdate: (url, sha256) => {
        window.nativeInstallRequests.push({ url, sha256 });
        localStorage.setItem('e2e-native-install-state', JSON.stringify({
          id: `notes-install-${window.nativeInstallRequests.length}`, status: 'installing',
          expectedVersionCode: release.versionCode,
          actualVersionCode: window.InhouseNative.getAppVersionCode()
        }));
        window.handleInhouseUpdateResult({ status: 'ready' });
      }
    };
  }, release);
  const fulfill = async route => {
    expect(route.request().url()).toContain('/updates/B.json');
    const catalog = { schemaVersion: 1, channel: 'B', latest: release, releases: [release] };
    await route.fulfill({ json: route.request().url().includes('api.github.com')
      ? { content: Buffer.from(JSON.stringify(catalog)).toString('base64') } : catalog });
  };
  await page.route('https://raw.githubusercontent.com/**', fulfill);
  await page.route('https://api.github.com/**', fulfill);
  return release;
}

test('returning from a cancelled Android installation enables retry via persisted native state', async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 360, height: 800 });
  const release = await installResultFixture(page);
  await page.goto('/?e2e=1&inhouse_app=1');
  const download = page.getByRole('link', { name: 'Download APK', exact: true });
  await expect(download).toBeVisible();
  await expect(download).toHaveAttribute('href', release.apkUrl.replace('/releases/download/', '/releases/tag/').replace(/\/[^/]+\.apk$/, ''));
  await page.getByRole('button', { name: 'Update', exact: true }).click();
  await expect(download).toBeVisible();
  await expect(page.getByRole('button', { name: 'Open installer', exact: true })).toBeEnabled();
  await expect(page.getByRole('heading', { name: 'Update installed', exact: true })).toHaveCount(0);

  // Android finishes while Notes is backgrounded; no JavaScript result callback
  // is delivered. Returning focus must read the new getInstallState contract.
  await page.evaluate(() => {
    const result = JSON.parse(localStorage.getItem('e2e-native-install-state'));
    localStorage.setItem('e2e-native-install-state', JSON.stringify({
      ...result, status: 'cancelled', message: 'Installation cancelled in Android.'
    }));
    window.dispatchEvent(new Event('focus'));
  });
  const retry = page.getByRole('button', { name: 'Try again', exact: true });
  await expect(retry).toBeEnabled();
  await expect(page.getByRole('status')).toContainText('Installation cancelled in Android.');
  await expect(page.getByRole('button', { name: 'Later', exact: true })).toBeEnabled();
  expect(await page.evaluate(() => window.nativeInstallStateReads.at(-1).status)).toBe('cancelled');
  const bounds = await page.locator('.isu-card').boundingBox();
  expect(bounds.x).toBeGreaterThanOrEqual(0);
  expect(bounds.x + bounds.width).toBeLessThanOrEqual(360);
  await expect(retry).toBeInViewport();
  const screenshot = testInfo.outputPath('installation-cancelled-360px.png');
  await page.screenshot({ path: screenshot });
  await testInfo.attach('installation-cancelled-360px', { path: screenshot, contentType: 'image/png' });

  await retry.click();
  expect(await page.evaluate(() => window.nativeInstallRequests)).toEqual([
    { url: release.apkUrl, sha256: release.apkSha256 },
    { url: release.apkUrl, sha256: release.apkSha256 }
  ]);
  await expect(page.getByRole('button', { name: 'Open installer', exact: true })).toBeEnabled();
});

test('Notes restart reports a persisted successful installation and Done dismisses it permanently', async ({ page }) => {
  const release = await installResultFixture(page);
  await page.goto('/?e2e=1&inhouse_app=1');
  await page.getByRole('button', { name: 'Update', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Open installer', exact: true })).toBeEnabled();
  await expect(page.getByRole('heading', { name: 'Update installed', exact: true })).toHaveCount(0);

  // The updated APK starts a fresh Notes runtime with the package manager's
  // current version and the previous native attempt's persisted outcome.
  await page.evaluate(release => {
    const result = JSON.parse(localStorage.getItem('e2e-native-install-state'));
    localStorage.setItem('e2e-native-version', release.version);
    localStorage.setItem('e2e-native-version-code', String(release.versionCode));
    localStorage.setItem('e2e-native-install-state', JSON.stringify({
      ...result, status: 'installed', actualVersionCode: release.versionCode, message: 'Update installed.'
    }));
  }, release);
  await page.reload();
  const dialog = page.getByRole('dialog', { name: 'Update installed', exact: true });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText(`Inhouse Notes ${release.version}`);
  expect(await page.evaluate(() => window.nativeInstallStateReads.some(result => result.status === 'installed'))).toBe(true);
  await dialog.getByRole('button', { name: 'Done', exact: true }).click();
  await expect(page.locator('#inhouse-services-update')).toHaveCount(0);

  await page.reload();
  await page.waitForFunction(() => !!window.InhouseUpdater);
  await expect(page.getByRole('heading', { name: 'Update installed', exact: true })).toHaveCount(0);
  await page.locator('[data-inhouse-update-settings]').first().dispatchEvent('click');
  await expect(page.getByRole('dialog', { name: 'Updates', exact: true })).toContainText(`Inhouse Notes ${release.version}`);
});


test('the installed 1.0.16 shell escapes its broken installer through the versioned GitHub page', async ({ page }) => {
  let nativeCalls = 0;
  await page.exposeFunction('recordNativeInstall', () => { nativeCalls++; });
  await page.addInitScript(() => {
    window.InhouseNative = {
      getAppVersion: () => '1.0.16', getAppVersionCode: () => 17,
      getUpdateState: () => '{}', saveUpdateState: () => {},
      getInstallState: () => '{}',
      installInhouseUpdate: () => window.recordNativeInstall()
    };
  });
  const release = { version: '1.0.18', versionCode: 19,
    apkUrl: 'https://github.com/miguelcoxcaballero/inhousenotes/releases/download/android-v1.0.18/inhouse-notes-release-v1.0.18.apk',
    apkSha256: 'd'.repeat(64), apkSizeBytes: 3501180, changelog: 'Reliable installer and APK recovery.' };
  await page.route('https://raw.githubusercontent.com/**', route => route.fulfill({ json: { schemaVersion: 1, channel: 'B', latest: release, releases: [release] } }));
  const releasePage = 'https://github.com/miguelcoxcaballero/inhousenotes/releases/tag/android-v1.0.18';
  await page.route(releasePage, route => route.fulfill({ contentType: 'text/html', body: '<h1>Release assets</h1>' }));
  await page.goto('/?e2e=1&inhouse_app=1');
  await page.getByRole('button', { name: 'Update', exact: true }).click();
  await expect(page).toHaveURL(releasePage);
  expect(nativeCalls).toBe(0);
});
