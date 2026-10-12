import { expect, test } from '@playwright/test';

for (const useWorker of [true, false]) {
  test(`an invalid interior cannot hide beneath a native grid in ${useWorker ? 'a worker' : 'legacy processing'}`, async ({ page }) => {
    test.setTimeout(60_000);
    await page.goto('/scanner/index.html?embed=1&session=interior-quality', { waitUntil: 'domcontentloaded' });
    await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-developable.js' });
    const result = await page.evaluate(async useWorker => {
      if (!useWorker) window.Worker = undefined;
      const source = ScannerDevelopableFixtures.render({ radius: 0, axis: 0,
        pitch: .1, yaw: .1, stretchedDome: 35 }).source;
      const original = await new Promise(resolve => source.toBlob(resolve, 'image/png'));
      const stages = [];
      const scan = await ScannerPro.Lightweight.processPageAsync(original, {
        maxDimension: 1200, onProgress(stage) { stages.push(stage); }
      });
      const before = new Uint8Array(await original.arrayBuffer());
      const after = new Uint8Array(await scan.blob.arrayBuffer());
      return { corrected: scan.corrected, templateApplied: scan.templateApplied,
        inkLayer: scan.inkLayer, quality: scan.detection.quality, alignment: scan.dotAlignment,
        method: scan.detection.method, confidence: scan.detection.confidence,
        unchanged: before.length === after.length && before.every((value, index) => value === after[index]),
        dimensions: [scan.width, scan.height], stages, worker: scan.timings.worker };
    }, useWorker);
    expect(result.method).toBe('marker-guided');
    expect(result.confidence, 'yellow edges alone look convincing').toBeGreaterThan(.8);
    expect(result.alignment?.status).toBe('failed');
    expect(result.alignment.observations).toBeGreaterThan(500);
    expect(result.corrected).toBe(false);
    expect(result.templateApplied).toBe(false);
    expect(result.inkLayer).not.toBe(true);
    expect(result.quality.valid).toBe(false);
    expect(result.quality.reason).toBe('interior-dots-misaligned');
    expect(result.unchanged, 'a rejected correction preserves the original photograph exactly').toBe(true);
    expect(result.dimensions).toEqual([900, 1180]);
    expect(result.worker).toBe(useWorker);
    expect(result.stages).not.toContain('template');
  });
}

test('a genuinely flat bullet page still receives the native template', async ({ page }) => {
  test.setTimeout(60_000);
  await page.goto('/scanner/index.html?embed=1&session=flat-interior', { waitUntil: 'domcontentloaded' });
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-developable.js' });
  const result = await page.evaluate(async () => {
    const source = ScannerDevelopableFixtures.render({ radius: 0, axis: 0, pitch: .1, yaw: .1 }).source;
    const input = await new Promise(resolve => source.toBlob(resolve, 'image/png'));
    const scan = await ScannerPro.Lightweight.processPageAsync(input, { maxDimension: 1200 });
    return { corrected: scan.corrected, templateApplied: scan.templateApplied,
      inkLayer: scan.inkLayer, alignment: scan.dotAlignment, quality: scan.outputQuality };
  });
  expect(result.alignment?.status).toBe('verified');
  expect(result.alignment.observations).toBeGreaterThan(500);
  expect(result.corrected).toBe(true);
  expect(result.templateApplied).toBe(true);
  expect(result.inkLayer).toBe(true);
  expect(result.quality.paperResidualFraction).toBeLessThan(.08);
});
