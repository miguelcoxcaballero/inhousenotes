import { expect, test } from '@playwright/test';

// These casts model a camera white-balance error, independently of detector
// thresholds. The cool case reproduces the paper RGB ratios in the report;
// an exposure gradient additionally darkens one side of the curved sheet.
const cases = [
  { name: 'cool-daylight-with-shadow', gains: [.59, .77, 1], reverse: false },
  { name: 'warm-lamp-with-shadow', gains: [1, .82, .58], reverse: true },
  { name: 'cool-daylight-camera-jpeg', gains: [.59, .77, 1], reverse: false, jpeg: true }
];

for (const definition of cases) {
  test(`complete scan recovers geometry and writing under ${definition.name}`, async ({ page }, testInfo) => {
    test.setTimeout(60_000);
    await page.goto('/scanner/index.html?embed=1&session=lighting-pipeline', { waitUntil: 'domcontentloaded' });
    await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-developable.js' });
    const result = await page.evaluate(async definition => {
      const fixture = ScannerDevelopableFixtures;
      const rendered = fixture.render(fixture.CASES[2]);
      const { source } = rendered;
      const context = source.getContext('2d', { willReadFrequently: true });
      // A faint neutral pencil line crosses multiple grid columns. It is
      // unrelated to the printed dots and must survive template cleanup.
      context.strokeStyle = '#aaa';
      context.lineWidth = 1.25;
      context.beginPath();
      for (let x = 27; x <= 182; x += .5) {
        const point = rendered.map(x / 210, 93 / 297);
        if (x === 27) context.moveTo(point.x, point.y);
        else context.lineTo(point.x, point.y);
      }
      context.stroke();
      const input = context.getImageData(0, 0, source.width, source.height);
      for (let y = 0; y < source.height; y += 1) {
        for (let x = 0; x < source.width; x += 1) {
          const fraction = definition.reverse ? 1 - x / source.width : x / source.width;
          const exposure = .73 + .24 * fraction;
          const offset = (y * source.width + x) * 4;
          for (let channel = 0; channel < 3; channel += 1) {
            input.data[offset + channel] *= exposure * definition.gains[channel];
          }
        }
      }
      context.putImageData(input, 0, 0);
      if (definition.jpeg) {
        // Match the camera capture encoder, including its chroma subsampling.
        const blob = await new Promise(resolve => source.toBlob(resolve, 'image/jpeg', .94));
        const url = URL.createObjectURL(blob);
        try {
          const photo = new Image();
          photo.src = url;
          await photo.decode();
          context.drawImage(photo, 0, 0);
        } finally {
          URL.revokeObjectURL(url);
        }
      }
      // Neither exact corners nor a fixture surface is supplied to production.
      const detection = await ScannerPro.Lightweight.detectPageAsync(source);
      if (detection.quality?.valid === false) return { quality: detection.quality, confidence: detection.confidence };
      const metricDiagnostics = {};
      if (!detection.frame?.metricSurface) ScannerPro.MetricSurface.fit(source, detection, { diagnostics: metricDiagnostics });
      const warped = await ScannerPro.Lightweight.warp(source, detection.pageQuad, detection.frame, { maxDimension: 1200 });
      function pencilContrast(canvas) {
        const { width, height } = canvas;
        const pixels = canvas.getContext('2d').getImageData(0, 0, width, height).data;
        const luminance = (x, y) => {
          const i = (Math.round(y) * width + Math.round(x)) * 4;
          return .2126 * pixels[i] + .7152 * pixels[i + 1] + .0722 * pixels[i + 2];
        };
        const contrasts = [];
        for (const xMm of [43, 53, 63, 87, 97, 107, 127, 137, 147, 167]) {
          const x = xMm / 210 * width;
          const y = 93 / 297 * height;
          const paper = (luminance(x, y - 12) + luminance(x, y + 12)) / 2;
          let darkest = paper;
          for (let dy = -7; dy <= 7; dy += 1) darkest = Math.min(darkest, luminance(x, y + dy));
          contrasts.push(paper - darkest);
        }
        return { minimum: Math.min(...contrasts), mean: contrasts.reduce((a, b) => a + b, 0) / contrasts.length };
      }
      const pencilBefore = pencilContrast(warped);
      await ScannerPro.Lightweight.correctColors(warped, { useStencil: true, preciseStencil: true });
      const diagonal = Math.hypot(source.width, source.height);
      const cornerMaximumRatio = Math.max(...detection.pageQuad.map((point, index) =>
        Math.hypot(point.x - rendered.pageQuad[index].x, point.y - rendered.pageQuad[index].y) / diagonal));
      const pixels = warped.getContext('2d').getImageData(0, 0, warped.width, warped.height).data;
      const whites = [];
      for (const [u, v] of [[.25, .25], [.45, .31], [.65, .53], [.8, .71], [.02, .5], [.98, .5], [.5, .02], [.5, .98]]) {
        const offset = (Math.round(v * warped.height) * warped.width + Math.round(u * warped.width)) * 4;
        whites.push(Array.from(pixels.slice(offset, offset + 3)));
      }
      return {
        quality: detection.quality, confidence: detection.confidence, metricDiagnostics,
        metric: detection.frame?.metricSurface?.type,
        cornerMaximumRatio, lines: fixture.lineErrors(warped),
        pencilBefore, pencilAfter: pencilContrast(warped), whites,
        source: source.toDataURL('image/png'), result: warped.toDataURL('image/png')
      };
    }, definition);
    for (const key of ['source', 'result']) if (result[key]) {
      await testInfo.attach(`${definition.name}-${key}`, { body: Buffer.from(result[key].split(',')[1], 'base64'), contentType: 'image/png' });
      delete result[key];
    }
    console.log(definition.name, JSON.stringify(result));
    expect(result.quality?.valid, JSON.stringify(result)).toBe(true);
    expect(result.metric, 'interior lattice supports a physical 3D reconstruction').toBe('developable-pinhole');
    expect(result.cornerMaximumRatio).toBeLessThan(.03);
    expect(result.lines.maximum, 'both coloured pen directions remain straight and in place').toBeLessThan(.006);
    expect(result.lines.mean).toBeLessThan(.002);
    expect(result.pencilAfter.minimum, 'faint pencil is preserved across the sheet').toBeGreaterThan(18);
    expect(result.pencilAfter.mean).toBeGreaterThan(result.pencilBefore.mean * .6);
    for (const rgb of result.whites) {
      expect(Math.min(...rgb), 'paper exposure is restored').toBeGreaterThan(220);
      expect(Math.max(...rgb) - Math.min(...rgb), 'paper loses the illumination cast').toBeLessThan(18);
    }
  });
}

