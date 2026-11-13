// 14b-sw-upgrade (TESTING §2 row 14, ARCHITECTURE §10.4/§12): a browser that runs 5.18.0 with its service worker
// and shell cache; the site is then replaced by v6. The open 5.18.0 page must reload into v6 at its next release
// check, the registration must stay at /service-worker.js, and `v=5.18.0` assets must still be answered.
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect, test } from '@playwright/test';
import { createStaticServer } from '../../../scripts/serve.mjs';

const V5_COMMIT = '1d6e8a0'; // 5.18.0
let v5Root, server, origin;
const site = { roots: [], deny: null };

test.beforeAll(async () => {
  // The 5.18.0 site exactly as published (needs the commit in the clone: CI checks out with fetch-depth 0).
  v5Root = mkdtempSync(join(tmpdir(), 'ihn-5.18.0-'));
  const archive = execFileSync('git', ['archive', V5_COMMIT, '--', '.', ':!android app', ':!tests', ':!rust-core'], { maxBuffer: 256 << 20 });
  execFileSync('tar', ['-x', '-C', v5Root], { input: archive });
  site.roots = [v5Root];
  server = createStaticServer(site);
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  origin = `http://127.0.0.1:${server.address().port}`;
});

test.afterAll(async () => {
  await new Promise(resolve => server.close(resolve));
  rmSync(v5Root, { recursive: true, force: true });
});

test('an open 5.18.0 page with its service worker upgrades into v6', async ({ context }) => {
  test.setTimeout(90_000);
  const page = await context.newPage();
  const pageErrors = [];
  page.on('pageerror', error => pageErrors.push(error.message));
  await context.route(url => url.hostname !== '127.0.0.1', route => route.fulfill({ contentType: 'text/javascript', body: '' }));

  // 1. 5.18.0 running, its worker active and its shell cached; the start-up release check has seen 5.18.0.
  const firstCheck = page.waitForResponse(response => response.url().includes('release-check='));
  await page.goto(`${origin}/`);
  await expect(page.locator('[data-app-version]').first()).toHaveText('v5.18.0');
  await page.evaluate(() => navigator.serviceWorker.ready);
  await expect.poll(() => page.evaluate(async () => (await (await caches.open('inhouse-shell-5.18.0')).keys()).length)).toBeGreaterThan(100);
  await expect.poll(() => page.evaluate(() => !!navigator.serviceWorker.controller)).toBe(true);
  expect(await (await firstCheck).text()).toContain("const VERSION='5.18.0'");
  const registered = await page.evaluate(async () => (await navigator.serviceWorker.getRegistration()).active.scriptURL);
  expect(registered).toBe(`${origin}/service-worker.js`);

  // 2. v6 is deployed. Two 5.18.0 files are made unreachable on the server to prove where answers come from.
  site.roots = ['build', '.'];
  const offline = ['/app-v5.js', '/runtime-core-v5.js'];
  site.deny = pathname => offline.includes(pathname);

  // 3. The browser's update check installs the v6 worker (same URL, classic), which takes control of the old page.
  await page.evaluate(async () => {
    const changed = new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, { once: true }));
    await (await navigator.serviceWorker.getRegistration()).update();
    await changed;
  });
  const during = await page.evaluate(async paths => ({
    caches: (await caches.keys()).filter(key => key.startsWith('inhouse-shell-')).sort(),
    version: document.querySelector('[data-app-version]').textContent,
    // What the still-open 5.18.0 page may request before it reloads.
    legacy: await Promise.all(paths.map(async path => {
      const response = await fetch(`${path}?v=5.18.0`);
      return [response.status, (await response.text()).length > 1000];
    })),
    uncached: (await fetch('/app-v5.js?v=5.17.0')).status
  }), offline);
  expect(during).toEqual({
    caches: ['inhouse-shell-5.18.0', 'inhouse-shell-6.0.0'],
    version: 'v5.18.0',
    legacy: [[200, true], [200, true]],
    uncached: 404
  });
  // The registry knows this window has not announced itself as v6.
  const ask = () => page.evaluate(() => new Promise(resolve => {
    const channel = new MessageChannel();
    channel.port1.onmessage = event => resolve(event.data);
    navigator.serviceWorker.controller.postMessage({ type: 'ihn6-clients' }, [channel.port2]);
  }));
  expect(await ask()).toEqual({ type: 'ihn6-clients', windows: 1, v6: 0 });

  // 4. The page's own release watcher (boot-v5.js) notices 6.0.0 at its next check and reloads, once.
  const reloaded = page.waitForEvent('load');
  await page.evaluate(() => window.dispatchEvent(new PageTransitionEvent('pageshow', { persisted: true })));
  await reloaded;
  await expect(page.locator('[data-app-version]').first()).toHaveText('v6.0.0');
  await expect(page.locator('#welcome-view')).toBeVisible();
  const after = await page.evaluate(async () => ({
    scriptURL: (await navigator.serviceWorker.getRegistration()).active.scriptURL,
    registrations: (await navigator.serviceWorker.getRegistrations()).length,
    controlled: !!navigator.serviceWorker.controller,
    v6Host: !!document.querySelector('script[type="module"][src^="host/main.js?v=6.0.0"]'),
    v5Scripts: document.querySelectorAll('script[src*="-v5.js"]').length,
    reloadGuard: sessionStorage.getItem('ihn-release-reload'),
    legacy: (await fetch('/app-v5.js?v=5.18.0')).status,
    shell: (await (await caches.open('inhouse-shell-6.0.0')).keys()).map(request => new URL(request.url).pathname)
  }));
  expect(after).toMatchObject({
    scriptURL: registered, registrations: 1, controlled: true, v6Host: true, v5Scripts: 0, reloadGuard: '6.0.0', legacy: 200
  });
  expect(after.shell).toEqual(expect.arrayContaining(['/index.html', '/boot.js', '/host/main.js', '/wasm/ihn.wasm']));
  expect(after.shell.filter(path => /-v5\.js$|jspdf/.test(path))).toEqual([]);
  await expect.poll(ask).toEqual({ type: 'ihn6-clients', windows: 1, v6: 1 });

  // 5. Offline, v6 starts from its own shell; the worker has no background-sync handlers (N-5).
  site.deny = () => true;
  await page.reload();
  await expect(page.locator('[data-app-version]').first()).toHaveText('v6.0.0');
  site.deny = null;
  expect(pageErrors).toEqual([]);
});
