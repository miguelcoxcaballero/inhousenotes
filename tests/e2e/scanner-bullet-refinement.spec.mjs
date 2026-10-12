import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=bullet-refinement', { waitUntil: 'domcontentloaded' });
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-developable.js' });
});

test('measured bullet rows correct remaining local bow and report only observed dots before fitting', async ({ page }) => {
  test.setTimeout(60_000);
  const report = await page.evaluate(async () => {
    const fixture = ScannerDevelopableFixtures;
    // Independently rendered camera/3D geometry: a shallow second curvature
    // remains after the dominant developable bend. Some printed dots are absent.
    const rendered = fixture.render({ ...fixture.CASES[2], stretchedDome: 2, missingGrid: true });
    const frame = fixture.exactFrame(rendered.map), diagnostics = {};
    let measured, returned = false, callbackBeforeReturn = false;
    const surface = ScannerPro.MetricSurface.fit(rendered.source, { frame }, {
      diagnostics,
      onObservations(detail) { measured = detail; callbackBeforeReturn = !returned; },
    });
    returned = true;
    if (!surface) return { accepted: false, diagnostics };
    const base = { ...surface, refinement: undefined }, errors = [], beforeErrors = [];
    for (let y = 27; y <= 257; y += 7) for (let x = 27; x <= 183; x += 7) {
      const u = x / 210, v = y / 297, truth = rendered.map(u, v);
      const after = ScannerPro.MetricSurface.map(surface, u, v), before = ScannerPro.MetricSurface.map(base, u, v);
      errors.push(Math.hypot(after.x - truth.x, after.y - truth.y));
      beforeErrors.push(Math.hypot(before.x - truth.x, before.y - truth.y));
    }
    const warped = await ScannerPro.Lightweight.warp(rendered.source, rendered.pageQuad, { ...frame, metricSurface: surface }, { maxDimension: 1200 });
    const baseline = await ScannerPro.Lightweight.warp(rendered.source, rendered.pageQuad, { ...frame, metricSurface: base }, { maxDimension: 1200 });
    const dotIds = new Set(measured.points.map(p => `${p.row},${p.col}`));
    const inventedInMissingArea = measured.points.filter(p => p.col > 14 && p.col < 20 && p.row > 22 && p.row < 30).length;
    const malformedRuns = [...measured.rows.map(run => ({ ...run, along: 'col', axis: 'row' })), ...measured.columns.map(run => ({ ...run, along: 'row', axis: 'col' }))]
      .filter(run => run.points.some((index, at) => {
        const point = measured.points[index], previous = measured.points[run.points[at - 1]];
        return !point || point[run.axis] !== run.index || (at && (point[run.along] <= previous[run.along] || point[run.along] - previous[run.along] > 2));
      })).length;
    return { accepted: true, diagnostics: surface.diagnostics, externalRMS: diagnostics.rms,
      callbackBeforeReturn, points: measured.points.length, unique: dotIds.size,
      inventedInMissingArea, malformedRuns, maximum: Math.max(...errors), beforeMaximum: Math.max(...beforeErrors),
      writing: fixture.lineErrors(warped), beforeWriting: fixture.lineErrors(baseline),
      canonicalPoints: measured.points.every(p => p.u === (20 + p.col * 5) / 210 && p.v === (19.3 + p.row * 5) / 297),
      dimensions: [measured.width, measured.height], sourceDimensions: [rendered.source.width, rendered.source.height] };
  });
  console.log(JSON.stringify(report));
  expect(report.accepted, JSON.stringify(report)).toBe(true);
  expect(report.callbackBeforeReturn).toBe(true);
  expect(report.points).toBe(report.diagnostics.observations);
  expect(report.unique).toBe(report.points);
  expect(report.inventedInMissingArea).toBe(0);
  expect(report.malformedRuns).toBe(0);
  expect(report.canonicalPoints).toBe(true);
  expect(report.dimensions).toEqual(report.sourceDimensions);
  expect(report.diagnostics.measuredRefinement.accepted).toBe(true);
  expect(report.diagnostics.measuredRefinement.validation).toBeGreaterThan(500);
  expect(report.diagnostics.measuredRefinement.afterRMS).toBeLessThan(report.diagnostics.measuredRefinement.beforeRMS * 0.5);
  expect(report.externalRMS).toBe(report.diagnostics.rms);
  expect(report.maximum).toBeLessThan(0.95);
  expect(report.maximum).toBeLessThan(report.beforeMaximum * 0.55);
  expect(report.writing.maximum).toBeLessThan(0.003);
  expect(report.writing.mean).toBeLessThan(report.beforeWriting.mean);
});

test('blank and incompatible sheets cannot invent dots or bypass physical surface rejection', async ({ page }) => {
  const report = await page.evaluate(() => {
    const f = ScannerDevelopableFixtures;
    const rendered = f.render({ axis: 0, pitch: 0.1, yaw: 0.1, stretchedDome: 65 });
    let seen = 0;
    const incompatible = ScannerPro.MetricSurface.fit(rendered.source, { frame: f.exactFrame(rendered.map) }, { onObservations(d) { seen = d.points.length; } });
    const blank = document.createElement('canvas'); blank.width = rendered.source.width; blank.height = rendered.source.height;
    blank.getContext('2d').fillStyle = '#fff'; blank.getContext('2d').fillRect(0, 0, blank.width, blank.height);
    let invented = false;
    const blankResult = ScannerPro.MetricSurface.fit(blank, { frame: f.exactFrame(rendered.map) }, { onObservations() { invented = true; } });
    return { incompatible: !!incompatible, seen, blank: !!blankResult, invented };
  });
  expect(report.incompatible).toBe(false);
  expect(report.blank).toBe(false);
  expect(report.invented).toBe(false);
});
