// 06-images (TESTING §2, BUILD-PLAN W4.3): paste and drop images on a v6 container opened through the shared
// link, the placement rules, a PNG kept byte-exact (asset = SHA-256 of the file) and stored once by hash, a JPEG
// re-encoded, a transform to a rotated image, undo/redo, and a reopen that paints the same images.
import crypto from 'node:crypto';
import zlib from 'node:zlib';
import { test, expect, seedSession, ANA } from '../mock/fixtures.mjs';

test.use({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 } });

const ASSET = '6a'.repeat(32);
const PNG1 = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64');

/** A three-page PDF with `/Keywords` and embedded files (as 04-ink). */
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
  object(3, `<< /Title (Cuaderno Digital) /Keywords (${keywords}) >>`);
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
const opened = page => expect.poll(async () => (await cmd(page, 'doc.open.state')).log.some(e => e[0] === 'v6-doc' && e[1] === true), { timeout: 15_000 }).toBe(true);
const settled = page => expect.poll(async () => !(await cmd(page, 'doc.open.state')).running, { timeout: 15_000 }).toBe(true);
const session = page => cmd(page, 'doc.session.state');
const images = page => cmd(page, 'editor.images.list');
const imgState = page => cmd(page, 'editor.images.state');
const drawn = page => expect.poll(async () => (await cmd(page, 'editor.pages.state')).rows.filter(r => r.active && r.drawn).length).toBeGreaterThan(0);
/** The page-1 bitmap at page coordinates (x, y), as `r,g,b`. */
const pixel = (page, x, y) => page.evaluate(([x, y]) => {
  const c = document.querySelector('canvas.page-canvas[data-page="0"]');
  const s = c.width / 794;
  return Array.from(c.getContext('2d').getImageData(Math.round(x * s), Math.round(y * s), 1, 1).data).slice(0, 3).join();
}, [x, y]);

/** JPEG blue (the encoder may round 255 to 254). */
const blue = async (page, x, y) => { const [r, g, b] = (await pixel(page, x, y)).split(',').map(Number); return r < 40 && g < 40 && b > 215; };

/** A `w × h` image of one colour, encoded by the page's canvas → base64. */
const makeImage = (page, w, h, color, type) => page.evaluate(async ([w, h, color, type]) => {
  const c = new OffscreenCanvas(w, h);
  const ctx = c.getContext('2d');
  ctx.fillStyle = color;
  ctx.fillRect(0, 0, w, h);
  const blob = await c.convertToBlob({ type, quality: 0.9 });
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let s = '';
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s);
}, [w, h, color, type]);

/** Dispatches a paste (on the document) or a drop (on #canvas-viewport at x, y) carrying files. */
const deliver = (page, kind, files, x = 0, y = 0) => page.evaluate(([kind, files, x, y]) => {
  const dt = new DataTransfer();
  for (const f of files) {
    const bytes = Uint8Array.from(atob(f.b64), c => c.charCodeAt(0));
    dt.items.add(new File([bytes], f.name, { type: f.type }));
  }
  if (kind === 'paste') {
    const e = new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true });
    document.body.dispatchEvent(e);
    return e.defaultPrevented;
  }
  const viewport = document.getElementById('canvas-viewport');
  const over = new DragEvent('dragover', { dataTransfer: dt, clientX: x, clientY: y, bubbles: true, cancelable: true });
  viewport.dispatchEvent(over);
  const drop = new DragEvent('drop', { dataTransfer: dt, clientX: x, clientY: y, bubbles: true, cancelable: true });
  viewport.dispatchEvent(drop);
  return [over.defaultPrevented, drop.defaultPrevented];
}, [kind, files, x, y]);

const sha = b64 => crypto.createHash('sha256').update(Buffer.from(b64, 'base64')).digest('hex');

