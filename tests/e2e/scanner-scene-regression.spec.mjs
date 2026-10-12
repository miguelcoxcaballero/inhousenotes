import { expect, test } from '@playwright/test';
import fs from 'node:fs';

const physicalFixture = fs.readFileSync(new URL('./fixtures/scanner-physical.js', import.meta.url), 'utf8');
const sceneFixture = fs.readFileSync(new URL('./fixtures/scanner-scene.js', import.meta.url), 'utf8');

for (const name of ['s-bend-on-plain-table', 's-bend-on-pale-book', 'warm-s-bend-on-pale-book']) {
  test(`complete processing keeps only the marked sheet: ${name}`, async ({ page }, testInfo) => {
    test.setTimeout(60_000);
    await page.goto('/scanner/index.html?embed=1&session=scene-regression', { waitUntil: 'domcontentloaded' });
    await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-physical.js' });
    await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-scene.js' });
    const result = await page.evaluate(async caseName => {
      const fixture = ScannerSceneFixtures;
      const rendered = fixture.render(fixture.CASES.find(candidate => candidate.name === caseName));
      // Use the actual asynchronous capture detector. Ground-truth corners and
      // masks only measure its output; neither is passed into detection/warp.
      const detection = await ScannerPro.Lightweight.detectPageAsync(rendered.source);
      const options = { maxDimension: 1200 };
      const warped = await ScannerPro.Lightweight.warp(rendered.source, detection.pageQuad, detection.frame, options);
      const warpedMask = await ScannerPro.Lightweight.warp(rendered.mask, detection.pageQuad, detection.frame, options);
      const before = ScannerPhysicalFixtures.lineErrors(warped);
      const foreignInteriorRatio = fixture.foreignInteriorRatio(warpedMask);
      await ScannerPro.Lightweight.correctColors(warped, { useStencil: true, preciseStencil: true });
      const corrected = ScannerPhysicalFixtures.lineErrors(warped);
      const diagonal = Math.hypot(rendered.source.width, rendered.source.height);
      const cornerMaximumRatio = Math.max(...detection.pageQuad.map((point, index) => (
        Math.hypot(point.x - rendered.pageQuad[index].x, point.y - rendered.pageQuad[index].y) / diagonal
      )));
      return {
        method: detection.method, confidence: detection.confidence,
        coordinateSource: detection.markerCalibration?.coordinateSource,
        sideSupports: detection.frame?.sideSupports,
        cornerMaximumRatio, foreignInteriorRatio, before, corrected,
        source: rendered.source.toDataURL('image/png'), result: warped.toDataURL('image/png')
      };
    }, name);
    for (const key of ['source', 'result']) {
      await testInfo.attach(`${name}-${key}`, {
        body: Buffer.from(result[key].split(',')[1], 'base64'), contentType: 'image/png'
      });
      delete result[key];
    }
    console.log(name, JSON.stringify(result));
    expect(result.cornerMaximumRatio, 'detected sheet corners stay on the independent physical paper').toBeLessThan(.03);
    expect(result.foreignInteriorRatio, 'book, logo and table stay outside the scan').toBeLessThan(.005);
    expect(result.before.maximum, 'warping preserves all interior pen lines').toBeLessThan(.012);
    expect(result.before.mean, 'warping preserves interior proportions').toBeLessThan(.004);
    expect(result.corrected.maximum, 'final cleanup keeps interior pen lines').toBeLessThan(.012);
    expect(result.corrected.mean, 'final cleanup keeps interior proportions').toBeLessThan(.004);
  });
}

function installSceneCamera() {
  window.__sceneDetections = [];
  window.addEventListener('scanner-live-detection', event => {
    if (event.detail) window.__sceneDetections.push(event.detail);
  });
  Object.defineProperty(navigator.mediaDevices, 'getUserMedia', {
    configurable: true,
    value: async () => {
      const canvas = ScannerSceneFixtures.render(ScannerSceneFixtures.CASES[1]).source;
      const stream = canvas.captureStream(20);
      const track = stream.getVideoTracks()[0];
      const context = canvas.getContext('2d');
      const refresh = setInterval(() => {
        context.fillStyle = '#a47b51'; context.fillRect(0, 0, 2, 2);
      }, 50);
      const stop = track.stop.bind(track);
      track.stop = () => { clearInterval(refresh); stop(); };
      track.applyConstraints = async () => {};
      return stream;
    }
  });
}

