import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=worker-pipeline', { waitUntil: 'domcontentloaded' });
});

test('three photographs complete their full corrections concurrently outside the UI thread', async ({ page }) => {
  test.setTimeout(45_000);
  const workers = [];
  page.on('worker', worker => workers.push(worker.url()));
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-developable.js' });
  const result = await page.evaluate(async () => {
    const sources = [0, 1, 2].map(index => ScannerDevelopableFixtures.render(ScannerDevelopableFixtures.CASES[index]).source);
    const blobs = await Promise.all(sources.map(source => new Promise(resolve => source.toBlob(resolve, 'image/jpeg', .94))));
    sources.forEach(source => { source.width = source.height = 0; });
    // Any accidental UI-thread fallback must fail this test, including warp
    // and cleanup after an otherwise successful detection worker.
    for (const name of ['detectPage', 'warp', 'correctColors']) {
      ScannerPro.Lightweight[name] = () => { throw new Error(`${name} ran on the UI thread`); };
    }
    const starts = [];
    const finishes = [];
    let ticks = 0;
    const interval = setInterval(() => { ticks++; }, 16);
    const scans = await Promise.all(blobs.map((blob, index) => ScannerPro.Lightweight.processPageAsync(blob, {
      onProgress(stage) { if (stage === 'detection') starts[index] = performance.now(); }
    }).then(scan => {
      finishes[index] = performance.now();
      return { name: ScannerDevelopableFixtures.CASES[index].name,
        corrected: scan.corrected, width: scan.width, height: scan.height,
        quality: scan.detection.quality, surface: scan.detection.frame?.metricSurface?.diagnostics,
        timings: scan.timings, size: scan.blob.size, type: scan.blob.type,
        previewType: scan.previewBlob?.type, inkLayer: scan.inkLayer, templateKind: scan.templateKind,
        vectors: { version: scan.editableInk?.version, backend: scan.editableInk?.backend,
          count: scan.editableInk?.strokes?.length,
          colours: [...new Set(scan.editableInk?.strokes?.map(stroke => stroke.color))] } };
    }, error => { throw new Error(`${ScannerDevelopableFixtures.CASES[index].name}: ${error.message}`); })));
    clearInterval(interval);
    return { scans, starts, finishes, ticks };
  });
  expect(workers.filter(url => url.includes('detection-worker.js'))).toHaveLength(3);
  expect(result.starts).toHaveLength(3);
  expect(Math.max(...result.starts)).toBeLessThan(Math.min(...result.finishes));
  expect(result.ticks).toBeGreaterThan(10);
  for (const scan of result.scans) {
    expect(scan.corrected, scan.name).toBe(true);
    expect(scan.quality.valid, scan.name).toBe(true);
    expect(scan.surface?.validation, scan.name).toBeGreaterThan(500);
    expect(scan.timings.worker).toBe(true);
    expect(scan.timings.warpMs).toBeGreaterThan(0);
    expect(scan.timings.colorMs).toBeGreaterThan(0);
    expect(scan.width).toBe(1800);
    expect(scan.height).toBe(2546);
    expect(scan.size).toBeGreaterThan(1000);
    expect(scan.type).toBe('image/png');
    expect(scan.previewType).toBe('image/png');
    expect(scan.inkLayer).toBe(true);
    expect(scan.templateKind).toBe('default');
    expect(scan.vectors.version).toBe(1);
    expect(scan.vectors.backend).toBe('rust');
    expect(scan.vectors.count).toBeGreaterThan(0);
    expect(scan.vectors.colours.every(colour => ['#4D4D4D','#002FD9','#E81010'].includes(colour))).toBe(true);
    expect(scan.timings.vectorMs).toBeGreaterThan(0);
  }
});

