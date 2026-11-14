// 04c-select (TESTING §2 row 04, BUILD-PLAN W4.2): the lasso, the selection box and its handles (move, scale,
// rotate, previewed until the lift), recolour, duplicate, delete, an image in the lasso and a cross-page drag of an
// old stroke onto a page with a newer highlighter (drawn on top) — on the v6 fixture container.
import zlib from 'node:zlib';
import { test, expect, seedSession, ANA } from '../mock/fixtures.mjs';

test.use({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 } });

const ASSET = '6a'.repeat(32);
// A 1×1 PNG: the bytes of the photo asset.
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64');

/** A three-page PDF with `/Keywords` and embedded files, written object by object. */
function containerPdf({ keywords, attachments = [], pages = 3 }) {
  const chunks = [Buffer.from('%PDF-1.7\n%\xe2\xe3\xcf\xd3\n', 'latin1')], offsets = [];
  let size = chunks[0].length;
  const push = buffer => { chunks.push(buffer); size += buffer.length; };
  const object = (number, body, stream) => {
    offsets[number] = size;
    push(Buffer.from(`${number} 0 obj\n${body}\n`, 'latin1'));
    if (stream) { push(Buffer.from('stream\n', 'latin1')); push(stream); push(Buffer.from('\nendstream\n', 'latin1')); }
    push(Buffer.from('endobj\n', 'latin1'));
  };
  const first = 4, files = first + pages;
  const names = attachments.map((a, i) => `(${a.name}) ${files + i * 2} 0 R`).join(' ');
  object(1, `<< /Type /Catalog /Pages 2 0 R${attachments.length ? ` /Names << /EmbeddedFiles << /Names [${names}] >> >>` : ''} >>`);
  object(2, `<< /Type /Pages /Kids [${Array.from({ length: pages }, (_, i) => `${first + i} 0 R`).join(' ')}] /Count ${pages} >>`);
  object(3, `<< /Title (Cuaderno Digital) /Subject (Notas con S-Pen) /Creator (Cuaderno Digital App) /Keywords (${keywords}) >>`);
  for (let i = 0; i < pages; i++) object(first + i, '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595.5 842.25] >>');
  attachments.forEach((a, i) => {
    object(files + i * 2, `<< /Type /Filespec /F (${a.name}) /UF (${a.name}) /EF << /F ${files + i * 2 + 1} 0 R >> >>`);
    object(files + i * 2 + 1, `<< /Type /EmbeddedFile /Length ${a.data.length} >>`, a.data);
  });
  const count = files + attachments.length * 2, xref = size;
  let table = `xref\n0 ${count}\n0000000000 65535 f \n`;
  for (let n = 1; n < count; n++) table += `${String(offsets[n]).padStart(10, '0')} 00000 n \n`;
  push(Buffer.from(`${table}trailer\n<< /Size ${count} /Root 1 0 R /Info 3 0 R >>\nstartxref\n${xref}\n%%EOF\n`, 'latin1'));
  return Buffer.concat(chunks);
}

const cmd = (page, name, args) => page.evaluate(([n, a]) => window.__IHN_TEST_API__.cmd(n, a), [name, args]);
async function open(page, url) {
  await page.goto(url);
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  expect(await page.evaluate(() => window.__IHN_TEST_API__.ready())).toBe(true);
}
const opened = (page, id, ok = true) => expect.poll(async () => (await cmd(page, 'doc.open.state')).log.some(e => e[0] === id && e[1] === ok), { timeout: 15_000 }).toBe(true);
const settled = page => expect.poll(async () => { const s = await cmd(page, 'doc.open.state'); return !s.running; }, { timeout: 15_000 }).toBe(true);
/** The fixture document as a v6 container in Ana's Drive. */
async function seedContainer(page, mock, { id = 'v6-doc', permissions = [] } = {}) {
  await open(page, '/?e2e=1');
  const snapshot = Buffer.from(await cmd(page, 'doc.session.fixture'), 'base64');
  const data = containerPdf({ keywords: `STROKES_Z:!;IHN6:${zlib.deflateSync(snapshot).toString('base64')}`, attachments: [{ name: `IHN6-${ASSET}`, data: PNG }] });
  mock.drive.addFile({ id, name: 'Fixture.pdf', owner: ANA, data, permissions, modifiedTime: '2026-10-29T09:00:00.000Z' });
  return { snapshot, size: data.length };
}


const session = page => cmd(page, 'doc.session.state');
const sel = page => cmd(page, 'editor.selection.state');
const history = page => cmd(page, 'editor.history.state');

