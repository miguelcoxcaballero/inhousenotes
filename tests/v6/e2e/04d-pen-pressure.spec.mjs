// 04d-pen-pressure (W7): pen ink width follows the stylus pressure (a filled outline), constant pressure keeps the
// chosen width, and the S Pen side button erases while held without changing the toolbar tool — on the v6 fixture
// container of 04-ink. Screenshots of the writing are attached to the test output.
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
const history = page => cmd(page, 'editor.history.state');
const state = page => cmd(page, 'editor.pages.state');

/** A stylus stroke through CDP: `points` are [x, y, force] in CSS px; `buttons` 3 = tip + side button. */
async function pen(cdp, points, buttons = 1) {
  const send = (type, p, b) => cdp.send('Input.dispatchMouseEvent', { type, x: p[0], y: p[1], button: 'left', buttons: b, clickCount: 1, pointerType: 'pen', force: p[2] ?? 0.5 });
  await send('mousePressed', points[0], buttons);
  for (const p of points.slice(1)) await send('mouseMoved', p, buttons);
  await send('mouseReleased', points.at(-1), 0);
}

/** Handwriting-like loops: `n` loops from (x, y), `force(t)` along the stroke (t in 0..1). */
function loops(at, x, y, n, force) {
  const out = [];
  const steps = 40 * n;
  for (let i = 0; i <= steps; i++) {
    const t = i / steps, a = t * n * 2 * Math.PI;
    out.push([...at(x + t * n * 46 + 14 * Math.sin(a), y - 22 * Math.sin(a / 2) ** 2 + 8 * Math.cos(a)), force(t)]);
  }
  return out;
}

/** Width of the ink on page-1's bitmap along the vertical line x (page px): the count of inked pixels. */
const inkHeight = (page, x, y0, y1) => page.evaluate(([x, y0, y1]) => {
  const c = document.querySelector('canvas.page-canvas[data-page="0"]');
  const s = c.width / 794;
  const data = c.getContext('2d').getImageData(Math.round(x * s), Math.round(y0 * s), 1, Math.round((y1 - y0) * s)).data;
  let n = 0;
  for (let i = 0; i < data.length; i += 4) if (data[i] < 160 && data[i + 3] > 128) n++;
  return n / s;
}, [x, y0, y1]);

test('pen width follows pressure; the side button erases', async ({ page, context, mock, problems }) => {
  await seedSession(context, mock);
  await seedContainer(page, mock);
  await open(page, '/?drive=v6-doc&e2e=1');
  await opened(page, 'v6-doc');
  await settled(page);
  await expect.poll(async () => (await state(page)).rows.filter(r => r.active && r.drawn).length).toBeGreaterThan(0);
  await page.click('#mode-toggle-btn');
  await expect(page.locator('#editor-view')).not.toHaveClass(/read-only/);
  const base = (await session(page)).live;
  const box = await page.locator('canvas.page-canvas[data-page="0"]').boundingBox();
  const at = (x, y) => [box.x + x * box.width / 794, box.y + y * box.height / 1123];
  const cdp = await context.newCDPSession(page);

  // Straight lines pressed lightly, normally and hard (a real stylus never reports one exact value: ±0.05; a
  // stroke whose pressure does not change at all is drawn at the chosen width, like mouse, touch and old data);
  // and one that presses harder along its length.
  const line = (y, f) => Array.from({ length: 29 }, (_, i) => [...at(120 + i * 10, y), f + 0.05 * Math.sin(i)]);
  await pen(cdp, line(260, 0.12));
  await pen(cdp, line(300, 0.45));
  await pen(cdp, line(340, 0.9));
  const ramp = Array.from({ length: 30 }, (_, i) => [...at(120 + i * 10, 400), 0.05 + 0.95 * i / 29]);
  await pen(cdp, ramp);
  await expect.poll(async () => (await session(page)).live).toBe(base + 4);
  const [light, mid, heavy] = [await inkHeight(page, 260, 250, 270), await inkHeight(page, 260, 290, 310), await inkHeight(page, 260, 330, 350)];
  expect(light).toBeLessThan(mid);
  expect(mid).toBeLessThan(heavy);
  expect(await inkHeight(page, 150, 390, 410)).toBeLessThan(await inkHeight(page, 390, 390, 410));
  // Constant pressure (mouse, touch, old data) keeps the chosen width (the black pen: 2 px).
  await pen(cdp, [[...at(120, 450), 0.5], [...at(400, 450), 0.5]]);
  const before = await inkHeight(page, 260, 440, 460);
  expect(before).toBeGreaterThan(1.2);
  expect(before).toBeLessThan(3.2);

  // Handwriting: the same loops with natural pressure (light at the ends) and with a constant one.
  const natural = t => 0.15 + 0.6 * Math.sin(Math.PI * t) ** 0.7;
  await pen(cdp, loops(at, 110, 560, 6, natural));
  await pen(cdp, loops(at, 110, 660, 6, () => 0.5));
  await expect.poll(async () => (await session(page)).live).toBe(base + 7);
  await page.screenshot({ path: test.info().outputPath('pressure.png'), clip: { x: at(90, 220)[0], y: at(90, 220)[1], width: box.width * 420 / 794, height: box.height * 480 / 1123 } });

  // S Pen side button: pressed in the air (a pointerdown with buttons 2 and no pressure), then the tip touches (a
  // chorded move, buttons 3), sweeps across the light line, lifts (buttons 2) and the button is released.
  const tool = await cmd(page, 'editor.input.state');
  const mouse = (type, p, button, buttons, force) => cdp.send('Input.dispatchMouseEvent', { type, x: p[0], y: p[1], button, buttons, clickCount: 1, pointerType: 'pen', force });
  await mouse('mouseMoved', at(260, 230), 'none', 0, 0);
  await mouse('mousePressed', at(260, 230), 'right', 2, 0);
  expect((await cmd(page, 'editor.input.state')).contact).toBe(false);
  await mouse('mousePressed', at(260, 240), 'left', 3, 0.5);
  for (let y = 245; y <= 280; y += 5) await mouse('mouseMoved', at(260, y), 'left', 3, 0.5);
  expect((await cmd(page, 'editor.input.state')).contact).toBe(true);
  await mouse('mouseReleased', at(260, 280), 'left', 2, 0);
  await mouse('mouseReleased', at(260, 280), 'right', 0, 0);
  await expect.poll(() => inkHeight(page, 260, 250, 270)).toBe(0);
  expect((await cmd(page, 'editor.input.state')).tool).toBe(tool.tool);
  expect((await cmd(page, 'editor.input.state')).contact).toBe(false);
  await page.screenshot({ path: test.info().outputPath('side-button.png'), clip: { x: at(90, 220)[0], y: at(90, 220)[1], width: box.width * 420 / 794, height: box.height * 280 / 1123 } });
  // Released: the pen writes again.
  await pen(cdp, [[...at(480, 300), 0.5], [...at(700, 300), 0.5]]);
  await expect.poll(() => inkHeight(page, 600, 290, 310)).toBeGreaterThan(1);
  expect(problems).toEqual([]);
});