test('cancelling during worker cleanup terminates that candidate without publishing a scan', async ({ page }) => {
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-synthetic.js' });
  const closed = [];
  page.on('worker', worker => worker.on('close', () => closed.push(worker.url())));
  const result = await page.evaluate(async () => {
    const source = ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
    const blob = await new Promise(resolve => source.toBlob(resolve, 'image/jpeg', .94));
    const controller = new AbortController();
    const stages = [];
    try {
      await ScannerPro.Lightweight.processPageAsync(blob, {
        signal: controller.signal,
        onProgress(stage) {
          stages.push(stage);
          if (stage === 'colors') controller.abort();
        }
      });
      return { completed: true, stages };
    } catch (error) { return { name: error.name, stages }; }
  });
  expect(result.name).toBe('AbortError');
  expect(result.stages).toContain('colors');
  expect(result.stages).not.toContain('encoding');
  await expect.poll(() => closed.filter(url => url.includes('detection-worker.js')).length).toBe(1);
});

test('an unverified photograph is returned byte-for-byte without correction in workers and legacy fallback', async ({ page }) => {
  const reports = await page.evaluate(async () => {
    const canvas = document.createElement('canvas'); canvas.width = 640; canvas.height = 800;
    canvas.getContext('2d').fillStyle = '#7799aa'; canvas.getContext('2d').fillRect(0, 0, 640, 800);
    const blob = await new Promise(resolve => canvas.toBlob(resolve, 'image/jpeg', .94));
    const original = new Uint8Array(await blob.arrayBuffer());
    const results = [];
    const WorkerConstructor = window.Worker;
    for (const fallback of [false, true]) {
      if (fallback) window.Worker = undefined;
      const result = await ScannerPro.Lightweight.processPageAsync(blob);
      const bytes = new Uint8Array(await result.blob.arrayBuffer());
      results.push({ corrected: result.corrected, reason: result.failureReason, width: result.width, height: result.height,
        worker: result.timings.worker, warp: result.timings.warpMs, colors: result.timings.colorMs,
        equal: bytes.length === original.length && bytes.every((value, index) => value === original[index]) });
    }
    window.Worker = WorkerConstructor;
    return results;
  });
  for (const [index, result] of reports.entries()) {
    expect(result.corrected).toBe(false);
    expect(result.reason).toBeTruthy();
    expect(result.width).toBe(640);
    expect(result.height).toBe(800);
    expect(result.worker).toBe(index === 0);
    expect(result.warp).toBe(0);
    expect(result.colors).toBe(0);
    expect(result.equal).toBe(true);
  }
});

test('a worker startup failure still completes a verified scan on older WebViews', async ({ page }) => {
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-synthetic.js' });
  const result = await page.evaluate(async () => {
    const source = ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
    const blob = await new Promise(resolve => source.toBlob(resolve, 'image/jpeg', .94));
    window.Worker = class { constructor() { throw new Error('Workers are unavailable'); } };
    const scan = await ScannerPro.Lightweight.processPageAsync(blob, { maxDimension: 512 });
    const decoded = await createImageBitmap(scan.blob);
    const result = { corrected: scan.corrected, quality: scan.detection.quality,
      worker: scan.timings.worker, height: scan.height, decodedHeight: decoded.height };
    decoded.close();
    return result;
  });
  expect(result.corrected).toBe(true);
  expect(result.quality.valid).toBe(true);
  expect(result.worker).toBe(false);
  expect(result.height).toBe(512);
  expect(result.decodedHeight).toBe(512);
});

test('an image supported only by the document decoder retains the original import', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const blob = new Blob(['<svg xmlns="http://www.w3.org/2000/svg" width="640" height="800"><rect width="640" height="800" fill="#7799aa"/></svg>'], { type: 'image/svg+xml' });
    const scan = await ScannerPro.Lightweight.processPageAsync(blob);
    return { corrected: scan.corrected, width: scan.width, height: scan.height,
      type: scan.blob.type, original: await scan.blob.text() === await blob.text() };
  });
  expect(result).toEqual({ corrected: false, width: 640, height: 800, type: 'image/svg+xml', original: true });
});
