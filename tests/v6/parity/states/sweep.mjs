// Cross-area sequences (TESTING §3.3 row "sweep"), owned by W6.1: a document opened, left for Home and reopened;
// Night mode switched with the editor open; the window resized phone ↔ desktop with a menu open. No edits are made,
// so nothing is uploaded and both apps settle without a save in flight (N-1).
import { parity } from '../run.mjs';
import { ANA } from '../../mock/fixtures.mjs';

const card = id => `[data-drive-item-id="${id}"]`;

// Open → Home (the document's card in Recents) → reopen from that card.
parity('sweep/reopen', {}, async t => {
  await t.open({ account: ANA, scene: 'template' });
  await t.click('#btn-drive-home svg');
  await t.advance(3000);
  await t.capture('home');
  await t.click(`#drive-recents ${card('scene-template')}`);
  await t.advance(3000);
});

// Night mode from the editor's profile menu; the menu is closed with Escape.
parity('sweep/theme-editor', {}, async t => {
  await t.open({ account: ANA, scene: 'template' });
  await t.click('#btn-drive-profile-editor');
  await t.advance(300);
  await t.click('label[for="theme-toggle-editor"]');
  await t.advance(400);
  await t.capture('menu');
  await t.press('Escape');
  await t.advance(400);
});

// Home's profile menu open while the window changes between phone and desktop widths.
parity('sweep/resize-menu', { only: { viewports: ['desktop-1280', 'phone-360'] } }, async t => {
  await t.open({ account: ANA });
  await t.click('#btn-drive-profile');
  await t.advance(300);
  const { width, height } = t.page.viewportSize();
  await t.page.setViewportSize(width > 700 ? { width: 360, height: 800 } : { width: 1280, height: 800 });
  await t.settle();
  await t.advance(500);
  await t.capture('resized');
  await t.page.setViewportSize({ width, height });
  await t.settle();
  await t.advance(500);
});
