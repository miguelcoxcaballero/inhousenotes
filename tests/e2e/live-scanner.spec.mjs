import { expect, test } from '@playwright/test';
import fs from 'node:fs';

const fixtureSource = fs.readFileSync(new URL('./fixtures/scanner-synthetic.js', import.meta.url), 'utf8');

// Supply real video frames and MediaStream tracks. Only the physical camera
// boundary is replaced: detection, drawing, capture, warp and handoff stay real.
function installCameraFixture(options) {
  if (options.legacyVideoFrames) {
    Object.defineProperty(HTMLVideoElement.prototype, 'requestVideoFrameCallback', { configurable: true, value: undefined });
    Object.defineProperty(HTMLVideoElement.prototype, 'getVideoPlaybackQuality', {
      configurable: true, value: () => ({ totalVideoFrames: 0 })
    });
  }
  if (options.slowEncoding) {
    const original = HTMLCanvasElement.prototype.toBlob;
    HTMLCanvasElement.prototype.toBlob = function (callback, ...args) {
      original.call(this, blob => setTimeout(() => callback(blob), 450), ...args);
    };
  }
  const fake = window.__scannerCameraFixture = {
    mode: options.mode || 'document',
    requests: [],
    streams: [],
    detections: [],
    pending: [],
    stopFrames: [],
    frames: 0
  };
  addEventListener('scanner-live-detection', event => {
    fake.detections.push(event.detail);
    if (fake.detections.length > 40) fake.detections.shift();
  });
  const createStream = () => {
    const definition = ScannerSyntheticFixtures.CASES[0];
    const original = ScannerSyntheticFixtures.renderCase(definition).source;
    const canvas = document.createElement('canvas');
    canvas.width = original.width;
    canvas.height = original.height;
    const context = canvas.getContext('2d');
    const paint = () => {
      context.filter = fake.mode === 'blurred' ? 'blur(16px)' : 'none';
      context.drawImage(original, 0, 0);
      context.filter = 'none';
      if (fake.mode === 'unmarked') {
        context.fillStyle = '#957151';
        context.fillRect(0, 0, canvas.width, canvas.height);
        context.fillStyle = '#fff';
        context.fillRect(130, 65, 640, 1040);
        context.strokeStyle = '#174fca';
        context.lineWidth = 3;
        for (let y = 200; y < 950; y += 50) {
          context.beginPath();
          context.moveTo(205, y);
          context.lineTo(650, y + 5);
          context.stroke();
        }
      }
      fake.frames += 1;
    };
    paint();
    const stream = canvas.captureStream(20);
    const track = stream.getVideoTracks()[0];
    const stop = track.stop.bind(track);
    const interval = setInterval(paint, 50);
    fake.stopFrames.push(() => clearInterval(interval));
    track.stop = () => { clearInterval(interval); stop(); };
    track.applyConstraints = async () => {};
    fake.streams.push(stream);
    return stream;
  };
  fake.resolvePending = () => fake.pending.splice(0).forEach(resolve => resolve(createStream()));
  fake.setMode = mode => { fake.mode = mode; };
  fake.trackStates = () => fake.streams.flatMap(stream => stream.getTracks().map(track => track.readyState));
  fake.setHidden = hidden => {
    Object.defineProperty(document, 'hidden', { configurable: true, value: hidden });
    document.dispatchEvent(new Event('visibilitychange'));
  };
  Object.defineProperty(navigator.mediaDevices, 'getUserMedia', {
    configurable: true,
    value: async constraints => {
      fake.requests.push(constraints);
      if (fake.mode === 'denied') throw new DOMException('Camera denied by test', 'NotAllowedError');
      if (fake.mode === 'pending') return new Promise(resolve => fake.pending.push(resolve));
      return createStream();
    }
  });
}

async function fakeCamera(page, options = {}) {
  await page.addInitScript({ content: `${fixtureSource}\n;(${installCameraFixture.toString()})(${JSON.stringify(options)});` });
}

async function openScanner(page, options = {}) {
  await fakeCamera(page, options);
  await page.goto('/scanner/index.html?embed=1&session=e2e-live-camera', { waitUntil: 'domcontentloaded' });
  await expect(page.locator('#liveScanner')).toBeVisible();
}

