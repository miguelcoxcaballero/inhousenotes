// Home (TESTING §3.3 row "home"): each tab, long lists with the fast-scroll thumb, folder cards in the palette
// colours, an opened folder with deep breadcrumbs, the folder zoom forwards and backwards, pull-to-refresh, search,
// offline cards, sync badges and skeletons.
import { parity } from '../run.mjs';
import { fixtures, blankPdf, ANA } from '../../mock/fixtures.mjs';
import { FOLDER } from '../../mock/drive.mjs';

const TOUCH = { only: { viewports: ['phone-360', 'phone-412', 'tablet-800'] } };
const home = async t => { await t.open({ account: ANA }); };
const tab = async (t, name) => { await t.click(`#home-nav [data-home-tab="${name}"]`); await t.advance(400); };
const card = id => `[data-drive-item-id="${id}"]`;

/** School ▸ Maths ▸ Algebra ▸ Exercises: breadcrumbs four deep, documents on every level. */
function deep(mock) {
  fixtures.library(mock);
  const d = mock.drive, pdf = blankPdf(), at = day => `2026-10-${String(day).padStart(2, '0')}T08:00:00.000Z`;
  d.addFile({ id: 'deep-algebra', name: 'Algebra', mimeType: FOLDER, owner: ANA, parents: ['lib-folder-maths'], folderColorRgb: '#f83a22', modifiedTime: at(14) });
  d.addFile({ id: 'deep-exercises', name: 'Exercises with a rather long folder name', mimeType: FOLDER, owner: ANA, parents: ['deep-algebra'], folderColorRgb: '#fbe983', modifiedTime: at(15) });
  for (let i = 0; i < 7; i++) d.addFile({ id: `deep-doc-${i}`, name: `Sheet ${i + 1}.pdf`, owner: ANA, parents: ['lib-folder-school'], data: pdf, starred: i === 2, modifiedTime: at(2 + i) });
  d.addFile({ id: 'deep-doc-ex', name: 'Exercise 1.pdf', owner: ANA, parents: ['deep-exercises'], data: pdf, modifiedTime: at(16) });
  return mock;
}

// 5.18.0's folder palette (Drive's colours), in two halves of twelve, plus an uncoloured folder.
const COLOURS = ['#f83a22', '#fa573c', '#d06b64', '#ac725e', '#ff7537', '#ffad46', '#fad165', '#fbe983', '#b3dc6c', '#7bd148', '#16a765', '#42d692',
  '#92e1c0', '#9fe1e7', '#9fc5e8', '#4986e7', '#b99aff', '#a479e2', '#cd74e6', '#f691b2', '#cca6ac', '#cabdbf', '#c2c2c2', '#8f8f8f'];
function coloured(mock) {
  fixtures.empty(mock);
  COLOURS.forEach((colour, i) => mock.drive.addFile({ id: `colour-${i}`, name: `Folder ${String(i + 1).padStart(2, '0')}`, mimeType: FOLDER, owner: ANA, folderColorRgb: colour, starred: i % 5 === 0, modifiedTime: '2026-10-10T08:00:00.000Z' }));
  mock.drive.addFile({ id: 'colour-plain', name: 'Plain', mimeType: FOLDER, owner: ANA, modifiedTime: '2026-10-10T08:00:00.000Z' });
  return mock;
}

parity('home/tabs', { systemDark: true, app: true, drive: fixtures.library }, async t => {
  await home(t);
  await t.capture('home');
  for (const name of ['starred', 'shared', 'files']) { await tab(t, name); await t.capture(name); }
});

parity('home/long-list', { drive: fixtures.long }, async t => {
  await home(t);
  await tab(t, 'files');
  await t.capture('top');
  await t.page.locator('.drive-main').hover();
  await t.wheel(0, 2400);
  await t.advance(120);
  await t.capture('scrolled');   // the fast-scroll thumb is showing
  await t.advance(3000);
  await t.capture('thumb-gone');
  await tab(t, 'home');
  await t.capture('recents');
});

parity('home/folder-colours', { systemDark: true, drive: coloured }, async t => {
  await home(t);
  await tab(t, 'files');
  await t.capture('first');
  await t.page.locator('.drive-main').hover();
  await t.wheel(0, 900);
  await t.advance(3000);
  await t.capture('more');
  await tab(t, 'starred');
  await t.capture('starred');
});

parity('home/folder-open', { systemDark: true, drive: deep }, async t => {
  await home(t);
  await tab(t, 'files');
  await t.click(`#drive-folders ${card('lib-folder-school')}`);
  await t.advance(1500);
  await t.capture('school');
  for (const [id, label] of [['lib-folder-maths', 'maths'], ['deep-algebra', 'algebra'], ['deep-exercises', 'exercises']]) {
    await t.click(`#drive-view-folders ${card(id)}`);
    await t.advance(1500);
    await t.capture(label);
  }
  await t.click('#drive-breadcrumbs .drive-breadcrumb:nth-of-type(2)');
  await t.advance(1500);
  await t.capture('crumb-school');
  await t.press('Escape');
  await t.advance(1500);
  await t.capture('closed');
});