test('lighting correction cannot invent 3D grid geometry on a coloured card', async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=lighting-negative', { waitUntil: 'domcontentloaded' });
  const result = await page.evaluate(async () => {
    const canvas = document.createElement('canvas');
    canvas.width = 720; canvas.height = 960;
    const context = canvas.getContext('2d');
    context.fillStyle = '#644c35'; context.fillRect(0, 0, 720, 960);
    context.fillStyle = '#88b4ec'; context.fillRect(80, 80, 550, 810);
    context.strokeStyle = '#759794'; context.lineWidth = 3; context.strokeRect(120, 115, 465, 725);
    context.fillStyle = '#3c597e'; context.font = '32px sans-serif';
    context.fillText('A plain blue card', 150, 240);
    const preview = document.createElement('canvas');
    preview.width = 360; preview.height = 480;
    const previewContext = preview.getContext('2d');
    previewContext.drawImage(canvas, 0, 0, 360, 480);
    const live = ScannerPro.LiveDetection.detect(previewContext.getImageData(0, 0, 360, 480));
    const detection = await ScannerPro.Lightweight.detectPageAsync(canvas);
    return { metric: !!detection.frame?.metricSurface, confidence: live?.confidence || 0, markers: live?.markers?.length || 0 };
  });
  // A coloured card with a border can resemble a white page under tinted
  // illumination. Without an observed lattice, it cannot imply a 3D surface.
  expect(result.metric).toBe(false);
  expect(result.markers).toBe(0);
  expect(result.confidence, 'unverified cast correction cannot trigger automatic capture').toBeLessThan(.78);
});