test('live video draws the detected yellow frame and captures a stable page for review', async ({ page }) => {
  test.setTimeout(45_000);
  await openScanner(page);
  await expect(page.locator('#liveVideo')).toBeVisible();
  await page.waitForFunction(() => {
    const video = document.getElementById('liveVideo');
    return video.srcObject instanceof MediaStream && video.videoWidth > 0 && video.currentTime > 0;
  });
  await expect(page.locator('#liveOverlay')).toHaveAttribute('data-detected', 'true', { timeout: 15_000 });
  const overlay = await page.evaluate(() => {
    const canvas = document.getElementById('liveOverlay');
    const pixels = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data;
    const detection = window.__scannerCameraFixture.detections.filter(Boolean).at(-1);
    const definition = ScannerSyntheticFixtures.CASES[0];
    const map = ScannerSyntheticFixtures.mappingFor(definition);
    const { u0, u1, v0, v1 } = ScannerSyntheticFixtures.FRAME;
    const expected = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]].map(([u, v]) => map(u, v));
    const cornerError = detection ? Math.max(...detection.quad.map((point, index) =>
      Math.hypot(point.x - expected[index].x / definition.width,
        point.y - expected[index].y / definition.height))) : Infinity;
    let yellow = 0;
    let left = canvas.width, right = 0, top = canvas.height, bottom = 0;
    for (let y = 0; y < canvas.height; y += 1) {
      for (let x = 0; x < canvas.width; x += 1) {
        const offset = (y * canvas.width + x) * 4;
        if (pixels[offset] < 180 || pixels[offset + 1] < 160
          || pixels[offset + 2] > 100 || pixels[offset + 3] < 100) continue;
        yellow += 1;
        left = Math.min(left, x); right = Math.max(right, x);
        top = Math.min(top, y); bottom = Math.max(bottom, y);
      }
    }
    return { yellow, left: left / canvas.width, right: right / canvas.width,
      top: top / canvas.height, bottom: bottom / canvas.height,
      source: detection?.source, markers: detection?.markers?.length, cornerError,
      constraints: window.__scannerCameraFixture.requests[0] };
  });
  expect(overlay.yellow).toBeGreaterThan(100);
  expect(overlay.left).toBeGreaterThan(.02);
  expect(overlay.right).toBeLessThan(.98);
  expect(overlay.top).toBeGreaterThan(.01);
  expect(overlay.bottom).toBeLessThan(.99);
  expect(overlay.source).toBe('yellow');
  expect(overlay.markers).toBe(2);
  expect(overlay.cornerError).toBeLessThan(.025);
  expect(overlay.constraints.audio).toBe(false);
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review', { timeout: 30_000 });
  await expect(page.locator('#livePreview')).toBeVisible();
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
  expect(await page.evaluate(() => window.__scannerCameraFixture.trackStates())).toEqual(['ended']);
  expect(await page.locator('#livePreview').evaluate(image => image.naturalWidth / image.naturalHeight))
    .toBeCloseTo(21 / 29.7, 1);
});

test('retaking releases the previous camera and only confirmation inserts the scanned page', async ({ page }) => {
  test.setTimeout(60_000);
  await fakeCamera(page);
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(async () => {
    await window.__IHN_TEST_API__.ready();
    await window.__IHN_TEST_API__.resetLocalDocument(1, 'Live scan handoff');
    await window.__IHN_TEST_API__.showEditorForTest();
  });
  await page.locator('#btn-edit-pages').click();
  await page.locator('#btn-scan-page').click();
  const scanner = page.frameLocator('#scanner-editor-frame');
  await expect(scanner.locator('#liveReview')).toBeVisible({ timeout: 30_000 });
  expect(await page.evaluate(() => window.__IHN_TEST_API__.snapshot().pages.length)).toBe(1);
  await scanner.locator('#liveRetakeBtn').click();
  await expect(scanner.locator('#liveCameraView')).toBeVisible();
  await expect(scanner.locator('#liveReview')).toBeVisible({ timeout: 30_000 });
  expect(await scanner.locator('body').evaluate(() => window.__scannerCameraFixture.trackStates()))
    .toEqual(['ended', 'ended']);
  expect(await page.evaluate(() => window.__IHN_TEST_API__.snapshot().pages.length)).toBe(1);
  await scanner.locator('#liveAddBtn').click();
  await expect(page.locator('#scanner-editor-overlay')).not.toHaveClass(/visible/, { timeout: 15_000 });
  const saved = await page.evaluate(async () => {
    const result = await window.__IHN_TEST_API__.checkpointBeforeLeaving();
    return result.snapshot;
  });
  expect(saved.pages).toHaveLength(2);
  expect(saved.pages[1].backgroundSource).toBe('custom');
  expect(saved.pages[1].backgroundImage).toMatch(/^data:image\/jpeg/);
  expect(saved.dirtyPages).toEqual([]);
});

