import { expect, test } from '@playwright/test';

const cases = [
  { name: 'strong-left-curl', label: 'one-pixel horizontal motion', dx: -1 },
  { name: 'strong-left-curl', label: 'camera JPEG compression', jpeg: .85 },
  { name: 'strong-bottom-curl', label: 'one-pixel vertical motion', dy: -1 },
  { name: 'faded-yellow', label: 'subpixel camera motion', dx: .5, dy: .5 },
  { name: 'faded-yellow', label: 'camera JPEG compression', jpeg: .85 }
];

for (const scenario of cases) {
  test(`${scenario.name} preserves observed geometry after ${scenario.label}`, async ({ page }) => {
    test.setTimeout(60_000);
    await page.goto('/scanner/index.html?embed=1&session=e2e-raster-geometry', { waitUntil: 'domcontentloaded' });
    await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-synthetic.js' });
    const result = await page.evaluate(async scenario => {
      const fixture = ScannerSyntheticFixtures;
      const definition = fixture.CASES.find(candidate => candidate.name === scenario.name);
      const rendered = fixture.renderCase(definition);
      const source = document.createElement('canvas');
      source.width = rendered.source.width;
      source.height = rendered.source.height;
      const context = source.getContext('2d');
      context.drawImage(rendered.source, scenario.dx || 0, scenario.dy || 0);
      if (scenario.jpeg) {
        const image = new Image();
        image.src = source.toDataURL('image/jpeg', scenario.jpeg);
        await image.decode();
        context.drawImage(image, 0, 0);
      }
      // Transform the known page geometry with the raster. This tests the real
      // detector against physical coordinates, not an expected internal branch.
      const truth = {
        ...definition,
        quad: definition.quad.map(([x, y]) => [x + (scenario.dx || 0), y + (scenario.dy || 0)])
      };
      const detection = ScannerPro.Lightweight.detectPage(source);
      const geometry = fixture.evaluateDetection(truth, detection);
      source.width = rendered.source.width = 0;
      return { ...geometry, quality: detection.quality };
    }, scenario);

    expect(result.method).toBe('marker-guided');
    expect(result.quality?.valid, result.quality?.reason).toBe(true);
    expect(result.confidence).toBeGreaterThan(.9);
    expect(result.cornerMaximumRatio).toBeLessThan(.023);
    expect(result.mappedCornerMaximumRatio).toBeLessThan(.02);
    expect(result.curveMeanRatio).toBeLessThan(.007);
    expect(result.curveMaximumRatio).toBeLessThan(.015);
    expect(result.frameSupport).toBeGreaterThan(.9);
    expect(result.boxSupport).toBeGreaterThan(.9);
  });
}
