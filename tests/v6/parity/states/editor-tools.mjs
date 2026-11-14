// Editor tools (TESTING §3.3 row "editor-tools"), W3.5's states: live pen frames after 1 / 5 / 20 samples, the
// lift, the highlighter live and committed, undo and redo, and a single-tap pen dot (B-E3: v6 keeps it in later
// repaints). The pen is a CDP stylus (`t.pen`), the same events in both apps. W4.x add their own state files.
// Scenes are 5.x fixtures opened through the shared link, so v6 needs the 5.x import (W3.4) to show them.
import { parity } from '../run.mjs';
import { ANA } from '../../mock/fixtures.mjs';

const doc = t => t.open({ account: ANA, scene: 'r0' });

/** Edit mode (the toolbar's flash finished). */
async function edit(t) {
  await t.click('#mode-toggle-btn', { force: true });   // phones: the floating toggle bar fails the actionability check in both apps
  await t.advance(1000);
}

/** Page-1 coordinates → CSS px (the window centre when the editor did not open). */
async function mapper(t) {
  const box = await t.page.locator('canvas.page-canvas[data-page="0"]').boundingBox().catch(() => null)
    || { x: 0, y: 0, width: 794, height: 1123 };
  return (x, y) => ({ x: box.x + x * box.width / 794, y: box.y + y * box.height / 1123 });
}

/** `n` samples of a handwriting-like curve from page (200, 300), 8 ms apart. */
async function curve(t, n, { x0 = 200, y0 = 300 } = {}) {
  const at = await mapper(t);
  return Array.from({ length: n }, (_, i) => ({ ...at(x0 + i * 9, y0 + 22 * Math.sin(i / 3)), t: i * 8 }));
}

for (const n of [1, 5, 20]) {
  parity(`editor-tools/pen-live-${n}`, {}, async t => {
    await doc(t);
    await edit(t);
    await t.pen(await curve(t, n), { lift: false });
    await t.advance(32);
  });
}

parity('editor-tools/pen-lift', {}, async t => {
  await doc(t);
  await edit(t);
  await t.pen(await curve(t, 20));
  await t.advance(400);
});

parity('editor-tools/highlighter', {}, async t => {
  await doc(t);
  await edit(t);
  await t.click('#toolbar .tool-btn[data-tool="highlighter"]', { force: true });
  await t.pen(await curve(t, 24, { y0: 420 }), { lift: false });
  await t.advance(32);
  await t.capture('live');
  await t.pen([(await curve(t, 25, { y0: 420 })).at(-1)], { lift: true });
  await t.advance(400);
  await t.capture('committed');
});

parity('editor-tools/undo-redo', {}, async t => {
  await doc(t);
  await edit(t);
  await t.pen(await curve(t, 12));
  await t.pen(await curve(t, 12, { y0: 520 }));
  await t.advance(400);
  await t.click('#btn-undo', { force: true });
  await t.advance(400);
  await t.capture('undone');
  await t.click('#btn-redo', { force: true });
  await t.advance(400);
  await t.capture('redone');
});

// A tap: the dot is drawn live in both apps; after a repaint (zoom in and back, the 90 ms resync) 5.18.0 drops
// it and v6 keeps it.
parity('editor-tools/single-tap-dot', {
  tags: ['B-E3'],
  expected: async t => {
    const p = (await mapper(t))(300, 300);
    return [{ x: p.x - 6, y: p.y - 6, width: 12, height: 12 }];
  }
}, async t => {
  await doc(t);
  await edit(t);
  const at = await mapper(t);
  await t.pen([{ ...at(300, 300), t: 0 }]);
  await t.advance(400);
  await t.capture('live');
  const c = at(400, 500);
  await t.page.mouse.move(c.x, c.y);
  await t.page.keyboard.down('Control');
  await t.page.mouse.wheel(0, -100);
  await t.advance(16);
  await t.page.mouse.wheel(0, 100);
  await t.page.keyboard.up('Control');
  await t.advance(800);
});
