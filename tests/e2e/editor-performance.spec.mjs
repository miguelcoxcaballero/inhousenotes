import { expect, test } from '@playwright/test';

for (const tool of ['eraser-stroke', 'eraser-area']) {
  test(`${tool} repaints a dense page before the pointer is lifted`, async ({ page, context }) => {
    await page.goto('/?e2e=1');
    await page.waitForFunction(() => window.__IHN_TEST_API__);
    await page.evaluate(tool => window.__IHN_TEST_API__.prepareEraserFixtureForTest(tool), tool);
    await page.waitForFunction(() => !window.__IHN_TEST_API__.densePaintStatusForTest().pending);
    expect((await page.evaluate(() => window.__IHN_TEST_API__.eraserPixelsForTest())).center[0]).toBeLessThan(30);
    const target = await page.evaluate(() => {
      const canvas = document.querySelector('canvas.page-canvas[data-page="0"]');
      canvas.addEventListener('pointerdown', event => { window.__eraserPointerId = event.pointerId; }, { once: true });
      canvas.scrollIntoView();
      const rect = canvas.getBoundingClientRect();
      return { x: rect.left + 153 * rect.width / 794, y: rect.top + 203 * rect.height / 1123 };
    });
    const cdp = await context.newCDPSession(page);
    await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', ...target, button: 'left', buttons: 1, clickCount: 1, pointerType: 'pen' });
    // Keep the pen-active guard hot. Merely holding still would let its idle
    // timer expire and hide the regression where full repaints wait for idle.
    for (let i = 0; i < 10; i++) {
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', ...target, buttons: 1, pointerType: 'pen' });
      await page.waitForTimeout(16);
    }
    const pixels = await page.evaluate(() => window.__IHN_TEST_API__.eraserPixelsForTest());
    expect(pixels.center).toEqual([255, 255, 255, 255]);
    expect(pixels.penActive).toBe(true);
    expect(await page.evaluate(() => document.querySelector('canvas.page-canvas[data-page="0"]').hasPointerCapture(window.__eraserPointerId))).toBe(true);
    const edge = pixels.edge[0];
    if (tool === 'eraser-area') expect(edge).toBeLessThan(30);
    else expect(edge).toBeGreaterThan(200);
    await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', ...target, button: 'left', buttons: 0, clickCount: 1, pointerType: 'pen' });
  });
}

test('remote eraser preview removes dense-page ink before a final gesture or snapshot', async ({ page }) => {
  await page.goto('/?e2e=1');
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.prepareEraserFixtureForTest('pen'));
  await page.waitForFunction(() => !window.__IHN_TEST_API__.densePaintStatusForTest().pending);
  expect((await page.evaluate(() => window.__IHN_TEST_API__.eraserPixelsForTest())).center[0]).toBeLessThan(30);
  await page.evaluate(() => window.__IHN_TEST_API__.receiveErasePreviewForTest());
  await expect.poll(() => page.evaluate(() => window.__IHN_TEST_API__.eraserPixelsForTest()), { timeout: 1000, intervals: [16] }).toMatchObject({ center: [255, 255, 255, 255] });
});

test('lossless direct PDF rasters render identically to the PNG path and keep metadata readable', async ({ page }) => {
  await page.goto('/?e2e=1');
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  const result = await page.evaluate(async () => {
    const processing = window.InhouseDocumentProcessing;
    const input = { json: JSON.stringify({ images: [], strokes: [
      { tool: 'pen', color: '#142dd2', width: 3, points: [{ x: 10, y: 10 }, { x: 40, y: 90 }, { x: 110, y: 20 }] },
      { tool: 'highlighter', color: '#ffdd00', width: 14, points: [{ x: 10, y: 40 }, { x: 110, y: 40 }] },
      { tool: 'eraser-area', width: 7, points: [{ x: 60, y: 0 }, { x: 60, y: 120 }] }
    ] }), bounds: { x: 0, y: 0, width: 128, height: 128 }, scale: 1 };
    const png = await processing.run('renderOverlay', input);
    const raw = await processing.run('renderOverlay', { ...input, pdfRaster: true });
    const workerLib = await processing.pdfLib(window.PDFLib);
    async function render(library, pixels) {
      const doc = await library.PDFDocument.create();
      try {
        const sheet = await doc.addPage([128, 128]);
        const image = await doc.embedPng(pixels);
        await sheet.drawImage(image, { x: 0, y: 0, width: 128, height: 128 });
        await doc.setKeywords(['STROKES_Z:YWJjZA==;IH_TEST:123']);
        const pdf = await window.pdfjsLib.getDocument({ data: await doc.save() }).promise;
        const metadata = (await pdf.getMetadata()).info.Keywords;
        const pdfPage = await pdf.getPage(1);
        const canvas = document.createElement('canvas'); canvas.width = canvas.height = 128;
        await pdfPage.render({ canvasContext: canvas.getContext('2d'), viewport: pdfPage.getViewport({ scale: 1 }) }).promise;
        const samples = canvas.getContext('2d').getImageData(0, 0, 128, 128).data;
        await pdf.destroy();
        return { samples, metadata };
      } finally { await doc.dispose?.(); }
    }
    const expected = await render(window.PDFLib, png);
    const actual = await render(workerLib, raw);
    const imported = await render(workerLib, png);
    return { rawMatches: expected.samples.every((v, i) => v === actual.samples[i]),
      pngMatches: expected.samples.every((v, i) => v === imported.samples[i]), metadata: actual.metadata };
  });
  expect(result).toEqual({ rawMatches: true, pngMatches: true, metadata: 'STROKES_Z:YWJjZA==;IH_TEST:123' });
});

