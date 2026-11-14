// 02-home (TESTING §2): tabs, open folder (the zoom runs and ends), breadcrumbs, back / Escape, search (250 ms
// debounce, one refresh), pull-to-refresh issues one list refresh, offline → cached lists + offline status, sync
// badges from seeded `docs` flags, and the sync folder / `ihn6Kind` files never listed.
import { test, expect, seedSession, fixtures, ANA } from '../mock/fixtures.mjs';
import { FOLDER } from '../mock/drive.mjs';

test.use({ serviceWorkers: 'block' });

const cmd = (page, name, args) => page.evaluate(([n, a]) => window.__IHN_TEST_API__.cmd(n, a), [name, args]);
const state = page => cmd(page, 'home.state');
const ids = (page, selector) => page.locator(selector).evaluateAll(nodes => nodes.map(node => node.dataset.driveItemId));
const names = (page, selector) => page.locator(`${selector} .drive-card-title`).evaluateAll(nodes => nodes.map(node => node.getAttribute('title')));
// List requests (not the enumeration, the probe or `about`).
const lists = mock => mock.calls.filter(call => call.host === 'www.googleapis.com' && call.path === '/drive/v3/files' && /[?&]orderBy=/.test(call.url));
const q = call => new URL(call.url).searchParams.get('q');

/** Sync files as SYNC §6 creates them: the folder, a relay file inside it, and one that escaped into My Drive. */
function addSyncFiles(mock) {
  const d = mock.drive, at = '2026-10-29T08:00:00.000Z';
  const folder = d.addFile({ id: 'sync-folder', name: 'Inhouse Notes sync', mimeType: FOLDER, owner: ANA, appProperties: { ihn6Kind: 'folder' }, modifiedTime: at });
  d.addFile({ id: 'sync-relay', name: 'Ideas relay.pdf', owner: ANA, parents: [folder.id], appProperties: { ihn6Kind: 'relay' }, data: Buffer.from('%PDF-1.4\n'), starred: true, modifiedTime: at });
  d.addFile({ id: 'sync-stray', name: 'Ideas stray.pdf', owner: ANA, appProperties: { ihn6Kind: 'asset' }, data: Buffer.from('%PDF-1.4\n'), modifiedTime: at });
}

async function open(page, context, mock, { seed = fixtures.library, url = '/?e2e=1' } = {}) {
  seed(mock);
  await seedSession(context, mock);
  await page.goto(url);
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  expect(await page.evaluate(() => window.__IHN_TEST_API__.ready())).toBe(true);
  await expect(page.locator('#drive-home')).toBeVisible();
  await expect(page.locator('#drive-recents .drive-card').first()).toBeVisible();
  await expect.poll(async () => (await state(page)).requests).toBe(0);
}

