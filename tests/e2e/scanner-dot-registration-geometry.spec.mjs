import { expect, test } from '@playwright/test';

test('exact bullet registration uses real source centres and leaves missing sites as interpolation only', async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=exact-bullet-geometry', { waitUntil: 'domcontentloaded' });
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-developable.js' });
  const report = await page.evaluate(() => {
    const f = ScannerDevelopableFixtures, rendered = f.render({ ...f.CASES[2], missingGrid: true });
    let seen, registered;
    const surface = ScannerPro.MetricSurface.fit(rendered.source, { frame: f.exactFrame(rendered.map) }, {
      onObservations(detail) { seen = detail; }, onRegistration(detail) { registered = detail; },
    });
    if (!surface?.registration || !registered) return { accepted: false, diagnostics: surface?.diagnostics };
    const missing = registered.points.filter(p => (p.col * 17 + p.row * 7) % 13 < 2 ||
      (p.col > 14 && p.col < 20 && p.row > 23 && p.row < 30));
    const errors = registered.points.map(p => {
      const q = ScannerPro.MetricSurface.map(surface, p.u, p.v);
      return Math.hypot(q.x - p.x, q.y - p.y);
    });
    const truthErrors = registered.points.map(p => {
      const q = rendered.map(p.u, p.v);
      return Math.hypot(q.x - p.x, q.y - p.y);
    }).sort((a, b) => a - b);
    const g = surface.registration, folded = [];
    for (let y = 0; y < g.rowPositions.length - 1; y++) for (let x = 0; x < g.colPositions.length - 1; x++) {
      const a = ScannerPro.MetricSurface.map(surface, g.colPositions[x], g.rowPositions[y]),
        b = ScannerPro.MetricSurface.map(surface, g.colPositions[x + 1], g.rowPositions[y]),
        c = ScannerPro.MetricSurface.map(surface, g.colPositions[x], g.rowPositions[y + 1]);
      if ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x) <= 0) folded.push([x, y]);
    }
    const physicalMetrics = JSON.stringify([surface.diagnostics.rms, surface.diagnostics.p90, surface.diagnostics.p99]),
      calibrate = height => ScannerPro.MetricSurface.refineForRaster(rendered.source, surface, {width: Math.round(height * 210 / 297), height}),
      first = calibrate(900), firstCoordinates = JSON.stringify(surface.registration.points.map(p => [p.x, p.y])),
      repeated = calibrate(900), repeatedCoordinates = JSON.stringify(surface.registration.points.map(p => [p.x, p.y]));
    calibrate(1200); calibrate(900);
    const restoredCoordinates = JSON.stringify(surface.registration.points.map(p => [p.x, p.y]));
    return { accepted: true, type: surface.type, anchors: registered.points.length,
      idempotent: first === repeated && firstCoordinates === repeatedCoordinates,
      resolutionIndependent: firstCoordinates === restoredCoordinates,
      independentMetricsUnchanged: physicalMetrics === JSON.stringify([surface.diagnostics.rms, surface.diagnostics.p90, surface.diagnostics.p99]),
      sourceMeasurementsPreserved: first.points.every(p => Number.isFinite(p.measuredX) && Number.isFinite(p.measuredY)),
      seen: seen.points.length, missing: missing.length, missingPoints: missing, maximum: Math.max(...errors),
      p99TruthError: truthErrors[Math.floor(truthErrors.length * .99)], folded: folded.length,
      independentRMS: surface.diagnostics.rms, diagnostics: surface.diagnostics.registration,
      allRefined: registered.points.every(p => p.centerRefined), unique: new Set(registered.points.map(p => `${p.row}:${p.col}`)).size };
  });
  console.log(JSON.stringify(report));
  expect(report.accepted, JSON.stringify(report)).toBe(true);
  expect(report.type).toBe('developable-pinhole');
  expect(report.anchors).toBeGreaterThan(900);
  expect(report.unique).toBe(report.anchors);
  expect(report.allRefined).toBe(true);
  expect(report.missing).toBe(0);
  expect(report.maximum).toBeLessThan(1e-8);
  expect(report.diagnostics.maximumAnchorError).toBe(report.maximum);
  expect(report.independentRMS).toBeGreaterThan(report.maximum);
  expect(report.p99TruthError).toBeLessThan(0.4);
  expect(report.folded).toBe(0);
  expect(report.idempotent).toBe(true);
  expect(report.resolutionIndependent).toBe(true);
  expect(report.independentMetricsUnchanged).toBe(true);
  expect(report.sourceMeasurementsPreserved).toBe(true);
});
