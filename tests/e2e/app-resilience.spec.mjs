import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
const releaseVersion = JSON.parse(readFileSync(new URL('../../package.json', import.meta.url))).version;

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
  await expect(page.locator('[data-app-version]')).toHaveText(['v' + releaseVersion, 'v' + releaseVersion]);
  expect(await page.evaluate(() => !!(window.pdfjsLib && window.PDFLib && window.jspdf))).toBe(true);
  expect(violations).toEqual([]);
  expect(pageErrors).toEqual([]);
});

test('Drive resumable PDF upload resumes at the server-confirmed byte after a dropped chunk', async ({ page }) => {
  const uploadRanges = [];
  let interruptedFirstChunk = false;
  await page.route('https://www.googleapis.com/**', async route => {
    const request = route.request();
    const headers = {
      'Access-Control-Allow-Origin': '*',
      'Access-Control-Allow-Headers': '*',
      'Access-Control-Allow-Methods': 'GET,POST,PATCH,PUT,OPTIONS',
      'Access-Control-Expose-Headers': 'Location,Range'
    };
    if (request.method() === 'OPTIONS') {
      await route.fulfill({ status: 204, headers });
      return;
    }
    if (request.method() === 'POST' && request.url().includes('uploadType=resumable')) {
      await route.fulfill({
        status: 200,
        headers: { ...headers, Location: 'https://www.googleapis.com/upload/drive/v3/files/session-e2e' },
        body: ''
      });
      return;
    }
    if (request.url().endsWith('/session-e2e') && request.method() === 'PUT') {
      const range = request.headers()['content-range'] || '';
      uploadRanges.push(range);
      if (range.startsWith('bytes 0-') && !interruptedFirstChunk) {
        interruptedFirstChunk = true;
        await route.abort('timedout');
        return;
      }
      if (range.startsWith('bytes */')) {
        await route.fulfill({ status: 308, headers: { ...headers, Range: 'bytes=0-262143' }, body: '' });
        return;
      }
      if (range.startsWith('bytes 262144-')) {
        await route.fulfill({
          status: 200,
          headers: { ...headers, 'Content-Type': 'application/json' },
          body: JSON.stringify({ id: 'resumed-file', version: '7', headRevisionId: 'rev-resumed' })
        });
        return;
      }
    }
    await route.fulfill({ status: 500, headers, body: 'Unexpected mocked Drive request' });
  });

  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  const result = await page.evaluate(() => window.__IHN_TEST_API__.uploadBlobResumableForTest());
  expect(result.id).toBe('resumed-file');
  expect(uploadRanges).toEqual([
    'bytes 0-614399/614400',
    'bytes */614400',
    'bytes 262144-614399/614400'
  ]);
});

test('Drive upload recovers when headers arrive but the completion body stalls', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  const result = await page.evaluate(async () => {
    await window.__IHN_TEST_API__.ready();
    const originalFetch = window.fetch;
    const originalTimeout = window.setTimeout;
    let probes = 0;
    window.setTimeout = (fn, ms, ...args) => originalTimeout(fn, ms === 60000 ? 50 : ms, ...args);
    window.fetch = async (url, options) => {
      if (!String(url).startsWith('https://www.googleapis.com/')) return originalFetch(url, options);
      if (options.method === 'POST') return new Response('', { headers: { Location: 'https://www.googleapis.com/test-session' } });
      if (options.headers.get('Content-Range').startsWith('bytes */')) {
        probes++;
        return new Response(JSON.stringify({ id: 'confirmed-after-timeout' }));
      }
      return new Response(new ReadableStream({ start(controller) {
        options.signal.addEventListener('abort', () => controller.error(new DOMException('Timed out', 'AbortError')), { once: true });
      } }));
    };
    try { return { file: await window.__IHN_TEST_API__.uploadBlobResumableForTest(), probes }; }
    finally { window.fetch = originalFetch; window.setTimeout = originalTimeout; }
  });
  expect(result.file.id).toBe('confirmed-after-timeout');
  expect(result.probes).toBe(1);
});