test('tabs, folder zoom, breadcrumbs, back and Escape', async ({ page, context, mock, problems }) => {
  await open(page, context, mock, { seed: m => { fixtures.library(m); addSyncFiles(m); } });

  // Home: recents (PDFs by date), starred and shared previews; no sync file anywhere.
  expect(await names(page, '#drive-recents')).toEqual(['Shared plan.pdf', 'Syllabus.pdf', 'Algebra notes.pdf', 'Journal.pdf', 'Ideas.pdf']);
  expect(await ids(page, '#drive-starred [data-drive-item-id]')).toEqual(['lib-doc-journal']);
  expect((await ids(page, '#drive-shared [data-drive-item-id]')).sort()).toEqual(['lib-doc-shared', 'lib-folder-shared']);
  expect(await page.locator('#drive-home').getAttribute('data-home-tab')).toBe('home');

  // Tabs.
  for (const tab of ['starred', 'shared', 'files', 'home']) {
    await page.click(`#home-nav [data-home-tab="${tab}"]`);
    expect(await page.locator('#drive-home').getAttribute('data-home-tab')).toBe(tab);
    await expect(page.locator(`#home-nav [data-home-tab="${tab}"]`)).toHaveClass(/is-active/);
    expect(await page.evaluate(() => sessionStorage.getItem('ihn-home-tab'))).toBe(tab);
  }
  await page.click('#home-nav [data-home-tab="files"]');
  expect(await ids(page, '#drive-folders [data-drive-item-id]')).toEqual(['lib-folder-school', 'lib-folder-travel']);
  expect(await ids(page, '#drive-folder-files [data-drive-item-id]')).toEqual(['lib-doc-journal', 'lib-doc-ideas']);

  // Open a folder: the zoom animation runs and ends, the folder view lists its contents.
  await page.locator('#drive-folders [data-drive-item-id="lib-folder-school"]').click();
  expect((await state(page)).stack).toEqual(['My Drive', 'School']);
  await expect.poll(async () => (await state(page)).animating).toBe(false);
  await expect(page.locator('#drive-folder-view')).toBeVisible();
  await expect.poll(() => ids(page, '#drive-view-folders [data-drive-item-id]')).toEqual(['lib-folder-maths']);
  await expect.poll(() => ids(page, '#drive-view-files [data-drive-item-id]')).toEqual(['lib-doc-syllabus']);
  expect((await state(page)).viewOpen).toBe(true);

  // Deeper, then the breadcrumbs.
  await page.locator('#drive-view-folders [data-drive-item-id="lib-folder-maths"]').click();
  await expect.poll(async () => (await state(page)).animating).toBe(false);
  await expect.poll(() => ids(page, '#drive-view-files [data-drive-item-id]')).toEqual(['lib-doc-algebra']);
  expect((await state(page)).stack).toEqual(['My Drive', 'School', 'Maths']);
  expect((await page.locator('#drive-breadcrumbs').innerText()).replace(/\s+/g, ' ')).toMatch(/My Drive.*School.*Maths/);

  // Escape goes up one folder; Back leaves the view.
  await page.keyboard.press('Escape');
  await expect.poll(async () => (await state(page)).stack).toEqual(['My Drive', 'School']);
  await expect.poll(async () => (await state(page)).animating).toBe(false);
  await expect.poll(() => ids(page, '#drive-view-files [data-drive-item-id]')).toEqual(['lib-doc-syllabus']);
  await page.click('#btn-folder-view-back');
  await expect.poll(async () => (await state(page)).stack).toEqual(['My Drive']);
  await expect.poll(async () => { const s = await state(page); return s.viewOpen || s.animating; }).toBe(false);
  await expect(page.locator('#drive-folder-view')).toBeHidden();

  // A breadcrumb jumps straight back to My Drive from two levels down.
  await page.locator('#drive-folders [data-drive-item-id="lib-folder-school"]').click();
  await expect.poll(async () => (await state(page)).animating).toBe(false);
  await page.locator('#drive-view-folders [data-drive-item-id="lib-folder-maths"]').click();
  await expect.poll(async () => (await state(page)).animating).toBe(false);
  await page.locator('#drive-breadcrumbs button, #drive-breadcrumbs [data-crumb-index]').first().click();
  await expect.poll(async () => (await state(page)).stack).toEqual(['My Drive']);
  await expect.poll(async () => { const s = await state(page); return s.viewOpen || s.animating; }).toBe(false);

  // The sync folder and every `ihn6Kind` file stayed out of every list and out of the catalogue.
  const all = await ids(page, '#drive-home [data-drive-item-id]');
  expect(all.filter(id => id.startsWith('sync-'))).toEqual([]);
  expect((await cmd(page, 'home.folder', 'root')).filter(id => id.startsWith('sync-'))).toEqual([]);
  expect(problems).toEqual([]);
});

test('search waits 250 ms and lists once; sync files never match', async ({ page, context, mock, problems }) => {
  await open(page, context, mock, { seed: m => { fixtures.library(m); addSyncFiles(m); } });
  await expect.poll(async () => (await cmd(page, 'drive.home.state')).crawling).toBe(false);
  const before = lists(mock).length;
  await page.locator('#drive-search').pressSequentially('Idea', { delay: 30 });
  expect(lists(mock).length).toBe(before);   // nothing while typing
  await expect.poll(() => lists(mock).length).toBeGreaterThan(before);
  await expect.poll(async () => (await state(page)).requests).toBe(0);
  const sent = lists(mock).slice(before);
  // One refresh: every list at most once, each with the whole term (starred has no name filter in 5.18.0).
  const searched = sent.filter(call => /name contains/.test(q(call)));
  expect(searched.length).toBeGreaterThan(0);
  expect(new Set(sent.map(call => call.url)).size).toBe(sent.length);
  for (const call of searched) expect(q(call)).toContain("name contains 'Idea'");
  expect((await state(page)).search).toBe('Idea');
  expect(await names(page, '#drive-recents')).toEqual(['Ideas.pdf']);
  await page.click('#home-nav [data-home-tab="files"]');
  expect(await ids(page, '#drive-folder-files [data-drive-item-id]')).toEqual(['lib-doc-ideas']);
  expect(await ids(page, '#drive-folders [data-drive-item-id]')).toEqual([]);

  // No results.
  const mark = lists(mock).length;
  await page.locator('#drive-search').fill('zzzz');
  await expect.poll(() => lists(mock).length).toBeGreaterThan(mark);
  await expect.poll(async () => (await state(page)).requests).toBe(0);
  await expect.poll(() => ids(page, '#drive-folder-files [data-drive-item-id]')).toEqual([]);
  await expect(page.locator('#drive-folder-empty')).toBeVisible();
  expect(problems).toEqual([]);
});

