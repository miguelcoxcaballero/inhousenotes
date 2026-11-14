// Export (TESTING §3.3 row "export"), W5.4's states: the export modal with the name prefilled and selected, and the
// save indicator once both apps settled after an edit (5.18.0: the PDF upload; v6: the relay seal, N-1 — the
// Alone relay flush runs 5 s after the last batch, so the state advances past it).
import { parity } from '../run.mjs';
import { ANA } from '../../mock/fixtures.mjs';

const doc = t => t.open({ account: ANA, scene: 'r0' });

async function edit(t) {
  await t.click('#mode-toggle-btn', { force: true });
  await t.advance(1000);
}

async function write(t) {
  const box = await t.page.locator('canvas.page-canvas[data-page="0"]').boundingBox().catch(() => null) || { x: 0, y: 0, width: 794, height: 1123 };
  const at = (x, y) => ({ x: box.x + x * box.width / 794, y: box.y + y * box.height / 1123 });
  await t.pen(Array.from({ length: 16 }, (_, i) => ({ ...at(200 + i * 9, 300 + 22 * Math.sin(i / 3)), t: i * 8 })));
}

parity('export/saved-after-edit', {}, async t => {
  await doc(t);
  await edit(t);
  await write(t);
  await t.advance(6000);
});

parity('export/modal', {}, async t => {
  await doc(t);
  await edit(t);
  // The button sits in the pages panel; both apps take the same click.
  await t.page.locator('#btn-export-pdf').dispatchEvent('click');
  await t.settle();
  await t.advance(32);
});
