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

async function openControlledShell(page) {
  await page.goto('/?e2e=1');
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  await page.evaluate(() => navigator.serviceWorker.ready);
  await page.waitForFunction(() => !!navigator.serviceWorker.controller);
  // Let this page's own start-up release check (1.2 s after load) finish
  // first, so a version published below is seen by the reload, not by it.
  await page.waitForTimeout(1500);
  const running = await page.evaluate(() => document.getElementById('inhouse-app-version').textContent.match(/'(\d+\.\d+\.\d+)'/)[1]);
  const html = await (await page.request.get('/index.html')).text();
  return { running, html };
}

const visibleVersion = page => page.evaluate(() => document.getElementById('inhouse-app-version')?.textContent || '');
const shellRoute = /\/(index\.html)?(\?.*)?$/;

test('the complete cached shell opens before network revalidation and rejects a mismatched offline copy', async ({ page, context }) => {
  const { running, html } = await openControlledShell(page);
  await publish(page, running);
  let releaseNetwork;
  const networkGate = new Promise(resolve => { releaseNetwork = resolve; });
  let started = 0, finished = 0;
  await context.route(shellRoute, async route => {
    // Gate the actual worker-owned fetch, not navigation. The cached page
    // must become usable while that network response is still outstanding.
    if (!route.request().serviceWorker()) return route.continue();
    started += 1;
    await networkGate;
    await route.fulfill({ status: 200, contentType: 'text/html', body: html.replaceAll(running, '99.0.0') });
    finished += 1;
  });
  try {
    await page.reload({ waitUntil: 'domcontentloaded', timeout: 5000 });
    await expect.poll(() => started).toBeGreaterThan(0);
    await expect.poll(() => visibleVersion(page)).toContain(`'${running}'`);
    expect(finished).toBe(0);
    await page.evaluate(() => window.__IHN_TEST_API__.ready());
  } finally {
    releaseNetwork();
  }
  await expect.poll(() => finished).toBeGreaterThan(0);
  await context.unroute(shellRoute);
  await context.setOffline(true);
  await page.reload();
  await expect.poll(() => visibleVersion(page)).toContain(`'${running}'`);
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
});

test('a published version check escapes an old service worker shell exactly once', async ({ page, context }) => {
  const { running, html } = await openControlledShell(page);
  await context.route(shellRoute, async route => {
    const request = route.request();
    if (request.resourceType() !== 'document' && !request.serviceWorker()) return route.continue();
    await route.fulfill({ status: 200, contentType: 'text/html', body: html.replaceAll(running, '99.0.0') });
  });
  await publish(page, '99.0.0');
  let navigations = 0;
  page.on('framenavigated', frame => { if (frame === page.mainFrame()) navigations += 1; });
  // The first navigation may use the complete old shell. The published
  // version check must evict that old HTML and reload into the new release.
  await page.reload();
  await expect.poll(() => visibleVersion(page), { timeout: 10000 }).toContain("'99.0.0'");
  await expect.poll(() => page.evaluate(() => sessionStorage.getItem('ihn-release-reload'))).toBe('99.0.0');
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  await page.waitForTimeout(2500);
  expect(navigations).toBe(2);
});
