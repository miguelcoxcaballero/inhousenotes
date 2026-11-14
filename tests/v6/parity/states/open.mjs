// Opening a document (TESTING §3.3 rows "open" and "profile-menu — editor menu with doc stats"), owned by W3.1: the
// loading overlay while Drive has not answered, the opened document's shell (viewer banner, save indicator, editor
// menu with the statistics line) and a failing open. Scenes are 5.x fixtures opened through the shared link
// (`t.open({scene})`, TESTING §3.1), so every state that shows an opened document also needs the 5.x import
// (W3.4) and the editor view (W3.2); until those merge, only the states before the document appears and the
// failing open can be pixel-equal (docs/v6/NOTES-W3.1.md lists what was measured).
import { parity } from '../run.mjs';
import { ANA, BEN } from '../../mock/fixtures.mjs';

const VERSION = { tags: ['VERSION-LABEL'], expected: t => t.page.evaluate(() => [...document.querySelectorAll('[data-app-version]')].map(n => n.getBoundingClientRect()).filter(r => r.width).map(r => ({ x: r.x, y: r.y, width: r.width, height: r.height }))) };
// Drive answers the link's `id,name,resourceKey` lookup, then never the document's metadata nor its content.
const held = mock => mock.inject({ host: 'www.googleapis.com', path: /\/drive\/v3\/files\/scene-/, query: /capabilities|alt=media/, times: Infinity, hold: true });

// The shared link of a signed-in user, Drive silent: the editor behind the dark overlay, `Opening PDF...`.
parity('open/loading', { drive: held }, async t => {
  await t.open({ account: ANA, scene: 'template', settleMs: 1500 });
});

// The same in the Android app and with the system in dark mode.
parity('open/loading-app', { systemDark: true, app: true, drive: held }, async t => {
  await t.open({ account: ANA, scene: 'template', settleMs: 1500 });
});

// The overlay fading out once the document is ready (0.3 s transition, sampled in the middle and after it).
parity('open/loaded', {}, async t => {
  await t.open({ account: ANA, scene: 'template', settleMs: 0, ready: false });
  await t.advance(2000);
  await t.capture('settled');
});

// The save indicator of a document that was just opened (`saved`), and the save-details popup over it.
parity('open/save-details', { tags: ['N-2'] }, async t => {
  await t.open({ account: ANA, scene: 'template' });
  await t.click('.mode-toggle-status');
  await t.advance(300);
});

// The editor profile menu: account, night toggle and the statistics line `N pages · M strokes · size`.
parity('open/editor-menu', { ...VERSION }, async t => {
  await t.open({ account: ANA, scene: 'strokes-data' });
  await t.click('#btn-drive-profile-editor');
  await t.advance(300);
});

// A reader of somebody else's document: the View Only banner, no mode toggle.
parity('open/viewer-banner', {}, async t => {
  await t.open({ account: BEN, scene: 'template', sceneOptions: { permissions: [{ type: 'user', role: 'reader', emailAddress: BEN }] } });
});

// Drive refuses the document: back on Home with `Could not open this document. Try again.`
parity('open/failed', { drive: mock => mock.inject({ host: 'www.googleapis.com', path: /\/drive\/v3\/files\/scene-/, times: Infinity, status: 404, reason: 'notFound' }) }, async t => {
  await t.open({ account: ANA, scene: 'template' });
  await t.advance(1000);
});
