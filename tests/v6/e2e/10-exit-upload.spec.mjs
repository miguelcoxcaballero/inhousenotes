// 10-exit-upload (TESTING §2 row 10, BUILD-PLAN W5.4): write, then close the page at once → the keepalive relay
// flush reached Drive (the edits are sealed); the Android headless page (`?inhouse_app=1&offline-worker=1` with the
// `InhouseBackgroundChannel` stub) completes the export and reports `complete({busy:false,pending:0,errors:[]})`; a
// fresh context opens the result. The service worker has no `sync` / `periodicsync` run (N-5).
import fs from 'node:fs';
import { test, expect, seedSession, addScene, sceneId, ANA } from '../mock/fixtures.mjs';

test.use({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 } });
test.describe.configure({ timeout: 120_000 });

const cmd = (page, name, args) => page.evaluate(([n, a]) => window.__IHN_TEST_API__.cmd(n, a), [name, args]);
const session = page => cmd(page, 'doc.session.state');

async function open(page, id) {
  await page.goto(`/?drive=${id}&e2e=1`);
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await expect.poll(async () => (await cmd(page, 'doc.open.state')).log.some(e => e[0] === id && e[1]), { timeout: 30_000 }).toBe(true);
  await expect.poll(async () => (await cmd(page, 'collab.state')).active, { timeout: 10_000 }).toBe(true);
}

async function stroke(page, points) {
  // The page must be on screen and drawn before the pen lands (a contact on a page still loading is not ink).
  await expect.poll(async () => (await cmd(page, 'editor.pages.state')).rows[0]?.drawn === true, { timeout: 15_000 }).toBe(true);
  const live = (await session(page)).live;
  const cdp = await page.context().newCDPSession(page);
  const box = await page.locator('canvas.page-canvas[data-page="0"]').boundingBox();
  const at = p => [box.x + p[0] * box.width / 794, box.y + p[1] * box.height / 1123];
  const send = (type, p, buttons) => cdp.send('Input.dispatchMouseEvent', { type, x: at(p)[0], y: at(p)[1], button: 'left', buttons, clickCount: 1, pointerType: 'pen', force: 0.5 });
  await send('mousePressed', points[0], 1);
  for (const p of points.slice(1)) await send('mouseMoved', p, 1);
  await send('mouseReleased', points.at(-1), 0);
  await cdp.detach();
  await expect.poll(async () => (await session(page)).live).toBe(live + 1);
}
const line = (x, y) => Array.from({ length: 8 }, (_, i) => [x + i * 14, y + (i % 2) * 7]);

test('close right after writing; the headless page exports; a fresh device opens it', async ({ browser, context, page, mock, problems }) => {
  await seedSession(context, mock);
  addScene(mock, 'template');
  const id = sceneId('template');
  await open(page, id);
  await page.click('#mode-toggle-btn');
  await stroke(page, line(100, 150));
  await expect.poll(async () => (await cmd(page, 'collab.state')).unsealed, { timeout: 20_000 }).toBe(0);
  const head = mock.drive.file(id).revisions.length;
  // A second stroke, then the page closes at once: `pagehide` sends the relay flush with keepalive.
  await stroke(page, line(100, 400));
  const live = (await session(page)).live;
  const writes = mock.calls.length;
  // The page's own `pagehide` is fired here first: requests of an already closed page never reach Playwright's
  // routes, so the flush is observed while the page still exists, then the page closes.
  await page.evaluate(() => window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: false })));
  await new Promise(r => setTimeout(r, 300));
  await page.close({ runBeforeUnload: true });
  await expect.poll(() => mock.calls.slice(writes).filter(c => c.method === 'PATCH' && c.status === 200).length, { timeout: 10_000 }).toBeGreaterThan(0);
  expect(mock.drive.file(id).revisions.length).toBe(head); // `pagehide` never exports

  // The Android WorkManager page: same origin and storage, no UI; it finishes the export and reports.
  const worker = await context.newPage();
  await worker.addInitScript(() => {
    window.__bg = [];
    window.InhouseBackgroundChannel = { postMessage: text => window.__bg.push(JSON.parse(text)) };
    window.InhouseNative = { getAppVersion: () => '1.0.28', getAppVersionCode: () => 29, getUpdateState: () => '{}', saveUpdateState() {} };
  });
  await worker.goto('/?inhouse_app=1&offline-worker=1&e2e=1');
  await expect.poll(() => worker.evaluate(() => window.__bg.filter(m => m.type === 'complete').map(m => m.result)), { timeout: 90_000 }).toEqual([{ busy: false, pending: 0, errors: [] }]);
  expect(mock.drive.file(id).revisions.length).toBeGreaterThan(head);
  expect(await worker.evaluate(() => !!document.querySelector('#editor-view:not(.hidden)'))).toBe(false);

  // A fresh device opens the exported document with both strokes.
  const other = await browser.newContext({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 } });
  await mock.install(other);
  await seedSession(other, mock, ANA);
  const fresh = await other.newPage();
  await open(fresh, id);
  await expect.poll(async () => (await session(fresh)).live, { timeout: 20_000 }).toBe(live);
  await other.close();

  // N-5: the service worker registers no background sync.
  const sw = fs.readFileSync(new URL('../../../service-worker.js', import.meta.url), 'utf8');
  expect(/addEventListener\(\s*['"](periodic)?sync['"]/.test(sw)).toBe(false);
  expect(problems).toEqual([]);
});
