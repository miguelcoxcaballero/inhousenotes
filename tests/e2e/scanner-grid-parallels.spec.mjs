import { expect, test } from "@playwright/test";

// An independently integrated paper surface supplies ground truth. Gaps are
// absent printed dots, not a degraded copy of the production detector's output.
test("ordered grid parallels recover 3D bends despite missing dots and coloured light", async ({
  page,
}) => {
  test.setTimeout(60_000);
  await page.goto("/scanner/index.html?embed=1&session=grid-parallels", {
    waitUntil: "domcontentloaded",
  });
  await page.addScriptTag({
    url: "/tests/e2e/fixtures/scanner-developable.js",
  });
  const reports = await page.evaluate(async () => {
    const fixture = ScannerDevelopableFixtures,
      reports = [];
    for (const definition of [
      {
        ...fixture.CASES[2],
        name: "cool-diagonal-curl",
        gains: [0.59, 0.77, 1],
      },
      {
        ...fixture.CASES[3],
        name: "warm-transverse-ripple",
        bendFrequency: 1.6,
        gains: [1, 0.78, 0.56],
      },
    ]) {
      const rendered = fixture.render({ ...definition, missingGrid: true }),
        source = rendered.source;
      const ctx = source.getContext("2d", { willReadFrequently: true }),
        image = ctx.getImageData(0, 0, source.width, source.height);
      for (let y = 0; y < source.height; y++)
        for (let x = 0; x < source.width; x++) {
          const exposure = 0.73 + (0.24 * x) / source.width,
            i = (y * source.width + x) * 4;
          for (let c = 0; c < 3; c++)
            image.data[i + c] *= exposure * definition.gains[c];
        }
      ctx.putImageData(image, 0, 0);
      const frame = fixture.exactFrame(rendered.map),
        diagnostics = {},
        surface = ScannerPro.MetricSurface.fit(
          source,
          { frame },
          { diagnostics },
        );
      if (!surface) {
        reports.push({ name: definition.name, accepted: false, diagnostics });
        continue;
      }
      const errors = [];
      for (let y = 27; y <= 257; y += 7)
        for (let x = 27; x <= 183; x += 7) {
          const actual = ScannerPro.MetricSurface.map(
              surface,
              x / 210,
              y / 297,
            ),
            truth = rendered.map(x / 210, y / 297);
          errors.push(Math.hypot(actual.x - truth.x, actual.y - truth.y));
        }
      const warped = await ScannerPro.Lightweight.warp(
        source,
        rendered.pageQuad,
        { ...frame, metricSurface: surface },
        { maxDimension: 1200 },
      );
      await ScannerPro.Lightweight.correctColors(warped, { useStencil: false });
      reports.push({
        name: definition.name,
        accepted: true,
        diagnostics: surface.diagnostics,
        maximum: Math.max(...errors),
        mean: errors.reduce((a, b) => a + b, 0) / errors.length,
        writing: fixture.lineErrors(warped),
      });
    }
    return reports;
  });
  console.log(JSON.stringify(reports));
  for (const report of reports) {
    expect(report.accepted, JSON.stringify(report)).toBe(true);
    expect(report.diagnostics.parallels.rows).toBeGreaterThan(7);
    expect(report.diagnostics.parallels.columns).toBeGreaterThan(5);
    expect(report.diagnostics.parallelValidation.samples).toBeGreaterThan(30);
    expect(report.diagnostics.validation).toBeGreaterThan(400);
    expect(report.diagnostics.observations).toBe(
      report.diagnostics.training + report.diagnostics.validation,
    );
    expect(report.maximum, report.name).toBeLessThan(1.4);
    expect(report.mean, report.name).toBeLessThan(0.8);
    expect(report.writing.maximum, report.name).toBeLessThan(0.003);
  }
});
