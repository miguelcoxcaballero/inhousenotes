// 03b-home-actions (TESTING §2 row 03, BUILD-PLAN W5.5): the create menu (keyboard, outside press), New folder,
// folder picker navigation and closing, the Download-app modal, the Offline documents dialog (queue line,
// retention saved) and the upload-error dialog opened from a `Retry upload` badge.
import { test, expect, seedSession, fixtures, ANA } from '../mock/fixtures.mjs';
import { FOLDER } from '../mock/drive.mjs';

test.use({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 } });
test.describe.configure({ timeout: 120_000 });

const cmd = (page, name, args) => page.evaluate(([n, a]) => window.__IHN_TEST_API__.cmd(n, a), [name, args]);
const dialogs = page => cmd(page, 'home.dialogs.state');

async function open(page, context, mock) {
  fixtures.library(mock);
  await seedSession(context, mock);
  await page.goto('/?e2e=1');
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await expect(page.locator('#drive-recents .drive-card').first()).toBeVisible();
  await expect.poll(async () => (await cmd(page, 'home.state')).requests).toBe(0);
}

test('create menu, new folder, picker, download app', async ({ page, context, mock, problems }) => {
  await open(page, context, mock);

  // Create menu: opens under the FAB with focus on the first item; arrows move, Escape closes, an outside press too.
  await page.click('#btn-drive-new');
  await expect(page.locator('#drive-create-menu')).toBeVisible();
  await expect(page.locator('#btn-drive-new')).toHaveAttribute('aria-expanded', 'true');
  await expect(page.locator('#drive-create-pdf')).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(page.locator('#drive-upload-file')).toBeFocused();
  await page.keyboard.press('ArrowUp');
  await expect(page.locator('#drive-create-pdf')).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.locator('#drive-create-menu')).toBeHidden();
  await expect(page.locator('#btn-drive-new')).toBeFocused();
  await page.click('#btn-drive-new');
  await page.mouse.click(640, 700);
  await expect(page.locator('#drive-create-menu')).toBeHidden();
  await expect(page.locator('#btn-drive-new')).toHaveAttribute('aria-expanded', 'false');

  // Create PDF opens the picker; navigate into School ▸ Maths and back with the breadcrumbs; Escape closes it.
  await page.click('#btn-drive-new');
  await page.click('#drive-create-pdf');
  await expect(page.locator('#modal-folder-picker')).toHaveClass(/visible/);
  await expect(page.locator('#folder-picker-meta')).toHaveClass(/hidden/);
  await expect(page.locator('#folder-picker-confirm')).toHaveText('Create');
  await page.locator('#folder-picker-list .folder-picker-item', { hasText: 'School' }).click();
  await page.locator('#folder-picker-list .folder-picker-item', { hasText: 'Maths' }).click();
  await expect(page.locator('#folder-picker-breadcrumbs .folder-picker-breadcrumb')).toHaveText(['My Drive', 'School', 'Maths']);
  await expect(page.locator('#folder-picker-empty')).not.toHaveClass(/hidden/);
  await page.locator('#folder-picker-breadcrumbs .folder-picker-breadcrumb', { hasText: 'My Drive' }).click();
  await expect.poll(async () => (await dialogs(page)).picker.stack).toEqual(['My Drive']);
  await page.keyboard.press('Escape');
  await expect(page.locator('#modal-folder-picker')).not.toHaveClass(/visible/);
  expect((await dialogs(page)).picker).toBe(null);

  // New folder in the Files tab: Enter creates it in the current folder, the list shows it.
  await page.click('#home-nav [data-home-tab="files"]');
  await page.click('#btn-drive-new-folder');
  await expect(page.locator('#modal-new-folder')).toHaveClass(/visible/);
  await expect(page.locator('#new-folder-name')).toBeFocused();
  await page.keyboard.press('Enter');   // empty: nothing
  await expect(page.locator('#modal-new-folder')).toHaveClass(/visible/);
  await page.keyboard.type('Projects');
  await page.keyboard.press('Enter');
  await expect(page.locator('#modal-new-folder')).not.toHaveClass(/visible/);
  await expect.poll(() => [...mock.drive.files.values()].some(f => f.name === 'Projects' && f.mimeType === FOLDER)).toBe(true);
  await expect(page.locator('#drive-folders .drive-card-title', { hasText: 'Projects' })).toBeVisible();
  await page.click('#btn-drive-new-folder');
  await page.keyboard.press('Escape');
  await expect(page.locator('#modal-new-folder')).not.toHaveClass(/visible/);

  // Download app (web): opens, closes from its button, the overlay and 100 ms after the link.
  await page.click('#btn-download-app');
  await expect(page.locator('#modal-download-app')).toHaveClass(/visible/);
  await page.click('#download-app-close');
  await expect(page.locator('#modal-download-app')).not.toHaveClass(/visible/);
  await page.click('#btn-download-app');
  await page.mouse.click(5, 5);
  await expect(page.locator('#modal-download-app')).not.toHaveClass(/visible/);
  await page.click('#btn-download-app');
  await expect(page.locator('#download-app-link')).toHaveAttribute('href', /inhouse-notes-release-v1\.0\.11\.apk$/);
  // The download itself is not followed here.
  await page.evaluate(() => document.getElementById('download-app-link').addEventListener('click', e => e.preventDefault()));
  await page.locator('#download-app-link').dispatchEvent('click');
  await expect(page.locator('#modal-download-app')).not.toHaveClass(/visible/);

  expect(problems).toEqual([]);
});

