import { expect, test } from '@playwright/test';
import fs from 'node:fs';

const fixtures = fs.readFileSync(new URL('./fixtures/scanner-synthetic.js', import.meta.url), 'utf8');

async function camera(page, mode) {
  await page.addInitScript({ content: fixtures });
  await page.addInitScript(mode => {
    const fixture = window.liveResilience = { constraints: [], observed: 0, missed: 0, retained: 0, focus: [], draws: [], measurements: [] };
    if (mode.startsWith('visual-')) {
      // Record the actual rendered frame path, independently of detector events.
      const paths = new WeakMap();
      for (const name of ['beginPath', 'moveTo', 'lineTo', 'stroke']) {
        const original = CanvasRenderingContext2D.prototype[name];
        CanvasRenderingContext2D.prototype[name] = function (...args) {
          if (this.canvas.id === 'liveOverlay') {
            if (name === 'beginPath') paths.set(this, []);
            if (name === 'moveTo' || name === 'lineTo') paths.get(this)?.push({ x: args[0], y: args[1] });
            if (name === 'stroke' && this.lineWidth === 3 * Math.min(devicePixelRatio || 1, 2)) {
              const video = document.getElementById('liveVideo');
              const scale = Math.min(this.canvas.width / video.videoWidth, this.canvas.height / video.videoHeight);
              const width = video.videoWidth * scale, height = video.videoHeight * scale;
              const dx = (this.canvas.width - width) / 2, dy = (this.canvas.height - height) / 2;
              fixture.draws.push({ at: performance.now(), points: (paths.get(this) || []).map(point => ({
                x: (point.x - dx) / width, y: (point.y - dy) / height
              })) });
            }
          }
          return original.apply(this, args);
        };
      }
    }
    navigator.mediaDevices.getUserMedia = async () => {
      const canvas = ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
      const context = canvas.getContext('2d');
      if (mode.startsWith('blank-')) {
        canvas.width = 900;
        canvas.height = 1200;
        context.fillStyle = '#86694c';
        context.fillRect(0, 0, canvas.width, canvas.height);
        context.fillStyle = '#f6f5f1';
        context.fillRect(135, 145, 630, 891);
        context.strokeStyle = '#e8ce43';
        context.lineWidth = 3;
        const px = 30;
        context.strokeRect(135 + 1.5 * px, 145 + 1.43 * px, 18 * px, 27 * px);
        for (const start of [2, 11]) {
          context.strokeRect(135 + start * px, 145 + 27.43 * px, 8 * px, .5 * px);
          for (let column = .5; column < 8; column += .5) {
            context.beginPath();
            context.moveTo(135 + (start + column) * px, 145 + 27.43 * px);
            context.lineTo(135 + (start + column) * px, 145 + 27.93 * px);
            context.stroke();
          }
        }
        if (mode === 'blank-blurred') {
          const copy = document.createElement('canvas');
          copy.width = canvas.width; copy.height = canvas.height;
          copy.getContext('2d').drawImage(canvas, 0, 0);
          context.filter = 'blur(3px)';
          context.drawImage(copy, 0, 0);
          context.filter = 'none';
        }
      }
      const stream = canvas.captureStream(20);
      const track = stream.getVideoTracks()[0];
      const interval = setInterval(() => {
        context.fillStyle = '#957151';
        context.fillRect(0, 0, 2, 2);
      }, 50);
      const stop = track.stop.bind(track);
      track.stop = () => { clearInterval(interval); stop(); };
      track.getCapabilities = () => ({ focusMode: ['manual', 'continuous'],
        exposureMode: ['continuous'], whiteBalanceMode: ['manual'] });
      track.applyConstraints = async value => { fixture.constraints.push(value); };
      return stream;
    };
  }, mode);
  await page.goto('/scanner/index.html', { waitUntil: 'domcontentloaded' });
  await page.evaluate(mode => {
    if (mode === 'invalid-review') {
      // Exercise the supported main-thread fallback explicitly: the normal
      // pipeline now runs detection and warp in dedicated workers.
      window.Worker = undefined;
      liveResilience.warpCalls = 0;
      ScannerPro.Lightweight.detectPageAsync = async () => ({
        method: 'stencil', confidence: 1,
        pageQuad: [{ x: 0, y: 0 }, { x: 900, y: 0 }, { x: 900, y: 1170 }, { x: 0, y: 1170 }],
        quality: { valid: false, reason: 'The detected page disagrees with the camera frame', seedAgreement: 0 }
      });
      const warp = ScannerPro.Lightweight.warp;
      ScannerPro.Lightweight.warp = (...args) => {
        liveResilience.warpCalls++;
        return warp(...args);
      };
    }
    const original = ScannerPro.LiveDetection.detect;
    ScannerPro.LiveDetection.detect = (...args) => {
      const detection = original(...args);
      const fixture = window.liveResilience;
      const state = document.getElementById('liveScanner').dataset.state;
      if (mode.startsWith('visual-') && (fixture.base || detection?.source === 'yellow')) {
        fixture.base ||= structuredClone(detection);
        fixture.startedAt ||= performance.now();
        const at = performance.now(), elapsed = at - fixture.startedAt;
        if (mode === 'visual-gap' && elapsed > 600) return null;
        // Isolate temporal rendering with known measured motion and local
        // contour noise; the camera itself continues delivering real frames.
        const dx = Math.max(0, Math.min(.1, (elapsed - 750) * .0001));
        const noisy = ++fixture.observed % 2 ? .018 : -.018;
        const shifted = structuredClone(fixture.base);
        const shift = points => points.map(point => Array.isArray(point) ? shift(point) : { x: point.x + dx, y: point.y });
        for (const key of ['quad', 'pageQuad', 'frameEdges', 'pageEdges', 'markers', 'markerEdges']) {
          if (shifted[key]) shifted[key] = shift(shifted[key]);
        }
        shifted.frameEdges[0][16].y += noisy;
        shifted.contour = shifted.frameEdges.flat();
        shifted.confidence = .7;
        fixture.measurements.push({ at, dx, first: shifted.contour[0], mid: shifted.contour[16] });
        return shifted;
      }
      if (mode === 'burst-loss' && state === 'capturing') return null;
      if (mode === 'brief-loss' && detection?.source === 'yellow') {
        fixture.observed++;
        if (fixture.observed % 3 === 0) return null;
      }
      if (mode === 'soft-focus' && detection?.source === 'yellow') {
        fixture.observed++;
        if (fixture.observed % 3 === 0) return { ...detection, sharpness: 8, edgeSharpness: 8 };
      }
      if (mode === 'gone' && ++fixture.observed > 2) return null;
      return detection;
    };
    addEventListener('scanner-live-detection', event => {
      if (event.detail?.source === 'yellow') liveResilience.focus.push({
        interior: event.detail.sharpness, edge: event.detail.edgeSharpness
      });
      if (!event.detail) {
        liveResilience.missed++;
        if (document.getElementById('liveOverlay').dataset.detected === 'true') liveResilience.retained++;
      }
    });
    void ScannerPro.LiveCamera.open();
  }, mode);
}

