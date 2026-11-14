// Home dialogs (TESTING §3.3 row "home-dialogs"), W5.5's states: create menu, folder picker (root and nested, move
// with the file's path), new folder, card ⋮ menus, colour popup, move-to-bin slider at 0/50/100 %, Bin with its item
// menu, Download app (web), and the Offline documents dialog with nothing pending (`Up to date`).
import { parity } from '../run.mjs';
import { fixtures, ANA } from '../../mock/fixtures.mjs';

const home = async t => { await t.open({ account: ANA }); };
const card = id => `[data-drive-item-id="${id}"]`;
// The open profile menu's version label is the one intended difference (v5.18.0 → v6.0.0), as in profile-menu.mjs.
const VERSION = { tags: ['VERSION-LABEL'], expected: t => t.page.evaluate(() => [...document.querySelectorAll('[data-app-version]')].map(n => n.getBoundingClientRect()).filter(r => r.width).map(r => ({ x: r.x, y: r.y, width: r.width, height: r.height }))) };
const OPEN_MS = 400; // .modal-overlay 0.2 s, .modal 0.3 s
const tab = async (t, name) => { await t.click(`#home-nav [data-home-tab="${name}"]`); await t.advance(400); };
const more = async (t, grid, id) => { await t.click(`${grid} ${card(id)} .drive-card-more`); await t.advance(100); };

parity('home-dialogs/create-menu', { drive: fixtures.library }, async t => {
  await home(t);
  await t.click('#btn-drive-new');
  await t.advance(100);
});

parity('home-dialogs/folder-picker', { drive: fixtures.library }, async t => {
  await home(t);
  await t.click('#btn-drive-new');
  await t.click('#drive-create-pdf');
  await t.advance(OPEN_MS);
  await t.capture('root');
  await t.click('#folder-picker-list .folder-picker-item:has-text("School")');
  await t.advance(100);
  await t.capture('nested');
});

parity('home-dialogs/move-picker', { drive: fixtures.library }, async t => {
  await home(t);
  await tab(t, 'files');
  await more(t, '#drive-folder-files', 'lib-doc-ideas');
  await t.click('body > .drive-card-dropdown button[data-action="move"]');
  await t.advance(OPEN_MS);
});

parity('home-dialogs/new-folder', { drive: fixtures.library }, async t => {
  await home(t);
  await tab(t, 'files');
  await t.click('#btn-drive-new-folder');
  await t.advance(OPEN_MS);
});

parity('home-dialogs/card-menu', { drive: fixtures.library }, async t => {
  await home(t);
  await more(t, '#drive-recents', 'lib-doc-journal');
  await t.capture('file');
  await tab(t, 'files');
  await more(t, '#drive-folders', 'lib-folder-school');
  await t.capture('folder');
});

parity('home-dialogs/colour-popup', { drive: fixtures.library }, async t => {
  await home(t);
  await tab(t, 'files');
  await more(t, '#drive-folders', 'lib-folder-travel');
  await t.click('body > .drive-card-dropdown button[data-action="color"]');
  await t.advance(100);
});

parity('home-dialogs/trash-slider', { drive: fixtures.library }, async t => {
  await home(t);
  await more(t, '#drive-recents', 'lib-doc-journal');
  await t.click('body > .drive-card-dropdown button[data-action="trash"]');
  await t.advance(OPEN_MS);
  await t.capture('0');
  const handle = await t.page.locator('#trash-slider-handle').boundingBox();
  const slider = await t.page.locator('#trash-slider').boundingBox();
  const y = handle.y + handle.height / 2, start = handle.x + handle.width / 2, end = slider.x + slider.width - 4 - handle.width / 2;
  await t.page.mouse.move(start, y);
  await t.page.mouse.down();
  await t.page.mouse.move((start + end) / 2, y);
  await t.settle();
  await t.capture('50');
  await t.page.mouse.move(end, y);
  await t.settle();
  await t.capture('100');
});

parity('home-dialogs/bin', { drive: fixtures.library }, async t => {
  await home(t);
  await t.click('#btn-drive-bin');
  await t.advance(OPEN_MS);
  await t.capture('list');
  await t.click('#drive-bin-list .bin-item-more');
  await t.advance(100);
  await t.capture('menu');
});

parity('home-dialogs/download-app', { drive: fixtures.library }, async t => {
  await home(t);
  await t.click('#btn-download-app');
  await t.advance(OPEN_MS);
});

parity('home-dialogs/offline-settings', { ...VERSION, drive: fixtures.library }, async t => {
  await home(t);
  await t.click('#btn-drive-profile');
  await t.click('#drive-profile-menu [data-offline-settings]');
  await t.advance(100);
});
