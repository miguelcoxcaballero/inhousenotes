import { expect, test } from '@playwright/test';

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
    return { saved, elapsed: performance.now() - start, tasks };
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
