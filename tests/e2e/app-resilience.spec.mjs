import { expect, test } from '@playwright/test';

test('production app boots under CSP with external runtime modules', async ({ page }) => {
  const violations = [];
  const pageErrors = [];
  page.on('console', message => {
    if (/Refused to (load|execute|connect|frame|apply)/i.test(message.text())) violations.push(message.text());
  });
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  await expect(page.locator('#welcome-view')).toBeVisible();
  await expect(page.locator('[data-app-version]').first()).toHaveText('v5.11.0');
  expect(await page.evaluate(() => !!(window.pdfjsLib && window.PDFLib && window.jspdf))).toBe(true);
  expect(violations).toEqual([]);
  expect(pageErrors).toEqual([]);
});

test('manage pages opens the embedded Inhouse Scanner below Photo', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(async () => {
    await window.__IHN_TEST_API__.ready();
    await window.__IHN_TEST_API__.resetLocalDocument(1, 'Scanner UI test');
    await window.__IHN_TEST_API__.showEditorForTest();
  });

  await page.locator('#btn-edit-pages').click();
  await expect(page.locator('#btn-set-cover')).toBeVisible();
  await expect(page.locator('#btn-scan-page')).toBeVisible();
  expect(await page.evaluate(() => {
    const photo = document.getElementById('btn-set-cover');
    const scan = document.getElementById('btn-scan-page');
    return !!(photo.compareDocumentPosition(scan) & Node.DOCUMENT_POSITION_FOLLOWING);
  })).toBe(true);

  await page.locator('#btn-scan-page').click();
  await expect(page.locator('#scanner-editor-overlay')).toHaveClass(/visible/);
  await expect(page.locator('#scanner-editor-frame')).toHaveAttribute('src', /scanner\/index\.html.*embed=1/);
  const closed = await page.evaluate(() => window.__IHN_TEST_API__.closeScannerForTest());
  expect(closed).toBe(true);
});

test('multi-page scanner insertion is ordered, durable, and preserves shifted pages', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  const result = await page.evaluate(async () => {
    const api = window.__IHN_TEST_API__;
    await api.ready();
    await api.resetLocalDocument(2, 'Scanner insertion test');
    const shiftedStrokeId = await api.addSyntheticStroke(1, 'shifted-page');
    const insertion = await api.addScannedPagesForTest(2);
    const checkpoint = await api.checkpointBeforeLeaving();
    return { shiftedStrokeId, insertion, checkpoint };
  });

  expect(result.insertion.inserted).toBe(true);
  expect(result.insertion.snapshot.pages).toHaveLength(4);
  expect(result.insertion.snapshot.pages.slice(1, 3).every(pageData => (
    pageData.backgroundSource === 'custom'
    && /^data:image\/jpeg/.test(pageData.backgroundImage || '')
  ))).toBe(true);
  expect(result.insertion.snapshot.pages[3].strokes.map(stroke => stroke.id))
    .toContain(result.shiftedStrokeId);
  expect(new Set(result.insertion.snapshot.pages.map(pageData => pageData.pageId)).size).toBe(4);
  expect(result.checkpoint.snapshot.dirtyPages).toEqual([]);
});

test('embedded scanner exposes its complete document workflow', async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=e2e-scanner', { waitUntil: 'domcontentloaded' });
  await expect(page.locator('#landingPage')).toBeHidden();
  await expect(page.locator('#appContainer')).toBeVisible();
  await expect(page.locator('#closeEmbedBtn')).toBeVisible();
  await expect(page.locator('#btnDesktopAdd')).toBeAttached();
  await expect(page.locator('#cropBtn')).toBeAttached();
  await expect(page.locator('#stencilBtn')).toBeAttached();
  await expect(page.locator('#pageList')).toBeAttached();
  await expect(page.locator('#addToDocumentBtn')).toBeVisible();
  await expect(page.locator('#exportBtn')).toBeVisible();
  await expect(page.locator('#downloadEmbedStencilBtn')).toBeVisible();
});

test('blocked IndexedDB cannot trap startup', async ({ page }) => {
  await page.addInitScript(() => {
    const blockedDatabase = {
      open() {
        const request = {};
        setTimeout(() => request.onblocked?.({ target: request }), 0);
        return request;
      }
    };
    Object.defineProperty(window, 'indexedDB', {
      configurable: true,
      value: blockedDatabase
    });
  });
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__, null, { timeout: 8000 });
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  await expect(page.locator('#welcome-view')).toBeVisible();
});

