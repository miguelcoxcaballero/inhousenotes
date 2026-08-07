import { expect, test } from '@playwright/test';

test('scanner reconstructs 15 deterministic 3D, orientation and colour variants', async ({ page }) => {
  test.setTimeout(120_000);
  const caseFilter = process.env.SCANNER_CASE || null;
  await page.goto('/scanner/index.html?embed=1&session=e2e-synthetic-geometry', { waitUntil: 'domcontentloaded' });
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-synthetic.js' });

  const results = await page.evaluate(async filterName => {
    const findHorizontalYellow = (data, width, height, xRatio, yRatio) => {
      const x = Math.round(width * xRatio);
      const expected = Math.round(height * yRatio);
      let bestY = null;
      let bestScore = -Infinity;
      for (let y = Math.max(0, expected - 14); y <= Math.min(height - 1, expected + 14); y += 1) {
        let score = 0;
        for (let dx = -2; dx <= 2; dx += 1) {
          const offset = (y * width + Math.max(0, Math.min(width - 1, x + dx))) * 4;
          const r = data[offset], g = data[offset + 1], b = data[offset + 2];
          score += (r + g) * .5 - b - Math.abs(r - g) * .34;
        }
        if (score > bestScore) { bestScore = score; bestY = y; }
      }
      return { value: bestY, error: Math.abs(bestY - expected), score: bestScore / 5 };
    };
    const findVerticalYellow = (data, width, height, xRatio, yRatio) => {
      const y = Math.round(height * yRatio);
      const expected = Math.round(width * xRatio);
      let bestX = null;
      let bestScore = -Infinity;
      for (let x = Math.max(0, expected - 14); x <= Math.min(width - 1, expected + 14); x += 1) {
        let score = 0;
        for (let dy = -2; dy <= 2; dy += 1) {
          const sampleY = Math.max(0, Math.min(height - 1, y + dy));
          const offset = (sampleY * width + x) * 4;
          const r = data[offset], g = data[offset + 1], b = data[offset + 2];
          score += (r + g) * .5 - b - Math.abs(r - g) * .34;
        }
        if (score > bestScore) { bestScore = score; bestX = x; }
      }
      return { value: bestX, error: Math.abs(bestX - expected), score: bestScore / 5 };
    };
    const sampleCorrectedInk = (data, width, height, swatch) => {
      const centerX = Math.round((swatch.x0 + swatch.x1) * .5 / 21 * width);
      const centerY = Math.round((swatch.y - .08) / 29.7 * height);
      const radius = Math.max(4, Math.ceil(width / 21 * .2));
      let best = null;
      for (let y = centerY - radius; y <= centerY + radius; y += 1) {
        for (let x = centerX - radius; x <= centerX + radius; x += 1) {
          const offset = (Math.max(0, Math.min(height - 1, y)) * width
            + Math.max(0, Math.min(width - 1, x))) * 4;
          const rgb = [data[offset], data[offset + 1], data[offset + 2]];
          const [r, g, b] = rgb;
          const luminance = r * .2126 + g * .7152 + b * .0722;
          const score = swatch.name === 'red' ? r - (g + b) * .5
            : swatch.name === 'blue' ? b - (r + g) * .5
              : swatch.name === 'green' ? g - (r + b) * .5
                : 255 - luminance - (Math.max(r, g, b) - Math.min(r, g, b));
          if (!best || score > best.score) best = { rgb, score };
        }
      }
      const error = Math.sqrt(best.rgb.reduce((sum, value, channel) => (
        sum + (value - swatch.target[channel]) ** 2
      ), 0));
      return { ...best, error };
    };
    const medianPaper = (data, width, height) => {
      const values = [[], [], []];
      const centerX = Math.round(10 / 21 * width);
      const centerY = Math.round(26.15 / 29.7 * height);
      for (let y = centerY - 5; y <= centerY + 5; y += 1) {
        for (let x = centerX - 5; x <= centerX + 5; x += 1) {
          const offset = (y * width + x) * 4;
          for (let channel = 0; channel < 3; channel += 1) values[channel].push(data[offset + channel]);
        }
      }
      return values.map(channel => channel.sort((a, b) => a - b)[Math.floor(channel.length / 2)]);
    };
    const reports = [];
    for (const definition of ScannerSyntheticFixtures.CASES
      .filter(candidate => !filterName || candidate.name === filterName)) {
      const rendered = ScannerSyntheticFixtures.renderCase(definition);
      const detection = ScannerPro.Lightweight.detectPage(rendered.source);
      const geometry = ScannerSyntheticFixtures.evaluateDetection(definition, detection);
      const warped = await ScannerPro.Lightweight.warp(rendered.source, detection.pageQuad, detection.frame);
      const context = warped.getContext('2d', { willReadFrequently: true });
      const data = context.getImageData(0, 0, warped.width, warped.height).data;
      const horizontalRatios = [
        ScannerSyntheticFixtures.FRAME.v0,
        ScannerSyntheticFixtures.FRAME.boxTop,
        ScannerSyntheticFixtures.FRAME.boxBottom,
        ScannerSyntheticFixtures.FRAME.v1
      ];
      const horizontal = horizontalRatios.flatMap(yRatio => [0.18, 0.32, 0.68, 0.82]
        .map(xRatio => findHorizontalYellow(data, warped.width, warped.height, xRatio, yRatio)));
      const vertical = [ScannerSyntheticFixtures.FRAME.u0, ScannerSyntheticFixtures.FRAME.u1]
        .flatMap(xRatio => [0.14, 0.32, 0.58, 0.84]
          .map(yRatio => findVerticalYellow(data, warped.width, warped.height, xRatio, yRatio)));
      const colourResult = await ScannerPro.Lightweight.correctColors(warped, {
        useStencil: true,
        preciseStencil: true
      });
      const correctedData = context.getImageData(0, 0, warped.width, warped.height).data;
      const inkSamples = ScannerSyntheticFixtures.INK_SWATCHES.map(swatch => ({
        name: swatch.name,
        ...sampleCorrectedInk(correctedData, warped.width, warped.height, swatch)
      }));
      reports.push({
        name: definition.name,
        ...geometry,
        stencilQuad: detection.stencilQuad,
        markerCalibration: detection.markerCalibration,
        sideSupports: detection.frame?.sideSupports,
        detectedCurvature: detection.frame?.curvature,
        interiorTrace: detection.frame?.interiorTrace,
        colourCalibrated: colourResult.calibrated,
        colourSamples: colourResult.samples,
        inkSamples,
        inkMaximumError: Math.max(...inkSamples.map(sample => sample.error)),
        paperRgb: medianPaper(correctedData, warped.width, warped.height),
        warpMaximumRatio: Math.max(
          ...horizontal.map(value => value.error / warped.height),
          ...vertical.map(value => value.error / warped.width)
        )
      });
      warped.width = 0;
      warped.height = 0;
      rendered.source.width = 0;
      rendered.source.height = 0;
    }
    return reports;
  }, caseFilter);

  expect(results).toHaveLength(caseFilter ? 1 : 15);
  console.table(results.map(result => ({
    name: result.name,
    method: result.method,
    source: result.markerCalibration?.coordinateSource || '-',
    confidence: result.confidence.toFixed(3),
    corner: result.cornerMaximumRatio.toFixed(4),
    mapped: result.mappedCornerMaximumRatio.toFixed(4),
    curveMean: result.curveMeanRatio.toFixed(4),
    curveMax: result.curveMaximumRatio.toFixed(4),
    worst: Object.entries(result.curves || {}).sort((first, second) => second[1].maximum - first[1].maximum)[0]?.[0] || '-',
    frame: result.frameSupport.toFixed(3),
    box: result.boxSupport.toFixed(3),
    detectedCurve: Number(result.detectedCurvature || 0).toFixed(4),
    move: result.refinementMovement.toFixed(1),
    arms: result.refinementArmSupport.toFixed(0),
    warp: result.warpMaximumRatio.toFixed(4),
    ink: result.inkMaximumError.toFixed(1),
    paper: result.paperRgb.join('/')
  })));
  if (process.env.SCANNER_GEOMETRY_DIAGNOSTICS) {
    console.dir(results.map(result => ({
      name: result.name,
      stencilQuad: result.stencilQuad,
      coordinateSource: result.markerCalibration?.coordinateSource,
      coordinateDiagnostics: result.markerCalibration?.coordinateDiagnostics,
      curves: Object.fromEntries(Object.entries(result.curves || {}).map(([name, value]) => [name, {
        mean: value.mean,
        maximum: value.maximum
      }])),
      sideSupports: result.sideSupports,
      interiorTrace: result.interiorTrace,
      colourCalibrated: result.colourCalibrated,
      colourSamples: result.colourSamples,
      inkSamples: result.inkSamples,
      paperRgb: result.paperRgb
    })), { depth: 7 });
  }
  for (const result of results) {
    expect(result.method, result.name).toBe('marker-guided');
    expect(result.confidence, result.name).toBeGreaterThan(.9);
    expect(result.cornerMaximumRatio, result.name).toBeLessThan(.023);
    expect(result.mappedCornerMaximumRatio, result.name).toBeLessThan(.02);
    expect(result.curveMeanRatio, result.name).toBeLessThan(.007);
    expect(result.curveMaximumRatio, result.name).toBeLessThan(.015);
    expect(result.frameSupport, result.name).toBeGreaterThan(.9);
    expect(result.boxSupport, result.name).toBeGreaterThan(.9);
    // Pixel-centre rounding differs by one sample between Skia builds. Allow
    // exactly two pixels at the 900 px validation axis, while still rejecting
    // any visible residual bow beyond that rasterisation boundary.
    expect(result.warpMaximumRatio, result.name).toBeLessThanOrEqual(1 / 450);
    expect(result.colourCalibrated, result.name).toBe(true);
    expect(result.colourSamples, result.name).toBe(4);
    expect(result.inkMaximumError, result.name).toBeLessThan(92);
    expect(Math.min(...result.paperRgb), result.name).toBeGreaterThan(242);
  }
});
