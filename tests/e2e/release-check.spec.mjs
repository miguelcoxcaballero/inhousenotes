import { expect, test } from '@playwright/test';

// The boot script compares the running release with the one published on the
// live site (read from service-worker.js without any cache) and reloads once
// into a newer one.
const publish = (page, version) => page.route('**/service-worker.js?release-check=*', (route) => route.fulfill({
  status: 200,
  contentType: 'text/javascript',
  body: `'use strict';\nconst VERSION='${version}';\n`
}));

test('a newer published release reloads the app exactly once', async ({ page }) => {
  await publish(page, '999.0.0');
  let navigations = 0;
  page.on('framenavigated', (frame) => { if (frame === page.mainFrame()) navigations += 1; });
  await page.goto('/?e2e=1');
  await expect.poll(() => navigations, { timeout: 10000 }).toBe(2);
  await expect.poll(() => page.evaluate(() => sessionStorage.getItem('ihn-release-reload'))).toBe('999.0.0');
  await page.waitForTimeout(2500);
  expect(navigations).toBe(2);
});

test('the same published release does not reload', async ({ page }) => {
  const running = await (async () => {
    await page.goto('/?e2e=1');
    return page.evaluate(() => document.getElementById('inhouse-app-version').textContent.match(/'(\d+\.\d+\.\d+)'/)[1]);
  })();
  await publish(page, running);
  let navigations = 0;
  page.on('framenavigated', (frame) => { if (frame === page.mainFrame()) navigations += 1; });
  await page.reload();
  await page.waitForTimeout(3000);
  expect(navigations).toBe(1);
  expect(await page.evaluate(() => sessionStorage.getItem('ihn-release-reload'))).toBeNull();
});

test('an open document is never reloaded under the user', async ({ page }) => {
  // Keep the editor open for the whole start-up, as if a document were open.
  await page.addInitScript(() => {
    setInterval(() => document.getElementById('editor-view')?.classList.remove('hidden'), 20);
  });
  await publish(page, '999.0.0');
  let navigations = 0;
  page.on('framenavigated', (frame) => { if (frame === page.mainFrame()) navigations += 1; });
  await page.goto('/?e2e=1');
  await page.waitForTimeout(3000);
  expect(navigations).toBe(1);
});

test('with the offline service worker in control, a reload loads the newly published page', async ({ page, context }) => {
  await page.goto('/?e2e=1');
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  await page.evaluate(() => navigator.serviceWorker.ready);
  await page.waitForFunction(() => !!navigator.serviceWorker.controller);
  const running = await page.evaluate(() => document.getElementById('inhouse-app-version').textContent.match(/'(\d+\.\d+\.\d+)'/)[1]);
  // Publish a "next" release of the page; its versioned assets come from the same server.
  await context.route(/\/(index\.html)?(\?.*)?$/, async (route) => {
    // The page request reaches the network from the service worker, as a fetch.
    const request = route.request();
    if (request.resourceType() !== 'document' && !request.serviceWorker()) return route.continue();
    const response = await route.fetch();
    const body = (await response.text()).replaceAll(running, '99.0.0');
    await route.fulfill({ response, body });
  });
  await page.route('**/service-worker.js?release-check=*', (route) => route.fulfill({ status: 200, contentType: 'text/javascript', body: `const VERSION='${running}';` }));
  await page.reload();
  await expect.poll(() => page.evaluate(() => document.getElementById('inhouse-app-version').textContent)).toContain("'99.0.0'");
  // Offline, the worker still serves its own cached release.
  await context.unroute(/\/(index\.html)?(\?.*)?$/);
  await context.setOffline(true);
  await page.reload();
  await expect.poll(() => page.evaluate(() => document.getElementById('inhouse-app-version').textContent)).toContain(`'${running}'`);
});
