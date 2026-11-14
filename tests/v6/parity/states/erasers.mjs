// Erasers (TESTING §3.3 row "editor-tools", BUILD-PLAN W4.1): the stroke eraser paused mid-gesture (cursor on the
// overlay, the damage already repainted) and after the lift, the area eraser mid-gesture and after the lift, undo of
// an area erase (B-E4: v6 restores the pen under the newer highlighter), and a fast swipe whose samples miss a pen
// line (B-E6: v6 erases it). Desktop and tablet only: on phones the eraser button of the toolbar's initial position
// is below the window in 5.18.0 (see toolbar.mjs). Area mode is chosen in the eraser menu (W4.4).
import { parity } from '../run.mjs';
import { SAVE_DOT, rules } from '../support/editor.mjs';
import { ANA } from '../../mock/fixtures.mjs';

const only = { viewports: ['desktop-1280', 'desktop-1920', 'tablet-800'] };
const doc = t => t.open({ account: ANA, scene: 'r0' });

async function edit(t) {
  await t.click('#mode-toggle-btn', { force: true });
  await t.advance(1000);
}

async function mapper(t) {
  const box = await t.page.locator('canvas.page-canvas[data-page="0"]').boundingBox().catch(() => null)
    || { x: 0, y: 0, width: 794, height: 1123 };
  return (x, y) => ({ x: box.x + x * box.width / 794, y: box.y + y * box.height / 1123 });
}

/** `n` samples of a handwriting-like curve from page (x0, y0), 8 ms apart. */
async function curve(t, n, { x0 = 200, y0 = 300 } = {}) {
  const at = await mapper(t);
  return Array.from({ length: n }, (_, i) => ({ ...at(x0 + i * 9, y0 + 22 * Math.sin(i / 3)), t: i * 8 }));
}

/** A straight eraser path from page (x0, y0) to (x1, y1) in `n` steps, 8 ms apart. */
async function path(t, [x0, y0], [x1, y1], n) {
  const at = await mapper(t);
  return Array.from({ length: n + 1 }, (_, i) => ({ ...at(x0 + (x1 - x0) * i / n, y0 + (y1 - y0) * i / n), t: i * 8 }));
}

/** A pen curve with a highlighter over it (page y ≈ 420). */
async function inked(t) {
  await doc(t);
  await edit(t);
  await t.pen(await curve(t, 20, { y0: 420 }));
  await t.click('#toolbar .tool-btn[data-tool="highlighter"]', { force: true });
  await t.pen(await curve(t, 20, { y0: 426 }));
  await t.advance(400);
}

async function areaMode(t) {
  await t.click('#btn-eraser', { force: true });
  await t.click('#btn-eraser', { force: true });
  await t.click('#eraser-menu label.eraser-toggle', { force: true });
  await t.advance(400);
}

parity('erasers/stroke', { only, ...SAVE_DOT }, async t => {
  await inked(t);
  await t.click('#btn-eraser', { force: true });
  await t.pen(await path(t, [290, 380], [300, 470], 12), { lift: false });
  await t.advance(32);
  await t.capture('mid');
  await t.pen([(await path(t, [300, 470], [300, 470], 1))[0]], { lift: true });
  await t.advance(400);
  await t.capture('lift');
});

parity('erasers/area', { only, ...SAVE_DOT }, async t => {
  await inked(t);
  await areaMode(t);
  await t.pen(await path(t, [300, 380], [300, 470], 12), { lift: false });
  await t.advance(32);
  await t.capture('mid');
  await t.pen([(await path(t, [300, 470], [300, 470], 1))[0]], { lift: true });
  await t.advance(400);
  await t.capture('lift');
});

/** The pen curve's band: where 5.18.0 draws the restored pen over the highlighter. */
const band = async t => {
  const at = await mapper(t), a = at(190, 390), b = at(390, 460);
  return [{ x: a.x, y: a.y, width: b.x - a.x, height: b.y - a.y }];
};

parity('erasers/area-undo', { only, ...rules({ tags: ['B-E4'], expected: band }, SAVE_DOT) }, async t => {
  await inked(t);
  await areaMode(t);
  await t.pen(await path(t, [300, 380], [300, 470], 12));
  await t.advance(400);
  await t.click('#btn-undo', { force: true });
  await t.advance(400);
});

// Two samples 60 px apart across a pen line: 5.18.0 tests the samples only and misses it; v6 sweeps between them.
parity('erasers/fast-swipe', { only, ...rules({ tags: ['B-E6'], expected: band }, SAVE_DOT) }, async t => {
  await doc(t);
  await edit(t);
  await t.pen(await curve(t, 20, { y0: 420 }));
  await t.click('#btn-eraser', { force: true });
  await t.pen(await path(t, [300, 380], [300, 470], 1));
  await t.advance(400);
});