for (const mode of ['brief-loss', 'soft-focus']) {
  test(`small ${mode} fluctuations retain tracking and complete capture promptly`, async ({ page }) => {
    await camera(page, mode);
    await expect(page.locator('#liveReview')).toBeVisible({ timeout: 6500 });
    await expect(page.locator('#liveAddBtn')).toBeEnabled({ timeout: 15000 });
    expect(await page.locator('#livePreview').evaluate(image => image.complete && image.naturalWidth > 0)).toBe(true);
    const recorded = await page.evaluate(() => liveResilience);
    expect(recorded.observed).toBeGreaterThan(4);
    if (mode === 'brief-loss') {
      expect(recorded.missed).toBeGreaterThan(0);
      expect(recorded.retained).toBe(recorded.missed);
    }
    expect(recorded.constraints).toEqual([{ advanced: [{ focusMode: 'continuous', exposureMode: 'continuous' }] }]);
  });
}

test('a document that leaves the feed is cleared and never captured from retained tracking', async ({ page }) => {
  await camera(page, 'gone');
  await expect(page.locator('#liveOverlay')).toHaveAttribute('data-detected', 'true');
  await expect(page.locator('#liveOverlay')).toHaveAttribute('data-detected', 'false');
  await page.waitForTimeout(1200);
  await expect(page.locator('#liveReview')).toBeHidden();
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'searching');
});

test('clear frames saved during tracking survive a detection miss at the shutter', async ({ page }) => {
  await camera(page, 'burst-loss');
  await expect(page.locator('#liveReview')).toBeVisible({ timeout: 6500 });
  await expect(page.locator('#liveAddBtn')).toBeEnabled({ timeout: 15000 });
  expect(await page.locator('#livePreview').evaluate(image => image.naturalWidth)).toBeGreaterThan(600);
  expect(await page.locator('#liveAlternatives button').count()).toBeGreaterThanOrEqual(2);
});

