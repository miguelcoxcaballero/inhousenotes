// 16-timeline (TESTING §2 row 16, BUILD-PLAN W5.3): edits over two 5-minute windows coalesce as in 5.18.0 (captured
// by the editing device, author = itself); the imported 5.x history of a document is listed (grouped, `Original`
// last, the current entry selected, preview with diff highlights); restore through the slider → a `pre-restore`
// milestone, the restored content and a `restore` milestone; a second device sees the same entries.
import { test, expect, seedSession, addScene, sceneId, ANA } from '../mock/fixtures.mjs';

test.use({ serviceWorkers: 'block', viewport: { width: 1280, height: 900 } });
test.describe.configure({ timeout: 90_000 });

const cmd = (page, name, args) => page.evaluate(([n, a]) => window.__IHN_TEST_API__.cmd(n, a), [name, args]);
async function boot(page, url) {
  await page.goto(url);
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  expect(await page.evaluate(() => window.__IHN_TEST_API__.ready())).toBe(true);
}
const settled = page => expect.poll(async () => !(await cmd(page, 'doc.open.state')).running && !(await cmd(page, 'doc.v5open.state')).busy, { timeout: 20_000 }).toBe(true);
const versions = page => cmd(page, 'doc.versions.state');
const panel = page => cmd(page, 'panels.timeline.state');

async function openScene(page, context, mock, name) {
  await seedSession(context, mock);
  addScene(mock, name);
  await boot(page, `/?drive=${sceneId(name)}&e2e=1`);
  await settled(page);
}
async function edit(page) {
  await page.click('#mode-toggle-btn');
  await expect(page.locator('#editor-view')).not.toHaveClass(/read-only/);
}
async function openTimeline(page) {
  if (!(await page.locator('#pages-panel').evaluate(e => e.classList.contains('visible')))) await page.click('#btn-edit-pages');
  await expect(page.locator('#pages-panel')).toHaveClass(/visible/);
  await page.click('#btn-open-timeline');
  await expect(page.locator('#timeline-panel')).toHaveClass(/visible/);
  await expect.poll(async () => (await panel(page)).loading).toBe(false);
}

/** A stylus stroke on page 1 (CDP pen input, as 04-ink). */
async function draw(page, dx = 0) {
  const box = await page.locator('canvas.page-canvas[data-page="0"]').boundingBox();
  const cdp = await page.context().newCDPSession(page);
  const send = (type, x, y, buttons) => cdp.send('Input.dispatchMouseEvent', { type, x: box.x + x, y: box.y + y, button: 'left', buttons, clickCount: 1, pointerType: 'pen', force: 0.5 });
  await send('mousePressed', 120 + dx, 200, 1);
  for (let i = 1; i <= 8; i++) await send('mouseMoved', 120 + dx + i * 12, 200 + i * 6, 1);
  await send('mouseReleased', 120 + dx + 96, 248, 0);
  await cdp.detach();
}

/** Amber (added/changed) and red (removed) highlight pixels on the preview canvases. */
const highlightPixels = page => page.evaluate(() => {
  let amber = 0, red = 0;
  for (const c of document.querySelectorAll('#timeline-preview-wrap canvas')) {
    const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
    for (let i = 0; i < d.length; i += 4) {
      if (d[i] > 230 && d[i + 1] > 140 && d[i + 1] < 175 && d[i + 2] < 40) amber++;
      if (d[i] > 225 && d[i + 1] < 90 && d[i + 2] < 90) red++;
    }
  }
  return { amber, red };
});

/** Drags the slider handle to the end of its track. */
async function slide(page) {
  await expect(page.locator('#modal-confirm-slider')).toHaveClass(/visible/);
  const handle = await page.locator('#confirm-slider-handle').boundingBox();
  const track = await page.locator('#confirm-slider').boundingBox();
  const [x0, y0] = [handle.x + handle.width / 2, handle.y + handle.height / 2];
  await page.mouse.move(x0, y0);
  await page.mouse.down();
  for (let i = 1; i <= 10; i++) await page.mouse.move(x0 + (track.x + track.width + 10 - x0) * i / 10, y0);
  await page.mouse.up();
}

