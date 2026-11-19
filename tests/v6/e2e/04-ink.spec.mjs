// 04-ink (TESTING §2, BUILD-PLAN W3.5): the pen (CDP stylus input), the highlighter (mouse with "Draw with
// finger"), undo/redo through the buttons and Ctrl+Z / Ctrl+Y, and a reopen that shows the same strokes — on a v6
// container opened through the shared link (5.x fixtures need W3.4).
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

/** A stylus stroke through CDP: `points` in CSS px. */
async function pen(page, cdp, points) {
  const send = (type, p, buttons) => cdp.send('Input.dispatchMouseEvent', { type, x: p[0], y: p[1], button: 'left', buttons, clickCount: 1, pointerType: 'pen', force: 0.5 });
  await send('mousePressed', points[0], 1);
  for (const p of points.slice(1)) await send('mouseMoved', p, 1);
  await send('mouseReleased', points.at(-1), 0);
}
async function mouse(page, points) {
  await page.mouse.move(...points[0]);
  await page.mouse.down();
  for (const p of points.slice(1)) await page.mouse.move(...p, { steps: 2 });
  await page.mouse.up();
}
/** The page-1 bitmap at page coordinates (x, y), as `r,g,b,a`. */
const pixel = (page, x, y) => page.evaluate(([x, y]) => {
  const c = document.querySelector('canvas.page-canvas[data-page="0"]');
  const s = c.width / 794;
  return Array.from(c.getContext('2d').getImageData(Math.round(x * s), Math.round(y * s), 1, 1).data).join();
}, [x, y]);

test('pen, highlighter, undo/redo and reopen', async ({ page, context, mock, problems }) => {
  await seedSession(context, mock);
  await context.addInitScript(() => localStorage.setItem('touchDrawEnabled', 'true'));
  await seedContainer(page, mock);
  await open(page, '/?drive=v6-doc&e2e=1');
  await opened(page, 'v6-doc');
  await settled(page);
  await expect.poll(async () => (await state(page)).rows.filter(r => r.active && r.drawn).length).toBeGreaterThan(0);
  const base = (await session(page)).live;
  const blank = { pen: await pixel(page, 380, 430), marker: await pixel(page, 300, 605) };
  expect(await page.locator('#touch-draw-toggle').isChecked()).toBe(true);
  expect(await page.evaluate(() => document.querySelector('canvas.page-canvas').getContext('2d').getContextAttributes().desynchronized)).toBe(false);

  // Read mode: the pen does not draw.
  const box = await page.locator('canvas.page-canvas[data-page="0"]').boundingBox();
  const at = (x, y) => [box.x + x * box.width / 794, box.y + y * box.height / 1123];
  const cdp = await context.newCDPSession(page);
  await pen(page, cdp, [at(300, 200), at(400, 260)]);
  expect((await session(page)).live).toBe(base);
  await page.click('#mode-toggle-btn');
  await expect(page.locator('#editor-view')).not.toHaveClass(/read-only/);
  await expect(page.locator('#btn-undo')).toBeDisabled();

  // Pen: one stroke, drawn live on the page bitmap, one batch, one undo step.
  await pen(page, cdp, [at(300, 400), at(340, 420), at(380, 430), at(420, 470), at(460, 520)]);
  await expect.poll(async () => (await session(page)).live).toBe(base + 1);
  expect(await pixel(page, 380, 430)).not.toBe(blank.pen);
  expect(await history(page)).toMatchObject({ steps: 1, done: 1 });
  await expect(page.locator('#btn-undo')).toBeEnabled();
  await expect(page.locator('#btn-redo')).toBeDisabled();
  expect((await cmd(page, 'editor.input.state')).contact).toBe(false);

  // Highlighter with the mouse ("Draw with finger" on).
  await page.click('#toolbar .tool-btn[data-tool="highlighter"]');
  await expect(page.locator('#toolbar .tool-btn[data-tool="highlighter"]')).toHaveClass(/active/);
  await expect(page.locator('#toolbar .tool-btn[title="Blue Pen"]')).not.toHaveClass(/active/);
  await mouse(page, [at(200, 600), at(300, 605), at(400, 610), at(500, 600)]);
  await expect.poll(async () => (await session(page)).live).toBe(base + 2);
  expect(await pixel(page, 300, 605)).not.toBe(blank.marker);
  await page.screenshot({ path: test.info().outputPath('inked.png') });

  // Undo (button, then Ctrl+Z), redo (Ctrl+Y, then the button).
  await page.click('#btn-undo');
  await expect.poll(async () => (await session(page)).live).toBe(base + 1);
  await expect.poll(() => pixel(page, 300, 605)).toBe(blank.marker);
  await page.keyboard.press('Control+z');
  await expect.poll(async () => (await session(page)).live).toBe(base);
  await expect(page.locator('#btn-undo')).toBeDisabled();
  await page.keyboard.press('Control+y');
  await expect.poll(async () => (await session(page)).live).toBe(base + 1);
  await page.click('#btn-redo');
  await expect.poll(async () => (await session(page)).live).toBe(base + 2);
  await expect(page.locator('#btn-redo')).toBeDisabled();
  await expect.poll(() => pixel(page, 300, 605)).not.toBe(blank.marker);
  const before = await session(page);
  await expect.poll(async () => (await session(page)).uncommitted).toBe(0);

  // Reopen: the same document state, both strokes painted.
  await open(page, '/?drive=v6-doc&e2e=1');
  await opened(page, 'v6-doc');
  await settled(page);
  await expect.poll(async () => (await session(page)).live).toBe(base + 2);
  expect((await session(page)).snapshot).toBe(before.snapshot);
  await expect.poll(() => pixel(page, 380, 430)).not.toBe(blank.pen);
  await expect.poll(() => pixel(page, 300, 605)).not.toBe(blank.marker);
  expect(await history(page)).toMatchObject({ steps: 0, done: 0 });
  expect(problems).toEqual([]);
});

const state = page => cmd(page, 'editor.pages.state');
