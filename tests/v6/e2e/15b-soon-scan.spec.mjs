// 15b-soon-scan (TESTING §2, BEHAVIOUR-CHANGES S-1, BUILD-PLAN W5.1): "Scan" in the Manage-Pages panel keeps its
// button with a Soon badge and only shows the "Soon" toast for 1.8 s — no camera permission request, no network
// request, no scanner asset loaded; it is disabled in Read mode like 5.18.0.
import { test, expect, seedSession, addScene, sceneId } from '../mock/fixtures.mjs';

test.use({ serviceWorkers: 'block' });

for (const width of [390, 1280]) {
  test(`Scan answers "Soon" and nothing else (${width}px)`, async ({ page, context, mock, problems }) => {
    await page.setViewportSize({ width, height: 844 });
    await seedSession(context, mock);
    addScene(mock, 'template');
    await page.addInitScript(() => {
      window.__camera = 0;
      const media = navigator.mediaDevices;
      if (media) media.getUserMedia = () => { window.__camera += 1; return Promise.reject(new Error('blocked by test')); };
      const query = navigator.permissions?.query?.bind(navigator.permissions);
      if (query) navigator.permissions.query = descriptor => { if (descriptor?.name === 'camera') window.__camera += 1; return query(descriptor); };
    });
    await page.goto(`/?drive=${sceneId('template')}&e2e=1`);
    await page.waitForFunction(() => window.__IHN_TEST_API__);
    await page.evaluate(() => window.__IHN_TEST_API__.ready());
    await expect(page.locator('.page-row')).toHaveCount(3, { timeout: 20_000 });

    // Read mode: the button is disabled.
    await page.waitForTimeout(600); // the loading overlay's fade-out
    await page.click('#btn-edit-pages', { force: true });
    await expect(page.locator('#pages-panel')).toHaveClass(/visible/);
    await expect(page.locator('#btn-scan-page')).toBeDisabled();
    await page.waitForTimeout(400);
    await page.click('#panel-close');
    await page.waitForTimeout(400); // the overlay's 0.3 s fade
    // (On phones the floating toggle group sits over the button; the switch itself is not under test here.)
    await page.evaluate(() => document.getElementById('mode-toggle-btn').click());
    await expect(page.locator('#editor-view')).not.toHaveClass(/read-only/);
    await page.click('#btn-edit-pages', { force: true });
    const button = page.locator('#btn-scan-page');
    await expect(button).toBeEnabled();
    await expect(button.locator('.soon-badge')).toHaveText('Soon');
    await expect(button).toHaveAttribute('aria-label', 'Scan pages (soon)');
    // Scan stays directly after Photo (5.18.0 release smoke).
    expect(await page.evaluate(() => document.getElementById('btn-set-cover').nextElementSibling.id)).toBe('btn-scan-page');
    await page.waitForTimeout(400);

    const requests = [];
    page.on('request', request => { if (!request.url().includes('release-check=')) requests.push(request.url()); });
    const shownAt = Date.now();
    await button.click();
    const toast = page.locator('#soon-toast');
    await expect(toast).toHaveClass('visible');
    await expect(toast).toHaveText('Soon');
    await expect(toast).toHaveAttribute('role', 'status');
    await expect(page.locator('#pages-panel')).toHaveClass(/visible/);
    await page.waitForTimeout(Math.max(0, 1200 - (Date.now() - shownAt)));
    await expect(toast).toHaveClass('visible');
    await expect(toast).not.toHaveClass('visible', { timeout: 1500 });
    expect(Date.now() - shownAt).toBeGreaterThanOrEqual(1750);
    expect(requests.filter(url => !/googleapis\.com\/(drive|upload)|\/relay|accounts\.google/.test(url))).toEqual([]);
    expect(requests.filter(url => /scanner|calendar|opencv/i.test(new URL(url).pathname))).toEqual([]);
    expect(await page.evaluate(() => window.__camera)).toBe(0);
    expect(await page.evaluate(() => document.querySelectorAll('iframe, video').length)).toBe(0);
    expect(problems).toEqual([]);
  });
}
