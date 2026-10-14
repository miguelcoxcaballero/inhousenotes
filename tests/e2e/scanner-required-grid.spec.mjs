import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=required-grid', { waitUntil: 'domcontentloaded' });
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-developable.js' });
});

async function scan(page, { useWorker = true, pattern = 'complete', requireGrid = true } = {}) {
  return page.evaluate(async ({ useWorker, pattern, requireGrid }) => {
    if (!useWorker) window.Worker = undefined;
    const fixture = ScannerDevelopableFixtures.render({ radius: 0, axis: 0, pitch: .1, yaw: .1,
      ...(pattern === 'absent' ? { dotColor: '#ffffff' } : {}) });
    if (pattern === 'partial') {
      // Keep the yellow border, footer and lower dot rows sharply visible,
      // but simulate glare washing out most of the printed interior. A few
      // visible rows cannot establish the page's complete physical lattice.
      const context = fixture.source.getContext('2d');
      const corners = [[.082, .055], [.925, .055], [.925, .75], [.082, .75]];
      context.fillStyle = '#ffffff';
      context.beginPath();
      corners.forEach(([u, v], index) => {
        const point = fixture.map(u, v);
        if (index) context.lineTo(point.x, point.y); else context.moveTo(point.x, point.y);
      });
      context.closePath(); context.fill();
    }
    const blob = await new Promise(resolve => fixture.source.toBlob(resolve, 'image/png'));
    const stages = [];
    const result = await ScannerPro.Lightweight.processPageAsync(blob, {
      requireGrid, maxDimension: 1200, onProgress(stage) { stages.push(stage); }
    });
    const before = new Uint8Array(await blob.arrayBuffer());
    const after = new Uint8Array(await result.blob.arrayBuffer());
    return { corrected: result.corrected, templateApplied: result.templateApplied,
      inkLayer: result.inkLayer, vectors: result.editableInk?.strokes.length,
      reason: result.failureReason, quality: result.detection.quality,
      method: result.detection.method, confidence: result.detection.confidence,
      surface: result.detection.frame?.metricSurface?.diagnostics,
      alignment: result.dotAlignment, stages, timings: result.timings,
      unchanged: before.length === after.length && before.every((value, index) => value === after[index]) };
  }, { useWorker, pattern, requireGrid });
}

for (const useWorker of [true, false]) {
  const mode = useWorker ? 'worker' : 'legacy fallback';

  test(`a sharp physical bullet grid is certified before camera output in ${mode}`, async ({ page }) => {
    test.setTimeout(60_000);
    const result = await scan(page, { useWorker });
    expect(result.corrected, JSON.stringify(result)).toBe(true);
    expect(result.surface.observations).toBeGreaterThan(500);
    expect(result.surface.validation).toBeGreaterThan(200);
    expect(result.alignment.status).toBe('verified');
    expect(result.alignment.observations).toBeGreaterThan(500);
    expect(result.templateApplied).toBe(true);
    expect(result.inkLayer).toBe(true);
    expect(result.vectors).toBeGreaterThan(0);
    expect(result.timings.worker).toBe(useWorker);
  });

  for (const pattern of ['absent', 'partial']) {
    test(`camera rejects a sharp border with ${pattern} dots before warp in ${mode}`, async ({ page }) => {
      test.setTimeout(60_000);
      const result = await scan(page, { useWorker, pattern });
      expect(['marker-guided', 'stencil']).toContain(result.method);
      expect(result.confidence, 'the sharp yellow border is convincing on its own').toBeGreaterThan(.8);
      expect(result.corrected).toBe(false);
      expect(result.quality.reason).toBe('page-dot-grid-unverified');
      expect(result.reason).toContain('dots');
      expect(result.templateApplied).toBe(false);
      expect(result.inkLayer).not.toBe(true);
      expect(result.unchanged).toBe(true);
      expect(result.stages).not.toContain('warp');
      expect(result.stages).not.toContain('colors');
      expect(result.timings.worker).toBe(useWorker);
    });
  }

  for (const certification of ['not-applicable', 'missing']) {
    test(`camera cannot accept ${certification} final certification in ${mode}`, async ({ page }) => {
      test.setTimeout(60_000);
      // Fault-inject at the independent raster verifier, after real source
      // detection and physical fitting. Exercise the actual imported worker
      // module as well as the legacy main-thread pipeline.
      const suffix = certification === 'missing'
        ? '\ndelete ScannerPro.MetricSurface.certifyRaster;'
        : '\nScannerPro.MetricSurface.certifyRaster = () => ({status:"not-applicable",reason:"insufficient-visible-dot-pattern"});';
      if (useWorker) await page.route('**/scanner/processing/metric-surface.js*', async route => {
        const response = await route.fetch();
        await route.fulfill({ response, body: await response.text() + suffix });
      });
      else await page.evaluate(suffix => (0, eval)(suffix), suffix);
      const result = await scan(page, { useWorker });
      expect(result.surface.observations).toBeGreaterThan(500);
      expect(result.corrected).toBe(false);
      expect(result.quality.reason).toBe('interior-dot-grid-unverified');
      expect(result.reason).toContain('dots');
      expect(result.templateApplied).toBe(false);
      expect(result.unchanged).toBe(true);
      expect(result.stages).toContain('warp');
      expect(result.stages).not.toContain('colors');
      expect(result.stages).not.toContain('template');
      expect(result.timings.worker).toBe(useWorker);
    });
  }
}

test('ordinary bordered-photo imports remain supported without the camera grid requirement', async ({ page }) => {
  test.setTimeout(60_000);
  const result = await scan(page, { pattern: 'absent', requireGrid: false });
  expect(result.corrected, JSON.stringify(result)).toBe(true);
  expect(result.surface).toBeUndefined();
  expect(result.alignment.status).toBe('not-applicable');
  expect(result.templateApplied).toBe(true);
});
