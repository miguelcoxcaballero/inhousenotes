// The toolbar (BUILD-PLAN W4.4): the toolbar dragged to each snap position (left, right, bottom, free), the pen
// presets and the highlighter selected, the eraser menu (open, a size, the stroke toggle), Draw with finger in the
// editor's profile menu (and its `touchDrawEnabled` persistence), and the undo/redo buttons' states. The same mouse
// and CDP stylus events in both apps.
// Scenes are 5.x fixtures opened through the shared link, so v6 needs the 5.x import (W3.4) to show them.
import { parity } from '../run.mjs';
import { SAVE_DOT, VERSION } from '../support/editor.mjs';
import { ANA } from '../../mock/fixtures.mjs';

const doc = (t, extra = {}) => t.open({ account: ANA, scene: 'r0', ...extra });

/** Edit mode (the toolbar's flash finished). */
async function edit(t) {
  await t.click('#mode-toggle-btn', { force: true });
  await t.advance(1000);
}

/** Drags the toolbar by its handle to (x, y) (fractions of the window) and lets the snap animation finish. */
async function drag(t, fx, fy) {
  const box = await t.page.locator('#toolbar-handle').boundingBox();
  const size = t.page.viewportSize();
  const from = box ? { x: box.x + box.width / 2, y: box.y + box.height / 2 } : { x: 30, y: size.height / 2 };
  await t.page.mouse.move(from.x, from.y);
  await t.page.mouse.down();
  for (let i = 1; i <= 8; i++) {
    await t.page.mouse.move(from.x + (size.width * fx - from.x) * i / 8, from.y + (size.height * fy - from.y) * i / 8);
    await t.advance(16);
  }
  await t.page.mouse.up();
  await t.settle();
  await t.advance(600);
}

for (const [name, fx, fy] of [['left', 0.02, 0.5], ['right', 0.98, 0.5], ['bottom', 0.5, 0.97], ['free', 0.45, 0.45]]) {
  parity(`toolbar/snap-${name}`, {}, async t => {
    await doc(t);
    await edit(t);
    await drag(t, fx, fy);
  });
}

// The bottom-snapped (horizontal) toolbar opens the eraser menu above the button.
parity('toolbar/eraser-menu-horizontal', {}, async t => {
  await doc(t);
  await edit(t);
  await drag(t, 0.5, 0.97);
  await t.click('#btn-eraser', { force: true });
  await t.click('#btn-eraser', { force: true });
  await t.advance(400);
});

parity('toolbar/pen-presets', {}, async t => {
  await doc(t);
  await edit(t);
  await t.click('#toolbar .tool-btn[title="Black Pen"]', { force: true });
  await t.capture('black');
  await t.click('#toolbar .tool-btn[title="Red Pen"]', { force: true });
  await t.capture('red');
  await t.click('#toolbar .tool-btn[data-tool="highlighter"]', { force: true });
  await t.capture('highlighter');
});

// Desktop and tablet: on phones the eraser button of the toolbar's initial position is below the window (5.18.0);
// `eraser-menu-horizontal` covers phones.
parity('toolbar/eraser-menu', { only: { viewports: ['desktop-1280', 'desktop-1920', 'tablet-800'] } }, async t => {
  await doc(t);
  await edit(t);
  await t.click('#btn-eraser', { force: true });
  await t.capture('eraser');
  await t.click('#btn-eraser', { force: true });
  await t.advance(400);
  await t.capture('open');
  await t.click('#eraser-menu .size-btn[data-eraser-size="32"]', { force: true });
  await t.click('#eraser-menu label.eraser-toggle', { force: true });
  await t.advance(400);
  await t.capture('large-area');
  await t.page.mouse.click(t.page.viewportSize().width / 2, t.page.viewportSize().height * 0.4);
  await t.advance(400);
});

parity('toolbar/draw-with-finger', VERSION, async t => {
  await doc(t);
  await t.click('#btn-drive-profile-editor', { force: true });
  await t.advance(400);
  await t.capture('off');
  await t.click('#touch-draw-toggle', { force: true });
  await t.advance(400);
});

parity('toolbar/draw-with-finger-persisted', VERSION, async t => {
  await doc(t, { storage: { touchDrawEnabled: 'true' } });
  await t.click('#btn-drive-profile-editor', { force: true });
  await t.advance(400);
});

parity('toolbar/undo-redo-states', SAVE_DOT, async t => {
  await doc(t);
  await edit(t);
  await t.capture('empty');
  const box = await t.page.locator('canvas.page-canvas[data-page="0"]').boundingBox().catch(() => null)
    || { x: 0, y: 0, width: 794, height: 1123 };
  const at = (x, y) => ({ x: box.x + x * box.width / 794, y: box.y + y * box.height / 1123 });
  await t.pen(Array.from({ length: 12 }, (_, i) => ({ ...at(200 + i * 9, 300 + 20 * Math.sin(i / 3)), t: i * 8 })));
  await t.advance(400);
  await t.capture('undo-enabled');
  await t.click('#btn-undo', { force: true });
  await t.advance(400);
  await t.capture('redo-enabled');
  await t.click('#mode-toggle-btn', { force: true });
  await t.advance(400);
});
