import { expect, test } from '@playwright/test';

for (const definition of [
  { name: 'sharp independently rendered photograph', blur: 0, jpeg: 0 },
  { name: 'optically blurred JPEG photograph', blur: .6, jpeg: .82 },
  { name: 'small camera JPEG with subpixel bullet centres', blur: .5, jpeg: .9, sourceHeight: 740 }
]) {
  test(`photographed bullet pixels register to the template in a ${definition.name}`, async ({ page }) => {
    test.setTimeout(60_000);
    await page.goto('/scanner/index.html?embed=1&session=dot-registration', { waitUntil: 'domcontentloaded' });
    await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-developable.js' });
    await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-dot-metrics.js' });
    const result = await page.evaluate(async definition => {
      const fixture = ScannerDevelopableFixtures;
      const rendered = fixture.render(fixture.CASES[2]);
      const source = document.createElement('canvas');
      source.height = definition.sourceHeight || rendered.source.height;
      source.width = Math.round(rendered.source.width * source.height / rendered.source.height);
      const context = source.getContext('2d');
      if (definition.blur) context.filter = `blur(${definition.blur}px)`;
      context.drawImage(rendered.source, 0, 0, source.width, source.height); context.filter = 'none';
      if (definition.jpeg) {
        const blob = await new Promise(resolve => source.toBlob(resolve, 'image/jpeg', definition.jpeg));
        const bitmap = await createImageBitmap(blob); context.drawImage(bitmap, 0, 0); bitmap.close();
      }
      let accepted = [];
      const detection = await ScannerPro.Lightweight.detectPageAsync(source, {
        onProgress(stage, detail) { if (stage === 'surface' && detail?.points) accepted = detail.points; }
      });
      // Measure the original photographed glyphs. Neither cleaning them away
      // nor drawing ideal replacement dots can improve this measurement.
      const rawWarp = await ScannerPro.Lightweight.warp(source, detection.pageQuad, detection.frame, {
        maxDimension: 1800,
        onRegistration(detail) { accepted = detail.points; }
      });
      const dots = ScannerDotMetrics.measure(rawWarp, { excludeWriting: true });
      const acceptedDots = ScannerDotMetrics.measure(rawWarp, { excludeWriting: true, sites: accepted });
      const acceptedSites = new Set(accepted.map(point => `${point.row}:${point.col}`));
      const worst = [...dots.points].sort((a, b) => Math.hypot(b.dx, b.dy) - Math.hypot(a.dx, a.dy)).slice(0, 8)
        .map(({ row, col, dx, dy, strength }) => ({ row, col, dx, dy, strength, accepted: acceptedSites.has(`${row}:${col}`) }));
      delete dots.points;
      delete acceptedDots.points;
      return { quality: detection.quality, dots, writing: ScannerDotMetrics.measureWriting(rawWarp),
        registration: detection.frame?.metricSurface?.diagnostics?.registration,
        rasterRegistration: detection.frame?.metricSurface?.diagnostics?.rasterRegistration, acceptedDots, worst,
        width: rawWarp.width, height: rawWarp.height };
    }, definition);
    console.log(definition.name, JSON.stringify(result));
    expect(result.quality?.valid, JSON.stringify(result)).toBe(true);
    expect(result.dots.count, 'visible photographed dots remain present before template replacement').toBeGreaterThan(1200);
    expect(result.dots.error.p95, '95% of photographed centres land within half an output pixel of their template positions').toBeLessThan(.5);
    // Weak, ambiguous marks must not be forced into exact anchors. Require
    // nearly every clean glyph to be registered, and bound every such anchor;
    // the percentile and zigzag checks still include all photographed dots.
    expect(result.acceptedDots.count).toBeGreaterThan(1200);
    expect(result.acceptedDots.error.maximum, 'accepted anchors cannot leave isolated dots visibly displaced').toBeLessThan(1.2);
    expect(Math.abs(result.dots.meanX), 'no systematic horizontal pixel-centre offset').toBeLessThan(.12);
    expect(Math.abs(result.dots.meanY), 'no systematic vertical pixel-centre offset').toBeLessThan(.12);
    expect(result.dots.rowZigzag.p95, 'measured rows do not alternate above and below their straight targets').toBeLessThan(.3);
    expect(result.dots.columnZigzag.p95, 'measured columns do not alternate left and right').toBeLessThan(.3);
    expect(result.writing.maximum, 'pen strokes between anchors remain straight and in place').toBeLessThan(.0015);
    expect(result.writing.mean).toBeLessThan(.0007);
  });
}
