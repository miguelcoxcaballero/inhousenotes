// 05-pages (TESTING §2, BUILD-PLAN W5.1): pull-to-add, the Pages panel (list, thumbnails, drag reorder, delete with
// the slider), a photo page from the gallery in both fit modes, and Import PDF (replaces the pages, clears undo) —
// on a 5.x fixture opened through the shared link.
import fs from 'node:fs';
import path from 'node:path';
import { test, expect, seedSession, addScene, sceneId } from '../mock/fixtures.mjs';

test.use({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 } });

const V5 = path.resolve('tests/v6/fixtures/v5');
const cmd = (page, name, args) => page.evaluate(([n, a]) => window.__IHN_TEST_API__.cmd(n, a), [name, args]);
async function open(page, url) {
  await page.goto(url);
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  expect(await page.evaluate(() => window.__IHN_TEST_API__.ready())).toBe(true);
}
const settled = page => expect.poll(async () => !(await cmd(page, 'doc.open.state')).running && !(await cmd(page, 'doc.v5open.state')).busy, { timeout: 20_000 }).toBe(true);
const doc = page => cmd(page, 'doc.v5open.doc');
const panel = page => cmd(page, 'panels.pages_panel.state');

/** Drags with the mouse from the centre of `from` to (x, y) in `steps` moves. */
async function drag(page, from, x, y, steps = 12) {
  const b = await from.boundingBox();
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2);
  await page.mouse.down();
  for (let i = 1; i <= steps; i++) {
    await page.mouse.move(b.x + b.width / 2 + (x - b.x - b.width / 2) * i / steps, b.y + b.height / 2 + (y - b.y - b.height / 2) * i / steps);
    await page.waitForTimeout(16);
  }
  await page.mouse.up();
}

/** A `w × h` JPEG of one colour, encoded in the page. */
const jpeg = (page, w, h) => page.evaluate(async ([w, h]) => {
  const c = new OffscreenCanvas(w, h);
  const ctx = c.getContext('2d');
  ctx.fillStyle = '#2050e0';
  ctx.fillRect(0, 0, w, h);
  const bytes = new Uint8Array(await (await c.convertToBlob({ type: 'image/jpeg', quality: 0.9 })).arrayBuffer());
  return Array.from(bytes);
}, [w, h]);

/** A stylus stroke across the middle of the window (CDP pen input, as 04-ink). */
async function draw(page) {
  const cdp = await page.context().newCDPSession(page);
  const send = (type, x, y, buttons) => cdp.send('Input.dispatchMouseEvent', { type, x, y, button: 'left', buttons, clickCount: 1, pointerType: 'pen', force: 0.5 });
  await send('mousePressed', 560, 300, 1);
  for (let i = 1; i <= 8; i++) await send('mouseMoved', 560 + i * 12, 300 + i * 6, 1);
  await send('mouseReleased', 656, 348, 0);
  await cdp.detach();
}

async function openTemplate(page, context, mock) {
  await seedSession(context, mock);
  addScene(mock, 'template');
  await open(page, `/?drive=${sceneId('template')}&e2e=1`);
  await settled(page);
  await expect(page.locator('.page-row')).toHaveCount(3);
}

