import { expect, test } from '@playwright/test';

// The photographed sheet is rendered by an independently integrated surface.
// Missing bullet sites are never drawn, so a cosmetic regular-grid overlay
// cannot pass as a report of the points that the detector actually observed.
test('worker shows measured bullet rows before straightening and restores the printed template after preserving ink', async ({ page }) => {
  test.setTimeout(60_000);
  await page.goto('/scanner/index.html?embed=1&session=bullet-pipeline', { waitUntil: 'domcontentloaded' });
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-developable.js' });
  const report = await page.evaluate(async () => {
    const fixture = ScannerDevelopableFixtures;
    const rendered = fixture.render({ ...fixture.CASES[2], missingGrid: true });
    const input = await new Promise(resolve => rendered.source.toBlob(resolve, 'image/jpeg', .94));
    const events = [];
    const scan = await ScannerPro.Lightweight.processPageAsync(input, {
      maxDimension: 1200,
      previewProgress: true,
      onProgress(stage, detail) { events.push({ stage, detail }); }
    });
    const dots = events.find(event => event.stage === 'dots')?.detail;
    const registered = events.filter(event => event.stage === 'surface' && event.detail?.points?.length).at(-1)?.detail;
    const warp = events.find(event => event.stage === 'warp' && event.detail?.previewBlob)?.detail;
    const decode = async blob => {
      const bitmap = await createImageBitmap(blob);
      const canvas = document.createElement('canvas');
      canvas.width = bitmap.width; canvas.height = bitmap.height;
      canvas.getContext('2d').drawImage(bitmap, 0, 0);
      bitmap.close();
      return canvas;
    };
    const output = await decode(scan.previewBlob);
    const ink = await decode(scan.blob);
    const inkContext = ink.getContext('2d');
    const alpha = (x, y) => inkContext.getImageData(Math.round(x / 210 * ink.width),
      Math.round(y / 297 * ink.height), 1, 1).data[3];
    const result = { stages: events.map(event => event.stage), corrected: scan.corrected,
      templateApplied: scan.templateApplied, worker: scan.timings.worker,
      inkLayer: scan.inkLayer, templateKind: scan.templateKind, inkType: scan.blob.type,
      previewType: scan.previewBlob.type,
      outputQuality: scan.outputQuality,
      emptyInkAlpha: [[15, 145], [25, 276.8], [101.25, 276.8], [103.75, 276.8],
        [106.25, 276.8], [108.75, 276.8], [105, 290]].map(([x, y]) => alpha(x, y)),
      penInkAlpha: alpha(40, 35),
      finalWriting: fixture.lineErrors(output), dots: null, warp: null };
    if (dots) {
      const points = dots.points || [];
      const errors = points.map(point => {
        const truth = rendered.map((20 + point.col * 5) / 210, (19.3 + point.row * 5) / 297);
        return Math.hypot(point.x - truth.x, point.y - truth.y);
      }).sort((a, b) => a - b);
      // The fixture writes a black text baseline across row 23. Restrict the
      // empty-site check to the unprinted interior below that writing.
      const impossible = points.filter(point => point.col > 14 && point.col < 20
        && point.row > 23 && point.row < 30);
      const missingSites = points.filter(point => (point.col * 17 + point.row * 7) % 13 < 2);
      result.dots = { count: points.length, unique: new Set(points.map(point => `${point.row}:${point.col}`)).size,
        width: dots.width, height: dots.height, space: dots.space,
        impossible: impossible.length, missingSites: missingSites.length,
        medianError: errors[Math.floor(errors.length / 2)], p99Error: errors[Math.floor(errors.length * .99)],
        rows: dots.rows?.length, columns: dots.columns?.length,
        allCoordinatesFinite: points.every(point => ['x', 'y', 'u', 'v', 'row', 'col'].every(key => Number.isFinite(point[key]))) };
    }
    if (registered) {
      const measuredSites = new Set(dots.points.map(point => `${point.row}:${point.col}`));
      result.registration = { count: registered.points.length, space: registered.space,
        recovered: scan.detection?.frame?.metricSurface?.diagnostics?.registration?.recovered || 0,
        additionalPoints: registered.points.filter(point => !measuredSites.has(`${point.row}:${point.col}`)).length,
        impossible: registered.points.filter(point => point.col > 14 && point.col < 20 && point.row > 23 && point.row < 30).length,
        missingSites: registered.points.filter(point => (point.col * 17 + point.row * 7) % 13 < 2).length };
    }
    if (warp) {
      const beforeTemplate = await decode(warp.previewBlob);
      // The diagnostic JPEG is intentionally reduced to 720 pixels. Integrate
      // colour across a three-pixel band so chroma subsampling does not turn a
      // surviving fine stroke into a missing line at one sampled pixel.
      const context = beforeTemplate.getContext('2d');
      const width = beforeTemplate.width, height = beforeTemplate.height;
      const pixels = context.getImageData(0, 0, width, height).data;
      const errors = [];
      for (const horizontal of [true, false]) {
        const lines = horizontal ? [35, 75, 115, 155, 195, 235] : [35, 75, 115, 155, 180];
        const samples = horizontal ? [30, 45, 60, 90, 105, 120, 135, 150, 165] : [50, 90, 130, 170, 210, 250];
        const dimension = horizontal ? height : width, pageDimension = horizontal ? 297 : 210;
        for (const line of lines) for (const sample of samples) {
          const at = Math.round(sample / (horizontal ? 210 : 297) * (horizontal ? width : height));
          const expected = line / pageDimension * dimension;
          let weight = 0, position = 0;
          for (let along = Math.floor(expected - dimension * .025); along <= Math.ceil(expected + dimension * .025); along++) {
            for (let side = -1; side <= 1; side++) {
              const x = horizontal ? at + side : along, y = horizontal ? along : at + side;
              const offset = (y * width + x) * 4;
              const strength = horizontal ? pixels[offset + 2] - (pixels[offset] + pixels[offset + 1]) / 2
                : pixels[offset] - (pixels[offset + 1] + pixels[offset + 2]) / 2;
              const value = Math.max(0, strength - 10);
              weight += value; position += (along + .5) * value;
            }
          }
          errors.push(weight ? Math.abs(position / weight - expected) / dimension : 1);
        }
      }
      result.warp = { writing: { maximum: Math.max(...errors) }, space: warp.space,
        width: warp.width, height: warp.height, count: warp.points?.length,
        targetResidualMaximum: warp.points?.length ? Math.max(...warp.points.map(point =>
          Math.hypot(point.x - point.targetX, point.y - point.targetY))) : null,
        finitePositions: warp.points?.every(point => ['x', 'y', 'targetX', 'targetY'].every(key => Number.isFinite(point[key]))) };
    }
    const context = output.getContext('2d');
    const rgb = (x, y) => Array.from(context.getImageData(Math.round(x / 210 * output.width),
      Math.round(y / 297 * output.height), 1, 1).data.slice(0, 3));
    result.printedFrame = rgb(15, 145);
    result.printedBox = rgb(25, 276.8);
    result.printedMarkers = [101.25, 103.75, 106.25, 108.75].map(x => rgb(x, 276.8));
    return result;
  });
  console.log(JSON.stringify(report));
  expect(report.corrected, JSON.stringify(report)).toBe(true);
  expect(report.worker).toBe(true);
  expect(report.templateApplied).toBe(true);
  expect(report.inkLayer).toBe(true);
  expect(report.templateKind).toBe('default');
  expect(report.inkType).toBe('image/png');
  expect(report.previewType).toBe('image/png');
  expect(report.outputQuality?.paperSamples, 'output quality reaches the review from the processing worker').toBeGreaterThan(100);
  expect(report.outputQuality.confidence).toBeGreaterThan(.5);
  expect(report.outputQuality.paperResidualFraction).toBeGreaterThanOrEqual(0);
  expect(report.outputQuality.paperResidualFraction).toBeLessThan(.08);
  expect(report.emptyInkAlpha, 'the inserted ink asset contains no baked frame, footer markers or white paper').toEqual([0, 0, 0, 0, 0, 0, 0]);
  expect(report.penInkAlpha, 'the transparent asset retains the photographed handwriting').toBeGreaterThan(80);
  expect(report.dots, 'progress contains genuine measured points').not.toBeNull();
  expect(report.dots.space).toBe('source');
  expect([report.dots.width, report.dots.height]).toEqual([900, 1180]);
  expect(report.dots.count).toBeGreaterThan(500);
  expect(report.dots.unique).toBe(report.dots.count);
  expect(report.dots.allCoordinatesFinite).toBe(true);
  expect(report.dots.impossible, 'the deliberately empty central patch is not filled with invented points').toBe(0);
  expect(report.dots.missingSites / report.dots.count, 'most reported detections cannot occupy absent printed sites').toBeLessThan(.01);
  expect(report.dots.medianError).toBeLessThan(1.2);
  expect(report.dots.p99Error).toBeLessThan(2.5);
  expect(report.dots.rows).toBeGreaterThan(35);
  expect(report.dots.columns).toBeGreaterThan(25);
  expect(report.registration, 'the physical fit is followed by registration of accepted photographed dots').toBeTruthy();
  expect(report.registration.space).toBe('source');
  expect(report.registration.additionalPoints, 'new anchors must be reported as recovered observations').toBeLessThanOrEqual(report.registration.recovered);
  expect(report.registration.impossible, 'recovery cannot invent bullets in an empty region').toBe(0);
  expect(report.registration.missingSites / report.registration.count, 'recovered points must correspond to printed sites').toBeLessThan(.01);
  expect(report.registration.count).toBeGreaterThan(500);
  expect(report.registration.count).toBeLessThanOrEqual(35 * 51);
  expect(report.warp, 'the intermediate preview is the real warped photograph').not.toBeNull();
  expect(report.warp.space).toBe('corrected');
  expect(report.warp.count).toBe(report.registration.count);
  expect(report.warp.finitePositions).toBe(true);
  // Pixel measurements in scanner-dot-registration.spec.mjs independently
  // verify these targets in the raw image, before any replacement dots exist.
  expect(report.warp.targetResidualMaximum, 'accepted points and their actual warped positions agree with the template').toBeLessThan(.05);
  expect(report.warp.writing.maximum, 'ink is already straight before a replacement template can conceal distortion').toBeLessThan(.006);
  expect(report.finalWriting.maximum, 'the composed preview retains the original straightened pen strokes').toBeLessThan(.006);
  const first = stage => report.stages.indexOf(stage);
  expect(first('dots')).toBeLessThan(first('surface'));
  expect(first('surface')).toBeLessThan(first('colors'));
  expect(first('colors')).toBeLessThan(first('template'));
  expect(first('template')).toBeLessThan(first('encoding'));
  for (const pixel of [report.printedFrame, report.printedBox]) {
    expect(pixel[0]).toBeGreaterThan(185);
    expect(pixel[1]).toBeGreaterThan(150);
    expect(pixel[2]).toBeLessThan(140);
  }
  const [red, black, blue, green] = report.printedMarkers;
  expect(red[0] - Math.max(red[1], red[2])).toBeGreaterThan(100);
  expect(Math.max(...black)).toBeLessThan(80);
  expect(blue[2] - Math.max(blue[0], blue[1])).toBeGreaterThan(100);
  expect(green[1] - Math.max(green[0], green[2])).toBeGreaterThan(60);
});

