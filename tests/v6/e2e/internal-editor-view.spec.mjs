// internal-editor-view (BUILD-PLAN W3.2): the editor view against the real host — page rows, at most three active
// canvases with 5.18.0's backing sizes, zoom and pan through the wheel, the resolution resync after the gesture,
// Read/Edit, pull-to-add-page and the fast-scroll thumb — on a v6 container opened through the shared link (the
// container is built as in internal-doc.spec.mjs; 5.x fixtures need W3.4).
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
const media = mock => mock.calls.filter(call => /alt=media/.test(call.url)).length;
/** The fixture document as a v6 container in Ana's Drive. */
async function seedContainer(page, mock, { id = 'v6-doc', permissions = [] } = {}) {
  await open(page, '/?e2e=1');
  const snapshot = Buffer.from(await cmd(page, 'doc.session.fixture'), 'base64');
  const data = containerPdf({ keywords: `STROKES_Z:!;IHN6:${zlib.deflateSync(snapshot).toString('base64')}`, attachments: [{ name: `IHN6-${ASSET}`, data: PNG }] });
  mock.drive.addFile({ id, name: 'Fixture.pdf', owner: ANA, data, permissions, modifiedTime: '2026-10-29T09:00:00.000Z' });
  return { snapshot, size: data.length };
}


const state = page => cmd(page, 'editor.pages.state');
const view = page => cmd(page, 'editor.viewport.state');

test('rows, active canvases, zoom resync, read/edit and pull-to-add', async ({ page, context, mock, problems }) => {
  await seedSession(context, mock);
  await seedContainer(page, mock);
  await open(page, '/?drive=v6-doc&e2e=1');
  await opened(page, 'v6-doc');
  await settled(page);
  await expect(page.locator('.page-row')).toHaveCount(3);
  await expect(page.locator('#doc-title')).toHaveText('Fixture');
  await expect(page.locator('#editor-view')).toHaveClass(/read-only/);
  await expect(page.locator('#mode-toggle-label')).toHaveText('Read');
  await expect.poll(async () => (await state(page)).rows.filter(r => r.active && r.drawn).length).toBeGreaterThan(0);
  const s = await state(page);
  expect(s.rows.filter(r => r.active).length).toBeLessThanOrEqual(3);
  expect(s.rows[0]).toMatchObject({ active: true, px: [1588, 2246], scale: 2, lasso: [1072, 1516], cursor: [993, 1404] });
  await expect(page.locator('canvas.page-canvas[data-page="0"]')).toHaveAttribute('data-has-page-draw', '1');
  const v = await view(page);
  expect(v.zoom).toBe(1);
  expect(v.panY).toBe(20);
  // Centred column: (1280 - 794) / 2 - 30.
  expect(v.panX).toBe(213);
  await expect(page.locator('#canvas-container')).toHaveAttribute('style', /translate\(213px, 20px\) scale\(1\)/);
  await page.screenshot({ path: test.info().outputPath('opened.png') });

  // Ctrl+wheel zooms about the mouse; after the gesture the active canvases are re-rastered at the new scale.
  await page.mouse.move(640, 400);
  for (let i = 0; i < 12; i++) { await page.keyboard.down('Control'); await page.mouse.wheel(0, -100); await page.keyboard.up('Control'); }
  await expect.poll(async () => (await view(page)).zoom).toBeGreaterThan(1.9);
  await expect.poll(async () => (await state(page)).rows.find(r => r.active).scale, { timeout: 5000 }).toBeGreaterThan(3.9);
  await page.screenshot({ path: test.info().outputPath('zoomed.png') });
  // Back to 0.5: clamped.
  for (let i = 0; i < 40; i++) { await page.keyboard.down('Control'); await page.mouse.wheel(0, 100); await page.keyboard.up('Control'); }
  await expect.poll(async () => (await view(page)).zoom).toBe(0.5);
  // Page 1 and its neighbour are active (directly visible + 1); the canvas is back at the 100 % backing size.
  await expect.poll(async () => (await state(page)).rows.filter(r => r.active).length).toBe(2);
  await expect.poll(async () => (await state(page)).rows[0].scale).toBe(2);

  // Edit mode: the toggle, then a long scroll down past the end pulls a new page in.
  await page.click('#mode-toggle-btn');
  await expect(page.locator('#editor-view')).not.toHaveClass(/read-only/);
  await expect(page.locator('#mode-toggle-label')).toHaveText('Edit');
  await page.mouse.move(640, 400);
  let pending = false;
  for (let i = 0; i < 40 && !pending; i++) { await page.mouse.wheel(0, 120); pending = (await view(page)).pendingCreate; }
  expect(pending).toBe(true);
  await expect(page.locator('#add-page-indicator')).toHaveCSS('opacity', /^(1|0\.\d+)$/);
  await expect(page.locator('.page-row')).toHaveCount(4, { timeout: 5000 });
  await expect(page.locator('.page-wrapper[data-page="3"] .page-number')).toHaveText('4');
  expect((await cmd(page, 'doc.session.state')).pages).toBe(4);
  await page.screenshot({ path: test.info().outputPath('added.png') });
  expect(problems).toEqual([]);
});
