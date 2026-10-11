import { expect, test } from '@playwright/test';
import fs from 'node:fs';

const fixtureSource = fs.readFileSync(new URL('./fixtures/scanner-synthetic.js', import.meta.url), 'utf8');

function installCaptureCamera(options = {}) {
  window.__captureRegression = { streams: [], encodes: 0, invalidOutput: false };
  const originalEncode = HTMLCanvasElement.prototype.toBlob;
  HTMLCanvasElement.prototype.toBlob = function (callback, ...args) {
    const outputPage = Math.abs(this.width / this.height - 21 / 29.7) < .003;
    originalEncode.call(this, blob => {
      window.__captureRegression.encodes++;
      if (options.invalidFirstOutput && outputPage && !window.__captureRegression.invalidOutput) {
        window.__captureRegression.invalidOutput = true;
        callback(new Blob(['invalid encoded image'], { type: 'image/jpeg' }));
      } else callback(blob);
    }, ...args);
  };
  Object.defineProperty(navigator.mediaDevices, 'getUserMedia', {
    configurable: true,
    value: async () => {
      if (options.denied) throw new DOMException('Permission denied by fixture', 'NotAllowedError');
      const canvas = ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
      const stream = canvas.captureStream(20);
      const track = stream.getVideoTracks()[0];
      const context = canvas.getContext('2d');
      const refresh = setInterval(() => {
        context.fillStyle = '#8a674a';
        context.fillRect(0, 0, 2, 2);
      }, 50);
      const stop = track.stop.bind(track);
      track.stop = () => { clearInterval(refresh); stop(); };
      track.applyConstraints = async () => {};
      window.__captureRegression.streams.push(stream);
      return stream;
    }
  });
}

async function installFixture(page, options = {}) {
  await page.addInitScript({ content: `${fixtureSource}\n;(${installCaptureCamera.toString()})(${JSON.stringify(options)});` });
}

async function holdFirstWarp(frame) {
  await frame.evaluate(() => {
    const original = ScannerPro.Lightweight.warp;
    const probe = window.__captureRegression;
    probe.warpCalls = 0;
    probe.pendingWarp = false;
    probe.completedWarps = [];
    ScannerPro.Lightweight.warp = async function (...args) {
      const call = ++probe.warpCalls;
      try {
        if (call === 1) {
          probe.pendingWarp = true;
          await new Promise(resolve => { probe.releaseWarp = resolve; });
        }
        return await original.apply(this, args);
      } finally { probe.completedWarps.push(call); }
    };
  });
}

async function expectDecodedPreview(frame) {
  await expect(frame.locator('#livePreview')).toBeVisible();
  await expect.poll(() => frame.locator('#livePreview').evaluate(image =>
    image.complete && image.naturalWidth > 100 && image.naturalHeight > 100)).toBe(true);
}

async function openStandalone(page, options = {}) {
  await installFixture(page, options);
  await page.goto('/scanner/index.html?embed=1&session=capture-regression', { waitUntil: 'domcontentloaded' });
  await expect(page.locator('#liveScanner')).toBeVisible();
}

