// 04b-erase (TESTING §2 row 04, BUILD-PLAN W4.1): the stroke eraser mid-gesture (cursor on the overlay, the erased
// stroke gone from the bitmap before the lift, the model changed at the lift only), a fast swipe whose samples miss
// a thin stroke still erases it (B-E6), the area eraser splitting a pen stroke under a highlighter, and undo putting
// the stroke back at its original z (B-E4) — on the v6 fixture container of 04-ink.
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
const eraser = page => cmd(page, 'editor.eraser.state');

/** The page-1 bitmap (or an overlay) at page coordinates, as `r,g,b,a`. */
const pixel = (page, x, y, sel = 'canvas.page-canvas[data-page="0"]') => page.evaluate(([x, y, sel]) => {
  const c = document.querySelector(sel);
  const s = c.width / 794;
  return Array.from(c.getContext('2d').getImageData(Math.round(x * s), Math.round(y * s), 1, 1).data).join();
}, [x, y, sel]);

test('stroke and area erasers', async ({ page, context, mock, problems }) => {
  await seedSession(context, mock);
  await seedContainer(page, mock);
  await open(page, '/?drive=v6-doc&e2e=1');
  await opened(page, 'v6-doc');
  await settled(page);
  await expect.poll(async () => (await cmd(page, 'editor.pages.state')).rows.filter(r => r.active && r.drawn).length).toBeGreaterThan(0);
  await page.click('#mode-toggle-btn');
  await expect(page.locator('#editor-view')).not.toHaveClass(/read-only/);

  const box = await page.locator('canvas.page-canvas[data-page="0"]').boundingBox();
  const at = (x, y) => [box.x + x * box.width / 794, box.y + y * box.height / 1123];
  const cdp = await context.newCDPSession(page);
  const send = (type, p, buttons) => cdp.send('Input.dispatchMouseEvent', { type, x: p[0], y: p[1], button: 'left', buttons, clickCount: 1, pointerType: 'pen', force: 0.5 });
  const pen = async points => {
    await send('mousePressed', points[0], 1);
    for (const p of points.slice(1)) await send('mouseMoved', p, 1);
    await send('mouseReleased', points.at(-1), 0);
  };
  const line = (y, x0, x1) => Array.from({ length: (x1 - x0) / 10 + 1 }, (_, i) => at(x0 + i * 10, y));
  const blank = await pixel(page, 550, 300);
  const base = (await session(page)).live;

  // Two pen lines and a highlighter over the second.
  await pen(line(300, 500, 700));
  await pen(line(450, 500, 700));
  await page.click('#toolbar .tool-btn[data-tool="highlighter"]');
  await pen(line(455, 500, 700));
  await expect.poll(async () => (await session(page)).live).toBe(base + 3);
  expect(await pixel(page, 550, 300)).not.toBe(blank);

  // Stroke eraser: a swipe from 60 px above the first line to 60 px below it, in one sample (B-E6), paused.
  await page.click('#btn-eraser');
  expect((await cmd(page, 'editor.input.state')).tool).toBe('eraser-stroke');
  await send('mousePressed', at(550, 240), 1);
  await send('mouseMoved', at(550, 360), 1);
  await expect.poll(async () => (await eraser(page)).cut).toBe(1);
  await expect.poll(() => pixel(page, 550, 300)).toBe(blank);           // repainted before the lift
  expect((await session(page)).live).toBe(base + 3);                     // the model changes at the lift
  expect(await pixel(page, 550, 360, 'canvas.cursor-overlay[data-page="0"]')).not.toBe('0,0,0,0');
  await page.screenshot({ path: test.info().outputPath('stroke-erase-mid.png') });
  await send('mouseReleased', at(550, 360), 0);
  await expect.poll(async () => (await session(page)).live).toBe(base + 2);
  await expect.poll(() => pixel(page, 550, 360, 'canvas.cursor-overlay[data-page="0"]')).toBe('0,0,0,0');
  await page.click('#btn-undo');
  await expect.poll(async () => (await session(page)).live).toBe(base + 3);
  await expect.poll(() => pixel(page, 550, 300)).not.toBe(blank);

  // Area eraser across the pen line under the highlighter: two fragments, below the highlighter.
  await cmd(page, 'editor.eraser.mode', 'area');
  expect((await cmd(page, 'editor.input.state')).tool).toBe('eraser-area');
  const marked = await pixel(page, 600, 450);
  await send('mousePressed', at(600, 430), 1);
  await send('mouseMoved', at(600, 448), 1);
  await expect.poll(() => pixel(page, 600, 450)).not.toBe(marked);
  await send('mouseReleased', at(600, 448), 0);
  await expect.poll(async () => (await session(page)).live).toBe(base + 4);
  expect(await cmd(page, 'editor.history.state')).toMatchObject({ steps: 4, done: 4 });
  // Undo: the pen line is back whole, still under the highlighter (B-E4): the pixels equal those before the erase.
  await page.click('#btn-undo');
  await expect.poll(async () => (await session(page)).live).toBe(base + 3);
  await expect.poll(() => pixel(page, 600, 450)).toBe(marked);
  expect((await eraser(page)).commits).toBe(2);
  expect(problems).toEqual([]);
});