test('Drive upload stops repeated zero-progress acknowledgements instead of looping', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  const result = await page.evaluate(async () => {
    await window.__IHN_TEST_API__.ready();
    const originalFetch = window.fetch;
    let chunks = 0;
    window.fetch = async (url, options) => {
      if (!String(url).startsWith('https://www.googleapis.com/')) return originalFetch(url, options);
      if (options.method === 'POST') return new Response('', { headers: { Location: 'https://www.googleapis.com/test-session' } });
      chunks++;
      return new Response('', { status: 308 });
    };
    try { await window.__IHN_TEST_API__.uploadBlobResumableForTest(); return { chunks }; }
    catch (error) { return { chunks, error: error.message }; }
    finally { window.fetch = originalFetch; }
  });
  expect(result.chunks).toBe(4);
  expect(result.error).toContain('did not acknowledge');
});

test('pen-down paints its first point without rebuilding a stale page canvas', async ({ page, context }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(async () => {
    const api = window.__IHN_TEST_API__;
    await api.ready();
    await api.resetLocalDocument(1, 'Immediate pen test');
    await api.showEditorForTest();
  });
  const canvas = page.locator('.page-wrapper[data-page="0"] canvas.page-canvas');
  await expect(canvas).toBeVisible();
  await canvas.scrollIntoViewIfNeeded();
  await page.waitForTimeout(350);
  const target = await page.evaluate(() => {
    const sheet = document.querySelector('.page-wrapper[data-page="0"] canvas.page-canvas');
    const overlay = document.querySelector('.page-wrapper[data-page="0"] canvas.cursor-overlay');
    const rect = sheet.getBoundingClientRect();
    sheet.width -= 1; // Simulate a pending zoom/backing-resolution update.
    return {
      x: Math.round(rect.left + rect.width / 2),
      y: Math.round(rect.top + Math.min(rect.height / 2, 220)),
      staleWidth: sheet.width,
      overlayWidth: overlay.width
    };
  });
  const cdp = await context.newCDPSession(page);
  await cdp.send('Input.dispatchMouseEvent', {
    type: 'mousePressed', x: target.x, y: target.y,
    button: 'left', buttons: 1, clickCount: 1, pointerType: 'pen'
  });
  const firstFrame = await page.evaluate(({ x, y }) => {
    const sheet = document.querySelector('.page-wrapper[data-page="0"] canvas.page-canvas');
    // Definitive pen ink is painted on the page bitmap during contact.
    // Resetting sheet.width above cleared that bitmap, so existing template
    // pixels cannot satisfy this first-contact assertion.
    const rect = sheet.getBoundingClientRect();
    const px = Math.round((x - rect.left) * sheet.width / rect.width);
    const py = Math.round((y - rect.top) * sheet.height / rect.height);
    const data = sheet.getContext('2d').getImageData(px - 6, py - 6, 13, 13).data;
    return { width: sheet.width, painted: data.some((value, index) => index % 4 === 3 && value > 0) };
  }, target);
  expect(firstFrame.width).toBe(target.staleWidth);
  expect(firstFrame.painted).toBe(true);
  await cdp.send('Input.dispatchMouseEvent', {
    type: 'mouseReleased', x: target.x, y: target.y,
    button: 'left', buttons: 0, clickCount: 1, pointerType: 'pen'
  });
});

test('mode changes do not measure every page wrapper in the document', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(async () => {
    const api = window.__IHN_TEST_API__;
    await api.ready();
    await api.resetLocalDocument(12, 'Mode change performance');
    await api.showEditorForTest();
  });
  const measuredWrappers = await page.evaluate(async () => {
    const original = Element.prototype.getBoundingClientRect;
    let count = 0;
    Element.prototype.getBoundingClientRect = function (...args) {
      if (this.matches?.('.page-wrapper[data-page]')) count += 1;
      return original.apply(this, args);
    };
    const toggle = document.getElementById('mode-toggle-btn');
    toggle.click();
    toggle.click();
    await new Promise(resolve => setTimeout(resolve, 80));
    Element.prototype.getBoundingClientRect = original;
    return count;
  });
  expect(measuredWrappers).toBe(0);
});