test('offline documents and upload error dialogs', async ({ page, context, mock, problems }) => {
  await open(page, context, mock);
  await cmd(page, 'home.seed_docs', [{ id: 'lib-doc-ideas', created: true, relayPending: true, chunks: true, lastOpenedAt: 1 }]);
  await expect(page.locator('#drive-recents [data-drive-item-id="lib-doc-ideas"] .document-sync-status')).toHaveText('Waiting to upload');

  // Offline documents: retention 14 by default, the queue line, Save keeps the new retention.
  await page.click('#btn-drive-profile');
  await page.click('#drive-profile-menu [data-offline-settings]');
  const settings = page.locator('dialog#offline-settings');
  await expect(settings).toBeVisible();
  await expect(settings.locator('input[name="days"]')).toHaveValue('14');
  await expect(settings.locator('.offline-queue-status')).toHaveText('1 pending · Will sync automatically');
  await settings.locator('input[name="days"]').fill('30');
  await settings.locator('button[value="save"]').click();
  await expect(settings).toHaveCount(0);
  await page.click('#btn-drive-profile');
  await page.click('#drive-profile-menu [data-offline-settings]');
  await expect(settings.locator('input[name="days"]')).toHaveValue('30');
  await settings.locator('button[value="cancel"]').click();
  await expect(settings).toHaveCount(0);
  await page.keyboard.press('Escape');

  // An upload error: the badge offers `Retry upload`; its dialog shows the summary and the redacted report.
  await cmd(page, 'home.sync', { errors: [{ id: 'lib-doc-ideas', code: '403', message: 'Drive API error 403: Bearer abc123 storage quota exceeded', phase: 'relay' }] });
  const badge = page.locator('#drive-recents [data-drive-item-id="lib-doc-ideas"] .document-sync-status');
  await expect(badge).toHaveText('Retry upload');
  await badge.click();
  const error = page.locator('dialog#offline-sync-error');
  await expect(error).toBeVisible();
  await expect(error.locator('.sync-file-name')).toHaveText('Ideas.pdf');
  await expect(error.locator('.sync-error-summary')).toHaveText('Google Drive storage is full. Your local copy is safe.');
  const version = await page.evaluate(() => document.querySelector('[data-app-version]').textContent.slice(1));
  await expect(error.locator('.sync-error-detail')).toHaveText(`Inhouse Notes ${version}\nrelay · 403\nDrive API error 403: Bearer [redacted] storage quota exceeded`);
  await error.locator('.copy-sync-error').click();
  await expect(error.locator('.copy-sync-error')).toHaveText(/^(Copied|Select the error text)$/);
  await error.locator('button[value="close"]').click();
  await expect(error).toHaveCount(0);
  expect((await dialogs(page)).error).toBe(false);

  expect(problems).toEqual([]);
});