test('background ink renderer preserves pen, highlighter and eraser pixels', async ({ page }) => {
  await page.goto('/?e2e=1');
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  const difference = await page.evaluate(() => window.__IHN_TEST_API__.compareWorkerInkForTest([
    { tool: 'pen', color: '#123456', width: 3, points: [{ x: 10, y: 10 }, { x: 60, y: 60 }, { x: 100, y: 20 }] },
    { tool: 'highlighter', color: '#ffde00', width: 12, points: [{ x: 10, y: 10 }, { x: 60, y: 60 }, { x: 100, y: 20 }] },
    { tool: 'highlighter', color: '#ffde00', width: 12, points: [{ x: 80, y: 80 }] },
    { tool: 'eraser-area', width: 8, points: [{ x: 40, y: 10 }, { x: 40, y: 70 }] }
  ]));
  expect(difference).toBeLessThanOrEqual(1);
});

test('interactions do not discard PDF preparation work', async ({ page }) => {
  await page.goto('/?e2e=1');
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  expect(await page.evaluate(() => window.__IHN_TEST_API__.interactionDuringPreparationForTest())).toEqual({ aborted: false });
});

test('pen interaction does not cancel an upload already in flight', async ({ page }) => {
  let putStarted;
  const started = new Promise(resolve => { putStarted = resolve; });
  await page.route('https://www.googleapis.com/**', async route => {
    const headers = { 'Access-Control-Allow-Origin': '*', 'Access-Control-Allow-Headers': '*',
      'Access-Control-Allow-Methods': '*', 'Access-Control-Expose-Headers': 'Location' };
    if (route.request().method() === 'POST') {
      await route.fulfill({ status: 200, headers: { ...headers, Location: 'https://www.googleapis.com/upload/test-session' }, body: '' });
    } else if (route.request().method() === 'PUT') {
      putStarted();
      await new Promise(resolve => setTimeout(resolve, 300));
      await route.fulfill({ status: 200, headers, body: JSON.stringify({ id: 'saved-while-writing' }) });
    } else await route.fulfill({ status: 204, headers });
  });
  await page.goto('/?e2e=1');
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  const saving = page.evaluate(() => window.__IHN_TEST_API__.uploadBlobResumableForTest(1024, true));
  await started;
  await page.evaluate(() => document.dispatchEvent(new PointerEvent('pointerdown', { pointerType: 'pen', buttons: 1, pressure: 0.5, bubbles: true })));
  expect((await saving).id).toBe('saved-while-writing');
});

test('dense handwriting repaint leaves time for pen input', async ({ page }) => {
  await page.goto('/?e2e=1');
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.seedDensePageForTest());
  await page.waitForTimeout(800);
  const duration = await page.evaluate(() => window.__IHN_TEST_API__.redrawDensePageForTest());
  console.log('Dense page synchronous redraw ms:', duration);
  expect(duration).toBeLessThan(30);
});

