import { expect, test } from '@playwright/test';
import fs from 'node:fs';

const fixture = fs.readFileSync(new URL('./fixtures/scanner-synthetic.js', import.meta.url), 'utf8');

async function captureThree(page) {
  await page.addInitScript({ content: fixture });
  await page.addInitScript(() => {
    const probe = window.parallelReview = { calls: [], activeUrls: new Set() };
    const create = URL.createObjectURL.bind(URL), revoke = URL.revokeObjectURL.bind(URL);
    URL.createObjectURL = value => { const url = create(value); probe.activeUrls.add(url); return url; };
    URL.revokeObjectURL = url => { probe.activeUrls.delete(url); revoke(url); };
    navigator.mediaDevices.getUserMedia = async () => {
      const canvas = ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
      const context = canvas.getContext('2d'), stream = canvas.captureStream(20);
      const interval = setInterval(() => { context.fillStyle = '#987654'; context.fillRect(0, 0, 2, 2); }, 50);
      const track = stream.getVideoTracks()[0], stop = track.stop.bind(track);
      track.stop = () => { clearInterval(interval); stop(); };
      return stream;
    };
  });
  await page.goto('/scanner/index.html', { waitUntil: 'domcontentloaded' });
  await page.evaluate(async () => {
    const probe = parallelReview;
    const output = async color => {
      const canvas = document.createElement('canvas'); canvas.width = 300; canvas.height = 424;
      const context = canvas.getContext('2d'); context.fillStyle = color; context.fillRect(0, 0, 300, 424);
      return new Promise(resolve => canvas.toBlob(resolve, 'image/jpeg', .94));
    };
    probe.red = await output('#ed3027'); probe.green = await output('#28dc51');
    ScannerPro.Lightweight.processPageAsync = (blob, options) => new Promise((resolve, reject) => {
      const index = probe.calls.length;
      probe.calls.push({ blob, options, resolve, reject, index });
    });
    probe.finish = (index, mode = 'corrected') => {
      const call = probe.calls[index];
      call.resolve(mode === 'original' ? {
        blob: call.blob, corrected: false, detection: { quality: { valid: false } }
      } : {
        blob: mode === 'invalid' ? new Blob(['broken JPEG'], { type: 'image/jpeg' })
          : index === 2 ? probe.green : probe.red, corrected: true,
        // Dimensions are deliberately wrong: insertion uses decoded bytes.
        width: 1, height: 1,
        detection: { confidence: .9, quality: { valid: true }, frame: {
          metricSurface: { diagnostics: { pitch: 10, rms: index === 2 ? .1 : 1.3,
            p99: index === 2 ? .5 : 3.8, borderP95: index === 2 ? 1 : 10 } }
        } }
      });
    };
    // Keep all captures manual so this tests the batch independently of the
    // autofocus and five-frame auto-capture stability policy.
    const detect = ScannerPro.LiveDetection.detect;
    ScannerPro.LiveDetection.detect = (...args) => {
      const result = detect(...args);
      return result ? { ...result, confidence: .7 } : result;
    };
    await ScannerPro.LiveCamera.open();
  });
  await expect(page.locator('#liveCaptureBtn')).toBeEnabled();
  await page.locator('#liveCaptureBtn').click();
  // No job has resolved yet: reaching three proves concurrent dispatch.
  await expect.poll(() => page.evaluate(() => parallelReview.calls.length)).toBe(3);
  await expect(page.locator('#livePreview')).toBeVisible();
  await expect(page.locator('#liveAddBtn')).toBeDisabled();
}

async function previewColor(page) {
  return page.locator('#livePreview').evaluate(image => {
    const canvas = document.createElement('canvas'); canvas.width = canvas.height = 1;
    const context = canvas.getContext('2d'); context.drawImage(image, 0, 0, 1, 1);
    return [...context.getImageData(0, 0, 1, 1).data].slice(0, 3);
  });
}

test('all three photos run concurrently and the best corrected result wins regardless of completion order', async ({ page }) => {
  await captureThree(page);
  await page.evaluate(() => { parallelReview.finish(0, 'invalid'); parallelReview.finish(2); });
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'processing');
  await expect(page.locator('#liveAddBtn')).toBeDisabled();
  await page.evaluate(() => parallelReview.finish(1));
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review');
  await expect(page.locator('#liveAlternatives button').nth(2)).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
  await expect.poll(async () => (await previewColor(page))[1]).toBeGreaterThan(200);
  expect(await page.locator('#livePreview').evaluate(image => [image.naturalWidth, image.naturalHeight])).toEqual([300, 424]);

  await page.locator('#liveAlternatives button').nth(1).click();
  await expect.poll(async () => (await previewColor(page))[0]).toBeGreaterThan(200);
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
  await page.locator('#liveAlternatives button').nth(2).click();
  await expect.poll(async () => (await previewColor(page))[1]).toBeGreaterThan(200);
  expect(await page.evaluate(() => parallelReview.calls.length)).toBe(3);
  expect(await page.evaluate(() => parallelReview.calls.every(call => !call.options.signal.aborted))).toBe(true);
});

test('choosing a thumbnail keeps that choice while other corrections finish and uses their cached outputs', async ({ page }) => {
  await captureThree(page);
  await page.locator('#liveAlternatives button').nth(1).click();
  await page.evaluate(() => parallelReview.finish(1));
  await expect.poll(async () => (await previewColor(page))[0]).toBeGreaterThan(200);
  await expect(page.locator('#liveAddBtn')).toBeDisabled();
  await page.evaluate(() => { parallelReview.finish(2); parallelReview.finish(0, 'original'); });
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review');
  await expect(page.locator('#liveAlternatives button').nth(1)).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
  expect((await previewColor(page))[0]).toBeGreaterThan(200);
  await page.locator('#liveAlternatives button').nth(2).click();
  await expect.poll(async () => (await previewColor(page))[1]).toBeGreaterThan(200);
  expect(await page.evaluate(() => parallelReview.calls.length)).toBe(3);
});

test('closing aborts every pending photo and ignores late completions without retaining image URLs', async ({ page }) => {
  await captureThree(page);
  await page.locator('#liveCloseBtn').click();
  await expect(page.locator('#liveScanner')).toBeHidden();
  expect(await page.evaluate(() => parallelReview.calls.every(call => call.options.signal.aborted))).toBe(true);
  expect(await page.evaluate(() => parallelReview.activeUrls.size)).toBe(0);
  await page.evaluate(() => { parallelReview.finish(0); parallelReview.finish(1); parallelReview.finish(2); });
  await page.waitForTimeout(100);
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'closed');
  await expect(page.locator('#livePreview')).not.toHaveAttribute('src');
  await expect(page.locator('#liveAddBtn')).toBeDisabled();
  expect(await page.evaluate(() => parallelReview.activeUrls.size)).toBe(0);
});
