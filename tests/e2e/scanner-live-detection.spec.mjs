import { expect, test } from '@playwright/test';

const loadDetector = async page => {
  await page.setContent('<!doctype html><html><body></body></html>');
  await page.addScriptTag({ path: 'scanner/processing/liveDetection.js' });
  await page.addScriptTag({ path: 'tests/e2e/fixtures/scanner-synthetic.js' });
};

test('live detector follows printed rails through perspective, rotation, curl and lighting', async ({ page }) => {
  await loadDetector(page);
  const reports = await page.evaluate(() => ScannerSyntheticFixtures.CASES.map(definition => {
    const { source } = ScannerSyntheticFixtures.renderCase(definition);
    const canvas = document.createElement('canvas');
    const scale = 480 / Math.max(source.width, source.height);
    canvas.width = Math.round(source.width * scale);
    canvas.height = Math.round(source.height * scale);
    const context = canvas.getContext('2d', { willReadFrequently: true });
    context.drawImage(source, 0, 0, canvas.width, canvas.height);
    const input = context.getImageData(0, 0, canvas.width, canvas.height);
    const start = performance.now();
    const detection = ScannerPro.LiveDetection.detect(input);
    const duration = performance.now() - start;
    const mapping = ScannerSyntheticFixtures.mappingFor(definition);
    const frame = ScannerSyntheticFixtures.FRAME;
    const expected = [[frame.u0, frame.v0], [frame.u1, frame.v0], [frame.u1, frame.v1], [frame.u0, frame.v1]]
      .map(([u, v]) => mapping(u, v))
      .map(point => ({ x: point.x / source.width, y: point.y / source.height }));
    const cornerError = detection && Math.max(...detection.quad.map(point =>
      Math.min(...expected.map(reference => Math.hypot(point.x - reference.x, point.y - reference.y)))));
    return { name: definition.name, duration, cornerError, detection };
  }));
  for (const report of reports) {
    // This fixture deliberately removes measurable yellow hue. Keeping the
    // manual capture path is safer than claiming a coloured frame was seen.
    if (report.name === 'faded-yellow') {
      expect(report.detection?.confidence || 0).toBeLessThan(.78);
      continue;
    }
    expect(report.detection, report.name).toBeTruthy();
    expect(report.detection.source, report.name).toBe('yellow');
    expect(report.detection.confidence, report.name).toBeGreaterThan(.85);
    expect(report.cornerError, report.name).toBeLessThan(.055);
    expect(report.detection.sharpness, report.name).toBeGreaterThan(45);
    expect(report.detection.markers.length, report.name).toBe(report.name === 'strong-bottom-curl' ? 0 : 2);
  }
});

test('blur does not produce a sharp automatic capture candidate', async ({ page }) => {
  await loadDetector(page);
  const detection = await page.evaluate(() => {
    const { source } = ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]);
    const canvas = document.createElement('canvas');
    canvas.width = 366;
    canvas.height = 480;
    const context = canvas.getContext('2d');
    context.filter = 'blur(7px)';
    context.drawImage(source, 0, 0, canvas.width, canvas.height);
    return ScannerPro.LiveDetection.detect(context.getImageData(0, 0, canvas.width, canvas.height));
  });
  expect(!detection || detection.source !== 'yellow' || detection.sharpness < 45 || detection.confidence < .78).toBe(true);
});