test('a diagnostic thumbnail encoding failure does not discard a corrected worker scan', async ({ page }) => {
  await page.route('**/scanner/processing/detection-worker.js*', async route => {
    const response = await route.fetch();
    const fault = `
      const encodeDiagnosticTest = OffscreenCanvas.prototype.convertToBlob;
      OffscreenCanvas.prototype.convertToBlob = function (options) {
        if (Math.max(this.width, this.height) === 720) {
          self.postMessage({ progress: 'preview-encoder-rejected' });
          return Promise.reject(new DOMException('Diagnostic preview unavailable', 'EncodingError'));
        }
        return encodeDiagnosticTest.call(this, options);
      };
    `;
    await route.fulfill({ response, body: fault + await response.text() });
  });
  await page.goto('/scanner/index.html?embed=1&session=bullet-preview-error', { waitUntil: 'domcontentloaded' });
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-synthetic.js' });
  const report = await page.evaluate(async () => {
    const source = ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
    const input = await new Promise(resolve => source.toBlob(resolve, 'image/jpeg', .94));
    // The optional preview must be skipped inside the worker; it must not
    // restart a second correction on the application thread.
    ScannerPro.Lightweight.correctColors = () => { throw new Error('Unexpected main-thread correction'); };
    const stages = [], previews = [];
    const result = await ScannerPro.Lightweight.processPageAsync(input, { maxDimension: 1200,
      previewProgress: true, onProgress(stage, detail) {
        stages.push(stage);
        if (detail?.previewBlob) previews.push(stage);
      } });
    const decoded = await createImageBitmap(result.blob);
    const height = decoded.height; decoded.close();
    return { stages, previews, corrected: result.corrected, worker: result.timings.worker,
      templateApplied: result.templateApplied, height };
  });
  expect(report.stages).toContain('preview-encoder-rejected');
  expect(report.previews).toEqual([]);
  expect(report.stages).toContain('colors');
  expect(report.stages).toContain('template');
  expect(report.stages).toContain('encoding');
  expect(report.corrected).toBe(true);
  expect(report.templateApplied).toBe(true);
  expect(report.worker).toBe(true);
  expect(report.height).toBe(1200);
});
