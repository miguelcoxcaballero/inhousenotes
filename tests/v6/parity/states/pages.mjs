// The Manage-Pages panel (TESTING §3.3 row "pages", BUILD-PLAN W5.1): the panel open in Read and Edit mode with its
// thumbnails, a drag reorder captured mid-drag and dropped, the delete slider at 0 / 50 / 100 % and after the
// deletion, the photo page modal (empty, a photo chosen, "Photo ratio", the page added) and Scan → Soon.
// Scenes are 5.x fixtures opened through the shared link.
import zlib from 'node:zlib';
import { parity } from '../run.mjs';
import { ANA } from '../../mock/fixtures.mjs';

const doc = t => t.open({ account: ANA, scene: 'template' });

/** Edit mode (the toolbar's flash finished). */
async function edit(t) {
  // Phones: the floating toggle group lies over the button, so a pointer click misses it in both apps.
  await t.page.evaluate(() => document.getElementById('mode-toggle-btn').click());
  await t.settle();
  await t.advance(1000);
}
async function openPanel(t) {
  await t.click('#btn-edit-pages', { force: true });
  await t.advance(600);
}

/** Moves the held mouse from (x0, y0) to (x1, y1) in `n` steps, one frame each. */
async function glide(t, x0, y0, x1, y1, n = 8) {
  for (let i = 1; i <= n; i++) {
    await t.page.mouse.move(x0 + (x1 - x0) * i / n, y0 + (y1 - y0) * i / n);
    await t.advance(16);
  }
}

/** A deterministic 320×200 PNG: two colour bands and a diagonal. */
function photo() {
  const w = 320, h = 200, raw = Buffer.alloc((w * 3 + 1) * h);
  for (let y = 0; y < h; y++) {
    raw[y * (w * 3 + 1)] = 0;
    for (let x = 0; x < w; x++) {
      const o = y * (w * 3 + 1) + 1 + x * 3, on = Math.abs(x * h / w - y) < 4;
      raw[o] = on ? 20 : (y < h / 2 ? 240 : 40); raw[o + 1] = on ? 20 : 120; raw[o + 2] = on ? 20 : (y < h / 2 ? 60 : 220);
    }
  }
  const table = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
  const crc = b => { let c = 0xffffffff; for (const x of b) c = table[(c ^ x) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
  const chunk = (type, data) => { const len = Buffer.alloc(4), body = Buffer.concat([Buffer.from(type, 'latin1'), data]), sum = Buffer.alloc(4); len.writeUInt32BE(data.length); sum.writeUInt32BE(crc(body)); return Buffer.concat([len, body, sum]); };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw, { level: 9 })), chunk('IEND', Buffer.alloc(0))]);
}
const PHOTO = photo();

parity('pages/panel-read', {}, async t => {
  await doc(t);
  await openPanel(t);
});

parity('pages/panel-edit', {}, async t => {
  await doc(t);
  await edit(t);
  await openPanel(t);
});

// Page 2 dragged above page 1: the ghost and the insertion line mid-drag, then the new order.
parity('pages/drag-reorder', {}, async t => {
  await doc(t);
  await edit(t);
  await openPanel(t);
  const handle = await t.page.locator('.page-item[data-page-index="1"] .page-drag-handle').boundingBox();
  const first = await t.page.locator('.page-item[data-page-index="0"]').boundingBox();
  if (!handle || !first) return;
  const [x0, y0] = [handle.x + handle.width / 2, handle.y + handle.height / 2];
  await t.page.mouse.move(x0, y0);
  await t.page.mouse.down();
  await glide(t, x0, y0, x0 + 20, first.y + 30);
  await t.capture('mid-drag');
  await t.page.mouse.up();
  await t.advance(400);
  await t.capture('dropped');
});

// The slider of "Delete page?": at rest, half way, at the end (held), then the page deleted.
parity('pages/delete-slider', {}, async t => {
  await doc(t);
  await edit(t);
  await openPanel(t);
  await t.click('.page-item[data-page-index="1"] .page-action-btn.delete');
  await t.advance(400);
  await t.capture('0');
  const handle = await t.page.locator('#confirm-slider-handle').boundingBox();
  const track = await t.page.locator('#confirm-slider').boundingBox();
  if (!handle || !track) return;
  const [x0, y0] = [handle.x + handle.width / 2, handle.y + handle.height / 2];
  const end = track.x + track.width - 4 - handle.width / 2;
  await t.page.mouse.move(x0, y0);
  await t.page.mouse.down();
  await glide(t, x0, y0, x0 + (end - x0) / 2, y0);
  await t.capture('50');
  await glide(t, x0 + (end - x0) / 2, y0, end + 10, y0);
  await t.capture('100');
  await t.page.mouse.up();
  await t.advance(800);
  await t.capture('deleted');
});

// The photo modal: empty, a photo chosen (Document), Photo ratio, then the page added.
parity('pages/photo-modal', {}, async t => {
  await doc(t);
  await edit(t);
  await openPanel(t);
  await t.click('#btn-set-cover');
  await t.advance(400);
  await t.capture('empty');
  await t.page.setInputFiles('#cover-image-input', { name: 'stripes.png', mimeType: 'image/png', buffer: PHOTO });
  await t.settle();
  await t.advance(400);
  await t.capture('document');
  await t.click('label[for="cover-layout-photo"]');
  await t.advance(400);
  await t.capture('photo-ratio');
  await t.click('#cover-modal-confirm');
  await t.settle();
  await t.advance(1200);
  await t.capture('added');
});

parity('pages/scan-soon', {}, async t => {
  await doc(t);
  await edit(t);
  await openPanel(t);
  await t.click('#btn-scan-page');
  await t.advance(300);
});