test.describe('touch', () => {
  test.use({ hasTouch: true, viewport: { width: 360, height: 800 } });

  test('pull-to-refresh lists once', async ({ page, context, mock, problems }) => {
    await open(page, context, mock);
    await expect.poll(async () => (await cmd(page, 'drive.home.state')).crawling).toBe(false);
    const cdp = await context.newCDPSession(page);
    const touch = (type, y) => cdp.send('Input.dispatchTouchEvent', { type, touchPoints: type === 'touchEnd' ? [] : [{ x: 180, y }] });
    const home = page.locator('#drive-home');

    // A short pull shows the indicator and springs back without a request.
    const before = lists(mock).length;
    await touch('touchStart', 300);
    for (const y of [312, 330, 350]) await touch('touchMove', y);
    await expect(home).toHaveClass(/home-is-pulling/);
    expect(await page.locator('#home-pull-refresh').getAttribute('aria-label')).toBe('Pull to refresh');
    await touch('touchEnd');
    await expect(home).not.toHaveClass(/home-is-pulling/);
    expect(lists(mock).length).toBe(before);

    // Past the threshold (96 px): one refresh.
    await touch('touchStart', 300);
    for (const y of [312, 340, 380, 420]) await touch('touchMove', y);
    expect(await page.locator('#home-pull-refresh').getAttribute('aria-label')).toBe('Release to refresh');
    await touch('touchEnd');
    await expect.poll(() => lists(mock).length).toBeGreaterThan(before);
    await expect.poll(async () => (await state(page)).requests).toBe(0);
    await expect(home).not.toHaveClass(/home-is-refreshing/);
    const sent = lists(mock).slice(before);
    expect(new Set(sent.map(call => call.url)).size).toBe(sent.length);
    expect(sent.filter(call => /mimeType='application\/pdf' and trashed=false$/.test(q(call)) && !/in parents/.test(q(call)))).toHaveLength(1);
    expect(problems).toEqual([]);
  });
});

test('offline: cached lists, the offline line and badges from the docs flags', async ({ page, context, mock, problems }) => {
  await open(page, context, mock);
  await expect.poll(async () => (await cmd(page, 'drive.home.state')).crawling).toBe(false);
  await expect.poll(async () => (await state(page)).entries).toBeGreaterThanOrEqual(9);
  // The documents as the document and sync units leave them: one with unsent changes, one only opened.
  await cmd(page, 'home.seed_docs', [
    { id: 'lib-doc-ideas', created: true, relayPending: true, chunks: true, lastOpenedAt: 2 },
    { id: 'lib-doc-journal', created: true, chunks: true, lastOpenedAt: 1 }
  ]);
  const badge = id => page.locator(`#drive-recents [data-drive-item-id="${id}"] .document-sync-status`);
  await expect(badge('lib-doc-ideas')).toHaveText('Waiting to upload');
  await expect(badge('lib-doc-journal')).toHaveCount(0);
  await cmd(page, 'home.sync', { phase: 'uploading', id: 'lib-doc-ideas' });
  await expect(badge('lib-doc-ideas')).toHaveText('Uploading…');
  await cmd(page, 'home.sync', { phase: 'idle', id: '', errors: [{ id: 'lib-doc-ideas', code: '500', message: 'Drive API error 500: backend', phase: 'uploading' }] });
  await expect(badge('lib-doc-ideas')).toHaveText('Retry upload');
  await cmd(page, 'home.sync', { errors: [] });
  await expect(badge('lib-doc-ideas')).toHaveText('Waiting to upload');

  // The network goes away: the same lists from this device, the offline line, `Saved offline`.
  const shown = await names(page, '#drive-recents');
  mock.offline = true;
  await context.setOffline(true);
  await expect.poll(async () => (await cmd(page, 'shell.state')).offline).toBe(true);
  await expect(page.locator('#drive-status')).toHaveText('Offline · changes stay on this device', { useInnerText: false });
  await expect(badge('lib-doc-ideas')).toHaveText('Saved offline');
  expect((await names(page, '#drive-recents')).sort()).toEqual([...shown].sort());
  await expect(page.locator('#drive-recents [data-drive-item-id="lib-doc-journal"]')).toHaveAttribute('data-offline-available', 'true');
  await expect(page.locator('#drive-recents [data-drive-item-id="lib-doc-syllabus"]')).toHaveAttribute('aria-disabled', 'true');

  // Folders never opened online are browsable (the enumeration filled the catalogue).
  await page.click('#home-nav [data-home-tab="files"]');
  await page.locator('#drive-folders [data-drive-item-id="lib-folder-travel"]').click();
  await expect.poll(async () => (await state(page)).animating).toBe(false);
  expect((await state(page)).stack).toEqual(['My Drive', 'Travel']);
  await page.keyboard.press('Escape');
  await expect.poll(async () => (await state(page)).stack).toEqual(['My Drive']);

  // A reload offline starts from the cache.
  await context.setOffline(false);   // the page itself must load; Drive stays unreachable through the mock
  await page.reload();
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  // The Files tab is remembered for this browser tab; the lists come from this device.
  expect(await page.locator('#drive-home').getAttribute('data-home-tab')).toBe('files');
  await expect(page.locator('#drive-folder-files .drive-card').first()).toBeVisible();
  expect(await ids(page, '#drive-folders [data-drive-item-id]')).toEqual(['lib-folder-school', 'lib-folder-travel']);
  expect((await names(page, '#drive-recents')).sort()).toEqual([...shown].sort());
  await expect(page.locator('#drive-status')).toHaveText('Offline · changes stay on this device', { useInnerText: false });
  expect(problems).toEqual([]);
});