/** Page `i` coordinates → CSS px (read now: the zoom and pan move the canvas). */
async function mapper(page, i = 0) {
  const box = await page.locator(`canvas.page-canvas[data-page="${i}"]`).boundingBox();
  return (x, y) => [box.x + x * box.width / 794, box.y + y * box.height / 1123];
}
async function mouse(page, points, steps = 3) {
  await page.mouse.move(...points[0]);
  await page.mouse.down();
  for (const p of points.slice(1)) await page.mouse.move(...p, { steps });
  await page.mouse.up();
}
/** A lasso around page rectangle (x0, y0)–(x1, y1) of page `i`. */
async function lasso(page, x0, y0, x1, y1, i = 0) {
  const at = await mapper(page, i);
  await mouse(page, [at(x0, y0), at(x1, y0), at(x1, y1), at(x0, y1), at(x0, y0 + 5)]);
}
/** Page `i`'s bitmap at page coordinates (x, y), as `r,g,b,a`. */
const pixel = (page, x, y, i = 0) => page.evaluate(([x, y, i]) => {
  const c = document.querySelector(`canvas.page-canvas[data-page="${i}"]`);
  const s = c.width / 794;
  return Array.from(c.getContext('2d').getImageData(Math.round(x * s), Math.round(y * s), 1, 1).data).join();
}, [x, y, i]);
const box = i => `.selection-box[data-page="${i}"]`;

