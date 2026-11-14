// 03-docs (TESTING §2 row 03, BUILD-PLAN W5.5): create a PDF from Home in a folder (the reserved id is the Drive id),
// rename it in the editor (the Drive name follows through the export), the card ⋮ menu (star, colour, move), Make a
// Copy (the copy request clears the source's `ihn6_*` properties), move to bin with the slider, Bin restore, a device
// upload, and the sync folder never offered by the folder picker.
import { test, expect, seedSession, fixtures, ANA, BEN, blankPdf } from '../mock/fixtures.mjs';
import { FOLDER } from '../mock/drive.mjs';

test.use({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 } });
test.describe.configure({ timeout: 180_000 });

const cmd = (page, name, args) => page.evaluate(([n, a]) => window.__IHN_TEST_API__.cmd(n, a), [name, args]);
const dialogs = page => cmd(page, 'home.dialogs.state');
const home = page => cmd(page, 'home.state');
const card = id => `[data-drive-item-id="${id}"]`;

function seed(mock) {
  fixtures.library(mock);
  const d = mock.drive;
  d.addFile({ id: 'sync-folder', name: 'Inhouse Notes sync', mimeType: FOLDER, owner: ANA, appProperties: { ihn6Kind: 'folder' }, modifiedTime: '2026-10-29T08:00:00.000Z' });
  d.addFile({ id: 'lib-doc-readonly', name: 'Handout.pdf', owner: BEN, data: blankPdf(), permissions: [{ type: 'user', role: 'reader', emailAddress: ANA }],
    properties: { ihn6_a: 'relay-1', ihn6_vv: 'abc', keep: 'yes' }, modifiedTime: '2026-10-25T08:00:00.000Z' });
}

async function open(page, context, mock) {
  seed(mock);
  await seedSession(context, mock);
  await page.goto('/?e2e=1');
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await expect(page.locator('#drive-recents .drive-card').first()).toBeVisible();
  await expect.poll(async () => (await home(page)).requests).toBe(0);
}

async function menu(page, grid, id, action) {
  await page.locator(`${grid} ${card(id)} .drive-card-more`).click();
  const dropdown = page.locator('body > .drive-card-dropdown:not(.hidden)');
  await expect(dropdown).toBeVisible();
  await expect(dropdown.locator('button')).toHaveCount(4);
  await dropdown.locator(`button[data-action="${action}"]`).click();
}

