import {expect, test} from '@playwright/test';

test.beforeEach(async ({page}) => {
  await page.goto('/scanner/index.html', {waitUntil: 'domcontentloaded'});
  await page.addScriptTag({url: '/tests/e2e/fixtures/scanner-developable.js'});
});

for (const variant of ['uneven contrast', 'washed-out top rows', 'local crease']) {
  test(`measured lattice preserves writing with ${variant}`, async ({page}) => {
    const report = await page.evaluate(async variant => {
      const fixture = ScannerDevelopableFixtures;
      const definition = {axis: 0, pitch: .18, yaw: -.24};
      if (variant === 'uneven contrast') definition.dotColorAt = row => row < 14 ? '#f4f4f4' : '#cbc8bf';
      if (variant === 'washed-out top rows') definition.dotColorAt = row => row < 5 ? '#fff' : '#cbc8bf';
      if (variant === 'local crease') definition.crease = {at: -40, angle: .38, width: 5};
      const rendered = fixture.render(definition), frame = fixture.exactFrame(rendered.map);
      const diagnostics = {};
      const surface = ScannerPro.MetricSurface.fitLattice(rendered.source, {frame}, {diagnostics});
      if (!surface) return {diagnostics};
      const warped = await ScannerPro.Lightweight.warp(rendered.source, rendered.pageQuad,
        {...frame, metricSurface: surface}, {maxDimension: 1200});
      return {diagnostics, surface: surface.diagnostics,
        alignment: ScannerPro.MetricSurface.certifyRaster(warped), errors: fixture.lineErrors(warped)};
    }, variant);
    expect(report.surface, JSON.stringify(report.diagnostics)).toBeTruthy();
    if (variant !== 'local crease') {
      expect(report.diagnostics.faintRecoveryReason).toBe('incomplete-lattice-extent');
      expect(report.diagnostics.faintDotPass).toBe(true);
    }
    expect(report.surface.validation).toBeGreaterThan(300);
    expect(report.alignment.status).toBe('verified');
    expect(report.alignment.observations).toBeGreaterThan(1000);
    // Known ink lines are independent of the dot observations used to fit.
    expect(report.errors.maximum).toBeLessThan(.001);
    expect(report.errors.mean).toBeLessThan(.0002);
  });
}

test('a plausible perimeter without real dots cannot produce a recovery surface', async ({page}) => {
  const accepted = await page.evaluate(() => {
    const fixture = ScannerDevelopableFixtures;
    const rendered = fixture.render({dotColor: '#fff', crease: {at: -40, angle: .38, width: 5}});
    return !!ScannerPro.MetricSurface.fitLattice(rendered.source, {frame: fixture.exactFrame(rendered.map)});
  });
  expect(accepted).toBe(false);
});
