import { expect, test } from '@playwright/test';

// Each page is an independently rendered developable 3D surface. The bright
// neutral surroundings use a different illuminant: a global grey-world guess
// chooses the desk and cannot recover the yellow rails on these photographs.
const lighting = [
  { name: 'cool paper beside a bright neutral desk', gains: [.42, .65, 1], exposure: .8, neutralDesk: true },
  { name: 'warm paper beside a bright neutral desk', gains: [1, .62, .39], exposure: .75, neutralDesk: true },
  { name: 'magenta paper beside a bright neutral desk', gains: [1, .48, .87], exposure: .85, neutralDesk: true },
  { name: 'dim neutral paper', gains: [1, 1, 1], exposure: .28 }
];

for (const definition of lighting) test(`four measured references recover ${definition.name}`, async ({ page }, testInfo) => {
  test.setTimeout(60_000);
  await page.goto('/scanner/index.html?embed=1&session=marker-lighting', { waitUntil: 'domcontentloaded' });
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-developable.js' });
  const result = await page.evaluate(async definition => {
    const fixtures = ScannerDevelopableFixtures;
    const rendered = fixtures.render({ ...fixtures.CASES[2], dotColor: '#a8a8a8' });
    const { source } = rendered;
    const context = source.getContext('2d', { willReadFrequently: true });
    const photo = context.getImageData(0, 0, source.width, source.height);
    for (let y = 0; y < photo.height; y += 1) for (let x = 0; x < photo.width; x += 1) {
      const at = (y * photo.width + x) * 4;
      const background = photo.data[at] === 136 && photo.data[at + 1] === 98 && photo.data[at + 2] === 68;
      if (definition.neutralDesk && background) {
        photo.data[at] = photo.data[at + 1] = photo.data[at + 2] = 235;
        continue;
      }
      const shade = .85 + .15 * x / photo.width;
      for (let c = 0; c < 3; c += 1) photo.data[at + c] *= definition.gains[c] * definition.exposure * shade;
    }
    context.putImageData(photo, 0, 0);
    const references = ScannerPro.LiveDetection.findColourReferences(photo);
    const normalized = ScannerPro.LiveDetection.normalizeIllumination(photo);
    const started = performance.now();
    const detection = await ScannerPro.Lightweight.detectPageAsync(source);
    const elapsed = performance.now() - started;
    const untouched = context.getImageData(0, 0, source.width, source.height).data;
    const originalPreserved = photo.data.every((value, index) => value === untouched[index]);
    const warped = detection.frame?.metricSurface
      ? await ScannerPro.Lightweight.warp(source, detection.pageQuad, detection.frame, { maxDimension: 1200 }) : null;
    if (warped) await ScannerPro.Lightweight.correctColors(warped, { useStencil: true, preciseStencil: true });
    return {
      references, illumination: normalized.illumination, originalPreserved, elapsed,
      quality: detection.quality, method: detection.method, metric: detection.frame?.metricSurface?.type,
      cornerError: Math.max(...detection.pageQuad.map((point, i) =>
        Math.hypot(point.x - rendered.pageQuad[i].x, point.y - rendered.pageQuad[i].y))) / Math.hypot(source.width, source.height),
      lines: warped && fixtures.lineErrors(warped), source: source.toDataURL('image/png'),
      preview: warped?.toDataURL('image/png')
    };
  }, definition);
  for (const key of ['source', 'preview']) if (result[key]) {
    await testInfo.attach(key, { body: Buffer.from(result[key].split(',')[1], 'base64'), contentType: 'image/png' });
    delete result[key];
  }
  console.log('MARKER_LIGHTING', definition.name, JSON.stringify(result));
  expect(result.references?.points).toHaveLength(4);
  if (definition.neutralDesk) expect(result.illumination?.reference).toBe('colour-markers');
  expect(result.originalPreserved, 'analysis normalization never modifies the captured photo').toBe(true);
  expect(result.quality?.valid).toBe(true);
  expect(result.method).toBe('marker-guided');
  expect(result.metric, 'raw observed dots still independently certify the physical surface').toBe('developable-pinhole');
  expect(result.cornerError).toBeLessThan(.03);
  expect(result.lines.maximum).toBeLessThan(.006);
  expect(result.lines.mean).toBeLessThan(.002);
  expect(result.elapsed, 'bounded detector work stays within the existing still-analysis timeout').toBeLessThan(20_000);
});

test('four references cannot invent a completely clipped frame and dot lattice', async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=clipped-lighting', { waitUntil: 'domcontentloaded' });
  const result = await page.evaluate(async () => {
    const canvas = document.createElement('canvas'); canvas.width = 840; canvas.height = 1188;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    ScannerPro.ScanTemplate.render(context, canvas.width, canvas.height, { background: true });
    // Retain the independently visible calibration footer, but simulate a
    // large completely clipped highlight over every outer rail and grid dot.
    const strip = context.getImageData(75, 1090, 695, 32);
    context.fillStyle = '#fff'; context.fillRect(0, 0, canvas.width, canvas.height);
    context.putImageData(strip, 75, 1090);
    const references = ScannerPro.LiveDetection.findColourReferences(context.getImageData(0, 0, canvas.width, canvas.height));
    const detection = await ScannerPro.Lightweight.detectPageAsync(canvas);
    return { references, quality: detection.quality, metric: !!detection.frame?.metricSurface };
  });
  expect(result.references?.points).toHaveLength(4);
  expect(result.quality?.valid).not.toBe(true);
  expect(result.metric).toBe(false);
});