test('live camera review preserves the writing of the sheet lying on a pale book', async ({ page }, testInfo) => {
  test.setTimeout(40_000);
  await page.addInitScript({ content: `${physicalFixture}\n${sceneFixture}\n;(${installSceneCamera.toString()})();` });
  await page.goto('/scanner/index.html?embed=1&session=scene-live-regression', { waitUntil: 'domcontentloaded' });
  await expect(page.locator('#liveOverlay')).toHaveAttribute('data-detected', 'true', { timeout: 10_000 });
  // Capture the actual video image and its actual detector observation. This
  // catches a controller that draws the correct outline but discards that
  // observation before constructing a different, incorrect final surface.
  await page.evaluate(() => {
    const button = document.getElementById('liveCaptureBtn');
    if (!button.disabled) button.click();
  });
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review', { timeout: 25_000 });
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
  const result = await page.evaluate(async () => {
    const image = document.getElementById('livePreview');
    await image.decode();
    const canvas = document.createElement('canvas');
    canvas.width = image.naturalWidth; canvas.height = image.naturalHeight;
    canvas.getContext('2d').drawImage(image, 0, 0);
    return {
      aspect: canvas.width / canvas.height,
      lines: ScannerPhysicalFixtures.lineErrors(canvas),
      detection: (() => {
        const d = __sceneDetections.at(-1);
        return { source: d?.source, confidence: d?.confidence, quad: d?.quad, sideSupports: d?.frameEdgeSupport };
      })(), png: canvas.toDataURL('image/png')
    };
  });
  await testInfo.attach('live-pale-book-result', {
    body: Buffer.from(result.png.split(',')[1], 'base64'), contentType: 'image/png'
  });
  delete result.png;
  console.log('live-pale-book', JSON.stringify(result));
  expect(result.aspect).toBeCloseTo(21 / 29.7, 2);
  expect(result.lines.maximum, 'actual review keeps all interior pen lines').toBeLessThan(.012);
  expect(result.lines.mean, 'actual review keeps interior proportions').toBeLessThan(.004);
});


test('a faint scene is either reconstructed correctly or explicitly kept as the original', async ({ page }, testInfo) => {
  test.setTimeout(45_000);
  await page.addInitScript(() => {
    Object.defineProperty(navigator.mediaDevices, 'getUserMedia', {
      value: async () => { throw new DOMException('Fixture uses photo import', 'NotAllowedError'); }
    });
  });
  await page.goto('/scanner/index.html?embed=1&session=scene-faint-regression', { waitUntil: 'domcontentloaded' });
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-physical.js' });
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-scene.js' });
  const input = await page.evaluate(() => {
    const definition = ScannerSceneFixtures.CASES.find(candidate => candidate.name === 'faint-soft-s-bend-on-pale-book');
    window.__faintScene = ScannerSceneFixtures.render(definition);
    const process = ScannerPro.Lightweight.processPageAsync;
    ScannerPro.Lightweight.processPageAsync = async function (...args) {
      const result = await process.apply(this, args);
      window.__faintDetection = result.detection;
      return result;
    };
    return __faintScene.source.toDataURL('image/png').split(',')[1];
  });
  await page.locator('#liveFileInput').setInputFiles({
    name: 'synthetic-faint-scene.png', mimeType: 'image/png', buffer: Buffer.from(input, 'base64')
  });
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review', { timeout: 25_000 });
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
  const result = await page.evaluate(async () => {
    const image = document.getElementById('livePreview');
    await image.decode();
    const canvas = document.createElement('canvas');
    canvas.width = image.naturalWidth; canvas.height = image.naturalHeight;
    canvas.getContext('2d').drawImage(image, 0, 0);
    const detection = __faintDetection;
    const rejected = detection.quality?.valid === false;
    let foreignInteriorRatio = null, unchangedOriginal = false;
    if (rejected) {
      const response = await fetch(image.src);
      const actual = new Uint8Array(await response.arrayBuffer());
      const expected = new Uint8Array(await (await fetch(__faintScene.source.toDataURL('image/png'))).arrayBuffer());
      unchangedOriginal = actual.length === expected.length && actual.every((value, index) => value === expected[index]);
    } else {
      const mask = await ScannerPro.Lightweight.warp(__faintScene.mask, detection.pageQuad, detection.frame, { maxDimension: 1200 });
      foreignInteriorRatio = ScannerSceneFixtures.foreignInteriorRatio(mask);
    }
    return {
      rejected, quality: detection.quality, unchangedOriginal, foreignInteriorRatio,
      status: document.getElementById('liveReviewStatus').textContent,
      aspect: canvas.width / canvas.height,
      lines: rejected ? null : ScannerPhysicalFixtures.lineErrors(canvas),
      png: canvas.toDataURL('image/png')
    };
  });
  await testInfo.attach('faint-scene-review', {
    body: Buffer.from(result.png.split(',')[1], 'base64'), contentType: 'image/png'
  });
  delete result.png;
  console.log('faint-scene-review', JSON.stringify(result));
  if (result.rejected) {
    expect(result.status).toMatch(/original/i);
    expect(result.unchangedOriginal, 'rejected geometry keeps the exact imported photograph').toBe(true);
  } else {
    expect(result.aspect).toBeCloseTo(21 / 29.7, 2);
    expect(result.foreignInteriorRatio, 'accepted output excludes surrounding book and table').toBeLessThan(.005);
    expect(result.lines.maximum).toBeLessThan(.012);
    expect(result.lines.mean).toBeLessThan(.004);
  }
});