test('capture shows a decoded photo while correction is pending, then inserts only the decoded result', async ({ page }) => {
  test.setTimeout(60_000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await installFixture(page);
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(async () => {
    await __IHN_TEST_API__.ready();
    await __IHN_TEST_API__.resetLocalDocument(1, 'Captured preview regression');
    await __IHN_TEST_API__.showEditorForTest();
  });
  await page.locator('#btn-edit-pages').click();
  await page.locator('#btn-scan-page').click();
  const iframe = page.frameLocator('#scanner-editor-frame');
  const frame = await page.locator('#scanner-editor-frame').elementHandle().then(handle => handle.contentFrame());
  await frame.waitForFunction(() => !!window.ScannerPro?.Lightweight);
  await holdFirstWarp(frame);
  await frame.waitForFunction(() => window.__captureRegression.pendingWarp, null, { timeout: 20_000 });
  await expect(iframe.locator('#liveScanner')).toHaveAttribute('data-state', 'processing');
  await expectDecodedPreview(frame);
  await expect(iframe.locator('#liveAddBtn')).toBeDisabled();
  expect(await page.evaluate(() => __IHN_TEST_API__.snapshot().pages.length)).toBe(1);
  expect(await frame.evaluate(() => __captureRegression.streams.flatMap(stream => stream.getTracks().map(track => track.readyState))))
    .toEqual(['ended']);
  await frame.evaluate(() => __captureRegression.releaseWarp());
  await expect(iframe.locator('#liveScanner')).toHaveAttribute('data-state', 'review', { timeout: 20_000 });
  await expectDecodedPreview(frame);
  await expect(iframe.locator('#liveAddBtn')).toBeEnabled();
  expect(await iframe.locator('#livePreview').evaluate(image => image.naturalWidth / image.naturalHeight)).toBeCloseTo(21 / 29.7, 2);
  await iframe.locator('#liveAddBtn').click();
  await expect(page.locator('#scanner-editor-overlay')).not.toHaveClass(/visible/);
  const persisted = await page.evaluate(async () => {
    const saved = (await __IHN_TEST_API__.checkpointBeforeLeaving()).snapshot;
    const source = saved.pages[1].backgroundImage;
    const image = new Image(); image.src = source; await image.decode();
    const canvas = document.createElement('canvas'); canvas.width = 20; canvas.height = 20;
    const context = canvas.getContext('2d'); context.drawImage(image, 0, 0, 20, 20);
    const rgba = context.getImageData(0, 0, 20, 20).data;
    const brightness = Array.from(rgba).filter((_, index) => index % 4 < 3).reduce((total, value) => total + value, 0) / 1200;
    return { pages: saved.pages.length, width: image.naturalWidth, height: image.naturalHeight, brightness, dirtyPages: saved.dirtyPages };
  });
  expect(persisted.pages).toBe(2);
  expect(persisted.width / persisted.height).toBeCloseTo(21 / 29.7, 2);
  expect(persisted.brightness).toBeGreaterThan(170);
  expect(persisted.dirtyPages).toEqual([]);
  expect(errors).toEqual([]);
});

test('an undecodable processed image retains a valid original and a fresh capture recovers', async ({ page }) => {
  test.setTimeout(60_000);
  await openStandalone(page, { invalidFirstOutput: true });
  await page.waitForFunction(() => __captureRegression.invalidOutput, null, { timeout: 25_000 });
  await expect(page.locator('#liveScanner')).not.toHaveAttribute('data-state', 'processing', { timeout: 10_000 });
  await expectDecodedPreview(page);
  await expect(page.locator('#liveReviewStatus')).toContainText(/original|could not|couldn't|decode|open|failed|try again|unavailable/i);
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
  await page.locator('#liveRetakeBtn').click();
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review', { timeout: 25_000 });
  await expectDecodedPreview(page);
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
});

test('invalid imported image leaves processing and a subsequent valid photo can be reviewed', async ({ page }) => {
  test.setTimeout(35_000);
  await openStandalone(page, { denied: true });
  await page.locator('#liveFileInput').setInputFiles({ name: 'damaged.jpg', mimeType: 'image/jpeg', buffer: Buffer.from('not a JPEG') });
  await expect(page.locator('#liveScanner')).not.toHaveAttribute('data-state', 'processing', { timeout: 10_000 });
  await expect(page.locator('#liveAddBtn')).toBeDisabled();
  await expect(page.locator('#liveReviewStatus')).toContainText(/could not|couldn't|decode|open|failed|try again/i);
  await expect(page.locator('#livePreview')).toBeHidden();
  const valid = await page.evaluate(() => ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source.toDataURL('image/jpeg', .9).split(',')[1]);
  await page.locator('#liveFileInput').setInputFiles({ name: 'valid.jpg', mimeType: 'image/jpeg', buffer: Buffer.from(valid, 'base64') });
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review', { timeout: 20_000 });
  await expectDecodedPreview(page);
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
});

test('retaking during correction cancels the old result and keeps the new preview', async ({ page }) => {
  test.setTimeout(50_000);
  await openStandalone(page);
  await holdFirstWarp(page);
  await page.waitForFunction(() => __captureRegression.pendingWarp, null, { timeout: 20_000 });
  await expectDecodedPreview(page);
  await page.locator('#liveRetakeBtn').click();
  await expect(page.locator('#liveCameraView')).toBeVisible();
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review', { timeout: 20_000 });
  await expectDecodedPreview(page);
  const before = await page.locator('#livePreview').getAttribute('src');
  await page.evaluate(() => __captureRegression.releaseWarp());
  await expect.poll(() => page.evaluate(() => __captureRegression.completedWarps)).toContain(1);
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review');
  await expect(page.locator('#livePreview')).toHaveAttribute('src', before);
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
  expect(await page.evaluate(() => __captureRegression.streams.flatMap(stream => stream.getTracks().map(track => track.readyState))))
    .toEqual(['ended', 'ended']);
});
