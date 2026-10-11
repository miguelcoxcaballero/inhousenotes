import { expect, test } from '@playwright/test';

async function openSavedEditor(page) {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(async () => {
    await window.__IHN_TEST_API__.resetLocalDocument(1, 'Save status test');
    await window.__IHN_TEST_API__.showEditorForTest();
    await queueSave();
    refreshSaveIndicatorState();
  });
  await expect(page.locator('#save-status-label')).toHaveText('Local');
}

function saveRow(page, name) {
  return page.locator('.save-device-status').filter({ has: page.locator('.save-device-status-title', { hasText: name }) });
}

test('save details open by keyboard without changing editor mode and restore focus', async ({ page }) => {
  await openSavedEditor(page);
  const trigger = page.locator('.mode-toggle-status');
  const mode = page.locator('#mode-toggle-btn');
  const pressed = await mode.getAttribute('aria-pressed');
  await expect(trigger).toHaveJSProperty('tagName', 'BUTTON');

  await trigger.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog', { name: 'Save status' })).toBeVisible();
  await expect(trigger).toHaveAttribute('aria-expanded', 'true');
  await expect(page.getByRole('button', { name: 'Close save details' })).toBeFocused();
  await expect(mode).toHaveAttribute('aria-pressed', pressed);
  await expect(saveRow(page, 'This device').locator('.save-device-status-label')).toHaveText('Saved');
  await expect(saveRow(page, 'Google Drive').locator('.save-device-status-label')).toHaveText('Not connected');

  await page.keyboard.press('Escape');
  await expect(page.locator('.save-debug-panel')).not.toBeVisible();
  await expect(trigger).toHaveAttribute('aria-expanded', 'false');
  await expect(trigger).toBeFocused();
  await page.keyboard.press('Space');
  await expect(page.locator('.save-debug-panel')).toBeVisible();
  await page.getByRole('button', { name: 'Close save details' }).click();
  await expect(trigger).toBeFocused();
  await expect(mode).toHaveAttribute('aria-pressed', pressed);
});

test('local changes and storage failures cannot appear as saved', async ({ page }) => {
  await openSavedEditor(page);
  await page.evaluate(() => {
    saveQueued = true;
    refreshSaveIndicatorState();
  });
  await page.locator('.mode-toggle-status').click();
  await expect(page.locator('.mode-toggle-status')).toHaveAttribute('aria-label', 'Saving on this device. Open save details');
  await expect(saveRow(page, 'This device').locator('.save-device-status-label')).toHaveText('Saving');
  await expect(page.getByRole('button', { name: 'Save now', exact: true })).toBeEnabled();

  await page.evaluate(() => {
    noteLocalSaveFailure();
    refreshSaveIndicatorState();
  });
  await expect(page.locator('.mode-toggle-status')).toHaveAttribute('aria-label', 'Save needs attention. Open save details');
  await expect(saveRow(page, 'This device').locator('.save-device-status-label')).toHaveText('Needs attention');
  await expect(page.locator('.save-debug-notice')).toContainText('Keep this document open');
  await expect(page.getByRole('button', { name: 'Retry save', exact: true })).toBeEnabled();
});

test('Drive upload and offline waiting remain separate from the saved local copy', async ({ page, context }) => {
  await openSavedEditor(page);
  await page.evaluate(() => {
    state.driveAutosave = true;
    state.driveFileId = 'save-status-fixture';
    driveContentVersion = 2;
    driveUploadedContentVersion = 1;
    driveDirty = true;
    driveSaveInProgress = true;
    driveSavePhase = 'uploading';
    driveUploadProgress = { sent: 512, total: 1024 };
    refreshSaveIndicatorState();
  });
  await page.locator('.mode-toggle-status').click();
  await expect(page.locator('.mode-toggle-status')).toHaveAttribute('aria-label', 'Saving to Google Drive. Open save details');
  await expect(saveRow(page, 'This device').locator('.save-device-status-label')).toHaveText('Saved');
  await expect(saveRow(page, 'Google Drive').locator('.save-device-status-label')).toHaveText('Uploading 50%');
  await expect(page.locator('.save-debug-action')).toBeHidden();

  await context.setOffline(true);
  await page.evaluate(() => {
    driveSaveInProgress = false;
    driveSavePhase = 'idle';
    driveUploadProgress = null;
    refreshSaveIndicatorState();
  });
  await expect(page.locator('.mode-toggle-status')).toHaveAttribute('aria-label', 'Waiting for connection. Open save details');
  await expect(saveRow(page, 'This device').locator('.save-device-status-label')).toHaveText('Saved');
  await expect(saveRow(page, 'Google Drive').locator('.save-device-status-label')).toHaveText('Offline');
  await expect(page.locator('.save-debug-notice')).toBeHidden();
  await expect(page.locator('.save-debug-action')).toBeHidden();

  await page.evaluate(() => {
    noteLocalSaveFailure();
    refreshSaveIndicatorState();
  });
  const retry = page.getByRole('button', { name: 'Retry save', exact: true });
  await expect(retry).toBeEnabled();
  await retry.click();
  await expect(saveRow(page, 'This device').locator('.save-device-status-label')).toHaveText('Saved');
  await expect(page.locator('.mode-toggle-status')).toHaveAttribute('aria-label', 'Waiting for connection. Open save details');
  await expect(saveRow(page, 'Google Drive').locator('.save-device-status-label')).toHaveText('Offline');
});

