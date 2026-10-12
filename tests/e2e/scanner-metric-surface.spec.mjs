import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/scanner/index.html?embed=1&session=e2e-metric-surface", {
    waitUntil: "domcontentloaded",
  });
  await page.addScriptTag({ url: "/scanner/processing/metric-surface.js" });
  await page.addScriptTag({
    url: "/tests/e2e/fixtures/scanner-developable.js",
  });
});

test("physical 3D fit restores measured interiors for longitudinal and diagonal paper bends", async ({
  page,
}) => {
  test.setTimeout(90_000);
  const results = await page.evaluate(async () => {
    const fixture = ScannerDevelopableFixtures,
      reports = [];
    for (const definition of fixture.CASES) {
      const rendered = fixture.render(definition),
        frame = fixture.exactFrame(rendered.map);
      const diagnostics = {};
      const observations = ScannerPro.MetricSurface.observe(
        rendered.source,
        frame,
        diagnostics,
      );
      const surface = ScannerPro.MetricSurface.fit(
        rendered.source,
        { frame },
        { diagnostics },
      );
      const errors = [];
      let writing = null;
      if (surface)
        for (let y = 25; y <= 255; y += 10)
          for (let x = 25; x <= 185; x += 10) {
            const u = x / 210,
              v = y / 297,
              actual = ScannerPro.MetricSurface.map(surface, u, v),
              truth = rendered.map(u, v);
            errors.push(Math.hypot(actual.x - truth.x, actual.y - truth.y));
          }
      if (surface) {
        const warped = await ScannerPro.Lightweight.warp(
          rendered.source,
          rendered.pageQuad,
          { ...frame, metricSurface: surface },
          { maxDimension: 1200 },
        );
        writing = fixture.lineErrors(warped);
      }
      reports.push({
        writing,
        name: definition.name,
        diagnostics,
        observations: observations?.points.length,
        accepted: !!surface,
        maximum: Math.max(...errors),
        mean: errors.reduce((a, b) => a + b, 0) / errors.length,
        model: surface?.diagnostics,
      });
    }
    return reports;
  });
  console.table(
    results.map((result) => ({
      name: result.name,
      accepted: result.accepted ?? result.metric,
      maximum: result.maximum ?? result.errors?.maximum,
      mean: result.mean ?? result.errors?.mean,
      milliseconds: result.model?.elapsedMs,
    })),
  );
  for (const result of results) {
    expect(
      result.accepted,
      `${result.name}: ${JSON.stringify(result.diagnostics)}`,
    ).toBe(true);
    expect(result.maximum, result.name).toBeLessThan(2.5);
    expect(result.mean, result.name).toBeLessThan(0.9);
    expect(result.writing.maximum, result.name).toBeLessThan(0.003);
    expect(result.writing.mean, result.name).toBeLessThan(0.0015);
    expect(result.model.validation, result.name).toBeGreaterThan(500);
    expect(result.model.elapsedMs, result.name).toBeLessThan(1800);
  }
});

test("a border without enough interior observations cannot invent a metric 3D surface", async ({
  page,
}) => {
  const accepted = await page.evaluate(() => {
    const fixture = ScannerDevelopableFixtures,
      definition = fixture.CASES[1];
    const rendered = fixture.render(definition),
      blank = document.createElement("canvas");
    blank.width = rendered.source.width;
    blank.height = rendered.source.height;
    blank.getContext("2d").fillStyle = "#fff";
    blank.getContext("2d").fillRect(0, 0, blank.width, blank.height);
    return !!ScannerPro.MetricSurface.fit(blank, {
      frame: fixture.exactFrame(rendered.map),
    });
  });
  expect(accepted).toBe(false);
});

test("incompatible two-axis stretching is rejected instead of inventing a developable paper fit", async ({
  page,
}) => {
  const result = await page.evaluate(() => {
    const fixture = ScannerDevelopableFixtures;
    const rendered = fixture.render({
      radius: 0,
      axis: 0,
      pitch: 0.1,
      yaw: 0.1,
      stretchedDome: 65,
    });
    const diagnostics = {};
    const surface = ScannerPro.MetricSurface.fit(
      rendered.source,
      { frame: fixture.exactFrame(rendered.map) },
      { diagnostics },
    );
    return { accepted: !!surface, diagnostics };
  });
  expect(result.accepted, JSON.stringify(result.diagnostics)).toBe(false);
});

test("detected curved borders and the physical surface preserve writing through the complete warp", async ({
  page,
}) => {
  test.setTimeout(90_000);
  const results = await page.evaluate(async () => {
    const fixture = ScannerDevelopableFixtures,
      results = [];
    for (const definition of fixture.CASES) {
      const rendered = fixture.render(definition);
      const detection = await ScannerPro.Lightweight.detectPageAsync(
        rendered.source,
      );
      const warped = await ScannerPro.Lightweight.warp(
        rendered.source,
        detection.pageQuad,
        detection.frame,
        { maxDimension: 1200 },
      );
      results.push({
        name: definition.name,
        metric: !!detection.frame?.metricSurface,
        quality: detection.quality,
        errors: fixture.lineErrors(warped),
      });
    }
    return results;
  });
  console.table(
    results.map((result) => ({
      name: result.name,
      metric: result.metric,
      maximum: result.errors.maximum,
      mean: result.errors.mean,
    })),
  );
  for (const result of results) {
    expect(result.metric, result.name).toBe(true);
    expect(result.errors.maximum, result.name).toBeLessThan(0.006);
    expect(result.errors.mean, result.name).toBeLessThan(0.002);
  }
});