test('paste, drop, place, transform, undo, reopen', async ({ page, context, mock, problems }) => {
  await seedSession(context, mock);
  await open(page, '/?e2e=1');
  const snapshot = Buffer.from(await cmd(page, 'doc.session.fixture'), 'base64');
  const data = containerPdf({ keywords: `STROKES_Z:!;IHN6:${zlib.deflateSync(snapshot).toString('base64')}`, attachments: [{ name: `IHN6-${ASSET}`, data: PNG1 }] });
  mock.drive.addFile({ id: 'v6-doc', name: 'Fixture.pdf', owner: ANA, data, modifiedTime: '2026-10-29T09:00:00.000Z' });
  await open(page, '/?drive=v6-doc&e2e=1');
  await opened(page);
  await settled(page);
  await drawn(page);
  const base = await session(page);
  const before = (await images(page)).length;
  const red = await makeImage(page, 200, 100, '#ff0000', 'image/png');
  const jpg = await makeImage(page, 300, 300, '#0000ff', 'image/jpeg');

  // Read mode: a paste is not taken.
  const box = await page.locator('.page-wrapper[data-page="0"]').boundingBox();
  const at = (x, y) => [box.x + x * box.width / 794, box.y + y * box.height / 1123];
  await page.mouse.move(...at(300, 400));
  expect(await deliver(page, 'paste', [{ b64: red, name: 'red.png', type: 'image/png' }])).toBe(false);
  await page.click('#mode-toggle-btn');
  await expect(page.locator('#editor-view')).not.toHaveClass(/read-only/);

  // Paste at the last pointer position: the PNG is kept byte-exact (asset = SHA-256 of the file).
  await page.mouse.move(...at(300, 400));
  expect(await deliver(page, 'paste', [{ b64: red, name: 'red.png', type: 'image/png' }])).toBe(true);
  await expect.poll(async () => (await imgState(page)).added).toBe(1);
  let list = await images(page);
  expect(list.length).toBe(before + 1);
  const pasted = list.at(-1);
  expect(pasted).toMatchObject({ page: 0, asset: sha(red), w: 200, h: 100, rotation: 0 });
  expect(pasted.cx).toBeCloseTo(300, 0);
  expect(pasted.cy).toBeCloseTo(400, 0);
  await expect.poll(async () => (await session(page)).assets).toBe(base.assets + 1);
  await expect.poll(() => pixel(page, 300, 400)).toBe('255,0,0');
  await expect(page.locator('#toolbar .tool-btn[data-tool="lasso"]')).toHaveClass(/active/);
  await expect(page.locator('#btn-undo')).toBeEnabled();

  // Drop the same PNG and a JPEG on page 1: two images, one new asset (the PNG is stored once by its hash).
  const [x, y] = at(500, 700);
  expect(await deliver(page, 'drop', [{ b64: red, name: 'again.png', type: 'image/png' }, { b64: jpg, name: 'blue.jpg', type: 'image/jpeg' }], x, y)).toEqual([true, true]);
  await expect.poll(async () => (await imgState(page)).added).toBe(3);
  list = await images(page);
  const [again, jpeg] = list.slice(-2);
  expect(again).toMatchObject({ page: 0, asset: sha(red), w: 200, h: 100 });
  expect(again.cx).toBeCloseTo(500, 0);
  expect(jpeg).toMatchObject({ page: 0, w: 300, h: 300 });
  expect(jpeg.asset).not.toBe(sha(jpg));
  expect(jpeg.cx).toBeCloseTo(518, 0);
  expect(jpeg.cy).toBeCloseTo(718, 0);
  await expect.poll(async () => (await session(page)).assets).toBe(base.assets + 2);
  await expect.poll(() => blue(page, 518, 718)).toBe(true);

  // Transform the newest image (the selection box's commit): half size, rotated a quarter turn, moved.
  expect(await cmd(page, 'editor.images.transform', { sx: 0.5, sy: 0.5, rot: Math.PI / 4, tx: -200, ty: 0 })).toBe(true);
  list = await images(page);
  expect(list.at(-1)).toMatchObject({ w: 150, h: 150 });
  expect(list.at(-1).rotation).toBeCloseTo(Math.PI / 4, 12);
  expect(list.at(-1).cx).toBeCloseTo(318, 0);
  await expect.poll(() => blue(page, 518, 718)).toBe(false);
  await expect.poll(() => blue(page, 318, 718)).toBe(true);
  // A corner of the unrotated square is outside the rotated one.
  expect(await blue(page, 318 + 70, 718 - 70)).toBe(false);
  await page.screenshot({ path: test.info().outputPath('images.png') });

  // Undo the transform and the JPEG, redo both.
  await page.click('#btn-undo');
  await expect.poll(() => blue(page, 518, 718)).toBe(true);
  await page.keyboard.press('Control+z');
  await expect.poll(async () => (await images(page)).length).toBe(before + 2);
  await page.keyboard.press('Control+y');
  await page.keyboard.press('Control+y');
  await expect.poll(async () => (await images(page)).at(-1).rotation).toBeCloseTo(Math.PI / 4, 12);
  await expect.poll(async () => (await session(page)).uncommitted).toBe(0);
  const final = await session(page);
  const finalList = await images(page);

  // Reopen: the same document, the images painted from the stored assets.
  await open(page, '/?drive=v6-doc&e2e=1');
  await opened(page);
  await settled(page);
  await drawn(page);
  expect((await session(page)).snapshot).toBe(final.snapshot);
  expect(await images(page)).toEqual(finalList);
  expect((await session(page)).missing).toBe(0);
  await expect.poll(() => pixel(page, 300, 400)).toBe('255,0,0');
  await expect.poll(() => blue(page, 318, 718)).toBe(true);
  expect(problems).toEqual([]);
});