test('failed IndexedDB page transactions return a bounded error without freezing', async ({ page }) => {
  await page.addInitScript(() => {
    const originalTransaction = IDBDatabase.prototype.transaction;
    IDBDatabase.prototype.transaction = function transaction(storeNames, mode, options) {
      const names = Array.isArray(storeNames) ? storeNames : [storeNames];
      if (mode === 'readwrite' && names.includes('pages')) {
        throw new DOMException('Injected write failure', 'InvalidStateError');
      }
      return originalTransaction.call(this, storeNames, mode, options);
    };
  });
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  const result = await page.evaluate(async () => {
    await window.__IHN_TEST_API__.ready();
    await window.__IHN_TEST_API__.resetLocalDocument(1, 'Failed transaction test');
    await window.__IHN_TEST_API__.addSyntheticStroke(0, 'must-not-hang');
    const started = performance.now();
    try {
      await window.__IHN_TEST_API__.checkpointBeforeLeaving();
      return { failed: false, elapsed: performance.now() - started };
    } catch (error) {
      return { failed: true, elapsed: performance.now() - started, message: error.message };
    }
  });
  expect(result.failed).toBe(true);
  expect(result.elapsed).toBeLessThan(3000);
  expect(result.message).toMatch(/save every changed page/i);
  await expect(page.locator('#welcome-view')).toBeVisible();
});

