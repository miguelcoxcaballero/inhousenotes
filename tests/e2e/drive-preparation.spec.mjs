import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(async () => {
    await __IHN_TEST_API__.resetLocalDocument(1, 'Drive preparation');
    await queueSave();
    const previous = {
      build: buildPdfBlob, prepared: preparedDrivePdf, building: preparedDrivePdfBuild,
      timer: preparedDrivePdfTimer, version: driveContentVersion, uploaded: driveUploadedContentVersion,
      session: documentSessionId, autosave: state.driveAutosave, fileId: state.driveFileId,
      readOnly: state.isReadOnly, token: driveAccessToken, activity: lastUserInteractionAt,
      saving: driveSaveInProgress, phase: driveSavePhase, dirty: driveDirty,
      reconnect: driveAuthNeedsReconnect, blocked: driveSaveBlockedReason,
      localLock: localPageStructureMutationToken, remoteLock: remotePageMergeToken,
      strokeFlush: strokeOpsFlushPromise, strokeFlushing: strokeOpsFlushInProgress
    };
    clearTimeout(preparedDrivePdfTimer);
    preparedDrivePdfTimer = null;
    preparedDrivePdf = null;
    preparedDrivePdfBuild = null;
    driveContentVersion = 10;
    driveUploadedContentVersion = 9;
    state.driveAutosave = false;
    driveAccessToken = null;
    lastUserInteractionAt = 0;
    const fixture = window.__drivePreparationTest = {
      previous, builds: [], requests: [], releaseLocks: [], active: 0, maxActive: 0,
      async until(condition) {
        const deadline = performance.now() + 5000;
        while (!condition()) {
          if (performance.now() >= deadline) throw new Error('Preparation fixture did not settle');
          await new Promise(resolve => setTimeout(resolve, 1));
        }
      },
      start(options = {}) {
        const request = { settled: false };
        request.promise = prepareDrivePdfBlob(options);
        request.promise.then(() => { request.settled = true; }, () => { request.settled = true; });
        this.requests.push(request);
        return request.promise;
      },
      async finish() {
        await this.until(() => {
          this.builds.forEach(build => build.resolve());
          return this.requests.every(request => request.settled);
        });
      }
    };
    buildPdfBlob = async options => {
      fixture.active += 1;
      fixture.maxActive = Math.max(fixture.maxActive, fixture.active);
      let resolve;
      const gate = new Promise(done => { resolve = done; });
      const blob = new Blob([`PDF fixture ${fixture.builds.length}`], { type: 'application/pdf' });
      fixture.builds.push({ options, resolve, blob });
      try {
        await gate;
        return blob;
      } finally {
        fixture.active -= 1;
      }
    };
  });
});

test.afterEach(async ({ page }) => {
  await page.evaluate(async () => {
    const fixture = window.__drivePreparationTest;
    if (!fixture) return;
    clearTimeout(preparedDrivePdfTimer);
    preparedDrivePdfTimer = null;
    const p = fixture.previous;
    fixture.releaseLocks.forEach(release => release());
    localPageStructureMutationToken = p.localLock;
    strokeOpsFlushPromise = p.strokeFlush;
    strokeOpsFlushInProgress = p.strokeFlushing;
    await fixture.finish();
    remotePageMergeToken = p.remoteLock;
    buildPdfBlob = p.build;
    preparedDrivePdf = p.prepared;
    preparedDrivePdfBuild = p.building;
    preparedDrivePdfTimer = p.timer;
    driveContentVersion = p.version;
    driveUploadedContentVersion = p.uploaded;
    documentSessionId = p.session;
    state.driveAutosave = p.autosave;
    state.driveFileId = p.fileId;
    state.isReadOnly = p.readOnly;
    driveAccessToken = p.token;
    lastUserInteractionAt = p.activity;
    driveSaveInProgress = p.saving;
    driveSavePhase = p.phase;
    driveDirty = p.dirty;
    driveAuthNeedsReconnect = p.reconnect;
    driveSaveBlockedReason = p.blocked;
    delete window.__drivePreparationTest;
  });
});

test('prepared PDFs are reused only for the same document session and version', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const fixture = __drivePreparationTest;
    async function build() {
      const pending = fixture.start();
      await fixture.finish();
      return pending;
    }
    const first = await build();
    const cached = await fixture.start();
    const countAfterCacheHit = fixture.builds.length;
    driveContentVersion += 1;
    invalidatePreparedDrivePdf();
    const edited = await build();
    documentSessionId += 1;
    const nextDocument = await build();
    return {
      reused: cached === first && cached.blob === first.blob, countAfterCacheHit,
      editedBlobChanged: edited.blob !== first.blob, version: edited.version,
      nextSessionChanged: nextDocument.sessionToken !== edited.sessionToken,
      nextDocumentBlobChanged: nextDocument.blob !== edited.blob,
      builds: fixture.builds.length, maxActive: fixture.maxActive
    };
  });
  expect(result).toEqual({
    reused: true, countAfterCacheHit: 1, editedBlobChanged: true, version: 11,
    nextSessionChanged: true, nextDocumentBlobChanged: true, builds: 3, maxActive: 1
  });
});