test('pull-to-add, panel list, drag reorder, delete with the slider', async ({ page, context, mock, problems }) => {
  await page.setViewportSize({ width: 1280, height: 1200 });
  await openTemplate(page, context, mock);
  await page.click('#mode-toggle-btn');
  await expect(page.locator('#editor-view')).not.toHaveClass(/read-only/);

  // Pull past the end of the document: a fourth page.
  await page.mouse.move(640, 400);
  for (let i = 0; i < 60 && (await page.locator('.page-row').count()) < 4; i++) await page.mouse.wheel(0, 120);
  await expect(page.locator('.page-row')).toHaveCount(4, { timeout: 5000 });

  // The panel: four items with thumbnails, the default size line, the current page.
  await page.click('#btn-edit-pages');
  await expect(page.locator('#pages-panel')).toHaveClass(/visible/);
  await expect(page.locator('#panel-overlay')).toHaveClass(/visible/);
  await expect(page.locator('#pages-list .page-item')).toHaveCount(4);
  await expect(page.locator('#pages-default-size')).toHaveText('Default pageA4 · 210 × 297 mm');
  await expect(page.locator('#pages-default-size')).toHaveAttribute('title', 'A4 · 210 × 297 mm · 1 : 1.414');
  await expect(page.locator('#pages-list .page-item.is-current')).toHaveCount(1);
  await expect(page.locator('.page-item[data-page-index="0"] .page-thumbnail')).toHaveAttribute('style', 'width:136px;height:192px');
  await expect(page.locator('#thumb-0')).toHaveAttribute('width', '272');
  await expect(page.locator('.page-item[data-page-index="0"] .page-item-main')).toHaveAttribute('title', /^Page 1 · A4 · \d+ strokes?$/);
  // The thumbnail is painted (white page with ink or template: not transparent).
  expect(await page.evaluate(() => document.getElementById('thumb-0').getContext('2d').getImageData(4, 4, 1, 1).data[3])).toBe(255);
  expect(await page.locator('#btn-set-cover').isDisabled()).toBe(false);

  // Drag page 2 above page 1 (the panel's slide-in transition is over first).
  await page.waitForTimeout(400);
  const before = (await panel(page)).order;
  const first = await page.locator('.page-item[data-page-index="0"]').boundingBox();
  await drag(page, page.locator('.page-item[data-page-index="1"] .page-drag-handle'), first.x + first.width / 2, first.y + 20);
  await expect.poll(async () => (await panel(page)).moved).toBe(1);
  expect((await panel(page)).order).toEqual([before[1], before[0], before[2], before[3]]);
  expect(await page.locator('.page-reorder-preview, .page-reorder-indicator').count()).toBe(0);
  await expect(page.locator('#pages-list')).not.toHaveClass(/is-reordering/);
  await expect(page.locator('.page-row')).toHaveCount(4);

  // Delete page 2: the slider modal; a short slide springs back, a full one deletes.
  await page.click('.page-item[data-page-index="1"] .page-action-btn.delete');
  await expect(page.locator('#modal-confirm-slider')).toHaveClass(/visible/);
  await expect(page.locator('#confirm-slider-title')).toHaveText('Delete page?');
  await expect(page.locator('#confirm-slider-message')).toHaveText('This will delete page 2 and all its strokes. This cannot be undone.');
  await expect(page.locator('#confirm-slider-text')).toHaveText('Slide to delete');
  const track = await page.locator('#confirm-slider').boundingBox();
  await drag(page, page.locator('#confirm-slider-handle'), track.x + track.width / 2, track.y + track.height / 2);
  await page.waitForTimeout(300);
  expect((await panel(page)).deleted).toBe(0);
  await drag(page, page.locator('#confirm-slider-handle'), track.x + track.width + 30, track.y + track.height / 2);
  await expect(page.locator('#modal-confirm-slider')).not.toHaveClass(/visible/);
  await expect.poll(async () => (await panel(page)).deleted).toBe(1);
  const after = (await panel(page)).order;
  expect(after).toEqual([before[1], before[2], before[3]]);
  await expect(page.locator('#pages-list .page-item')).toHaveCount(3);
  await expect(page.locator('.page-row')).toHaveCount(3);

  // Cancel answers no.
  await page.click('.page-item[data-page-index="0"] .page-action-btn.delete');
  await page.click('#confirm-slider-cancel');
  await expect(page.locator('#modal-confirm-slider')).not.toHaveClass(/visible/);
  expect((await panel(page)).deleted).toBe(1);

  // Choosing a page goes there and closes the panel; Read mode disables Photo/Scan and the delete buttons.
  await page.click('.page-item[data-page-index="2"] .page-item-main');
  await expect(page.locator('#pages-panel')).not.toHaveClass(/visible/);
  expect((await panel(page)).current).toBe(2);
  await page.click('#mode-toggle-btn');
  await page.click('#btn-edit-pages');
  await expect(page.locator('#btn-set-cover')).toBeDisabled();
  await expect(page.locator('#btn-scan-page')).toBeDisabled();
  await expect(page.locator('#btn-import-pdf')).toBeDisabled();
  await expect(page.locator('.page-drag-handle')).toHaveCount(0);
  await expect(page.locator('.page-action-btn.delete[disabled]')).toHaveCount(3);
  await page.click('#panel-overlay', { position: { x: 900, y: 400 } });
  await expect(page.locator('#pages-panel')).not.toHaveClass(/visible/);
  expect(problems).toEqual([]);
});