test('lasso, move, scale, rotate, recolour, duplicate, delete, cross-page drag', async ({ page, context, mock, problems }) => {
  await seedSession(context, mock);
  await context.addInitScript(() => localStorage.setItem('touchDrawEnabled', 'true'));
  await seedContainer(page, mock);
  await open(page, '/?drive=v6-doc&e2e=1');
  await opened(page, 'v6-doc');
  await settled(page);
  await page.click('#mode-toggle-btn');
  await expect(page.locator('#editor-view')).not.toHaveClass(/read-only/);
  const base = (await session(page)).live;

  // Lasso around the fixture's pen stroke (100,100)…(300,260): the box, its handles and the outlines.
  await page.click('#toolbar .tool-btn[data-tool="lasso"]');
  await lasso(page, 80, 80, 320, 280);
  await expect.poll(() => sel(page)).toMatchObject({ page: 0, strokes: 1, images: 0, bounds: [100, 100, 300, 260] });
  await expect(page.locator(box(0))).not.toHaveClass(/hidden/);
  await expect(page.locator(box(0))).toHaveClass(/has-selection/);
  await expect(page.locator(`canvas.lasso-overlay[data-page="0"]`)).toBeVisible();
  const style = await page.locator(box(0)).getAttribute('style');
  expect(style).toContain('left: 100px; top: 100px; width: 200px; height: 160px;');

  // Move by (+50, +40): previewed while dragging, one batch and one undo step at the lift.
  let at = await mapper(page);
  await page.mouse.move(...at(200, 180));
  await page.mouse.down();
  await page.mouse.move(...at(250, 220), { steps: 4 });
  await expect.poll(async () => (await sel(page)).transforming).toBe(true);
  await expect.poll(async () => (await sel(page)).bounds).toEqual([150, 140, 350, 300]);
  expect((await sel(page)).commits).toBe(0);
  await page.mouse.up();
  await expect.poll(() => sel(page)).toMatchObject({ transforming: false, commits: 1, bounds: [150, 140, 350, 300] });
  expect(await history(page)).toMatchObject({ steps: 1, done: 1 });

  // Scale from the south-east corner (uniform), rotate from the rotate handle.
  const se = await page.locator(`${box(0)} .handle-se`).boundingBox();
  await mouse(page, [[se.x + se.width / 2, se.y + se.height / 2], [se.x + se.width / 2 + 40, se.y + se.height / 2 + 32]]);
  await expect.poll(async () => (await sel(page)).commits).toBe(2);
  const scaled = (await sel(page)).bounds;
  expect(scaled[2] - scaled[0]).toBeGreaterThan(200);
  const rot = await page.locator(`${box(0)} .rotate-handle`).boundingBox();
  const c = [(scaled[0] + scaled[2]) / 2, (scaled[1] + scaled[3]) / 2];
  at = await mapper(page);
  await mouse(page, [[rot.x + rot.width / 2, rot.y + rot.height / 2], at(c[0] + 200, c[1])]);
  await expect.poll(async () => (await sel(page)).commits).toBe(3);
  expect((await sel(page)).bounds).not.toEqual(scaled);

  // Recolour: the menu, then the red swatch.
  await page.click(`${box(0)} .selection-color-btn`);
  await expect(page.locator(`${box(0)} .selection-color-menu`)).toHaveClass(/open/);
  // (The menu opens above the box, here under the top bar: the pointerdown goes to the swatch itself.)
  await page.locator(`${box(0)} .selection-color-swatch[data-swatch-color="#E81010"]`).dispatchEvent('pointerdown', { pointerId: 1, pointerType: 'mouse', isPrimary: true });
  await expect.poll(async () => (await sel(page)).commits).toBe(4);
  await expect(page.locator(`${box(0)} .selection-color-menu`)).not.toHaveClass(/open/);
  expect((await sel(page)).strokes).toBe(1);

  // Duplicate (+18, +18), then delete the copy.
  const b = (await sel(page)).bounds;
  await page.click(`${box(0)} .selection-duplicate`);
  await expect.poll(async () => (await session(page)).live).toBe(base + 1);
  expect((await sel(page)).bounds).toEqual(b.map(v => v + 18));
  await page.click(`${box(0)} .selection-trash`);
  await expect.poll(async () => (await session(page)).live).toBe(base);
  await expect.poll(async () => (await sel(page)).page).toBe(-1);
  await expect(page.locator(box(0))).toHaveClass(/hidden/);
  expect(await history(page)).toMatchObject({ steps: 6, done: 6 });

  // The fixture image is selected by the lasso too.
  await lasso(page, 280, 520, 520, 680);
  await expect.poll(() => sel(page)).toMatchObject({ images: 1, strokes: 0 });
  await expect(page.locator(box(0))).toHaveClass(/has-image/);

  // Cross-page drag at 50 % zoom: an old stroke near page 1's bottom dropped on page 2 over a newer highlighter
  // draws on top of it.
  await page.mouse.move(640, 400);
  for (let i = 0; i < 12; i++) { await page.keyboard.down('Control'); await page.mouse.wheel(0, 100); await page.keyboard.up('Control'); }
  await expect.poll(async () => (await cmd(page, 'editor.viewport.state')).zoom).toBe(0.5);
  // Pan so that page 1's bottom is in the middle of the window.
  for (let i = 0; i < 4; i++) await page.mouse.wheel(0, 100);
  await expect.poll(async () => (await page.locator('canvas.page-canvas[data-page="1"]').boundingBox())?.y ?? 9999).toBeLessThan(560);
  await page.waitForTimeout(300);
  await expect.poll(async () => (await cmd(page, 'editor.pages.state')).rows.filter(r => r.active).length).toBeGreaterThan(1);
  await page.click('#toolbar .tool-btn[title="Blue Pen"]');
  at = await mapper(page);
  await mouse(page, [at(150, 1060), at(250, 1060), at(350, 1060), at(450, 1060), at(550, 1060)], 6);
  await page.click('#toolbar .tool-btn[data-tool="highlighter"]');
  const at2 = await mapper(page, 1);
  await mouse(page, [at2(450, 10), at2(450, 150), at2(450, 300)], 6);
  await expect.poll(async () => (await session(page)).live).toBe(base + 2);
  await page.click('#toolbar .tool-btn[data-tool="lasso"]');
  await lasso(page, 120, 1030, 600, 1100);
  await expect.poll(() => sel(page)).toMatchObject({ page: 0, strokes: 1 });
  const steps = (await history(page)).done;
  const inked = await pixel(page, 300, 1060, 0);
  at = await mapper(page);
  await page.mouse.move(...at(350, 1060));
  await page.mouse.down();
  await page.mouse.move(...at(350, 1260), { steps: 8 });
  await page.mouse.up();
  await expect.poll(async () => (await sel(page)).page).toBe(1);
  const moved = await sel(page);
  expect(moved.bounds[1]).toBeGreaterThan(0);
  expect(moved.bounds[1]).toBeLessThan(300);
  await expect(page.locator(box(1))).not.toHaveClass(/hidden/);
  await expect(page.locator(box(0))).toHaveClass(/hidden/);
  expect((await session(page)).live).toBe(base + 2);
  // Pen pixels: the same where it crosses the highlighter as where it does not.
  const y = moved.bounds[1];
  await expect.poll(() => pixel(page, 450, y, 1)).toBe(await pixel(page, 300, y, 1));
  expect((await pixel(page, 300, y, 1)).split(',').map(Number)[2]).toBeGreaterThan(150);
  await page.screenshot({ path: test.info().outputPath('crossed.png') });
  // Undo back to the drag's start: the stroke is on page 1 again.
  expect((await history(page)).done).toBeGreaterThan(steps);
  while ((await history(page)).done > steps) await page.click('#btn-undo');
  await expect.poll(async () => (await sel(page)).page).toBe(-1);
  await expect.poll(() => pixel(page, 300, 1060, 0)).toBe(inked);
  await page.screenshot({ path: test.info().outputPath('selection.png') });
  expect(problems).toEqual([]);
});