test('a recovered operation save clears the popup error along with the indicator', async ({ page }) => {
  await openSavedEditor(page);
  await page.locator('.mode-toggle-status').click();
  const result = await page.evaluate(() => {
    noteLocalSaveFailure('operations');
    setSaveDebugStatus('Local save failed', { error: true });
    refreshSaveIndicatorState();
    const before = document.querySelector('.save-device-status-label').textContent;
    confirmLocalSave(Date.now(), 'operations');
    refreshSaveIndicatorState();
    renderSaveDebugPopup();
    return {
      before,
      localStatus: document.querySelector('.save-device-status-label').textContent,
      noticeHidden: document.querySelector('.save-debug-notice').hidden,
      savedIndicator: document.getElementById('save-indicator').classList.contains('saved'),
      retryVisible: !document.querySelector('.save-debug-action').hidden
    };
  });
  expect(result).toEqual({
    before: 'Needs attention', localStatus: 'Saved', noticeHidden: true, savedIndicator: true, retryVisible: false
  });
});

test('a local save failure is retried before reconnecting Google Drive', async ({ page }) => {
  await openSavedEditor(page);
  await page.evaluate(() => {
    state.driveAutosave = true;
    state.driveFileId = 'reconnect-status-fixture';
    driveDirty = true;
    driveContentVersion = 2;
    driveUploadedContentVersion = 1;
    driveAuthNeedsReconnect = true;
    window.__saveActionCalls = { signIn: 0, upload: 0 };
    window.signInDrive = () => { window.__saveActionCalls.signIn++; };
    window.queueDriveSave = async () => { window.__saveActionCalls.upload++; };
    noteLocalSaveFailure();
    refreshSaveIndicatorState();
  });
  await page.locator('.mode-toggle-status').click();
  await expect(saveRow(page, 'This device').locator('.save-device-status-label')).toHaveText('Needs attention');
  const retry = page.getByRole('button', { name: 'Retry save', exact: true });
  await expect(retry).toBeEnabled();
  await retry.click();
  await expect(saveRow(page, 'This device').locator('.save-device-status-label')).toHaveText('Saved');
  await expect(saveRow(page, 'Google Drive').locator('.save-device-status-label')).toHaveText('Sign in needed');
  await expect(page.getByRole('button', { name: 'Reconnect Drive', exact: true })).toBeEnabled();
  expect(await page.evaluate(() => window.__saveActionCalls)).toEqual({ signIn: 0, upload: 0 });
});

test('connection changes refresh the save label without another edit', async ({ page, context }) => {
  await openSavedEditor(page);
  await page.evaluate(() => {
    state.driveAutosave = true;
    state.driveFileId = 'connection-status-fixture';
    driveDirty = true;
    driveContentVersion = 2;
    driveUploadedContentVersion = 1;
    refreshSaveIndicatorState();
  });
  await expect(page.locator('#save-status-label')).toHaveText('Pending');
  await context.setOffline(true);
  await expect(page.locator('#save-status-label')).toHaveText('Offline');
  await context.setOffline(false);
  await expect(page.locator('#save-status-label')).toHaveText('Pending');
});

for (const visible of [false, true]) {
  test(`${visible ? 'unchanged visible' : 'closed'} save details do not mutate their DOM during status updates`, async ({ page }) => {
    await openSavedEditor(page);
    await page.locator('.mode-toggle-status').click();
    if (!visible) await page.keyboard.press('Escape');
    const mutations = await page.evaluate(() => {
      const observer = new MutationObserver(() => {});
      observer.observe(document.querySelector('.save-debug-panel'), {
        attributes: true, childList: true, characterData: true, subtree: true
      });
      for (let index = 0; index < 20; index++) {
        recordSaveDebug('Local save status refreshed');
        renderSaveDebugPopup();
      }
      const records = observer.takeRecords().map(record => ({ type: record.type, attribute: record.attributeName }));
      observer.disconnect();
      return records;
    });
    expect(mutations).toEqual([]);
  });
}

for (const viewport of [{ width: 1280, height: 720 }, { width: 390, height: 844 }, { width: 320, height: 568 }]) {
  test(`save details fit the ${viewport.width} × ${viewport.height} viewport`, async ({ page }, testInfo) => {
    await page.setViewportSize(viewport);
    await openSavedEditor(page);
    const trigger = page.locator('.mode-toggle-status');
    await expect(trigger).toBeVisible();
    await expect(page.locator('#save-status-label')).toBeHidden();
    const appearance = await trigger.evaluate(element => {
      const style = getComputedStyle(element);
      const dotStyle = getComputedStyle(element.querySelector('.save-indicator'), '::after');
      return { border: style.borderWidth, background: style.backgroundColor, shadow: style.boxShadow,
        dotShadow: dotStyle.boxShadow, width: element.offsetWidth, height: element.offsetHeight };
    });
    expect(appearance).toMatchObject({ border: '0px', background: 'rgba(0, 0, 0, 0)', shadow: 'none', dotShadow: 'none' });
    expect(appearance.width).toBeGreaterThanOrEqual(40);
    expect(appearance.height).toBeGreaterThanOrEqual(40);
    await trigger.click();
    const panel = page.locator('.save-debug-panel');
    await expect(panel).toBeVisible();
    const bounds = await panel.boundingBox();
    expect(bounds.x).toBeGreaterThanOrEqual(0);
    expect(bounds.y).toBeGreaterThanOrEqual(0);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(viewport.width);
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(viewport.height);
    expect(await panel.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
    expect(bounds.height).toBeLessThan(180);
    await expect(panel.locator('.save-device-status')).toHaveCount(2);
    await expect(panel.getByRole('button')).toHaveCount(1);
    await expect(page.locator('.save-debug-notice')).toBeHidden();
    await page.screenshot({ path: testInfo.outputPath('save-status.png') });
    await expect(page.getByRole('button', { name: 'Close save details' })).toBeInViewport();
  });
}
