import { expect, test } from '@playwright/test';
import { writeFile } from 'node:fs/promises';

const cases = [
  { name: 'neutral-shadow', gains: [1, 1, 1] },
  { name: 'cool-shadow', gains: [.63, .81, 1] },
  { name: 'warm-shadow-jpeg', gains: [1, .84, .66], jpeg: true },
  { name: 'cool-shadow-with-missing-corner-dots', gains: [.63, .81, 1], missingCornerObservations: true }
];

for (const definition of cases) test(`white paper and stronger writing survive ${definition.name}`, async ({ page }, testInfo) => {
  test.setTimeout(60_000);
  await page.goto('/scanner/index.html?embed=1&session=paper-contrast', { waitUntil: 'domcontentloaded' });
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-paper-contrast.js' });
  const result = await page.evaluate(async definition => {
    const fixture = await ScannerPaperContrastFixtures.render(definition);
    const before = ScannerPaperContrastFixtures.measure(fixture.source, fixture);
    const original = fixture.source.toDataURL('image/png');
    const started = performance.now();
    await ScannerPro.Lightweight.correctColors(fixture.source, {
      useStencil: true, preciseStencil: true, dotObservations: fixture.dotObservations
    });
    const elapsedMs = performance.now() - started;
    const after = ScannerPaperContrastFixtures.measure(fixture.source, fixture);
    const cleaned = fixture.source.toDataURL('image/png');
    return { before, after, elapsedMs, observedDots: fixture.dotObservations.length,
      original, cleaned };
  }, definition);
  for (const key of ['original', 'cleaned']) {
    const path = testInfo.outputPath(`${definition.name}-${key}.png`);
    await writeFile(path, Buffer.from(result[key].split(',')[1], 'base64'));
    await testInfo.attach(`${definition.name}-${key}`, { path, contentType: 'image/png' });
    delete result[key];
  }
  console.log(definition.name, JSON.stringify(result));
  expect(result.observedDots).toBeGreaterThan(1300);
  expect(result.observedDots, 'opaque drawing hides real bullet observations').toBeLessThan(1785);
  for (const [zone, paper] of Object.entries(result.after.paper)) {
    expect(paper.count, zone).toBeGreaterThan(30);
    expect(paper.median, `${zone}: restored white paper`).toBeGreaterThan(248);
    expect(paper.p10, `${zone}: no broad gray remnants`).toBeGreaterThan(240);
    expect(paper.darkFraction, `${zone}: grain does not become dark speckles`).toBeLessThan(.025);
  }
  expect(result.after.writing.pencil.contrast, 'ordinary pencil is clearly legible').toBeGreaterThan(65);
  const shadedPencil = result.after.writing.pencil.byIllumination;
  expect(shadedPencil.hardShadow.count).toBeGreaterThan(5);
  expect(shadedPencil.hardShadow.contrast, 'white paper must not erase writing inside the shadow').toBeGreaterThan(60);
  expect(shadedPencil.shadowEdge.contrast, 'the pencil continues across the shadow edge').toBeGreaterThan(30);
  expect(result.after.writing.faintPencil.contrast, 'faint writing is strengthened instead of bleached').toBeGreaterThan(38);
  expect(result.after.writing.punctuation.contrast, 'isolated punctuation survives stencil cleanup').toBeGreaterThan(60);
  expect(result.after.writing.blackPen.median).toBeLessThan(65);
  expect(result.after.blueChroma, 'blue ink keeps its colour').toBeGreaterThan(70);
  expect(result.after.writing.grayDrawing.median, 'a gray filled drawing is ink, not a shadow').toBeLessThan(195);
  expect(result.after.writing.grayDrawing.median, 'a gray drawing is not crushed to black').toBeGreaterThan(60);
  expect(result.after.writing.charcoalDrawing.median).toBeLessThan(110);
});
