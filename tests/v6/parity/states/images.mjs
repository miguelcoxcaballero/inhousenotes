// Images (TESTING §3.3 row "editor-tools", BUILD-PLAN W4.3): an image pasted at the pointer (inserted and selected,
// the lasso tool active), a drop of two images (the second 18 px further), and an image rotated with the selection
// box's handle (jsmath). The image bytes are a fixed PNG made here, so both apps decode the same pixels; the PNG is
// kept byte-exact by both (≤ 4 MiB, no scaling). Scenes are 5.x fixtures (v6 needs W3.4); the box is W4.2's.
import zlib from 'node:zlib';
import { parity } from '../run.mjs';
import { SAVE_DOT } from '../support/editor.mjs';
import { ANA } from '../../mock/fixtures.mjs';

const only = { viewports: ['desktop-1280', 'desktop-1920', 'tablet-800'] };
const doc = t => t.open({ account: ANA, scene: 'r0' });

/** A `w × h` RGB PNG: left half red, right half blue, a dark diagonal (so a rotation shows). */
function png(w, h) {
  const chunk = (type, data) => {
    const len = Buffer.alloc(4), crc = Buffer.alloc(4), body = Buffer.concat([Buffer.from(type, 'latin1'), data]);
    len.writeUInt32BE(data.length);
    crc.writeUInt32BE(zlib.crc32(body) >>> 0);
    return Buffer.concat([len, body, crc]);
  };
  const raw = Buffer.alloc((w * 3 + 1) * h);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const o = y * (w * 3 + 1) + 1 + x * 3;
      const diag = Math.abs(x * h - y * w) < 2 * w;
      const rgb = diag ? [20, 20, 20] : x < w / 2 ? [230, 40, 40] : [40, 70, 220];
      raw.set(rgb, o);
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr.set([8, 2, 0, 0, 0], 8);
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}
const IMAGE = png(240, 160).toString('base64');

async function edit(t) {
  await t.click('#mode-toggle-btn', { force: true });
  await t.advance(1000);
}

/** Page-1 coordinates → CSS px. */
async function mapper(t) {
  const box = await t.page.locator('canvas.page-canvas[data-page="0"]').boundingBox().catch(() => null)
    || { x: 0, y: 0, width: 794, height: 1123 };
  return (x, y) => ({ x: box.x + x * box.width / 794, y: box.y + y * box.height / 1123 });
}

/** A paste on the document, or a drop on #canvas-viewport at (x, y), carrying `n` copies of the image. */
async function deliver(t, kind, n, p = { x: 0, y: 0 }) {
  await t.page.evaluate(([kind, n, b64, x, y]) => {
    const dt = new DataTransfer();
    const bytes = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
    for (let i = 0; i < n; i++) dt.items.add(new File([bytes], `image-${i}.png`, { type: 'image/png' }));
    if (kind === 'paste') document.body.dispatchEvent(new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true }));
    else {
      const viewport = document.getElementById('canvas-viewport');
      viewport.dispatchEvent(new DragEvent('dragover', { dataTransfer: dt, clientX: x, clientY: y, bubbles: true, cancelable: true }));
      viewport.dispatchEvent(new DragEvent('drop', { dataTransfer: dt, clientX: x, clientY: y, bubbles: true, cancelable: true }));
    }
  }, [kind, n, IMAGE, p.x, p.y]);
  await t.advance(600);
}

/** The centre of an element (the window centre when it is missing). */
async function centre(t, selector) {
  const b = await t.page.locator(selector).first().boundingBox().catch(() => null) || { x: 400, y: 300, width: 0, height: 0 };
  return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
}

parity('images/pasted', { only, ...SAVE_DOT }, async t => {
  await doc(t);
  await edit(t);
  const at = await mapper(t);
  const p = at(320, 420);
  await t.page.mouse.move(p.x, p.y);
  await deliver(t, 'paste', 1);
});

parity('images/dropped-two', { only, ...SAVE_DOT }, async t => {
  await doc(t);
  await edit(t);
  await deliver(t, 'drop', 2, (await mapper(t))(500, 700));
});

parity('images/rotated', { only, ...SAVE_DOT }, async t => {
  await doc(t);
  await edit(t);
  await deliver(t, 'drop', 1, (await mapper(t))(400, 500));
  const r = await centre(t, '.selection-box[data-page="0"] .rotate-handle');
  const to = { x: r.x + 140, y: r.y + 90 };
  await t.pen(Array.from({ length: 9 }, (_, i) => ({ x: r.x + (to.x - r.x) * i / 8, y: r.y + (to.y - r.y) * i / 8, t: i * 8 })));
  await t.advance(400);
});
