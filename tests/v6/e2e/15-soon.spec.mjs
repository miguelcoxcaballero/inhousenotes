// 15-soon (TESTING §2, BEHAVIOUR-CHANGES S-2): Diary and Agenda keep their menu buttons with a Soon badge and only
// show the "Soon" toast for 1.8 s — no camera permission request, no network request, no scanner asset loaded.
// (Scan in the Manage-Pages panel is 15b, W5.1.)
import { expect, test } from '@playwright/test';

test.use({ serviceWorkers: 'block' });

// A signed-in session: a stored token (5.18.0 `drive-token-v1`) and a Drive that only knows the profile.
async function signedIn(page) {
  await page.route('https://accounts.google.com/**', route => route.fulfill({ contentType: 'text/javascript', body: '' }));
  await page.route('https://www.googleapis.com/**', route => {
    const url = new URL(route.request().url()), headers = { 'access-control-allow-origin': '*', 'access-control-allow-headers': '*' };
    if (route.request().method() === 'OPTIONS') return route.fulfill({ status: 204, headers });
    if (url.pathname === '/drive/v3/about') return route.fulfill({ headers, json: { kind: 'drive#about', user: { displayName: 'Ana Test', emailAddress: 'ana@example.com', photoLink: '' } } });
    return route.fulfill({ status: 404, headers, json: { error: { code: 404, message: 'not mocked' } } });
  });
  await page.addInitScript(() => {
    localStorage.setItem('drive-remember-v1', '1');
    localStorage.setItem('drive-token-v1', JSON.stringify({ token: 'stored-token', expiry: Date.now() + 3_600_000, grantedScopes: 'https://www.googleapis.com/auth/drive' }));
  });
}

for (const width of [390, 1280]) {
  test(`Diary and Agenda answer "Soon" and nothing else (${width}px)`, async ({ page }) => {
    await page.setViewportSize({ width, height: 844 });
    const requests = [];
    // boot.js's published-release watcher runs on its own clock; it is not caused by the tap.
    page.on('request', request => { if (!request.url().includes('release-check=')) requests.push(request.url()); });
    await signedIn(page);
    await page.addInitScript(() => {
      window.__camera = 0;
      const media = navigator.mediaDevices;
      if (media) media.getUserMedia = () => { window.__camera += 1; return Promise.reject(new Error('blocked by test')); };
      const query = navigator.permissions?.query?.bind(navigator.permissions);
      if (query) navigator.permissions.query = descriptor => { if (descriptor?.name === 'camera') window.__camera += 1; return query(descriptor); };
    });
    await page.goto('/?e2e=1');
    await page.waitForFunction(() => window.__IHN_TEST_API__);
    await page.evaluate(() => window.__IHN_TEST_API__.ready());
    await expect(page.locator('#drive-home')).toBeVisible();
    await page.waitForTimeout(300); // boot requests (connectivity probe) are over

    for (const [id, label] of [['btn-drive-menu-diary', 'Diary'], ['btn-drive-menu-agenda', 'Agenda']]) {
      await page.click('#btn-drive-profile');
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

      const before = requests.length;
      const shownAt = Date.now();
      await button.click();
      const toast = page.locator('#soon-toast');
      await expect(toast).toHaveClass('visible');
      await expect(toast).toHaveText('Soon');
      await expect(toast).toHaveAttribute('role', 'status');
      await expect(toast).toHaveCSS('opacity', '1');
      await expect(page.locator('#drive-profile-menu')).toBeHidden();
      await expect(page.locator('#loading-overlay')).not.toHaveClass(/visible/);
      await expect(page.locator('#drive-home')).toBeVisible();
      // Still there after 1.2 s, gone after 1.8 s.
      await page.waitForTimeout(Math.max(0, 1200 - (Date.now() - shownAt)));
      await expect(toast).toHaveClass('visible');
      await expect(toast).not.toHaveClass('visible', { timeout: 1500 });
      expect(Date.now() - shownAt).toBeGreaterThanOrEqual(1750);
      expect(requests.slice(before)).toEqual([]);
    }
    expect(await page.locator('#soon-toast').count()).toBe(1);
    expect(await page.evaluate(() => window.__camera)).toBe(0);
    expect(requests.filter(url => /scanner|calendar/i.test(new URL(url).pathname))).toEqual([]);
    expect(await page.evaluate(() => document.querySelectorAll('iframe').length)).toBe(0);
  });
}

test('a second tap restarts the 1.8 s and the toast node is reused', async ({ page }) => {
  await signedIn(page);
  await page.goto('/?e2e=1');
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  await page.click('#btn-drive-profile');
  await page.click('#btn-drive-menu-diary');
  await page.waitForTimeout(1200);
  await page.click('#btn-drive-profile');
  await page.click('#btn-drive-menu-agenda');
  await page.waitForTimeout(1000); // 2.2 s after the first tap: still visible
  await expect(page.locator('#soon-toast')).toHaveClass('visible');
  await expect(page.locator('#soon-toast')).not.toHaveClass('visible', { timeout: 1500 });
  expect(await page.locator('#soon-toast').count()).toBe(1);
  expect(await page.evaluate(() => document.body.lastElementChild.id)).toBe('soon-toast');
});