test('dense PDF save keeps the event loop responsive', async ({ page, context }) => {
  await page.addInitScript(() => {
    const NativeWorker = window.Worker;
    window.__processingTimes = [];
    window.Worker = class extends NativeWorker {
      constructor(...args) {
        super(...args);
        this.pending = new Map();
        this.addEventListener('message', ({ data }) => {
          const task = this.pending.get(data?.id);
          if (task) window.__processingTimes.push({ type: task.type, ms: performance.now() - task.start, error: data.error });
          this.pending.delete(data?.id);
        });
      }
      postMessage(data, ...args) {
        if (data?.type && data?.id) this.pending.set(data.id, { type: data.type, start: performance.now() });
        return super.postMessage(data, ...args);
      }
    };
  });
  await page.goto('/?e2e=1');
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.seedDensePageForTest());
  await page.waitForTimeout(1000);
  const cdp = await context.newCDPSession(page);
  await cdp.send('Performance.enable');
  await cdp.send('Profiler.enable');
  await cdp.send('Profiler.start');
  const result = await page.evaluate(async () => {
    const tasks = [];
    const observer = new PerformanceObserver(list => tasks.push(...list.getEntries().map(e => ({ start: e.startTime, duration: e.duration }))));
    observer.observe({ type: 'longtask' });
    const start = performance.now();
    const saved = await window.__IHN_TEST_API__.buildPdfBlobForTest();
    await new Promise(resolve => setTimeout(resolve, 50));
    observer.disconnect();
    return { saved, elapsed: performance.now() - start, tasks, processing: window.__processingTimes };
  });
  console.log('Dense PDF save:', result);
  const { profile } = await cdp.send('Profiler.stop');
  const metrics = await cdp.send('Performance.getMetrics');
  const navigationStart = metrics.metrics.find(m => m.name === 'NavigationStart').value * 1000;
  const longest = result.tasks.slice().sort((a,b) => b.duration - a.duration)[0];
  let sampleTime = profile.startTime / 1000;
  const slow = new Map();
  const totals = new Map();
  const nodes = new Map(profile.nodes.map(node => [node.id, node]));
  profile.samples.forEach((id, i) => {
    sampleTime += profile.timeDeltas[i] / 1000;
    const frame = nodes.get(id).callFrame;
    const key = `${frame.functionName}:${frame.lineNumber + 1}`;
    if (longest && sampleTime - navigationStart >= longest.start && sampleTime - navigationStart <= longest.start + longest.duration) {
      slow.set(key, (slow.get(key) || 0) + profile.timeDeltas[i] / 1000);
    }
    totals.set(key, (totals.get(key) || 0) + profile.timeDeltas[i] / 1000);
  });
  console.log('Main thread CPU ms', [...totals].sort((a,b) => b[1] - a[1]).slice(0, 20));
  console.log('Longest task CPU:', [...slow].sort((a,b) => b[1] - a[1]).slice(0, 12));
  expect(result.saved.ok).toBe(true);
  // Allow CI/CPU variance, but reject the former multi-second freezes.
  expect(Math.max(0, ...result.tasks.map(t => t.duration))).toBeLessThan(250);
});

test('a stroke added during a staged repaint survives the completed repaint', async ({ page }) => {
  await page.goto('/?e2e=1');
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(async () => {
    await window.__IHN_TEST_API__.seedDensePageForTest();
    window.__IHN_TEST_API__.redrawDensePageForTest();
    await window.__IHN_TEST_API__.addSyntheticStroke(0, 'during-repaint');
  });
  await page.waitForFunction(() => !window.__IHN_TEST_API__.densePaintStatusForTest().pending);
  const status = await page.evaluate(() => window.__IHN_TEST_API__.densePaintStatusForTest());
  expect(status.count).toBe(2401);
  expect(status.pixel[0]).toBeLessThan(80);
  expect(status.pixel[3]).toBe(255);
});

test('saving immediately after reopening restores history and starts source recovery despite a queued save', async ({ page }) => {
  await page.goto('/?e2e=1');
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  const first = await page.evaluate(async () => {
    const doc = await window.PDFLib.PDFDocument.create();
    doc.addPage([200, 300]);
    await window.__IHN_TEST_API__.primePdfPageForTest(Array.from(await doc.save()));
    return window.__IHN_TEST_API__.buildPdfBlobForTest({ includeBytes: true, milestone: 'Before reopen' });
  });
  expect(first.ok).toBe(true);
  expect(first.history.length).toBeGreaterThan(0);
  const second = await page.evaluate(async bytes => {
    await window.__IHN_TEST_API__.importPdfBlobForTest(bytes);
    await window.__IHN_TEST_API__.addSyntheticStroke(0, 'after-reopen');
    return window.__IHN_TEST_API__.buildPdfBlobForTest({ queued: true });
  }, first.bytes);
  expect(second.ok).toBe(true);
  expect(second.hasLegacyBakedOverlay).toBe(false);
  expect(second.history.map(entry => entry.id)).toContain(first.history[0].id);
});