// The zoom into a folder: 600 ms, captured at 0 / 25 / 50 / 75 / 100 %, then settled.
parity('home/folder-zoom-open', { drive: deep }, async t => {
  await home(t);
  await tab(t, 'files');
  await t.click(`#drive-folders ${card('lib-folder-school')}`);
  await t.capture('0');
  for (const label of ['25', '50', '75', '100']) { await t.advance(150, { step: 10 }); await t.capture(label); }
  await t.advance(1000);
  await t.capture('settled');
});

// And back out: 560 ms.
parity('home/folder-zoom-close', { drive: deep }, async t => {
  await home(t);
  await tab(t, 'files');
  await t.click(`#drive-folders ${card('lib-folder-school')}`);
  await t.advance(1600);
  await t.click('#btn-folder-view-back');
  await t.capture('0');
  for (const label of ['25', '50', '75', '100']) { await t.advance(140, { step: 10 }); await t.capture(label); }
  await t.advance(1000);
  await t.capture('settled');
});

// Pull-to-refresh with a finger: 40 px, 80 px, past the threshold, released (refreshing), done.
parity('home/pull-to-refresh', { ...TOUCH, drive: fixtures.library }, async t => {
  await home(t);
  const touch = async (type, y) => { await t.cdp.send('Input.dispatchTouchEvent', { type, touchPoints: type === 'touchEnd' ? [] : [{ x: 180, y }] }); await t.settle(); };
  await touch('touchStart', 300);
  await touch('touchMove', 312);
  await touch('touchMove', 340);
  await t.capture('40');
  await touch('touchMove', 380);
  await t.capture('80');
  await touch('touchMove', 430);
  await t.capture('release-to-refresh');
  await touch('touchEnd');
  await t.capture('refreshing');
  await t.advance(1500);
  await t.capture('done');
  // A short pull springs back.
  await touch('touchStart', 300);
  await touch('touchMove', 312);
  await touch('touchMove', 350);
  await touch('touchEnd');
  await t.advance(100);
  await t.capture('springing-back');
  await t.advance(400);
  await t.capture('rest');
});

parity('home/search', { systemDark: true, drive: deep }, async t => {
  await home(t);
  await t.type('#drive-search', 'she');
  await t.advance(200);
  await t.capture('typing');      // before the 250 ms debounce
  await t.advance(1500);
  await t.capture('results');
  await tab(t, 'files');
  await t.capture('results-files');
  await t.type('#drive-search', 'zzzz');
  await t.advance(1500);
  await t.capture('none-files');
  await tab(t, 'home');
  await t.capture('none');
  await t.type('#drive-search', '');
  await t.advance(1500);
  await t.capture('cleared');
});

// Lists asked and not answered: first sign-in (no library on this device) inside the Files tab.
parity('home/skeletons', { drive: mock => { fixtures.library(mock); mock.inject({ host: 'www.googleapis.com', path: /\/drive\/v3\/files$/, times: Infinity, hold: true }); } }, async t => {
  await t.open();
  await t.click('#btn-welcome-start');
  await t.advance(400);
  await t.click('#btn-welcome-login');
  await t.advance(1500);
  await t.capture('home');
  await tab(t, 'files');
  await t.capture('files');
});

// Drive goes away after the library was listed: the same lists from this device, documents not downloaded greyed.
parity('home/offline', { drive: deep }, async t => {
  await home(t);
  await t.advance(3000);
  t.mock.offline = true;
  await t.context.setOffline(true);
  await t.page.evaluate(() => window.dispatchEvent(new Event('offline')));
  await t.advance(3000);
  await t.capture('home');
  await tab(t, 'files');
  await t.capture('files');
  await t.click(`#drive-folders ${card('lib-folder-school')}`);
  await t.advance(1500);
  await t.capture('folder');
});

// The badge of a document being synchronised, phase by phase. Each app is told through its own entry point (the
// sync engines differ by design); the badge itself is Home's.
parity('home/badges', { drive: fixtures.library }, async t => {
  await home(t);
  const report = (phase, id) => t.page.evaluate(([p, i]) => {
    if (window.InhouseOfflineLibrary) window.InhouseOfflineLibrary.setSyncProgress({ phase: p, id: i || null });
    else window.__IHN_TEST_API__.cmd('home.sync', { phase: p, id: i || '' });
  }, [phase, id]);
  for (const phase of ['checking', 'building', 'merging', 'preserving', 'uploading', 'other']) {
    await report(phase, 'lib-doc-ideas');
    await t.advance(100);
    await t.capture(phase);
  }
  await tab(t, 'files');
  await t.capture('files');
  await report('idle', '');
  await t.advance(100);
  await t.capture('idle');
});