for (const mode of ['blurred', 'unmarked']) {
  test(`a ${mode} camera image never triggers automatic capture`, async ({ page }) => {
    await openScanner(page, { mode });
    await page.waitForFunction(() => document.getElementById('liveVideo').currentTime > .5);
    // This observation interval exceeds the full automatic stability gate.
    await page.waitForTimeout(2200);
    await expect(page.locator('#liveReview')).toBeHidden();
    await expect(page.locator('#liveCameraView')).toBeVisible();
    expect(await page.evaluate(() => window.__scannerCameraFixture.trackStates())).toEqual(['live']);
    await page.evaluate(() => ScannerPro.LiveCamera.close());
    expect(await page.evaluate(() => window.__scannerCameraFixture.trackStates())).toEqual(['ended']);
  });
}

test('denied camera permission offers image import and retry without trapping the scanner', async ({ page }) => {
  test.setTimeout(45_000);
  await openScanner(page, { mode: 'denied' });
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'error');
  await expect(page.locator('#liveRetryBtn')).toBeVisible();
  await expect(page.locator('#liveImportBtn')).toBeVisible();
  await expect(page.locator('#liveCloseBtn')).toBeVisible();
  const image = await page.evaluate(() => ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0])
    .source.toDataURL('image/png').split(',')[1]);
  await page.locator('#liveFileInput').setInputFiles({
    name: 'camera-fallback.png', mimeType: 'image/png', buffer: Buffer.from(image, 'base64')
  });
  await expect(page.locator('#liveReview')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('#livePreview')).toBeVisible();
  // The original image appears immediately while correction is still running.
  // Wait for the controller's bounded review transition before testing Add.
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review', { timeout: 20_000 });
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
});

test('closing while permission is pending stops a camera stream granted later', async ({ page }) => {
  await openScanner(page, { mode: 'pending' });
  await page.waitForFunction(() => window.__scannerCameraFixture.pending.length === 1);
  await page.evaluate(() => ScannerPro.LiveCamera.close());
  await expect(page.locator('#liveScanner')).toBeHidden();
  await page.evaluate(() => window.__scannerCameraFixture.resolvePending());
  await expect.poll(() => page.evaluate(() => window.__scannerCameraFixture.trackStates())).toEqual(['ended']);
  await expect(page.locator('#liveScanner')).toBeHidden();
  expect(await page.locator('#liveVideo').evaluate(video => video.srcObject)).toBeNull();
});

for (const grantWhileHidden of [false, true]) {
  test(`Android permission visibility changes preserve camera startup (grant while hidden: ${grantWhileHidden})`, async ({ page }) => {
    await openScanner(page, { mode: 'pending' });
    await page.waitForFunction(() => window.__scannerCameraFixture.pending.length === 1);
    await expect(page.locator('#liveVideo')).toBeHidden();
    await expect(page.locator('#liveCaptureBtn')).toBeDisabled();
    await page.evaluate(() => window.__scannerCameraFixture.setHidden(true));
    await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'requesting');
    if (!grantWhileHidden) await page.evaluate(() => window.__scannerCameraFixture.setHidden(false));
    await page.evaluate(() => {
      window.__scannerCameraFixture.setMode('unmarked');
      window.__scannerCameraFixture.resolvePending();
    });
    if (grantWhileHidden) {
      await expect.poll(() => page.evaluate(() => window.__scannerCameraFixture.trackStates())).toEqual(['ended']);
      await page.evaluate(() => window.__scannerCameraFixture.setHidden(false));
    }
    await expect(page.locator('#liveVideo')).toBeVisible();
    await expect(page.locator('#liveCaptureBtn')).toBeEnabled();
    await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'searching');
    const streamStates = await page.evaluate(() => window.__scannerCameraFixture.trackStates());
    expect(streamStates).toEqual(grantWhileHidden ? ['ended', 'live'] : ['live']);
  });
}

