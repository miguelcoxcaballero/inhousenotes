// Canvas scenes (TESTING §3.3 row "canvas"), the pixel side of W2.3's trace goldens, owned by W3.2: a dense page
// (2,400 strokes: the staged repaint, final frame) at 100 % and after a zoom, the scroll previews of a long dense
// document, template backgrounds (bullet, agenda, diary) at several backing scales, and a photo page.
// Scenes are 5.x fixtures opened through the shared link (v6 needs W3.4 to show them).
import { parity } from '../run.mjs';
import { ANA } from '../../mock/fixtures.mjs';

async function wheel(t, n, dy, { ctrl = false, settleMs = 1500 } = {}) {
  // The editor may not be on screen (an open that failed): the same events then go to the middle of the window.
  const box = await t.page.locator('#canvas-viewport').boundingBox() || { x: 0, y: 0, ...t.page.viewportSize() };
  await t.page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  if (ctrl) await t.page.keyboard.down('Control');
  for (let i = 0; i < n; i++) { await t.page.mouse.wheel(0, dy); await t.settle({ frames: false }); await t.advance(16); }
  if (ctrl) await t.page.keyboard.up('Control');
  if (settleMs) await t.advance(settleMs);
}
const open = (t, scene) => t.open({ account: ANA, scene, settleMs: 5000 });

// The dense page painted in slices on a processing canvas and copied at once; then re-rastered at 200 %.
parity('canvas/dense-page', {}, async t => {
  await open(t, 'dense-page');
  await t.advance(2000);
  await t.capture('repaint');
  await wheel(t, 12, -100, { ctrl: true });
  await t.capture('zoom-2');
});

// A long dense document scrolled down: released pages keep their previews, the next ones show scroll previews.
parity('canvas/scroll-previews', {}, async t => {
  await open(t, 'dense-20');
  await wheel(t, 40, 100);
  await t.capture('scrolled');
  await wheel(t, 6, -100, { ctrl: true });
  await t.capture('overview');
});

// Template backgrounds: bullet (template notebook), agenda and diary (A5) at 100 % and at 300 %.
parity('canvas/templates', {}, async t => {
  await open(t, 'agenda-diary');
  await t.capture('agenda-diary');
  await wheel(t, 19, -100, { ctrl: true });
  await t.capture('agenda-diary-3');
});
parity('canvas/template-bullet', {}, async t => {
  await open(t, 'template');
  await t.capture('bullet');
  await wheel(t, 7, -100, { ctrl: true });
  await t.capture('bullet-1.5');
});

// A photo page: the image stretched over the page under the ink.
parity('canvas/photo-page', {}, async t => { await open(t, 'photo-page'); });
