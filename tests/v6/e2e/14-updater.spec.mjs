// 14-updater (TESTING §2): in app mode the shared updater is created exactly as 5.18.0 did — offer
// `1.0.28 → 1.0.29`, Update goes to `InhouseNative.installInhouseUpdate(url, sha)`, native progress is shown — and
// its settings entry appears in both profile menus. The boot script's release watcher reloads once into a newer
// published version and never while the editor is open. (The upgrade from a 5.18.0 service worker is 14b.)
import { expect, test } from '@playwright/test';

test.use({ serviceWorkers: 'block' });

const REPO = 'miguelcoxcaballero/inhousenotes';
const release = {
  version: '1.0.29', versionCode: 30, apkUrl: `https://github.com/${REPO}/releases/download/android-v1.0.29/inhouse-notes-release-v1.0.29.apk`,
  apkSha256: 'b'.repeat(64), apkSizeBytes: 400, changelog: 'Inhouse Notes 6.'
};

async function openApp(page, { catalog = { schemaVersion: 1, channel: 'B', latest: release, releases: [release] }, native = true } = {}) {
  const requested = [];
  await page.route('https://accounts.google.com/**', route => route.fulfill({ contentType: 'text/javascript', body: '' }));
  await page.route('https://www.googleapis.com/**', route => route.fulfill({ status: 401, headers: { 'access-control-allow-origin': '*' }, body: '{}' }));
  await page.route('https://raw.githubusercontent.com/**', route => { requested.push(route.request().url()); return route.fulfill({ json: catalog }); });
  await page.route('https://api.github.com/**', route => route.fulfill({ status: 404, body: '' }));
  if (native) {
    await page.addInitScript(() => {
      window.InhouseNative = {
        getAppVersion: () => '1.0.28', getAppVersionCode: () => 29, getUpdateState: () => '{}', saveUpdateState: () => {},
        installInhouseUpdate: (url, sha256) => {
          window.requestedUpdate = { url, sha256 };
          window.handleInhouseUpdateResult({ status: 'downloading', percent: 50, downloadedBytes: 200, totalBytes: 400 });
        }
      };
    });
  }
  await page.addInitScript(() => localStorage.setItem('drive-remember-v1', '1'));
  await page.goto(`/?e2e=1${native ? '&inhouse_app=1' : ''}`);
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  return requested;
}

test('app mode: offer 1.0.28 → 1.0.29, Update stays in the native bridge, progress 50 %', async ({ page }) => {
  const requested = await openApp(page);
  await expect(page.getByRole('heading', { name: 'Update available', exact: true })).toBeVisible();
  await expect(page.locator('.isu-versions')).toHaveText('1.0.28 → 1.0.29');
  expect(requested[0]).toContain(`/${REPO}/main/updates/B.json`);
  let navigations = 0;
  page.on('framenavigated', frame => { if (frame === page.mainFrame()) navigations += 1; });
  await page.getByRole('button', { name: 'Update', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.requestedUpdate)).toEqual({ url: release.apkUrl, sha256: release.apkSha256 });
  await expect(page.getByRole('progressbar', { name: 'Download' })).toHaveAttribute('value', '50');
  await expect(page.locator('.isu-progress-label')).toHaveText('Downloading… 50%');
  expect(navigations).toBe(0);
  expect(await page.evaluate(() => [typeof window.InhouseUpdater?.settings, typeof window.handleInhouseUpdateResult])).toEqual(['function', 'function']);
  expect(await page.evaluate(() => window.__IHN_TEST_API__.cmd('updater.state'))).toEqual({ created: true });
});