test('captures coalesce within 5 minutes and start a new entry after them', async ({ page, context, mock, problems }) => {
  await openScene(page, context, mock, 'template');
  await edit(page);
  // The fixture carries one 5.x entry (its author was signed out: `You`).
  const n = (await versions(page)).entries.length;
  expect(n).toBe(1);
  // The first stroke: captured after the quiet window, by this device, as the signed-in profile.
  await draw(page);
  await expect.poll(async () => (await versions(page)).entries.length, { timeout: 5000 }).toBe(n + 1);
  const first = (await versions(page)).entries.at(-1);
  expect(first).toMatchObject({ kind: 'autosave', milestone: false, summary: '+1 stroke', email: ANA, delta: false, cached: true, stored: true });
  // More strokes within the window are not captured on every idle (≤ 12 asset uploads per hour)…
  await draw(page, 40);
  await page.waitForTimeout(600);
  expect((await versions(page)).entries).toHaveLength(n + 1);
  expect((await versions(page)).dirty).toBe(true);
  // …and a capture 1 min later rewrites the same entry (5.18.0 coalescing: same author and device, < 5 min; the
  // summary compares with the entry it replaces, 5.18.0's quirk).
  const r1 = await cmd(page, 'doc.versions.capture', { ts: first.ts + 60_000 });
  expect(r1.id).toBe(first.id);
  let list = (await versions(page)).entries.slice(n);
  expect(list).toHaveLength(1);
  expect(list[0]).toMatchObject({ id: first.id, ts: first.ts + 60_000, summary: '+1 stroke' });
  // Six minutes later: a new entry, a delta of the previous snapshot in this device's chain.
  await draw(page, 80);
  await page.waitForTimeout(400);
  const r2 = await cmd(page, 'doc.versions.capture', { ts: first.ts + 60_000 + 6 * 60_000 });
  list = (await versions(page)).entries.slice(n);
  expect(list).toHaveLength(2);
  expect(list[1]).toMatchObject({ id: r2.id, summary: '+1 stroke', delta: true, device: list[0].device });
  // Identical content: nothing captured; a milestone always is.
  expect((await cmd(page, 'doc.versions.capture', {})).id).toBe(null);
  expect((await cmd(page, 'doc.versions.capture', { ts: first.ts + 8 * 60_000, milestone: true, kind: 'manual', summary: 'Checkpoint' })).id).not.toBe(null);
  expect((await versions(page)).entries.slice(n).map(e => e.kind)).toEqual(['autosave', 'autosave', 'manual']);
  // Leaving the page captures pending edits (Home's exit is W5.4's; it sends the same `VIEW` signal).
  await draw(page, 120);
  await page.waitForTimeout(300);
  expect((await versions(page)).dirty).toBe(true);
  await page.evaluate(() => window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: false })));
  await expect.poll(async () => (await versions(page)).entries.length).toBe(n + 4);
  expect(problems).toEqual([]);
});

