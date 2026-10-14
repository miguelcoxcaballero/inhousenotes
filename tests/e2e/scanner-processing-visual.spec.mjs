import { expect, test } from '@playwright/test';
import fs from 'node:fs';

const fixture = fs.readFileSync(new URL('./fixtures/scanner-synthetic.js', import.meta.url), 'utf8');

test.use({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2 });

async function prepare(page, camera = false) {
  await page.addInitScript({ content: fixture });
  await page.addInitScript(({ camera }) => {
    const probe = window.processingVisual = { calls: [], activeUrls: new Set() };
    const create = URL.createObjectURL.bind(URL), revoke = URL.revokeObjectURL.bind(URL);
    URL.createObjectURL = value => { const url = create(value); probe.activeUrls.add(url); return url; };
    URL.revokeObjectURL = url => { probe.activeUrls.delete(url); revoke(url); };
    navigator.mediaDevices.getUserMedia = async () => {
      if (!camera) throw new DOMException('Use the photo fixture', 'NotAllowedError');
      const canvas = ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
      const context = canvas.getContext('2d'), stream = canvas.captureStream(20);
      const interval = setInterval(() => { context.fillStyle = '#765432'; context.fillRect(0, 0, 2, 2); }, 50);
      const track = stream.getVideoTracks()[0], stop = track.stop.bind(track);
      track.stop = () => { clearInterval(interval); stop(); };
      return stream;
    };
  }, { camera });
  await page.goto('/scanner/index.html', { waitUntil: 'domcontentloaded' });
  const photo = await page.evaluate(async () => {
    const probe = processingVisual;
    const canvas = document.createElement('canvas'); canvas.width = 800; canvas.height = 1000;
    const context = canvas.getContext('2d'); context.fillStyle = '#eee'; context.fillRect(0, 0, 800, 1000);
    const input = canvas.toDataURL('image/png');
    canvas.width = 420; canvas.height = 594;
    ScannerPro.ScanTemplate.render(context,420,594,{background:true});
    probe.warp = await new Promise(resolve => canvas.toBlob(resolve, 'image/png'));
    probe.geometry = {
      width: 800, height: 1000, space: 'source', pitch: 100,
      points: [
        { x: 100, y: 200, u: .1, v: .1, row: 0, col: 0 },
        { x: 200, y: 200, u: .2, v: .1, row: 0, col: 1 },
        { x: 600, y: 200, u: .6, v: .1, row: 0, col: 5 },
        { x: 100, y: 400, u: .1, v: .3, row: 2, col: 0 },
        { x: 200, y: 430, u: .2, v: .3, row: 2, col: 1 },
        { x: 600, y: 500, u: .6, v: .5, row: 4, col: 5 }
      ],
      rows: [{ index: 0, points: [0, 1] }, { index: 0, points: [2] }, { index: 2, points: [3, 4] }],
      columns: [{ index: 0, points: [0, 3] }, { index: 1, points: [1, 4] }]
    };
    ScannerPro.Lightweight.processPageAsync = (blob, options) => new Promise(resolve => {
      probe.calls.push({ blob, options, resolve });
    });
    probe.progress = (index, stage, detail = probe.geometry) => probe.calls[index].options.onProgress(stage, detail);
    probe.finish = index => probe.calls[index].resolve({ blob: probe.warp, corrected: true,
      dotAlignment: { status: 'verified' },
      detection: { quality: { valid: true }, confidence: .9 } });
    const detect = ScannerPro.LiveDetection.detect;
    ScannerPro.LiveDetection.detect = (...args) => {
      const detection = detect(...args);
      return detection ? { ...detection, confidence: .7 } : detection;
    };
    await ScannerPro.LiveCamera.open();
    return input;
  });
  if (camera) {
    await expect(page.locator('#liveCaptureBtn')).toBeEnabled();
    await page.locator('#liveCaptureBtn').click();
  } else {
    await page.locator('#liveFileInput').setInputFiles({ name: 'bullet-page.png', mimeType: 'image/png',
      buffer: Buffer.from(photo.split(',')[1], 'base64') });
  }
  await expect.poll(() => page.evaluate(() => processingVisual.calls.length)).toBe(camera ? 3 : 1);
  await expect(page.locator('#livePreview')).toBeVisible();
}

async function overlaySamples(page, geometry, coordinates) {
  return page.evaluate(({ geometry, coordinates }) => {
    const canvas = document.getElementById('liveProcessingOverlay');
    const image = document.getElementById('livePreview');
    const imageRect = image.getBoundingClientRect(), canvasRect = canvas.getBoundingClientRect();
    const scale = Math.min(imageRect.width / image.naturalWidth, imageRect.height / image.naturalHeight);
    const width = image.naturalWidth * scale, height = image.naturalHeight * scale;
    const x0 = imageRect.left - canvasRect.left + (imageRect.width - width) / 2;
    const y0 = imageRect.top - canvasRect.top + (imageRect.height - height) / 2;
    const ratio = canvas.width / canvasRect.width, context = canvas.getContext('2d');
    return coordinates.map(([x, y]) => {
      const px = Math.round((x0 + x / geometry.width * width) * ratio);
      const py = Math.round((y0 + y / geometry.height * height) * ratio);
      const pixels = context.getImageData(px - 1, py - 1, 3, 3).data;
      return Math.max(...Array.from({ length: 9 }, (_, index) => pixels[index * 4 + 3]));
    });
  }, { geometry, coordinates });
}

