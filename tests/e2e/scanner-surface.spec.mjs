import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=e2e-physical-surface', { waitUntil: 'domcontentloaded' });
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-physical.js' });
});

test('perspective fallback restores interior writing using a camera homography', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const fixture = ScannerPhysicalFixtures;
    const rendered = fixture.render(fixture.CASES[0]);
    const canvas = await ScannerPro.Lightweight.warp(rendered.source, rendered.pageQuad, null, { maxDimension: 1200 });
    return fixture.lineErrors(canvas);
  });
  expect(result.maximum).toBeLessThan(.003);
  expect(result.mean).toBeLessThan(.0015);
});

test('observed guides flatten interior writing on independent physical paper surfaces', async ({ page }) => {
  test.setTimeout(90_000);
  const results = await page.evaluate(async () => {
    const fixture = ScannerPhysicalFixtures;
    const reports = [];
    for (const definition of fixture.CASES) {
      const rendered = fixture.render(definition);
      const detection = ScannerPro.Lightweight.detectPage(rendered.source);
      const canvas = await ScannerPro.Lightweight.warp(rendered.source, detection.pageQuad, detection.frame, { maxDimension: 1200 });
      const errors = fixture.lineErrors(canvas);
      reports.push({ name: definition.name, method: detection.method, frame: !!detection.frame, ...errors });
    }
    return reports;
  });
  console.table(results);
  for (const result of results) {
    expect(result.frame, result.name).toBe(true);
    // This measures real interior pen strokes, not the guide curves used to
    // fit the surface. A 1% page-axis error is already a visible 2–3 mm bow.
    expect(result.maximum, result.name).toBeLessThan(.01);
    expect(result.mean, result.name).toBeLessThan(.004);
  }
});

test('dense surface interpolation preserves physical cylindrical and S-bend writing', async ({ page }) => {
  const results = await page.evaluate(async () => {
    const fixture = ScannerPhysicalFixtures;
    const reports = [];
    for (const definition of fixture.CASES.filter(candidate => candidate.radius || candidate.sBend)) {
      const rendered = fixture.render(definition);
      const canvas = await ScannerPro.Lightweight.warp(rendered.source, rendered.pageQuad, fixture.exactFrame(rendered.map), { maxDimension: 1200 });
      reports.push({ name: definition.name, ...fixture.lineErrors(canvas) });
    }
    return reports;
  });
  for (const result of results) {
    expect(result.maximum, result.name).toBeLessThan(.003);
    expect(result.mean, result.name).toBeLessThan(.0015);
  }
});

test('missing calibration teeth cannot shift the inferred physical cell numbers', async ({ page }) => {
  const results = await page.evaluate(() => {
    const fixture = ScannerPhysicalFixtures;
    const definition = fixture.CASES.find(candidate => candidate.name === 'cylindrical-paper');
    return [[60], [60, 150]].map(missingTeeth => {
      // The outer frame, colour sequence and both calibration boxes remain
      // visible. Only one or two internal dividers are genuinely absent.
      const rendered = fixture.render({ ...definition, missingTeeth });
      const detection = ScannerPro.Lightweight.detectPage(rendered.source);
      return {
        missingTeeth,
        frameFound: !!detection.frame,
        anchors: detection.frame?.parameterization?.anchors,
        model: detection.frame?.parameterization?.model,
        finite: detection.pageQuad.every(point => Number.isFinite(point.x + point.y))
      };
    });
  });
  for (const result of results) {
    expect(result.frameFound, String(result.missingTeeth)).toBe(true);
    expect(result.finite, String(result.missingTeeth)).toBe(true);
    expect(result.anchors, String(result.missingTeeth)).toBe(0);
    expect(result.model, String(result.missingTeeth)).not.toBe('measured-surface');
  }
});