test('backgrounding an active camera stops tracks and keeps a visible retry on return', async ({ page }) => {
  await openScanner(page, { mode: 'unmarked' });
  await expect(page.locator('#liveCaptureBtn')).toBeEnabled();
  await page.evaluate(() => window.__scannerCameraFixture.setHidden(true));
  expect(await page.evaluate(() => window.__scannerCameraFixture.trackStates())).toEqual(['ended']);
  await page.evaluate(() => window.__scannerCameraFixture.setHidden(false));
  await expect(page.locator('#liveScanner')).toBeVisible();
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'error');
  await expect(page.locator('#liveStatus')).toHaveText('Camera paused');
  await page.locator('#liveRetryBtn').click();
  await expect(page.locator('#liveVideo')).toBeVisible();
  await expect(page.locator('#liveCaptureBtn')).toBeEnabled();
  expect(await page.evaluate(() => window.__scannerCameraFixture.trackStates())).toEqual(['ended', 'live']);
});

for (const interruption of ['ended', 'muted', 'stalled']) {
  test(`a ${interruption} camera feed stays in the scanner with retry instead of a frozen preview`, async ({ page }) => {
    await openScanner(page, { mode: 'unmarked' });
    await expect(page.locator('#liveCaptureBtn')).toBeEnabled();
    await page.evaluate(interruption => {
      const fake = window.__scannerCameraFixture;
      const track = fake.streams.at(-1).getVideoTracks()[0];
      if (interruption === 'ended') {
        track.stop();
        // Native revocation fires ended; explicit MediaStreamTrack.stop() does not.
        track.dispatchEvent(new Event('ended'));
      } else if (interruption === 'muted') {
        Object.defineProperty(track, 'muted', { configurable: true, value: true });
        track.dispatchEvent(new Event('mute'));
      } else {
        fake.stopFrames.forEach(stop => stop());
      }
    }, interruption);
    await expect(page.locator('#liveCaptureBtn')).toBeDisabled();
    await expect(page.locator('#liveVideo')).toBeHidden();
    await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'error');
    await expect(page.locator('#liveStatus')).toHaveText('Camera interrupted');
    await expect(page.locator('#liveScanner')).toBeVisible();
    expect(await page.evaluate(() => window.__scannerCameraFixture.trackStates())).toEqual(['ended']);
    await page.locator('#liveRetryBtn').click();
    await expect(page.locator('#liveCaptureBtn')).toBeEnabled();
    await expect(page.locator('#liveVideo')).toBeVisible();
  });
}

test('a short camera mute resumes live frames without capturing a stale frame', async ({ page }) => {
  await openScanner(page, { mode: 'unmarked' });
  await expect(page.locator('#liveCaptureBtn')).toBeEnabled();
  await page.evaluate(() => {
    const track = window.__scannerCameraFixture.streams.at(-1).getVideoTracks()[0];
    Object.defineProperty(track, 'muted', { configurable: true, value: true });
  });
  await expect(page.locator('#liveCaptureBtn')).toBeDisabled();
  await expect(page.locator('#liveVideo')).toBeHidden();
  await page.evaluate(() => {
    const track = window.__scannerCameraFixture.streams.at(-1).getVideoTracks()[0];
    Object.defineProperty(track, 'muted', { configurable: true, value: false });
  });
  await expect(page.locator('#liveVideo')).toBeVisible();
  await expect(page.locator('#liveCaptureBtn')).toBeEnabled();
  expect(await page.evaluate(() => window.__scannerCameraFixture.requests.length)).toBe(1);
  await expect(page.locator('#liveReview')).toBeHidden();
});

test('older WebViews can capture three live frames even when image encoding is slow', async ({ page }) => {
  test.setTimeout(45_000);
  await openScanner(page, { mode: 'unmarked', legacyVideoFrames: true, slowEncoding: true });
  await expect(page.locator('#liveVideo')).toBeVisible();
  await page.locator('#liveCaptureBtn').click();
  await expect(page.locator('#liveAddBtn')).toBeEnabled({ timeout: 30_000 });
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review');
  await expect(page.locator('#liveAlternatives button')).toHaveCount(3);
  expect(await page.evaluate(() => window.__scannerCameraFixture.trackStates())).toEqual(['ended']);
});