test('photo page in both fit modes; Import PDF replaces the pages', async ({ page, context, mock, problems }) => {
  await openTemplate(page, context, mock);
  await page.click('#mode-toggle-btn');
  // An undo step (a pen stroke) to show that a photo page clears the history (5.18.0).
  await draw(page);
  await expect(page.locator('#btn-undo')).toBeEnabled();
  const pagesBefore = (await doc(page)).pages.map(p => p.id);

  await page.click('#btn-edit-pages');
  await page.waitForTimeout(300);
  await page.click('#btn-set-cover');
  await expect(page.locator('#modal-cover-image')).toHaveClass(/visible/);
  await expect(page.locator('#cover-modal-confirm')).toBeDisabled();
  await expect(page.locator('#cover-modal-meta')).toHaveText('A4 · 210 × 297 mm · 1 : 1.414');
  await expect(page.locator('#cover-preview-stage')).toHaveClass(/is-empty/);
  const photo = Buffer.from(await jpeg(page, 400, 300));
  await page.setInputFiles('#cover-image-input', { name: 'beach.jpg', mimeType: 'image/jpeg', buffer: photo });
  await expect(page.locator('#cover-browse-label')).toHaveText('Replace photo');
  await expect(page.locator('#cover-upload-dropzone')).toHaveAttribute('aria-label', 'Replace beach.jpg');
  await expect(page.locator('#cover-preview-stage')).not.toHaveClass(/is-empty/);
  await expect(page.locator('#cover-preview-image')).toHaveAttribute('src', /^blob:/);
  await expect(page.locator('#cover-modal-confirm')).toBeEnabled();
  // Photo ratio: a 4:3 page; Document again: A4.
  await page.click('label:has(#cover-layout-photo)');
  await expect(page.locator('#cover-modal-meta')).toHaveText('210 × 157.7 mm · 1.332 : 1');
  await page.click('label:has(#cover-layout-fit)');
  await expect(page.locator('#cover-modal-meta')).toHaveText('A4 · 210 × 297 mm · 1 : 1.414');
  await page.click('#cover-modal-confirm');
  await expect(page.locator('#modal-cover-image')).not.toHaveClass(/visible/);
  await expect.poll(async () => (await doc(page)).pages.length).toBe(4);
  let d = await doc(page);
  const inserted = d.pages.find(p => !pagesBefore.includes(p.id));
  expect(inserted).toMatchObject({ w: 794, h: 1123, bg: 'custom', asset: true, template: 'Default' });
  await expect(page.locator('#btn-undo')).toBeDisabled();
  await expect(page.locator('#pages-list .page-item')).toHaveCount(4);

  // Photo ratio.
  await page.click('#btn-set-cover');
  await page.setInputFiles('#cover-image-input', { name: 'beach.jpg', mimeType: 'image/jpeg', buffer: photo });
  await page.click('label:has(#cover-layout-photo)');
  await page.click('#cover-modal-confirm');
  await expect.poll(async () => (await doc(page)).pages.length).toBe(5);
  d = await doc(page);
  expect(d.pages.filter(p => p.bg === 'custom').map(p => [p.w, p.h])).toContainEqual([794, 596]);
  // A non-image file is refused; Escape closes the modal.
  await page.click('#btn-set-cover');
  await page.setInputFiles('#cover-image-input', { name: 'notes.txt', mimeType: 'text/plain', buffer: Buffer.from('hi') });
  await page.waitForTimeout(300);
  expect(await cmd(page, 'panels.photo_page.state')).toMatchObject({ open: true, selected: false });
  await expect(page.locator('#cover-modal-confirm')).toBeDisabled();
  await page.keyboard.press('Escape');
  await expect(page.locator('#modal-cover-image')).not.toHaveClass(/visible/);

  // Import PDF: a plain PDF replaces every page (PDF backgrounds, source stored), undo cleared.
  await page.click('#panel-close');
  // The overlay stays on top while it fades out (same CSS as 5.18.0): draw once the page is under the pointer.
  await expect.poll(() => page.evaluate(() => document.elementFromPoint(560, 300)?.className)).toContain('page-canvas');
  await draw(page);
  await expect(page.locator('#btn-undo')).toBeEnabled();
  await page.setInputFiles('#pdf-input', path.join(V5, 'plain.pdf'));
  await expect.poll(async () => (await cmd(page, 'doc.import.state')).log.length).toBe(1);
  expect((await cmd(page, 'doc.import.state')).log[0]).toBe('PDF imported: 2 pages (as background)');
  d = await doc(page);
  expect(d.pages.map(p => [p.w, p.h, p.bg, p.asset])).toEqual([[595, 842, 'pdf:1', true], [595, 420, 'pdf:2', true]]);
  await expect(page.locator('.page-row')).toHaveCount(2);
  await expect(page.locator('#btn-undo')).toBeDisabled();
  await page.click('#btn-edit-pages');
  await expect(page.locator('#pages-list .page-item')).toHaveCount(2);
  await expect(page.locator('.page-item[data-page-index="0"] .page-item-main')).toHaveAttribute('title', 'Page 1 · 0 strokes · PDF');

  // A 5.x document: its pages and strokes.
  await page.setInputFiles('#pdf-input', { name: 'r0.pdf', mimeType: 'application/pdf', buffer: fs.readFileSync(path.join(V5, 'r0.pdf')) });
  await expect.poll(async () => (await cmd(page, 'doc.import.state')).log.length).toBe(2);
  const expected = JSON.parse(fs.readFileSync(path.join(V5, 'r0.expect.json'), 'utf8'));
  const strokes = expected.pages.reduce((n, p) => n + p.strokes.length, 0);
  expect((await cmd(page, 'doc.import.state')).log[1]).toBe(`PDF imported: ${expected.pageCount} pages, ${strokes} strokes restored`);
  d = await doc(page);
  expect(d.pages.length).toBe(expected.pageCount);
  expect(d.pages.reduce((n, p) => n + p.strokes.length, 0)).toBe(strokes);
  await expect.poll(async () => (await cmd(page, 'doc.session.state')).uncommitted).toBe(0);
  expect(problems).toEqual([]);
});