test('app mode: both profile menus get the updater settings entry', async ({ page }) => {
  await openApp(page, { catalog: { schemaVersion: 1, channel: 'B', latest: null, releases: [] } });
  const entries = page.locator('[data-inhouse-update-settings]');
  expect(await entries.evaluateAll(buttons => buttons.map(button => [button.hidden, button.textContent]))).toEqual([[false, 'Actualizaciones · A/B'], [false, 'Actualizaciones · A/B']]);
  await page.click('#btn-drive-profile');
  await page.locator('#drive-profile-menu [data-inhouse-update-settings]').click();
  await expect(page.getByRole('heading', { name: 'Updates', exact: true })).toBeVisible();
  await expect(page.locator('#inhouse-services-update')).toContainText('Inhouse Notes 1.0.28');
  await expect(page.getByRole('button', { name: /Version B/ })).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('button', { name: 'Done', exact: true }).click();
  await expect(page.locator('#inhouse-services-update')).toHaveCount(0);
  // The editor menu's entry opens the same dialog.
  await page.evaluate(() => window.__IHN_TEST_API__.cmd('shell.show', { view: 'editor' }));
  await page.click('#btn-drive-profile-editor');
  await page.locator('#drive-profile-menu-editor [data-inhouse-update-settings]').click();
  await expect(page.getByRole('heading', { name: 'Updates', exact: true })).toBeVisible();
});

test('web: no updater, no settings entry, the APP button stays', async ({ page }) => {
  const requested = await openApp(page, { native: false });
  expect(await page.locator('[data-inhouse-update-settings]').evaluateAll(buttons => buttons.map(button => button.hidden))).toEqual([true, true]);
  expect(await page.evaluate(() => [typeof window.InhouseUpdater, window.__IHN_TEST_API__.cmd('updater.state').created])).toEqual(['undefined', false]);
  await expect(page.locator('#btn-download-app')).toBeVisible();
  await expect(page.locator('#btn-drive-fullscreen')).toBeVisible();
  expect(requested).toEqual([]);
});

// The boot script compares the running release with the one published on the live site (read from
// service-worker.js without any cache) and reloads once into a newer one.
const publish = (page, version) => page.route('**/service-worker.js?release-check=*', route => route.fulfill({
  status: 200, contentType: 'text/javascript', body: `'use strict';\nconst VERSION='${version}';\n`
}));

test('release watcher: a newer published version reloads exactly once', async ({ page }) => {
  await page.route('https://accounts.google.com/**', route => route.fulfill({ contentType: 'text/javascript', body: '' }));
  await publish(page, '999.0.0');
  let navigations = 0;
  page.on('framenavigated', frame => { if (frame === page.mainFrame()) navigations += 1; });
  await page.goto('/?e2e=1');
  await expect.poll(() => navigations, { timeout: 10000 }).toBe(2);
  await expect.poll(() => page.evaluate(() => sessionStorage.getItem('ihn-release-reload'))).toBe('999.0.0');
  await page.waitForTimeout(2500);
  expect(navigations).toBe(2);
  await expect(page.locator('[data-app-version]').first()).toHaveText('v6.0.0');
});

test('release watcher: the running version and an open editor never reload', async ({ page }) => {
  await page.route('https://accounts.google.com/**', route => route.fulfill({ contentType: 'text/javascript', body: '' }));
  await publish(page, '6.0.0');
  let navigations = 0;
  page.on('framenavigated', frame => { if (frame === page.mainFrame()) navigations += 1; });
  await page.goto('/?e2e=1');
  await page.waitForTimeout(3000);
  expect(navigations).toBe(1);
  expect(await page.evaluate(() => sessionStorage.getItem('ihn-release-reload'))).toBeNull();

  // With a document open the user is never reloaded.
  await page.unroute('**/service-worker.js?release-check=*');
  await publish(page, '999.0.0');
  await page.addInitScript(() => {
    const show = () => window.__IHN_TEST_API__ ? window.__IHN_TEST_API__.ready().then(() => window.__IHN_TEST_API__.cmd('shell.show', { view: 'editor' })) : setTimeout(show, 10);
    show();
  });
  navigations = 0;
  await page.goto('/?e2e=1');
  await expect(page.locator('#editor-view')).toBeVisible();
  await page.waitForTimeout(3000);
  expect(navigations).toBe(1);
});
