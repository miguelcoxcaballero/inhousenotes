// 09-offline (TESTING §2 row 09, BUILD-PLAN W5.4): offline, write on two pages → Home shows `Saved offline`; a new
// document created offline (reserved id, `Created on this device`) is written; back online → the relay of the first
// is sealed, the new one is created in Drive with its reserved id, the badges are gone and the indicator is `saved`.
import { test, expect, seedSession, addScene, sceneId } from '../mock/fixtures.mjs';

test.use({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 } });
test.describe.configure({ timeout: 150_000 });

const cmd = (page, name, args) => page.evaluate(([n, a]) => window.__IHN_TEST_API__.cmd(n, a), [name, args]);
const session = page => cmd(page, 'doc.session.state');
const collab = page => cmd(page, 'collab.state');

async function stroke(page, index, points) {
  const canvas = page.locator(`canvas.page-canvas[data-page="${index}"]`);
  await canvas.scrollIntoViewIfNeeded();
  const cdp = await page.context().newCDPSession(page);
  const box = await canvas.boundingBox();
  const at = p => [box.x + p[0] * box.width / 794, box.y + Math.min(p[1] * box.height / 1123, 600 - box.y)];
  const send = (type, p, buttons) => cdp.send('Input.dispatchMouseEvent', { type, x: at(p)[0], y: at(p)[1], button: 'left', buttons, clickCount: 1, pointerType: 'pen', force: 0.5 });
  await send('mousePressed', points[0], 1);
  for (const p of points.slice(1)) await send('mouseMoved', p, 1);
  await send('mouseReleased', points.at(-1), 0);
  await cdp.detach();
}
const line = (x, y) => Array.from({ length: 8 }, (_, i) => [x + i * 14, y + (i % 2) * 7]);

test('offline edits and an offline document reach Drive when the connection is back', async ({ page, context, mock, problems }) => {
  await seedSession(context, mock);
  addScene(mock, 'template');
  const id = sceneId('template');
  await page.goto(`/?drive=${id}&e2e=1`);
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await expect.poll(async () => (await cmd(page, 'doc.open.state')).log.some(e => e[0] === id && e[1]), { timeout: 30_000 }).toBe(true);
  await page.click('#mode-toggle-btn');
  // Online first: the relay writer fills the id pool (SYNC §8.2) that offline creation takes from.
  await stroke(page, 0, line(100, 150));
  await expect.poll(async () => (await collab(page)).unsealed, { timeout: 20_000 }).toBe(0);

  mock.offline = true;
  await context.setOffline(true);
  await stroke(page, 0, line(100, 300));
  await stroke(page, 1, line(100, 200));
  await expect.poll(async () => (await session(page)).uncommitted).toBe(0);
  await page.click('#btn-drive-home svg');
  await expect(page.locator('#drive-home')).toBeVisible();
  await expect(page.locator('#drive-home .document-sync-status', { hasText: 'Saved offline' }).first()).toBeVisible({ timeout: 10_000 });

  // A new document offline: its id is reserved from the pool; it is not in Drive yet.
  expect(await cmd(page, 'doc.create.new', { folder: '', name: 'Offline notes' })).toBe(true);
  await expect.poll(async () => (await session(page)).name, { timeout: 10_000 }).toBe('Offline notes');
  const fresh = (await session(page)).docId;
  expect(fresh).not.toBe(id);
  expect(mock.drive.file(fresh)).toBeUndefined();
  await stroke(page, 0, line(200, 260));
  await expect.poll(async () => (await session(page)).uncommitted).toBe(0);
  await page.click('#btn-drive-home svg');
  await expect(page.locator('#drive-home')).toBeVisible();

  // Back online: the runner seals the first document's relay and creates the new one with its reserved id.
  mock.offline = false;
  await context.setOffline(false);
  await expect.poll(() => mock.drive.file(fresh)?.name, { timeout: 60_000 }).toBe('Offline notes.pdf');
  expect(mock.drive.file(fresh).appProperties.ihn6).toBe('1');
  // The foreground runner finishes the first document (its relay) once Home is back online.
  await expect.poll(async () => { const s = await cmd(page, 'export.headless.state'); return s.runs > 0 && !s.running ? s.done : []; }, { timeout: 90_000 }).toContain(id);
  await expect(page.locator('#drive-home .document-sync-status')).toHaveCount(0, { timeout: 15_000 });

  // Reopened: everything sealed, the indicator is `saved`.
  await page.goto(`/?drive=${id}&e2e=1`);
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await expect.poll(async () => (await cmd(page, 'doc.open.state')).log.some(e => e[0] === id && e[1]), { timeout: 30_000 }).toBe(true);
  await expect.poll(async () => (await session(page)).live, { timeout: 10_000 }).toBeGreaterThanOrEqual(3);
  await expect.poll(async () => (await cmd(page, 'doc.save_state.state')).phase, { timeout: 20_000 }).toBe('saved');
  expect(problems).toEqual([]);
});