test('waiters for an invalidated build share exactly one successor PDF', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const fixture = __drivePreparationTest;
    const owner = fixture.start();
    await fixture.until(() => fixture.builds.length === 1);
    driveContentVersion += 1;
    invalidatePreparedDrivePdf();
    const waiterA = fixture.start();
    const waiterB = fixture.start();
    fixture.builds[0].resolve();
    await fixture.finish();
    const results = await Promise.all([owner, waiterA, waiterB]);
    return {
      builds: fixture.builds.length, maxActive: fixture.maxActive,
      versions: results.map(result => result.version),
      sameBlob: results.every(result => result.blob === results[0].blob),
      buildReleased: preparedDrivePdfBuild === null
    };
  });
  expect(result).toEqual({ builds: 2, maxActive: 1, versions: [11, 11, 11], sameBlob: true, buildReleased: true });
});

test('aborting a waiting caller does not wait for or cancel the shared PDF build', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const fixture = __drivePreparationTest;
    const owner = fixture.start();
    await fixture.until(() => fixture.builds.length === 1);
    const controller = new AbortController();
    const waiting = fixture.start({ signal: controller.signal });
    controller.abort();
    let timeout;
    const outcome = await Promise.race([
      waiting.then(() => 'resolved', error => error.name),
      new Promise(resolve => { timeout = setTimeout(() => resolve('still waiting'), 300); })
    ]);
    clearTimeout(timeout);
    const ownerStillActive = fixture.active === 1 && !fixture.requests[0].settled;
    const ownerWasCancelled = fixture.builds[0].options.signal.aborted;
    fixture.builds[0].resolve();
    const completed = await owner;
    await fixture.finish();
    return { outcome, ownerStillActive, ownerWasCancelled, builds: fixture.builds.length, version: completed.version };
  });
  expect(result).toEqual({ outcome: 'AbortError', ownerStillActive: true, ownerWasCancelled: false, builds: 1, version: 10 });
});

test('repeated scheduling cannot postpone preparation of the same pending version', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const fixture = __drivePreparationTest;
    state.driveAutosave = true;
    state.isReadOnly = false;
    driveAccessToken = 'preparation-fixture';
    schedulePreparedDrivePdf(80);
    const firstTimer = preparedDrivePdfTimer;
    schedulePreparedDrivePdf(5000);
    const sameTimer = preparedDrivePdfTimer === firstTimer;
    await fixture.until(() => fixture.builds.length > 0);
    fixture.builds[0].resolve();
    await fixture.until(() => preparedDrivePdfBuild === null);
    return { sameTimer, builds: fixture.builds.length, cachedVersion: preparedDrivePdf.version };
  });
  expect(result).toEqual({ sameTimer: true, builds: 1, cachedVersion: 10 });
});

for (const lock of ['page structure', 'stroke checkpoint']) {
  test(`cancelling preparation while waiting for the ${lock} releases the queue for retry`, async ({ page }) => {
    const result = await page.evaluate(async lockName => {
      const fixture = __drivePreparationTest;
      let release;
      if (lockName === 'page structure') {
        const token = {};
        localPageStructureMutationToken = token;
        release = () => {
          if (localPageStructureMutationToken === token) localPageStructureMutationToken = null;
        };
      } else {
        let resolve;
        const pending = new Promise(done => { resolve = done; });
        strokeOpsFlushPromise = pending;
        release = () => {
          if (strokeOpsFlushPromise === pending) strokeOpsFlushPromise = null;
          resolve();
        };
      }
      fixture.releaseLocks.push(release);
      const controller = new AbortController();
      const waiting = fixture.start({ signal: controller.signal });
      await fixture.until(() => preparedDrivePdfBuild !== null);
      controller.abort();
      let timeout;
      const outcome = await Promise.race([
        waiting.then(() => 'resolved', error => error.name),
        new Promise(resolve => { timeout = setTimeout(() => resolve('still waiting'), 300); })
      ]);
      clearTimeout(timeout);
      const buildsBeforeUnlock = fixture.builds.length;
      // The cancelled owner must relinquish preparation even though the
      // unrelated editor operation still holds its lock.
      await fixture.until(() => preparedDrivePdfBuild === null);
      const lockStillHeld = lockName === 'page structure'
        ? localPageStructureMutationToken !== null : strokeOpsFlushPromise !== null;
      release();
      const retry = fixture.start();
      await fixture.finish();
      const prepared = await retry;
      return {
        outcome, buildsBeforeUnlock, lockStillHeld, builds: fixture.builds.length,
        version: prepared.version, queueReleased: preparedDrivePdfBuild === null,
        remoteLockReleased: remotePageMergeToken === null
      };
    }, lock);
    expect(result).toEqual({
      outcome: 'AbortError', buildsBeforeUnlock: 0, lockStillHeld: true, builds: 1,
      version: 10, queueReleased: true, remoteLockReleased: true
    });
  });
}

test('remote verification is presented separately from PDF preparation', async ({ page }) => {
  const result = await page.evaluate(() => {
    state.driveAutosave = true;
    state.driveFileId = 'preparation-status-fixture';
    driveSaveInProgress = true;
    driveAuthNeedsReconnect = false;
    driveSaveBlockedReason = '';
    driveSavePhase = 'building';
    const building = getSavePresentation();
    driveSavePhase = 'verifying';
    const verifying = getSavePresentation();
    return {
      buildingLabel: building.driveLabel, verifyingLabel: verifying.driveLabel,
      verifyingTitle: verifying.title, verifyingDetail: verifying.driveDetail
    };
  });
  expect(result.buildingLabel).toBe('Preparing');
  expect(result.verifyingLabel).toBe('Checking Drive');
  expect(result.verifyingTitle).toBe('Checking Google Drive');
  expect(result.verifyingDetail).toMatch(/checking.*Drive/i);
});