test('a crisp blank template captures using measured border focus despite its empty interior', async ({ page }) => {
  await camera(page, 'blank-crisp');
  await expect(page.locator('#liveReview')).toBeVisible({ timeout: 6500 });
  await expect(page.locator('#liveAddBtn')).toBeEnabled({ timeout: 20000 });
  const focus = await page.evaluate(() => liveResilience.focus);
  expect(focus.length).toBeGreaterThanOrEqual(5);
  expect(Math.max(...focus.map(sample => sample.interior))).toBeLessThan(32);
  expect(Math.min(...focus.map(sample => sample.edge))).toBeGreaterThanOrEqual(160);
  expect(await page.locator('#livePreview').evaluate(image => image.complete && image.naturalWidth > 0)).toBe(true);
});

test('a blurred blank template remains in the live feed instead of passing border focus', async ({ page }) => {
  await camera(page, 'blank-blurred');
  await expect(page.locator('#liveCaptureBtn')).toBeEnabled();
  await page.waitForTimeout(2200);
  await expect(page.locator('#liveReview')).toBeHidden();
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'searching');
  const focus = await page.evaluate(() => liveResilience.focus);
  expect(focus.length).toBeGreaterThan(3);
  expect(Math.max(...focus.map(sample => sample.interior))).toBeLessThan(32);
  expect(Math.max(...focus.map(sample => sample.edge))).toBeLessThan(160);
});

test('the complete displayed outline moves between analyses and damps local contour jitter', async ({ page }) => {
  await camera(page, 'visual-motion');
  await page.waitForFunction(() => liveResilience.startedAt && performance.now() - liveResilience.startedAt > 2800);
  const metrics = await page.evaluate(() => {
    const { startedAt, draws, measurements } = liveResilience;
    const between = (items, from, to) => items.filter(item => item.at - startedAt > from && item.at - startedAt < to);
    const motion = between(draws, 900, 1650);
    const observed = between(measurements, 900, 1650);
    const changed = motion.slice(1).filter((item, i) => Math.abs(item.points[0].x - motion[i].points[0].x) > .00001).length;
    const range = values => Math.max(...values) - Math.min(...values);
    return {
      changed, observed: observed.length,
      shownJitter: range(between(draws, 250, 720).map(item => item.points[16].y)),
      measuredJitter: range(between(measurements, 250, 720).map(item => item.mid.y)),
      finalMotion: draws.at(-1).points[0].x - measurements[0].first.x
    };
  });
  await test.info().attach('overlay-motion', { body: JSON.stringify(metrics, null, 2), contentType: 'application/json' });
  expect(metrics.changed).toBeGreaterThan(metrics.observed * 1.5);
  expect(metrics.shownJitter).toBeLessThan(metrics.measuredJitter * .65);
  expect(metrics.finalMotion).toBeGreaterThan(.09);
  expect(metrics.finalMotion).toBeLessThan(.105);
  await expect(page.locator('#liveReview')).toBeHidden();
});

test('visual interpolation expires after a lost document and stops when the camera closes', async ({ page }) => {
  await camera(page, 'visual-gap');
  await expect(page.locator('#liveOverlay')).toHaveAttribute('data-detected', 'true');
  await page.waitForFunction(() => liveResilience.startedAt && performance.now() - liveResilience.startedAt > 1050);
  await expect(page.locator('#liveOverlay')).toHaveAttribute('data-detected', 'false');
  await expect(page.locator('#liveReview')).toBeHidden();
  const before = await page.evaluate(() => { ScannerPro.LiveCamera.close(); return liveResilience.draws.length; });
  await page.waitForTimeout(200);
  expect(await page.evaluate(() => liveResilience.draws.length)).toBe(before);
});

test('invalid page geometry preserves the original photo even when detector confidence is maximal', async ({ page }) => {
  await camera(page, 'invalid-review');
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review', { timeout: 6500 });
  await expect(page.locator('#liveReviewStatus')).toContainText('Original photo ready');
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
  const preview = await page.locator('#livePreview').evaluate(image => ({
    visible: !image.hidden, complete: image.complete, width: image.naturalWidth
  }));
  expect(preview).toEqual({ visible: true, complete: true, width: 900 });
  expect(await page.evaluate(() => liveResilience.warpCalls)).toBe(0);
});