test('imported 5.x history: list, preview with highlights, restore with the slider', async ({ page, context, mock, problems }) => {
  await openScene(page, context, mock, 'versions-30');
  const imported = (await versions(page)).entries;
  expect(imported).toHaveLength(31);
  expect(imported.filter(e => e.milestone)).toHaveLength(10);
  expect(imported.every(e => e.stored)).toBe(true);
  expect(imported.map(e => e.depth).slice(0, 10)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 0, 1]);

  await edit(page);
  await openTimeline(page);
  // Newest first; the newest cluster expanded; Original last; the current entry selected, Revert disabled.
  await expect(page.locator('.timeline-entry')).toHaveCount(32);
  await expect(page.locator('.timeline-entry').last()).toHaveClass(/is-original/);
  await expect(page.locator('.timeline-entry.is-original .timeline-entry-milestone-tag')).toHaveText('Original');
  await expect(page.locator('.timeline-entry.is-current')).toHaveCount(1);
  await expect(page.locator('.timeline-entry.is-current')).toHaveClass(/is-selected/);
  await expect(page.locator('.timeline-entry.is-current .timeline-entry-current-tag')).toHaveText('Current');
  await expect(page.locator('.timeline-group-count').first()).toHaveText('31 edits');
  await expect(page.locator('.timeline-cluster-header').first()).toHaveAttribute('aria-expanded', 'true');
  await expect(page.locator('#btn-revert-to-version')).toBeDisabled();
  await expect(page.locator('#timeline-preview-empty')).toBeHidden();
  await expect(page.locator('#timeline-preview-meta')).toContainText('· 1 page');
  await expect(page.locator('.timeline-preview-page')).toHaveCount(1);
  await expect(page.locator('.timeline-preview-page-label')).toHaveText('Page 1');

  // An older version: its preview marks the stroke it added; Revert is enabled.
  const target = imported[10];
  await page.click(`.timeline-entry[data-version-id="${target.id}"]`);
  await expect(page.locator(`.timeline-entry[data-version-id="${target.id}"]`)).toHaveClass(/is-selected/);
  await expect(page.locator('#btn-revert-to-version')).toBeEnabled();
  await expect.poll(async () => (await highlightPixels(page)).amber).toBeGreaterThan(50);
  // Ctrl+wheel zooms the preview.
  const box = await page.locator('#timeline-preview-wrap').boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.keyboard.down('Control');
  await page.mouse.wheel(0, -100);
  await page.keyboard.up('Control');
  await expect.poll(async () => (await panel(page)).zoom).toBeGreaterThan(2);
  // Collapsing the cluster hides its entries; the month header hides the clusters.
  await page.click('.timeline-cluster-header >> nth=0');
  await expect(page.locator('.timeline-cluster-entries').first()).toBeHidden();
  await page.click('.timeline-cluster-header >> nth=0');
  await expect(page.locator('.timeline-cluster-entries').first()).toBeVisible();

  // Restore: the slider, then the pre-restore milestone, the restored content, the restore milestone.
  await page.click('#btn-revert-to-version');
  await expect(page.locator('#confirm-slider-title')).toHaveText('Restore this version?');
  await slide(page);
  await expect(page.locator('#timeline-panel')).not.toHaveClass(/visible/, { timeout: 5000 });
  await expect(page.locator('#save-indicator')).toHaveAttribute('aria-label', 'Document restored.');
  const after = (await versions(page)).entries;
  expect(after).toHaveLength(33);
  expect(after.slice(-2).map(e => [e.kind, e.milestone])).toEqual([['pre-restore', true], ['restore', true]]);
  expect(after.at(-2).summary).toMatch(/^Before restoring /);
  expect(after.at(-1).summary).toMatch(/^Restored version from /);
  const doc = await cmd(page, 'doc.v5open.doc');
  expect(doc.pages[0].strokes).toHaveLength(11);

  // The timeline again: the restore milestone is current and tagged; reverting to Original empties the page.
  await openTimeline(page);
  await expect(page.locator('.timeline-entry.is-current .timeline-entry-milestone-tag')).toHaveText('Restored');
  await page.click('.timeline-entry.is-original');
  await page.click('#btn-revert-to-version');
  await expect(page.locator('#confirm-slider-title')).toHaveText('Revert to the original document?');
  await slide(page);
  await expect(page.locator('#timeline-panel')).not.toHaveClass(/visible/, { timeout: 5000 });
  expect((await cmd(page, 'doc.v5open.doc')).pages.flatMap(p => p.strokes)).toHaveLength(0);
  expect((await versions(page)).entries.at(-1).kind).toBe('revert-original');
  expect(problems).toEqual([]);
});

test('a second device sees the same entries', async ({ browser, page, context, mock, problems }) => {
  await openScene(page, context, mock, 'template');
  await edit(page);
  await draw(page);
  await expect.poll(async () => (await versions(page)).entries.length, { timeout: 5000 }).toBe(2);
  await expect.poll(async () => (await cmd(page, 'collab.state')).unsealed, { timeout: 20_000 }).toBe(0);
  const mine = (await versions(page)).entries;

  const other = await browser.newContext({ serviceWorkers: 'block', viewport: { width: 1280, height: 900 } });
  await mock.install(other);
  await seedSession(other, mock);
  const page2 = await other.newPage();
  await boot(page2, `/?drive=${sceneId('template')}&e2e=1`);
  await settled(page2);
  await expect.poll(async () => (await versions(page2)).entries.map(e => [e.id, e.ts, e.summary, e.email]), { timeout: 30_000 })
    .toEqual(mine.map(e => [e.id, e.ts, e.summary, e.email]));
  await openTimeline(page2);
  await expect(page2.locator(`.timeline-entry[data-version-id="${mine[1].id}"]`)).toHaveClass(/is-current/);
  await other.close();
  expect(problems).toEqual([]);
});