test('an edit followed immediately by leaving is durable on reload', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  const strokeId = await page.evaluate(async () => {
    await window.__IHN_TEST_API__.resetLocalDocument(2, 'Immediate close test');
    return window.__IHN_TEST_API__.addSyntheticStroke(0, 'last-second');
  });
  const checkpoint = await page.evaluate(() => window.__IHN_TEST_API__.checkpointBeforeLeaving());
  expect(checkpoint.elapsedMs).toBeLessThan(5000);
  expect(checkpoint.snapshot.dirtyPages).toEqual([]);
  const beforeReload = await page.evaluate(async () => {
    const db = await new Promise((resolve, reject) => {
      const request = indexedDB.open('notebook-data-db-v1', 4);
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    const read = (storeName, key) => new Promise(resolve => {
      const request = db.transaction(storeName, 'readonly').objectStore(storeName).get(key);
      request.onsuccess = () => resolve(request.result || null);
      request.onerror = () => resolve(null);
    });
    return {
      metadata: await read('snapshots', 'latest'),
      page: await read('pages', 0),
      backup: localStorage.getItem('notebook-data-v1')
    };
  });
  expect(beforeReload.page?.strokes?.map(stroke => stroke.id)).toContain(strokeId);
  expect(beforeReload.metadata?.data?.pages?.length).toBe(2);

  await page.reload({ waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  await page.evaluate(() => window.__IHN_TEST_API__.loadPage(0));
  const restoredIds = await page.evaluate(() => (
    window.__IHN_TEST_API__.snapshot().pages.flatMap(pageData => pageData.strokes.map(stroke => stroke.id))
  ));
  expect(restoredIds).toContain(strokeId);
});

test('concurrent local and out-of-order peer strokes survive a stale snapshot', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  const result = await page.evaluate(async () => {
    const api = window.__IHN_TEST_API__;
    await api.ready();
    await api.resetLocalDocument(1, 'Concurrent stroke convergence');
    const prepared = await api.prepareLiveDocument('e2e-concurrent-live-file');
    const pageId = prepared.pages[0].pageId;
    const localStrokeId = await api.addSyntheticStroke(0, 'local-concurrent');
    const stalePages = prepared.pages;
    const basePacket = {
      v: 1,
      type: 'live-stroke',
      fileId: 'e2e-concurrent-live-file',
      actorId: 'e2e-remote-peer:e2e-remote-tab',
      strokeId: 'e2e-remote-concurrent-stroke',
      pageId,
      tool: 'pen',
      color: '#654321',
      width: 3,
      syncStamp: { clock: 900, actor: 'e2e-remote-peer:e2e-remote-tab' },
      finalBatch: true,
      totalPoints: 4,
      cancel: false,
      sentAt: Date.now()
    };
    const tail = {
      ...basePacket,
      sequence: 102,
      offset: 2,
      points: [{ x: 40, y: 40, p: 0.5 }, { x: 50, y: 50, p: 0.5 }],
      final: true
    };
    const head = {
      ...basePacket,
      sequence: 101,
      offset: 0,
      points: [{ x: 20, y: 20, p: 0.5 }, { x: 30, y: 30, p: 0.5 }],
      final: false
    };
    const afterPeer = await api.receiveRemoteLiveStrokePackets([tail, head]);
    const afterDuplicate = await api.receiveRemoteLiveStrokePackets([head, tail]);
    const afterStaleMerge = await api.mergeRemotePagesForTest(stalePages);
    const ids = snapshot => snapshot.pages[0].strokes.map(stroke => stroke.id);
    return {
      localStrokeId,
      remoteStrokeId: basePacket.strokeId,
      afterPeerIds: ids(afterPeer),
      afterDuplicateIds: ids(afterDuplicate),
      afterStaleMergeIds: ids(afterStaleMerge.snapshot)
    };
  });

  expect(result.afterPeerIds).toEqual(expect.arrayContaining([
    result.localStrokeId,
    result.remoteStrokeId
  ]));
  expect(result.afterDuplicateIds.filter(id => id === result.remoteStrokeId)).toHaveLength(1);
  expect(result.afterStaleMergeIds).toEqual(expect.arrayContaining([
    result.localStrokeId,
    result.remoteStrokeId
  ]));
});

test('untrusted shared HTML and hostile PDF sources fail closed', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.InhouseSecurityCore);
  const result = await page.evaluate(async () => {
    const core = window.InhouseSecurityCore;
    const malicious = [
      '<div class="sp-day-section evil" onclick="window.pwned=1">',
      '<script>window.pwned=1</script>',
      '<div class="sp-events-list">',
      '<button class="sp-event" data-sp-event="same" style="--sp-event-color:#112233;background:url(javascript:alert(1))">A</button>',
      '<button class="sp-event" data-sp-event="same">duplicate</button>',
      '</div></div>'
    ].join('');
    const sanitized = core.sanitizeCalendarPanelHtml(malicious, document);
    const blocked = ['javascript:alert(1)', 'data:application/pdf;base64,AA==', 'file:///tmp/bad.pdf']
      .map(url => {
        try {
          core.hardenPdfDocumentParams({ url }, { locationHref: location.href });
          return false;
        } catch (error) {
          return true;
        }
      });
    let largeBlocked = false;
    try {
      core.hardenPdfDocumentParams({ data: new ArrayBuffer(core.MAX_PDF_BYTES + 1) });
    } catch (error) {
      largeBlocked = true;
    }
    const safeParams = core.hardenPdfDocumentParams({ url: 'blob:http://127.0.0.1/test' });
    const malformedStarted = performance.now();
    let malformedBlocked = false;
    try {
      await core.loadPdfDocument(window.pdfjsLib, { data: new Uint8Array([0, 1, 2, 3]) });
    } catch (error) {
      malformedBlocked = true;
    }
    return {
      sanitized,
      blocked,
      largeBlocked,
      malformedBlocked,
      malformedElapsed: performance.now() - malformedStarted,
      evalAllowed: safeParams.isEvalSupported
    };
  });
  expect(result.sanitized).not.toMatch(/script|onclick|javascript:|evil/);
  expect((result.sanitized.match(/data-sp-event="same"/g) || []).length).toBe(1);
  expect(result.blocked).toEqual([true, true, true]);
  expect(result.largeBlocked).toBe(true);
  expect(result.malformedBlocked).toBe(true);
  expect(result.malformedElapsed).toBeLessThan(2000);
  expect(result.evalAllowed).toBe(false);
});

test('slow network work is bounded instead of hanging the app', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.InhouseRuntimeCore);
  const result = await page.evaluate(async () => {
    const started = performance.now();
    try {
      await window.InhouseRuntimeCore.fetchWithDeadline('/__slow?delay=3000', {}, 1000);
      return { timedOut: false, elapsed: performance.now() - started };
    } catch (error) {
      return { timedOut: error.name === 'TimeoutError', elapsed: performance.now() - started };
    }
  });
  expect(result.timedOut).toBe(true);
  expect(result.elapsed).toBeLessThan(2000);
});
