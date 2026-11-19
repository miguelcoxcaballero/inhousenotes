// 00-boot (TESTING §2): boot signed out under the page's CSP → Welcome, both version labels, the wasm import
// allowlist, no pdf.js, no console or page error; plus the ABI self-test against the real host.
import { expect, test } from '@playwright/test';

async function open(page, url = '/?e2e=1') {
  const problems = [];
  const requests = [];
  page.on('console', message => { if (message.type() === 'error') problems.push(`console: ${message.text()}`); });
  page.on('pageerror', error => problems.push(`pageerror: ${error.message}`));
  page.on('request', request => requests.push(request.url()));
  // The Google Identity Services client is the only external script; the mock (W1.2) replaces this stub.
  await page.route('https://accounts.google.com/**', route => route.fulfill({ contentType: 'text/javascript', body: '' }));
  // The connectivity probe (W1.3; 5.18.0 `probeConnection`) must not reach the real Drive: its 401 is a console error.
  await page.route('https://www.googleapis.com/**', route => route.fulfill({ headers: { 'access-control-allow-origin': '*' }, json: { kind: 'drive#about' } }));
  await page.addInitScript(() => {
    window.__cspViolations = [];
    document.addEventListener('securitypolicyviolation', event => window.__cspViolations.push(`${event.violatedDirective} ${event.blockedURI}`));
  });
  await page.goto(url);
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  expect(await page.evaluate(() => window.__IHN_TEST_API__.ready())).toBe(true);
  return { problems, requests };
}

test('boots signed out into Welcome with v6.0.0 labels and a clean console', async ({ page }) => {
  const { problems, requests } = await open(page);
  await expect(page.locator('#welcome-view')).toBeVisible();
  await expect(page.locator('#drive-home')).toBeHidden();
  await expect(page.locator('#editor-view')).toBeHidden();
  const labels = await page.locator('[data-app-version]').allTextContents();
  expect(labels).toEqual(['v6.0.0', 'v6.0.0']);
  expect(await page.evaluate(() => document.documentElement.dataset.bootDestination)).toBe('welcome');

  const api = await page.evaluate(() => {
    const test = window.__IHN_TEST_API__;
    return { state: test.state(), imports: test.wasmImports().sort(), allowed: test.allowedImports().sort() };
  });
  expect(api.imports).toEqual(api.allowed);
  expect(api.imports).toHaveLength(9);
  expect(api.state).toMatchObject({ version: '6.0.0', abi: 6, ready: true });
  expect(api.state.modules).toContain('shell');
  expect(api.state.flags & 4).toBe(4); // e2e

  // Nothing of 5.x and no PDF library on the boot path.
  const local = requests.map(url => new URL(url)).filter(url => url.hostname === '127.0.0.1').map(url => url.pathname);
  expect(local.filter(path => /vendor\/(pdfjs|pdf-lib|jspdf)|-v5\.js|rust-core/.test(path))).toEqual([]);
  // `?e2e=1` boots the test build (W6.2t): production code plus the test API.
  expect(local).toEqual(expect.arrayContaining(['/boot.js', '/host/main.js', '/host/dom.js', '/wasm/ihn-test.wasm']));
  expect(local).not.toContain('/wasm/ihn.wasm');
  expect(await page.evaluate(() => [typeof window.pdfjsLib, typeof window.PDFLib, typeof window.jspdf])).toEqual(['undefined', 'undefined', 'undefined']);
  expect(await page.evaluate(() => performance.getEntriesByName('ihn-wasm-ready').length)).toBe(1);

  expect(await page.evaluate(() => window.__cspViolations)).toEqual([]);
  expect(problems).toEqual([]);
});

test('the test API exists only on localhost with ?e2e=1', async ({ page }) => {
  await page.route('https://accounts.google.com/**', route => route.fulfill({ contentType: 'text/javascript', body: '' }));
  const wasm = [];
  page.on('request', request => { if (request.url().includes('/wasm/')) wasm.push(new URL(request.url()).pathname); });
  await page.goto('/');
  await expect(page.locator('[data-app-version]').first()).toHaveText('v6.0.0');
  expect(await page.evaluate(() => typeof window.__IHN_TEST_API__)).toBe('undefined');
  expect(wasm).toEqual(['/wasm/ihn.wasm']); // the production build, without the test API
});

test('a returning user boots to the Home skeleton, not Welcome', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('drive-remember-v1', '1'));
  await open(page);
  await expect(page.locator('#welcome-view')).toBeHidden();
  await expect(page.locator('#drive-home')).toBeVisible();
});

