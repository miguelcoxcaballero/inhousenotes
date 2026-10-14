import { expect, test } from '@playwright/test';

test('scan processing profile keeps geometry and reports measured stage timings', async ({ page }) => {
  test.setTimeout(60000);
  await page.goto('/scanner/index.html', { waitUntil: 'domcontentloaded' });
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-developable.js' });
  const profile = await page.evaluate(async () => {
    const source = ScannerDevelopableFixtures.render(ScannerDevelopableFixtures.CASES[0]).source;
    const blob = await new Promise(resolve => source.toBlob(resolve, 'image/jpeg', .94));
    source.width = source.height = 0;
    const scan = await ScannerPro.Lightweight.processPageAsync(blob, { previewProgress: true });
    return { corrected: scan.corrected, timings: scan.timings, surface: scan.detection.frame?.metricSurface?.diagnostics, width: scan.width, height: scan.height };
  });
  console.log('SCAN_PROFILE', JSON.stringify(profile));
  expect(profile.corrected).toBe(true);
  expect(profile.surface.validation).toBeGreaterThan(500);
});
