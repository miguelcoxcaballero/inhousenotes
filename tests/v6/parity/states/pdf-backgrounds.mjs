// PDF backgrounds (W3.3, TESTING §3.3 rows "open — PDF-background doc" and "canvas"): pdf.js rasters of the active
// pages at 100 % and 200 %, a pan in progress (input within 240 ms: rasters wait, previews stay) and after it settles,
// and agenda/diary template pages scrolled through. Scenes are 5.x fixtures opened through the shared link.
import { parity } from '../run.mjs';
import { ANA } from '../../mock/fixtures.mjs';

async function wheel(t, n, dy, { ctrl = false, settleMs = 1500 } = {}) {
  const box = await t.page.locator('#canvas-viewport').boundingBox() || { x: 0, y: 0, ...t.page.viewportSize() };
  await t.page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  if (ctrl) await t.page.keyboard.down('Control');
  for (let i = 0; i < n; i++) { await t.page.mouse.wheel(0, dy); await t.settle({ frames: false }); await t.advance(16); }
  if (ctrl) await t.page.keyboard.up('Control');
  if (settleMs) await t.advance(settleMs);
}
const open = (t, scene) => t.open({ account: ANA, scene, settleMs: 5000 });

// An imported PDF with ink over it: the source pages rendered by pdf.js under the strokes, then re-shown at 200 %.
parity('pdf-backgrounds/zoom', {}, async t => {
  await open(t, 'imported-overlays');
  await t.capture('zoom-1');
  await wheel(t, 12, -100, { ctrl: true });
  await t.capture('zoom-2');
});

// A plain PDF (A4 + half-height page) panned: mid-pan nothing new is rendered, after it the rasters arrive.
parity('pdf-backgrounds/pan', {}, async t => {
  await open(t, 'plain');
  await wheel(t, 6, 100, { settleMs: 0 });
  await t.advance(100);
  await t.capture('during');
  await t.advance(2000);
  await t.capture('after');
});

// Agenda and diary template pages scrolled into view (their SVG templates, not pdf.js).
parity('pdf-backgrounds/agenda-diary', {}, async t => {
  await open(t, 'agenda-diary');
  await wheel(t, 10, 100);
  await t.capture('scrolled');
});
