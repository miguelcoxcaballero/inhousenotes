import { expect, test } from '@playwright/test';
import fs from 'node:fs';

const fixtures = fs.readFileSync(new URL('./fixtures/scanner-synthetic.js', import.meta.url), 'utf8');

test('a sharp page moving in the live feed captures without waiting for stationary corners', async ({ page }) => {
  await page.addInitScript({ content: fixtures });
  await page.addInitScript(() => {
    window.movingCapture = { observations: [], encodedAt: [] };
    navigator.mediaDevices.getUserMedia = async () => {
      const source = ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
      const canvas = document.createElement('canvas');
      canvas.width = source.width; canvas.height = source.height;
      const context = canvas.getContext('2d');
      let frame = 0;
      const paint = () => {
        context.fillStyle = '#957151'; context.fillRect(0, 0, canvas.width, canvas.height);
        // Alternate a 5.6% displacement, exceeding the old 2.5% hold reset.
        context.drawImage(source, ++frame % 2 ? -25 : 25, 0);
      };
      paint();
      const stream = canvas.captureStream(20), track = stream.getVideoTracks()[0];
      const interval = setInterval(paint, 50), stop = track.stop.bind(track);
      track.stop = () => { clearInterval(interval); stop(); };
      track.applyConstraints = async () => {};
      return stream;
    };
    const toBlob = HTMLCanvasElement.prototype.toBlob;
    HTMLCanvasElement.prototype.toBlob = function (callback, ...args) {
      if (args[0] === 'image/jpeg' && document.getElementById('liveScanner')?.dataset.state === 'capturing') {
        movingCapture.encodedAt.push(performance.now());
      }
      return toBlob.call(this, callback, ...args);
    };
    addEventListener('scanner-live-detection', event => {
      const detection = event.detail;
      movingCapture.observations.push({ at: performance.now(), detection });
      if (detection?.source === 'yellow' && detection.confidence >= .78
          && Math.max(detection.sharpness / 32, detection.edgeSharpness / 160) >= 1
          && detection.brightness >= 60 && detection.brightness <= 252 && detection.coverage >= .16
          && detection.quad.every(p => p.x >= .008 && p.y >= .008 && p.x <= .992 && p.y <= .992)) {
        movingCapture.qualifiedAt ||= performance.now();
      }
    });
  });
  await page.goto('/scanner/index.html?embed=1&session=moving-capture', { waitUntil: 'domcontentloaded' });
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review', { timeout: 15000 });
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
  const timing = await page.evaluate(() => ({
    delay: movingCapture.encodedAt[0] - movingCapture.qualifiedAt,
    observationsBeforeCapture: movingCapture.observations.length,
    photos: movingCapture.encodedAt.length
  }));
  expect(timing.delay).toBeLessThan(150);
  expect(timing.observationsBeforeCapture).toBeLessThan(5);
  expect(timing.photos).toBeGreaterThanOrEqual(1);
  await expect(page.locator('#liveReviewStatus')).not.toContainText('Original photo ready');
  await test.info().attach('moving-capture-timing', { body: JSON.stringify(timing), contentType: 'application/json' });
});