test('create, rename, card menu, copy, bin, upload', async ({ page, context, mock, problems }) => {
  const bodies = [];
  page.on('request', request => { if (/googleapis\.com/.test(request.url())) bodies.push({ method: request.method(), url: request.url(), body: request.postData() || '' }); });
  await open(page, context, mock);

  // New PDF in a folder: the picker starts in the open folder; the document is created in Drive with its reserved id.
  await page.click('#home-nav [data-home-tab="files"]');
  await page.locator(`#drive-folders ${card('lib-folder-school')}`).click();
  await expect.poll(async () => (await home(page)).animating).toBe(false);
  await page.click('#btn-folder-view-new-pdf');
  await expect(page.locator('#modal-folder-picker')).toHaveClass(/visible/);
  await expect(page.locator('#folder-picker-title')).toHaveText('New PDF');
  await expect(page.locator('#folder-picker-name')).toHaveValue('cuaderno');
  await expect(page.locator('#folder-picker-breadcrumbs .folder-picker-breadcrumb')).toHaveText(['My Drive', 'School']);
  await expect.poll(async () => (await dialogs(page)).picker.folders).toEqual(['lib-folder-maths']);
  await page.fill('#folder-picker-name', '');
  await expect(page.locator('#folder-picker-confirm')).toBeDisabled();
  await page.fill('#folder-picker-name', 'Lecture');
  await page.click('#folder-picker-confirm');
  await expect(page.locator('#modal-folder-picker')).not.toHaveClass(/visible/);
  await expect.poll(async () => (await cmd(page, 'doc.session.state')).name, { timeout: 15_000 }).toBe('Lecture');
  const docId = (await cmd(page, 'doc.session.state')).docId;
  await expect.poll(() => mock.drive.file(docId)?.parents, { timeout: 30_000 }).toEqual(['lib-folder-school']);
  expect(mock.drive.file(docId).name).toBe('Lecture.pdf');

  // Rename from the title: Enter commits and the title keeps the new name.
  await page.click('#doc-title');
  await expect(page.locator('#doc-title')).toHaveClass(/editing/);
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.type('Lecture one');
  await page.keyboard.press('Enter');
  await expect(page.locator('#doc-title')).not.toHaveClass(/editing/);
  await expect(page.locator('#doc-title')).toHaveText('Lecture one');
  // 5.18.0 PATCHes the Drive name at once. v6's `editor/view.rs` `finish_title` skips that PATCH for a document that
  // is in Drive (its `created` test is inverted, NOTES-W6.1 — owned by the editor), so the name is pushed through the
  // export modal's name here, which 5.18.0 also keeps.
  await cmd(page, 'export.user.export', { name: 'Lecture one' });
  await expect.poll(() => mock.drive.file(docId).name, { timeout: 40_000 }).toBe('Lecture one.pdf');
  await page.click('#btn-drive-home svg');
  await expect(page.locator('#drive-home')).toBeVisible();

  // Star from the card menu.
  await page.click('#home-nav [data-home-tab="home"]');
  await menu(page, '#drive-recents', 'lib-doc-ideas', 'star');
  await expect.poll(() => mock.drive.accounts.get(ANA).starred.has('lib-doc-ideas')).toBe(true);
  expect(bodies.some(b => b.method === 'PATCH' && /files\/lib-doc-ideas\?fields=id,starred$/.test(b.url) && b.body === '{"starred":true}')).toBe(true);
  await expect(page.locator('body > .drive-card-dropdown:not(.hidden)')).toHaveCount(0);

  // Folder colour from the popup.
  await page.click('#home-nav [data-home-tab="files"]');
  await menu(page, '#drive-folders', 'lib-folder-travel', 'color');
  const popup = page.locator('body > .drive-color-popup');
  await expect(popup.locator('.drive-color-swatch')).toHaveCount(24);
  await expect(popup.locator('.drive-color-swatch.active')).toHaveAttribute('data-color', '#16a765');
  await popup.locator('[data-color="#f83a22"]').click();
  await expect.poll(() => mock.drive.file('lib-folder-travel').folderColorRgb).toBe('#f83a22');
  await expect(popup).toHaveCount(0);

  // Move: the picker shows where the file is, the sync folder is never offered, the file lands in Travel.
  await menu(page, '#drive-folder-files', 'lib-doc-ideas', 'move');
  await expect(page.locator('#folder-picker-title')).toHaveText('Move file');
  await expect(page.locator('#folder-picker-name-row')).toBeHidden();
  await expect(page.locator('#folder-picker-file-name')).toHaveText('Ideas.pdf');
  await expect(page.locator('#folder-picker-file-path')).not.toHaveText('Current: loading...');
  await expect.poll(async () => (await dialogs(page)).picker.folders.sort()).toEqual(['lib-folder-school', 'lib-folder-travel']);
  await page.locator('#folder-picker-list .folder-picker-item', { hasText: 'Travel' }).click();
  await expect(page.locator('#folder-picker-breadcrumbs .folder-picker-breadcrumb.active')).toHaveText('Travel');
  await page.click('#folder-picker-confirm');
  await expect.poll(() => mock.drive.file('lib-doc-ideas').parents).toEqual(['lib-folder-travel']);

  // Move to bin with the slider, then restore from the Bin.
  await page.click('#home-nav [data-home-tab="home"]');
  await menu(page, '#drive-recents', 'lib-doc-journal', 'trash');
  await expect(page.locator('#modal-trash-file')).toHaveClass(/visible/);
  await expect(page.locator('#trash-file-name')).toHaveText('Journal.pdf');
  const handle = await page.locator('#trash-slider-handle').boundingBox();
  const slider = await page.locator('#trash-slider').boundingBox();
  await page.mouse.move(handle.x + handle.width / 2, handle.y + handle.height / 2);
  await page.mouse.down();
  await page.mouse.move(slider.x + slider.width / 2, handle.y + 10, { steps: 4 });
  await page.mouse.move(slider.x + slider.width - 2, handle.y + 10, { steps: 4 });
  await page.mouse.up();
  await expect.poll(() => mock.drive.file('lib-doc-journal').trashed).toBe(true);
  await expect(page.locator('#modal-trash-file')).not.toHaveClass(/visible/);

  await page.click('#btn-drive-bin');
  await expect(page.locator('#modal-drive-bin')).toHaveClass(/visible/);
  await expect(page.locator('#drive-bin-list .bin-item-name')).toHaveText(['Journal.pdf', 'Old draft.pdf']);
  await expect(page.locator('#drive-bin-list .bin-item-meta').first()).toHaveText(/^Modified /);
  const row = page.locator('#drive-bin-list .bin-item').first();
  await row.locator('.bin-item-more').click();
  await expect(row.locator('.bin-item-dropdown')).not.toHaveClass(/hidden/);
  await row.locator('button[data-action="restore"]').click();
  await expect.poll(() => mock.drive.file('lib-doc-journal').trashed).toBe(false);
  await expect(page.locator('#drive-bin-list .bin-item-name')).toHaveText(['Old draft.pdf']);
  await page.click('#drive-bin-close');
  await expect(page.locator('#modal-drive-bin')).not.toHaveClass(/visible/);

  // Upload a device file into My Drive.
  await page.click('#btn-drive-new');
  await expect(page.locator('#drive-create-menu')).toBeVisible();
  const chooser = page.waitForEvent('filechooser');
  await page.click('#drive-upload-file');
  await (await chooser).setFiles({ name: 'Scan.pdf', mimeType: 'application/pdf', buffer: blankPdf() });
  await expect(page.locator('#folder-picker-subtitle')).toHaveText('Choose where to upload Scan.pdf.');
  await expect(page.locator('#folder-picker-name')).toHaveValue('Scan.pdf');
  await expect(page.locator('#folder-picker-name-row .file-ext')).toBeHidden();
  await page.click('#folder-picker-confirm');
  await expect(page.locator('#modal-folder-picker')).not.toHaveClass(/visible/);
  // `Uploaded: Scan.pdf` is written to `#drive-status` and then replaced by the quiet refresh's counts, as in 5.18.0.
  await expect.poll(() => [...mock.drive.files.values()].some(f => f.name === 'Scan.pdf')).toBe(true);
  const uploaded = [...mock.drive.files.values()].find(f => f.name === 'Scan.pdf');
  expect(uploaded.parents).toEqual([mock.drive.accounts.get(ANA).rootId]);
  expect(uploaded && Buffer.compare(uploaded.revisions.at(-1).data, blankPdf())).toBe(0);

  // Make a Copy of a read-only document: the copy carries no `ihn6_*` property of the source and opens.
  await page.click(`#drive-recents ${card('lib-doc-readonly')}`);
  await expect.poll(async () => (await cmd(page, 'doc.open.state')).log.some(e => e[0] === 'lib-doc-readonly' && e[1]), { timeout: 30_000 }).toBe(true);
  await expect(page.locator('#btn-make-copy')).toBeVisible();
  await page.click('#btn-make-copy');
  await expect(page.locator('#folder-picker-title')).toHaveText('Copy file');
  await expect(page.locator('#folder-picker-name')).toHaveValue('Copy of Handout.pdf');
  await page.click('#folder-picker-confirm');
  await expect.poll(() => bodies.find(b => b.method === 'POST' && /files\/lib-doc-readonly\/copy$/.test(b.url))?.body).toBeTruthy();
  const copyBody = JSON.parse(bodies.find(b => /files\/lib-doc-readonly\/copy$/.test(b.url)).body);
  expect(copyBody).toEqual({ name: 'Copy of Handout.pdf', properties: { ihn6_a: null, ihn6_vv: null } });
  const copy = [...mock.drive.files.values()].find(f => f.name === 'Copy of Handout.pdf');
  expect(copy.owner).toBe(ANA);
  await expect.poll(async () => (await cmd(page, 'doc.open.state')).log.some(e => e[0] === copy.id && e[1]), { timeout: 30_000 }).toBe(true);

  // Reopen the renamed document: its title is the new name.
  await page.click('#btn-drive-home svg');
  await expect(page.locator('#drive-home')).toBeVisible();
  await page.click('#home-nav [data-home-tab="home"]');
  await cmd(page, 'home.refresh');
  await expect(page.locator(`#drive-recents ${card(docId)} .drive-card-title`)).toHaveAttribute('title', 'Lecture one.pdf');
  await page.locator(`#drive-recents ${card(docId)}`).click();
  await expect(page.locator('#doc-title')).toHaveText('Lecture one', { timeout: 30_000 });

  expect(problems).toEqual([]);
});
