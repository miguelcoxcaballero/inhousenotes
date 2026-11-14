// Selection (TESTING §3.3 row "editor-tools", BUILD-PLAN W4.2): the lasso paused mid-gesture, the selection box with
// its handles and outlines, a move paused mid-drag and lifted, a corner scale, a rotation (jsmath), the colour menu
// and a recolour, duplicate, delete, and a cross-page drag of an old stroke onto a page with a newer highlighter
// (drawn on top). The pen is a CDP stylus (`t.pen`) in both apps; the box is dragged with it too. Desktop and tablet
// only: on phones the lasso button of the toolbar's initial position is below the window in 5.18.0.
import { parity } from '../run.mjs';
import { SAVE_DOT } from '../support/editor.mjs';
import { ANA } from '../../mock/fixtures.mjs';

const only = { viewports: ['desktop-1280', 'desktop-1920', 'tablet-800'] };
const doc = t => t.open({ account: ANA, scene: 'r0' });

async function edit(t) {
  await t.click('#mode-toggle-btn', { force: true });
  await t.advance(1000);
}

/** Page `i` coordinates → CSS px. */
async function mapper(t, i = 0) {
  const box = await t.page.locator(`canvas.page-canvas[data-page="${i}"]`).boundingBox().catch(() => null)
    || { x: 0, y: 0, width: 794, height: 1123 };
  return (x, y) => ({ x: box.x + x * box.width / 794, y: box.y + y * box.height / 1123 });
}

/** The centre of an element (the window centre when it is missing). */
async function centre(t, selector) {
  const b = await t.page.locator(selector).first().boundingBox().catch(() => null) || { x: 400, y: 300, width: 0, height: 0 };
  return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
}

/** A straight pen path from a to b (CSS px) in `n` steps, 8 ms apart. */
const line = (a, b, n = 8) => Array.from({ length: n + 1 }, (_, i) => ({ x: a.x + (b.x - a.x) * i / n, y: a.y + (b.y - a.y) * i / n, t: i * 8 }));

/** A lasso around page rectangle (x0, y0)–(x1, y1): its corner points, 8 samples per side. */
async function lassoPath(t, x0, y0, x1, y1, i = 0) {
  const at = await mapper(t, i);
  const c = [at(x0, y0), at(x1, y0), at(x1, y1), at(x0, y1), at(x0, y0 + 6)];
  return c.slice(1).flatMap((p, k) => line(c[k], p).slice(k ? 1 : 0)).map((p, k) => ({ ...p, t: k * 8 }));
}

/** Edit mode, the lasso tool, strokes 1 and 2 of page 1 selected. */
async function selected(t) {
  await doc(t);
  await edit(t);
  await t.click('#toolbar .tool-btn[data-tool="lasso"]', { force: true });
  await t.pen(await lassoPath(t, 50, 75, 200, 190));
  await t.advance(400);
}

parity('selection/lasso-live', { only }, async t => {
  await doc(t);
  await edit(t);
  await t.click('#toolbar .tool-btn[data-tool="lasso"]', { force: true });
  await t.pen((await lassoPath(t, 50, 75, 200, 190)).slice(0, 20), { lift: false });
  await t.advance(32);
});

parity('selection/box', { only }, async t => {
  await selected(t);
});

parity('selection/move', { only, ...SAVE_DOT }, async t => {
  await selected(t);
  const at = await mapper(t);
  const path = line(at(110, 130), at(260, 330), 10);
  await t.pen(path, { lift: false });
  await t.advance(32);
  await t.capture('mid');
  await t.pen([path.at(-1)], { lift: true, press: false });   // the same contact lifts (no second press)
  await t.advance(400);
  await t.capture('lift');
});

parity('selection/scale', { only, ...SAVE_DOT }, async t => {
  await selected(t);
  const se = await centre(t, '.selection-box[data-page="0"] .handle-se');
  await t.pen(line(se, { x: se.x + 60, y: se.y + 40 }));
  await t.advance(400);
});

parity('selection/rotate', { only, ...SAVE_DOT }, async t => {
  await selected(t);
  const r = await centre(t, '.selection-box[data-page="0"] .rotate-handle');
  await t.pen(line(r, { x: r.x + 140, y: r.y + 90 }));
  await t.advance(400);
});

parity('selection/recolour', { only, ...SAVE_DOT }, async t => {
  await selected(t);
  await t.pen([{ ...(await centre(t, '.selection-box[data-page="0"] .selection-color-btn')), t: 0 }]);
  await t.advance(300);
  await t.capture('menu');
  await t.pen([{ ...(await centre(t, '.selection-box[data-page="0"] .selection-color-swatch[data-swatch-color="#E81010"]')), t: 0 }]);
  await t.advance(400);
  await t.capture('red');
});

parity('selection/duplicate-delete', { only, ...SAVE_DOT }, async t => {
  await selected(t);
  await t.pen([{ ...(await centre(t, '.selection-box[data-page="0"] .selection-duplicate')), t: 0 }]);
  await t.advance(400);
  await t.capture('duplicated');
  await t.pen([{ ...(await centre(t, '.selection-box[data-page="0"] .selection-trash')), t: 0 }]);
  await t.advance(400);
  await t.capture('deleted');
});

// Strokes 5 and 6 of page 1 (y ≈ 340–400) are older than a highlighter drawn on page 2; dragged 900 px down they
// cross page 1's bottom + 50 px and land on page 2 on top of it.
parity('selection/cross-page', { only, ...SAVE_DOT }, async t => {
  await doc(t);
  await edit(t);
  const c = await centre(t, '#canvas-viewport');
  await t.page.mouse.move(c.x, c.y);
  for (let i = 0; i < 12; i++) {
    await t.page.keyboard.down('Control');
    await t.page.mouse.wheel(0, 100);
    await t.page.keyboard.up('Control');
    await t.advance(16);
  }
  // Page 1's bottom and page 2's top in the window.
  for (let i = 0; i < 3; i++) { await t.page.mouse.wheel(0, 100); await t.advance(16); }
  await t.advance(800);
  await t.click('#toolbar .tool-btn[data-tool="highlighter"]', { force: true });
  const p2 = await mapper(t, 1);
  await t.pen(line(p2(60, 120), p2(260, 300), 12));
  await t.advance(400);
  await t.click('#toolbar .tool-btn[data-tool="lasso"]', { force: true });
  await t.pen(await lassoPath(t, 50, 320, 200, 420));
  await t.advance(400);
  const at = await mapper(t);
  await t.pen(line(at(110, 370), at(110, 1270), 18));
  await t.advance(400);
});