test('measured bullet points and curved runs align with the contained photo without filling missing dots', async ({ page }) => {
  await prepare(page);
  await page.evaluate(() => processingVisual.progress(0, 'dots'));
  const overlay = page.locator('#liveProcessingOverlay');
  await expect(overlay).toBeVisible();
  await expect(overlay).toHaveAttribute('data-point-count', '6');
  await expect(overlay).toHaveAttribute('data-mode', 'observed');
  await expect(page.locator('#liveReviewStatus')).toHaveText('Finding dots…');
  await expect(page.locator('[data-step="dots"]')).toHaveAttribute('aria-current', 'step');
  // The third location lies in a long missing run. The fourth is an ideal
  // target that differs from the measured dot and must not replace it.
  let alpha = await overlaySamples(page, { width: 800, height: 1000 }, [[100, 200], [200, 430], [400, 200], [80, 100]]);
  expect(alpha[0]).toBeGreaterThan(230); expect(alpha[1]).toBeGreaterThan(230);
  expect(alpha[2]).toBe(0); expect(alpha[3]).toBe(0);
  await page.evaluate(() => processingVisual.progress(0, 'surface', {}));
  await expect(page.locator('#liveReviewStatus')).toHaveText('Straightening…');
  await expect(page.locator('[data-step="surface"]')).toHaveAttribute('aria-current', 'step');
  // Rotation/resize changes letterboxing, not the measured source positions.
  await page.setViewportSize({ width: 844, height: 390 });
  await expect.poll(async () => (await overlaySamples(page, { width: 800, height: 1000 }, [[100, 200]]))[0]).toBeGreaterThan(230);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('warp progress displays actual corrected observations and removes all guides from the finished scan', async ({ page }) => {
  await prepare(page);
  expect(await page.evaluate(() => processingVisual.calls[0].options.previewProgress)).toBe(true);
  await page.evaluate(() => {
    processingVisual.progress(0, 'dots');
    const detail = { ...processingVisual.geometry, width: 420, height: 594, space: 'corrected',
      previewBlob: processingVisual.warp, points: processingVisual.geometry.points.map((point, index) => ({
        ...point, x: 50 + index * 40, y: 160 + index * 12, targetX: point.u * 420, targetY: point.v * 594
      })) };
    processingVisual.progress(0, 'warp', detail);
    processingVisual.progress(0, 'colors', {});
  });
  const overlay = page.locator('#liveProcessingOverlay');
  await expect(overlay).toHaveAttribute('data-mode', 'corrected');
  await expect(overlay).toBeVisible();
  expect(await page.locator('#livePreview').evaluate(image => [image.naturalWidth, image.naturalHeight])).toEqual([420, 594]);
  expect((await overlaySamples(page, { width: 420, height: 594 }, [[50, 160]]))[0]).toBeGreaterThan(230);
  await page.evaluate(() => processingVisual.progress(0, 'template', {}));
  await expect(overlay).toBeVisible();
  await expect(page.locator('[data-step="template"]')).toHaveAttribute('aria-current', 'step');
  await page.evaluate(() => processingVisual.finish(0));
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review');
  await expect(overlay).toBeHidden();
  await expect(overlay).not.toHaveAttribute('data-point-count');
  await expect(page.locator('#liveProcessingSteps')).toBeHidden();
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
  const pixels = await page.locator('#livePreview').evaluate(image => {
    const canvas = document.createElement('canvas'); canvas.width = 420; canvas.height = 594;
    const context = canvas.getContext('2d'); context.drawImage(image, 0, 0);
    return [...context.getImageData(0, 0, 420, 594).data];
  });
  expect(pixels.some(value => value < 200)).toBe(true);
  const before = await page.evaluate(() => processingVisual.activeUrls.size);
  await page.evaluate(() => processingVisual.progress(0, 'warp', { previewBlob: processingVisual.warp }));
  expect(await page.evaluate(() => processingVisual.activeUrls.size)).toBe(before);
  await expect(overlay).toBeHidden();
});

test('each capture keeps its own observed dots and closed scans ignore late progress and preview blobs', async ({ page }) => {
  await prepare(page, true);
  await page.evaluate(() => {
    processingVisual.progress(0, 'dots');
    processingVisual.progress(1, 'dots', { ...processingVisual.geometry,
      points: processingVisual.geometry.points.slice(0, 2), rows: [{ index: 0, points: [0, 1] }], columns: [] });
  });
  await expect(page.locator('#liveProcessingOverlay')).toHaveAttribute('data-point-count', '6');
  await expect(page.locator('#liveReviewStatus')).toHaveText('Finding dots…');
  await page.locator('#liveAlternatives button').nth(1).click();
  await expect(page.locator('#liveProcessingOverlay')).toHaveAttribute('data-point-count', '2');
  await expect(page.locator('#liveReviewStatus')).toHaveText('Finding dots…');
  await page.evaluate(() => processingVisual.progress(0, 'template', {}));
  await expect(page.locator('#liveReviewStatus')).toHaveText('Finding dots…');
  await page.locator('#liveCloseBtn').click();
  await page.evaluate(() => {
    processingVisual.progress(1, 'warp', { previewBlob: processingVisual.warp });
    processingVisual.calls.forEach((_, index) => processingVisual.finish(index));
  });
  await expect(page.locator('#liveProcessingOverlay')).toBeHidden();
  await expect(page.locator('#livePreview')).not.toHaveAttribute('src');
  expect(await page.evaluate(() => processingVisual.activeUrls.size)).toBe(0);
});

test('the automatically selected frame shows its finished scan while the remaining candidates still run', async ({ page }) => {
  await prepare(page, true);
  await page.evaluate(() => processingVisual.progress(0, 'dots'));
  await expect(page.locator('#liveProcessingOverlay')).toBeVisible();
  await page.evaluate(() => processingVisual.finish(0));
  await expect.poll(() => page.locator('#livePreview').evaluate(image => image.naturalWidth)).toBe(420);
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review');
  await expect(page.locator('#liveAlternatives button').first()).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('#liveReviewStatus')).toHaveText('Ready');
  await expect(page.locator('#liveProcessingOverlay')).toBeHidden();
  await expect(page.locator('#liveProcessingSteps')).toBeHidden();
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
  await page.evaluate(() => {
    processingVisual.progress(1, 'dots');
    processingVisual.progress(0, 'warp', { previewBlob: processingVisual.warp });
  });
  await expect(page.locator('#liveProcessingOverlay')).toBeHidden();
  await expect(page.locator('#liveReviewStatus')).toHaveText('Ready');
  await page.evaluate(() => { processingVisual.finish(1); processingVisual.finish(2); });
  await expect(page.locator('#liveScanner')).toHaveAttribute('data-state', 'review');
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
});

test('a selected backup that fails correction cannot replace a successful scan',async({page})=>{
  await prepare(page,true);
  await page.locator('#liveAlternatives button').nth(1).click();
  await page.evaluate(()=>{
    processingVisual.calls[1].resolve({blob:processingVisual.calls[1].blob,corrected:false,
      detection:{quality:{valid:false,reason:'live-contour-not-observed'}}});
    processingVisual.finish(0);
  });
  await expect(page.locator('#liveAlternatives button').first()).toHaveAttribute('aria-pressed','true');
  await expect(page.locator('#liveAlternatives button').nth(1)).toBeDisabled();
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
  await expect(page.locator('#liveReviewStatus')).toHaveText('Ready');
  await page.evaluate(()=>processingVisual.calls[2].resolve({blob:processingVisual.calls[2].blob,corrected:false}));
  await expect(page.locator('#liveReviewStatus')).toHaveText('Ready');
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
});


test('dots, yellow frame and four reference circles stay visible through the final processing stages',async({page})=>{
  await prepare(page);
  const sample=()=>page.locator('#livePreview').evaluate(image=>{
    const c=document.createElement('canvas');c.width=image.naturalWidth;c.height=image.naturalHeight;
    const x=c.getContext('2d');x.drawImage(image,0,0);
    return [[15,60],[20,19.3],[101.25,276.8],[103.75,276.8],[106.25,276.8],[108.75,276.8]].map(([a,b])=>
      [...x.getImageData(Math.floor(a*c.width/210),Math.floor(b*c.width/210),1,1).data]);
  });
  await page.evaluate(()=>processingVisual.progress(0,'warp',{...processingVisual.geometry,width:420,height:594,
    space:'corrected',previewBlob:processingVisual.warp}));
  await expect.poll(()=>page.locator('#livePreview').evaluate(i=>i.naturalWidth)).toBe(420);
  const reference=await sample();
  expect(reference[0][2]).toBeLessThan(200);expect(reference[1][0]).toBeLessThan(230);
  expect(reference[2][0]).toBeGreaterThan(reference[2][2]);expect(reference[3][0]).toBeLessThan(100);
  expect(reference[4][2]).toBeGreaterThan(reference[4][0]);expect(reference[5][1]).toBeGreaterThan(reference[5][0]);
  for(const stage of ['colors','vectors','template','encoding']){
    await page.evaluate(stage=>processingVisual.progress(0,stage,{}),stage);
    await expect(page.locator('#livePreview')).toBeVisible();
    await expect(page.locator('#liveProcessingOverlay')).toBeVisible();
    expect(await sample()).toEqual(reference);
  }
  await page.emulateMedia({reducedMotion:'reduce'});
  expect(await page.locator('#liveReviewStatus').evaluate(e=>getComputedStyle(e,'::before').animationName)).toBe('none');
  await page.evaluate(()=>processingVisual.finish(0));
  await expect(page.locator('#liveAddBtn')).toBeEnabled();
  expect(await sample()).toEqual(reference);
});