test('opening a PDF leaves compressed timeline history untouched until Timeline is requested', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  const result = await page.evaluate(async () => {
    const api = window.__IHN_TEST_API__;
    await api.ready();
    const pdf = await window.PDFLib.PDFDocument.create();
    pdf.addPage([595, 842]);
    const bytes = await pdf.save();
    await api.importPdfBlobForTest(Array.from(bytes));
    const stayedLazy = api.hasPendingTimelineRestoreForTest();
    const timeline = await api.openTimelineForTest();
    return { stayedLazy, timeline };
  });
  expect(result.stayedLazy).toBe(true);
  expect(result.timeline.pending).toBe(false);
  expect(result.timeline.visible).toBe(true);
});

test('legacy page migration yields to the editor while the user is active', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(async () => {
    const api = window.__IHN_TEST_API__;
    await api.ready();
    api.noteUserActivityForTest();
    await api.startDeferredPageMigrationForTest(4);
  });
  await page.waitForTimeout(1000);
  const state = await page.evaluate(() => window.__IHN_TEST_API__.deferredMigrationStateForTest());
  expect(state.active).toBe(false);
  expect(state.pendingPages).toBe(4);
  await page.waitForFunction(
    () => window.__IHN_TEST_API__.deferredMigrationStateForTest().pendingPages === 0,
    null,
    { timeout: 7000 }
  );
});

test('Scan pages stays below Photo with a Soon badge and only shows a Soon notice', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(async () => {
    await window.__IHN_TEST_API__.ready();
    await window.__IHN_TEST_API__.resetLocalDocument(1, 'Scan soon test');
    await window.__IHN_TEST_API__.showEditorForTest();
  });

  await page.locator('#btn-edit-pages').click();
  const scan = page.locator('#btn-scan-page');
  await expect(scan).toBeVisible();
  await expect(scan).toBeEnabled();
  await expect(scan.locator('.soon-badge')).toHaveText('Soon');
  expect(await page.evaluate(() => {
    const photo = document.getElementById('btn-set-cover');
    const button = document.getElementById('btn-scan-page');
    return !!(photo.compareDocumentPosition(button) & Node.DOCUMENT_POSITION_FOLLOWING);
  })).toBe(true);

  const framesBefore = page.frames().length;
  await scan.click();
  await expect(page.locator('#save-indicator')).toHaveAttribute('aria-label', 'Soon');
  expect(page.frames().length).toBe(framesBefore);
  expect(await page.locator('iframe[src*="scanner"]').count()).toBe(0);
  await expect(page.locator('#pages-panel')).toHaveClass(/visible/);
});

test('photo page insertion is durable and preserves shifted pages', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  const result = await page.evaluate(async () => {
    const api = window.__IHN_TEST_API__;
    await api.ready();
    await api.resetLocalDocument(2, 'Photo insertion test');
    const shiftedStrokeId = await api.addSyntheticStroke(1, 'shifted-page');
    const insertion = await api.addPhotoPageForTest();
    const checkpoint = await api.checkpointBeforeLeaving();
    return { shiftedStrokeId, insertion, checkpoint };
  });

  expect(result.insertion.inserted).toBe(true);
  expect(result.insertion.snapshot.pages).toHaveLength(3);
  expect(result.insertion.snapshot.pages[1].backgroundSource).toBe('custom');
  expect(result.insertion.snapshot.pages[1].backgroundImage).toMatch(/^data:image\/jpeg/);
  expect(result.insertion.snapshot.pages[2].strokes.map(stroke => stroke.id))
    .toContain(result.shiftedStrokeId);
  expect(new Set(result.insertion.snapshot.pages.map(pageData => pageData.pageId)).size).toBe(3);
  expect(result.checkpoint.snapshot.dirtyPages).toEqual([]);
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


