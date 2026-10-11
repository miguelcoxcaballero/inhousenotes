import { expect, test } from '@playwright/test';
import fs from 'node:fs';

const fixtures = fs.readFileSync(new URL('./fixtures/scanner-synthetic.js', import.meta.url), 'utf8');

async function camera(page, mode) {
  await page.addInitScript({ content: fixtures });
  await page.addInitScript(mode => {
    const fixture = window.liveResilience = { constraints: [], observed: 0, missed: 0, retained: 0, focus: [] };
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
    const original = ScannerPro.LiveDetection.detect;
    ScannerPro.LiveDetection.detect = (...args) => {
      const detection = original(...args);
      const fixture = window.liveResilience;
      const state = document.getElementById('liveScanner').dataset.state;
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
