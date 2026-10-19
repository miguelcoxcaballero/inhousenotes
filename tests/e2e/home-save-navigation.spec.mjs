import { test, expect } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.route('https://accounts.google.com/**', r => r.fulfill({ body: '' }));
  await page.addInitScript(() => localStorage.setItem('drive-hint-email', 'home-save@example.test'));
  await page.goto('/?e2e=1');
  await page.evaluate(() => __IHN_TEST_API__.ready());
});

async function openCached(page, dirty = false) {
  await page.evaluate(async dirty => {
    await __IHN_TEST_API__.resetLocalDocument(1, 'Cached');
    state.driveFileId = 'cached-home'; state.driveFileName = 'Cached.pdf';
    state.driveCanEdit = true; state.driveAutosave = true;
    state.pages[0].strokes.push({ id: 'saved-ink', tool: 'pen', color: '#000000', width: 2, points: [{ x: 10, y: 10 }, { x: 50, y: 60 }] });
    resetDriveContentSyncVersion(); driveDirty = dirty;
    if (dirty) bumpDriveContentVersion();
    await InhouseOfflineLibrary.capture();
    await __IHN_TEST_API__.resetLocalDocument(1, 'Other');
    showDriveHome();
    await openDriveFile({ id: 'cached-home', name: 'Cached.pdf' });
  }, dirty);
}

test('opening an unchanged cached document remains saved and Home needs no Drive request', async ({ page, context }) => {
  await context.setOffline(true);
  await openCached(page);
  await expect(page.locator('#save-indicator')).toHaveClass(/saved/);
  await page.locator('#btn-drive-home').click();
  await expect(page.locator('#drive-home')).toBeVisible({ timeout: 2500 });
  expect(await page.evaluate(() => readPendingExitUpload())).toBeNull();
});

test('offline Home persists actual edits and can reopen them', async ({ page, context }) => {
  await context.setOffline(true);
  await openCached(page, true);
  await page.locator('#btn-drive-home').click();
  await expect(page.locator('#drive-home')).toBeVisible({ timeout: 3000 });
  await page.evaluate(() => openDriveFile({ id: 'cached-home', name: 'Cached.pdf' }));
  expect(await page.evaluate(() => state.pages[0].strokes.map(s => s.id))).toContain('saved-ink');
  expect(await page.evaluate(async () => (await InhouseOfflineStore.read('documents', InhouseOfflineLibrary.account(), 'cached-home')).dirty)).toBe(true);
});

test('Home does not wait for stalled Drive authentication or an inherited upload', async ({ page }) => {
  await openCached(page, true);
  await page.evaluate(() => {
    window.ensureDriveToken = () => new Promise(() => {});
    lifecycleExitSavePromise = new Promise(() => {});
    lifecycleExitSaveFileId = state.driveFileId;
  });
  await page.locator('#btn-drive-home').click();
  await expect(page.locator('#drive-home')).toBeVisible({ timeout: 3000 });
  expect(await page.evaluate(async () => {
    const doc = await InhouseOfflineStore.read('documents', InhouseOfflineLibrary.account(), 'cached-home');
    return doc.dirty && doc.snapshot.pages[0].strokes.some(s => s.id === 'saved-ink');
  })).toBe(true);
});

test('failed local checkpoint keeps the document and real edits open', async ({ page, context }) => {
  await context.setOffline(true);
  await openCached(page, true);
  await page.evaluate(() => {
    window.savePageToIndexedDb = async () => false;
    markPageDirty(0, 'full');
  });
  await page.locator('#btn-drive-home').click();
  await expect(page.locator('#loading-overlay')).not.toHaveClass(/visible/);
  await expect(page.locator('#save-indicator')).toHaveClass(/error/);
  await expect(page.locator('#editor-view')).toBeVisible();
  await expect(page.locator('#drive-home')).toBeHidden();
  expect(await page.evaluate(() => state.pages[0].strokes[0].id)).toBe('saved-ink');
});

test('cache checkpoint and expired authentication do not dirty an unchanged document', async ({ page, context }) => {
  await context.setOffline(true);
  await openCached(page);
  const result = await page.evaluate(async () => {
    await persistExitLocalCheckpoint(state.driveFileId);
    markDriveAuthNeedsReconnect('expired test token');
    refreshSaveIndicatorState();
    return { marker: readPendingExitUpload(), pending: hasPendingDriveSave(), error: localSaveError };
  });
  expect(result).toEqual({ marker: null, pending: false, error: '' });
  await expect(page.locator('#save-indicator')).toHaveClass(/saved/);
  await page.locator('#btn-drive-home').click();
  await expect(page.locator('#drive-home')).toBeVisible({ timeout: 2000 });
});

test('a save requested during deferred import finishes the page cache instead of reporting a failure', async ({ page, context }) => {
  await context.setOffline(true);
  await openCached(page);
  const result = await page.evaluate(async () => {
    await queueSave();
    return { error: localSaveError, migration: hasPendingLegacyPageMigration(), pending: hasPendingLocalSave() };
  });
  expect(result).toEqual({ error: '', migration: false, pending: false });
});

test('a departing document upload cannot dirty or save the next document', async ({ page, context }) => {
  await context.setOffline(true);
  await openCached(page, true);
  const result = await page.evaluate(async () => {
    driveAccessToken = 'test';
    let release;
    window.waitForLifecycleDriveSave = () => new Promise(resolve => { release = resolve; });
    let uploads = 0;
    window.queueDriveSave = async () => { uploads++; };
    const departing = startLifecycleExitSave(state.driveFileId, Promise.resolve(true));
    await Promise.resolve();
    beginDocumentSession();
    state.driveFileId = 'next-document';
    resetDriveContentSyncVersion();
    release(true);
    await departing;
    return { uploads, dirty: driveDirty, version: driveContentVersion };
  });
  expect(result).toEqual({ uploads: 0, dirty: false, version: 0 });
});
