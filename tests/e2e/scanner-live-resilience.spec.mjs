import { expect, test } from '@playwright/test';
import fs from 'node:fs';

const fixtures = ['scanner-synthetic.js', 'scanner-developable.js']
  .map(name => fs.readFileSync(new URL(`./fixtures/${name}`, import.meta.url), 'utf8')).join('\n');

async function camera(page, mode) {
  await page.addInitScript({ content: fixtures });
  await page.addInitScript(mode => {
    const fixture = window.liveResilience = { constraints: [], observed: 0, missed: 0, retained: 0, focus: [], draws: [], measurements: [], readiness: [], encodings: [] };
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
      fixture.opened = (fixture.opened || 0) + 1;
      // The camera pipeline now verifies a physically straightenable lattice.
      // Use an independently projected real sheet; the old Coons fixture
      // stretches paper and legitimately fails the stricter physical fit.
      const canvas = mode.startsWith('visual-')
        ? ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source
        : ScannerDevelopableFixtures.render({ radius: 0, axis: 0, pitch: .1, yaw: .1 }).source;
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
        if (mode === 'blank-markers') {
          ['#ff0000', '#000000', '#0000ff', '#6eff12'].forEach((colour, index) => {
            context.fillStyle = colour;
            context.beginPath();
            context.arc(135 + (10.125 + index * .25) * px, 145 + 27.68 * px, .125 * px, 0, Math.PI * 2);
            context.fill();
            context.stroke();
          });
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
    if (mode === 'pending-readiness') {
      const inspect = ScannerPro.CaptureQuality.inspectReadinessAsync;
      const gate = new Promise(resolve => { liveResilience.releaseReadiness = resolve; });
      ScannerPro.CaptureQuality.inspectReadinessAsync = async (...args) => {
        const result = await inspect(...args);
        liveResilience.pendingReadiness = (liveResilience.pendingReadiness || 0) + 1;
        // Simulate a late completed worker response, including one whose
        // wrapper ignores cancellation, so the controller must reject it.
        await gate;
        liveResilience.releasedReadiness = (liveResilience.releasedReadiness || 0) + 1;
        return result;
      };
    }
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
    const toBlob = HTMLCanvasElement.prototype.toBlob;
    HTMLCanvasElement.prototype.toBlob = function(callback, ...args) {
      if (args[0] === 'image/jpeg' && document.getElementById('liveScanner').dataset.state === 'capturing') {
        const entry = { startedAt: performance.now() };
        liveResilience.encodings.push(entry);
        return toBlob.call(this, blob => setTimeout(() => {
          entry.finishedAt = performance.now(); callback(blob);
        }, mode === 'slow-encoding' ? 450 : 0), ...args);
      }
      return toBlob.call(this, callback, ...args);
    };
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
      if (mode === 'single-clear' && fixture.readiness.some(sample => sample.valid)) return null;
      if (['single-clear', 'slow-encoding'].includes(mode) && detection?.source === 'yellow'
          && detection.confidence >= .78 && Math.max(detection.sharpness / 32, detection.edgeSharpness / 160) >= 1) {
        fixture.firstQualifiedAt ||= performance.now();
      }
      if (mode === 'burst-loss' && state === 'capturing') return null;
      if (mode === 'brief-loss' && detection?.source === 'yellow') {
        fixture.observed++;
        if (fixture.observed % 3 === 0) return null;
      }
      if (mode === 'soft-focus' && detection?.source === 'yellow') {
        fixture.observed++;
        // Autofocus initially hunts, then settles. A permanently soft frame
        // every third observation should not satisfy sustained sharpness.
        if (fixture.observed <= 6 && fixture.observed % 3 === 0) return { ...detection, sharpness: 8, edgeSharpness: 8 };
      }
      if (mode === 'gone') {
        if (++fixture.observed > 2) return null;
        return detection ? { ...detection, confidence: .7 } : null;
      }
      return detection;
    };
    addEventListener('scanner-capture-quality', event => { liveResilience.lastPhotoQuality = event.detail; });
    addEventListener('scanner-capture-readiness', event => {
      liveResilience.readiness.push({ at: performance.now(), ...event.detail });
    });
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
    expect(recorded.observed).toBeGreaterThanOrEqual(1);
    expect(recorded.readiness.find(sample => sample.ready)?.samples).toBeGreaterThanOrEqual(3);
    if (mode === 'brief-loss') {
      // Tracking may survive a brief miss; retained geometry alone must not
      // authorize the shutter, which still needs fresh sharp, gridded frames.
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

test('a verified clear frame survives a detection miss during the backup burst', async ({ page }) => {
  await camera(page, 'burst-loss');
  await expect(page.locator('#liveReview')).toBeVisible({ timeout: 6500 });
  await expect(page.locator('#liveAddBtn')).toBeEnabled({ timeout: 15000 });
  expect(await page.locator('#livePreview').evaluate(image => image.naturalWidth)).toBeGreaterThan(600);
  await expect(page.locator('#liveAlternatives')).toBeEmpty();
});

for (const mode of ['blank-crisp', 'blank-markers']) {
  test(`a crisp ${mode} page never captures without a measured bullet lattice`, async ({ page }) => {
    await camera(page, mode);
    await page.waitForFunction(() => liveResilience.readiness.length >= 3);
    await expect(page.locator('#liveReview')).toBeHidden();
    await expect(page.locator('#liveAddBtn')).toBeDisabled();
    const measured = await page.evaluate(() => liveResilience);
    expect(measured.encodings).toEqual([]);
    expect(measured.focus.length).toBeGreaterThanOrEqual(3);
    expect(Math.max(...measured.focus.map(sample => sample.interior))).toBeLessThan(32);
    expect(Math.min(...measured.focus.map(sample => sample.edge))).toBeGreaterThanOrEqual(160);
    expect(measured.readiness.every(sample => !sample.valid && !sample.ready && !sample.grid.valid)).toBe(true);
    expect(measured.readiness.some(sample => sample.reason === 'grid-not-found')).toBe(true);
    // The explicit shutter must not bypass the same failed photo evidence.
    await page.locator('#liveCaptureBtn').click();
    await expect(page.locator('#liveReview')).toBeHidden();
    expect(await page.evaluate(() => liveResilience.encodings)).toEqual([]);
  });
}

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

test('invalid page geometry cannot be inserted and automatically returns to the camera', async ({ page }) => {
  await camera(page, 'invalid-review');
  await expect.poll(() => page.evaluate(() => liveResilience.opened), {timeout:10000}).toBeGreaterThanOrEqual(2);
  await expect(page.locator('#liveAddBtn')).toBeDisabled();
  expect(await page.evaluate(() => liveResilience.warpCalls)).toBe(0);
});

test('one clear video frame cannot trigger a capture after the page disappears', async ({ page }) => {
  await camera(page, 'single-clear');
  await page.waitForFunction(() => liveResilience.readiness.some(sample => sample.valid));
  await page.waitForTimeout(1600);
  await expect(page.locator('#liveReview')).toBeHidden();
  await expect(page.locator('#liveAddBtn')).toBeDisabled();
  await expect(page.locator('#liveOverlay')).toHaveAttribute('data-detected', 'false');
  const measured = await page.evaluate(() => liveResilience);
  expect(measured.encodings).toEqual([]);
  expect(measured.readiness.filter(sample => sample.valid)).toHaveLength(1);
  expect(measured.readiness.some(sample => sample.ready)).toBe(false);
});

test('a late readiness result cannot capture or reopen a closed camera', async ({ page }) => {
  await camera(page, 'pending-readiness');
  await page.waitForFunction(() => liveResilience.pendingReadiness > 0);
  await page.evaluate(() => { ScannerPro.LiveCamera.close(); liveResilience.releaseReadiness(); });
  await page.waitForFunction(() => liveResilience.releasedReadiness > 0);
  await page.waitForTimeout(250);
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'closed');
  await expect(page.locator('#liveScanner')).toBeHidden();
  await expect(page.locator('#liveAddBtn')).toBeDisabled();
  const measured = await page.evaluate(() => ({ encodings: liveResilience.encodings, opened: liveResilience.opened }));
  expect(measured.encodings).toEqual([]);
  expect(measured.opened).toBe(1);
});

test('sustained sharp grid evidence precedes a burst that freezes before slow JPEG encoding finishes', async ({ page }) => {
  await camera(page, 'slow-encoding');
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review', { timeout: 15000 });
  const measured = await page.evaluate(() => liveResilience);
  expect(measured.encodings).toHaveLength(3);
  const ready = measured.readiness.find(sample => sample.ready);
  expect(ready).toBeTruthy();
  expect(ready.valid).toBe(true);
  expect(ready.grid.valid).toBe(true);
  expect(ready.samples).toBeGreaterThanOrEqual(3);
  expect(ready.elapsedMs).toBeGreaterThanOrEqual(300);
  expect(measured.encodings[0].startedAt).toBeGreaterThanOrEqual(ready.at);
  expect(measured.encodings[0].startedAt - ready.at).toBeLessThan(150);
  expect(measured.encodings[0].startedAt - measured.firstQualifiedAt).toBeLessThan(3000);
  expect(measured.encodings[2].startedAt).toBeLessThan(measured.encodings[0].finishedAt);
  expect(measured.encodings[0].finishedAt - measured.encodings[0].startedAt).toBeGreaterThanOrEqual(450);
  expect(await page.locator('#liveAlternatives button').count()).toBe(3);
  await test.info().attach('capture-timing', { body: JSON.stringify({ qualifiedAt: measured.firstQualifiedAt, readiness: measured.readiness, encodings: measured.encodings }), contentType: 'application/json' });
});
