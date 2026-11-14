// The editor view (TESTING §3.3 row "editor-view"), owned by W3.2: an opened document at zoom 0.5 / 1 / 2 / 3, the
// pan clamps at both ends, Read/Edit, pull-to-add (indicator at its thresholds, then the new page) and the viewer
// without toolbar. Zoom and pan use the wheel over the page column (Ctrl for zoom), the same events in both apps.
// Scenes are 5.x fixtures opened through the shared link, so v6 needs the 5.x import (W3.4) to show them.
import { parity } from '../run.mjs';
import { ANA, BEN } from '../../mock/fixtures.mjs';

/** Wheel `n` times at the centre of the page column (`ctrl`: zoom), then lets the gesture settle and re-raster. */
async function wheel(t, n, dy, { ctrl = false, settleMs = 600 } = {}) {
  // The editor may not be on screen (an open that failed): the same events then go to the middle of the window.
  const box = await t.page.locator('#canvas-viewport').boundingBox() || { x: 0, y: 0, ...t.page.viewportSize() };
  await t.page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  if (ctrl) await t.page.keyboard.down('Control');
  for (let i = 0; i < n; i++) { await t.page.mouse.wheel(0, dy); await t.settle({ frames: false }); await t.advance(16); }
  if (ctrl) await t.page.keyboard.up('Control');
  if (settleMs) await t.advance(settleMs);
}
const doc = (t, scene = 'r0', extra = {}) => t.open({ account: ANA, scene, ...extra });

// Two pages at 100 %: the column centred, the top at 20 px, the fast-scroll thumb at the top.
parity('editor-view/zoom-1', {}, async t => { await doc(t); });

// Ctrl+wheel about the centre: 0.94^12 (clamped to 0.5), 1.06^12 ≈ 2.01, 1.06^19 (clamped to 3).
parity('editor-view/zoom-0.5', {}, async t => { await doc(t); await wheel(t, 12, 100, { ctrl: true }); });
parity('editor-view/zoom-2', {}, async t => { await doc(t); await wheel(t, 12, -100, { ctrl: true }); });
parity('editor-view/zoom-3', {}, async t => { await doc(t); await wheel(t, 19, -100, { ctrl: true }); });

// Scrolled far past both ends in Read mode: 5 px of the column stay visible at the top; the bottom stops at
// `viewport − 100 − content height` (no overscroll in Read mode).
parity('editor-view/pan-clamps', {}, async t => {
  await doc(t);
  await wheel(t, 30, -100);
  await t.capture('top');
  await wheel(t, 60, 100);
  await t.capture('bottom');
});

// Read → Edit: the toggle, the enabled toolbar (its flash animation finished).
parity('editor-view/edit-mode', {}, async t => {
  await doc(t);
  await t.click('#mode-toggle-btn', { force: true });   // phones: the floating toggle bar fails the actionability check in both apps
  await t.advance(1000);
});

// Edit mode, wheel past the end: the `+` indicator fading in with the overscroll (captured before the 300 ms
// release), then the new page after it (fade-in finished).
parity('editor-view/pull-to-add', {}, async t => {
  await doc(t);
  await t.click('#mode-toggle-btn', { force: true });   // phones: the floating toggle bar fails the actionability check in both apps
  await t.advance(1000);
  await wheel(t, 24, 100, { settleMs: 0 });
  await t.capture('pulling');
  await t.advance(1500);
  await t.capture('added');
});

// A reader of somebody else's document: no toolbar, no mode toggle, the banner.
parity('editor-view/viewer', {}, async t => {
  await doc(t, 'r0', { account: BEN, sceneOptions: { permissions: [{ type: 'user', role: 'reader', emailAddress: BEN }] } });
});