test('a reduced-motion preference reaches Rust at boot and when it changes', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await open(page);
  const flags = () => page.evaluate(() => window.__IHN_TEST_API__.state().flags & 128);
  expect(await flags()).toBe(128);
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await expect.poll(flags).toBe(0);
});

test('a pen stroke over queued background work sets no timer', async ({ page }) => {
  await open(page);
  // Count the host timers set while pointer events are delivered to a Rust listener.
  const sets = await page.evaluate(async () => {
    const test = window.__IHN_TEST_API__;
    test.cmd('app.selftest');
    await new Promise(resolve => setTimeout(resolve, 100));
    test.cmd('app.pen_probe');
    const button = document.querySelector('#ihn-selftest button');
    const original = window.setTimeout;
    let count = 0;
    window.setTimeout = (...args) => { count += 1; return original(...args); };
    const init = { bubbles: true, pointerId: 7, pointerType: 'pen', pressure: 0.5, buttons: 1 };
    button.dispatchEvent(new PointerEvent('pointerdown', init));
    for (let i = 0; i < 200; i += 1) button.dispatchEvent(new PointerEvent('pointermove', { ...init, clientX: i }));
    window.setTimeout = original;
    const seen = test.cmd('app.pen_probe_result');
    test.cmd('app.selftest_result');
    return { count, seen };
  });
  expect(sets).toEqual({ count: 0, seen: 201 });
});

test('every DOM op, timer, frame, idle slot and synchronous import works against the real host', async ({ page }) => {
  const { problems } = await open(page);
  expect(await page.evaluate(() => window.__IHN_TEST_API__.cmd('app.selftest'))).toBe(true);
  const dom = await page.evaluate(() => {
    const root = document.getElementById('ihn-selftest');
    const span = root.querySelector('span'), input = root.querySelector('input');
    return {
      children: [...root.children].map(node => node.tagName),
      made: root.firstElementChild.outerHTML,
      span: { text: span.textContent, classes: span.className, k: span.dataset.k, gone: span.hasAttribute('data-gone'),
        color: span.style.color, background: span.style.backgroundColor, custom: span.style.getPropertyValue('--ihn-x') },
      input: { value: input.value, disabled: input.disabled },
      bound: root.dataset.bound,
      dialogOpen: root.querySelector('dialog').open,
      animations: span.getAnimations().length
    };
  });
  expect(dom).toEqual({
    children: ['P', 'SPAN', 'INPUT', 'BUTTON', 'DIALOG', 'I'],
    made: '<p class="c d"><b>k</b></p>',
    span: { text: 'héllo', classes: 'b', k: 'v', gone: false, color: 'red', background: '', custom: '1' },
    input: { value: 'typed', disabled: true },
    bound: '1',
    dialogOpen: false,
    animations: 0
  });
  // Drag events carry `dataTransfer.types` and `relatedTarget`; the listener's result sets `dropEffect`.
  const drag = await page.evaluate(() => {
    const root = document.getElementById('ihn-selftest'), button = root.querySelector('button');
    const dataTransfer = { types: ['text/plain', 'Files'], files: [], dropEffect: 'none' };
    const over = new Event('dragover', { bubbles: true, cancelable: true });
    Object.defineProperty(over, 'dataTransfer', { value: dataTransfer });
    button.dispatchEvent(over);
    button.dispatchEvent(new MouseEvent('dragleave', { bubbles: true, relatedTarget: root.querySelector('span') }));
    return { prevented: over.defaultPrevented, dropEffect: dataTransfer.dropEffect };
  });
  expect(drag).toEqual({ prevented: true, dropEffect: 'copy' });
  // Two interval ticks, the idle slot, the observers and the deferred job need a few turns of the event loop.
  await page.waitForTimeout(300);
  const result = await page.evaluate(() => window.__IHN_TEST_API__.cmd('app.selftest_result'));
  expect(result).toEqual({
    clicks: 1, pathOk: true, timer: true, interval: 2, frame: true, idle: true, finish: true, work: true,
    dragover: true, dragleave: true, resized: true, mutated: true,
    sync: {
      connected: 1, bodyWidth: 1280, innerWidth: 1280, lsSet: true, lsGet: 'ß1', session: true, lsMissing: true,
      random: true, lower: 'àb', compare: '-1', number: '1,234.5', date: 'January', now: true, time: true,
      color: 'rgb(255, 0, 0)', custom: '1', media: true, hit: true, active: true, capture: false
    }
  });
  expect(await page.evaluate(() => [document.getElementById('ihn-selftest'), localStorage.getItem('ihn-selftest')])).toEqual([null, null]);
  expect(await page.evaluate(() => window.__IHN_TEST_API__.cmd('nope.nothing'))).toEqual({ error: 'unknown test command nope.nothing' });
  expect(problems).toEqual([]);
});
