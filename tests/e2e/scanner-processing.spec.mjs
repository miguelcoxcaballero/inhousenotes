import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=e2e-processing', { waitUntil: 'domcontentloaded' });
});

test('still detection runs in a worker and cancellation leaves the camera responsive', async ({ page }) => {
  const workers = [];
  page.on('worker', worker => workers.push(worker.url()));
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-synthetic.js' });
  const result = await page.evaluate(async () => {
    const source = ScannerSyntheticFixtures.renderCase(ScannerSyntheticFixtures.CASES[0]).source;
    let ticks = 0;
    const timer = setInterval(() => { ticks += 1; }, 16);
    const detection = await ScannerPro.Lightweight.detectPageAsync(source);
    clearInterval(timer);
    const controller = new AbortController();
    const interrupted = ScannerPro.Lightweight.detectPageAsync(source, { signal: controller.signal });
    controller.abort();
    let cancellation;
    try { await interrupted; } catch (error) { cancellation = error.name; }
    return { ticks, method: detection.method, cancellation };
  });
  expect(workers.some(url => url.includes('detection-worker.js'))).toBe(true);
  expect(result.method).toBe('marker-guided');
  expect(result.ticks).toBeGreaterThan(2);
  expect(result.cancellation).toBe('AbortError');
});

test('a capture contour absent from the still image cannot certify a different crop', async ({ page }) => {
  await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-physical.js' });
  const result = await page.evaluate(async () => {
    const source = ScannerPhysicalFixtures.render(ScannerPhysicalFixtures.CASES[0]).source;
    const preview = document.createElement('canvas');
    preview.width = 360; preview.height = Math.round(360 * source.height / source.width);
    const context = preview.getContext('2d');
    context.drawImage(source, 0, 0, preview.width, preview.height);
    const seed = ScannerPro.LiveDetection.detect(context.getImageData(0, 0, preview.width, preview.height));
    const good = await ScannerPro.Lightweight.detectPageAsync(source, { liveDetection: seed });
    const misplaced = structuredClone(seed);
    for (const point of misplaced.quad) point.x += .12;
    for (const edge of misplaced.frameEdges) for (const point of edge) point.x += .12;
    const bad = await ScannerPro.Lightweight.detectPageAsync(source, { liveDetection: misplaced });
    return { good: good.quality, bad: bad.quality };
  });
  expect(result.good.valid).toBe(true);
  expect(result.bad.valid, 'stale camera coordinates must fall back to the original photo').toBe(false);
});

test('surface resampling is cancellable between bands without completing a stale image', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const source = document.createElement('canvas');
    source.width = 600; source.height = 800;
    source.getContext('2d').fillRect(0, 0, 600, 800);
    const controller = new AbortController();
    let progress = 0;
    try {
      await ScannerPro.Lightweight.warp(source, [{ x: 0, y: 0 }, { x: 600, y: 0 },
        { x: 600, y: 800 }, { x: 0, y: 800 }], null, {
        signal: controller.signal,
        onProgress(value) { progress = value; if (value > .1) controller.abort(); }
      });
      return { completed: true, progress };
    } catch (error) { return { name: error.name, progress }; }
  });
  expect(result.name).toBe('AbortError');
  expect(result.progress).toBeLessThan(.2);
});

test('colour processing removes deep smooth shadows while retaining fine pencil and coloured ink', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const canvas = document.createElement('canvas');
    canvas.width = 630; canvas.height = 891;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    const shade = context.createLinearGradient(0, 0, canvas.width, 0);
    shade.addColorStop(0, '#696969'); shade.addColorStop(1, '#f4f4f4');
    context.fillStyle = shade; context.fillRect(0, 0, canvas.width, canvas.height);
    context.strokeStyle = 'rgba(0,0,0,.23)'; context.lineWidth = 1.6;
    for (const y of [250, 420, 590]) {
      context.beginPath(); context.moveTo(70, y); context.lineTo(560, y); context.stroke();
    }
    context.fillStyle = '#143eb0'; context.fillRect(80, 650, 180, 3);
    context.fillStyle = '#bf2929'; context.fillRect(370, 650, 180, 3);
    context.fillStyle = '#3c3c3c'; context.fillRect(90, 710, 120, 70);
    context.fillStyle = '#898989'; context.fillRect(380, 720, 120, 45);
    const before = context.getImageData(0, 0, canvas.width, canvas.height).data;
    await ScannerPro.Lightweight.correctColors(canvas, { useStencil: false });
    const after = context.getImageData(0, 0, canvas.width, canvas.height).data;
    const at = (data, x, y) => Array.from(data.slice((y * canvas.width + x) * 4, (y * canvas.width + x) * 4 + 3));
    return {
      paper: [90, 180, 320, 520].map(x => at(after, x, 320)[0]),
      pencilContrast: [90, 180, 320, 520].map(x => at(after, x, 320)[0] - at(after, x, 420)[0]),
      originalShadow: at(before, 90, 320)[0],
      blue: at(after, 160, 651), red: at(after, 460, 651),
      charcoal: at(after, 150, 745)[0], greyFill: at(after, 440, 740)[0]
    };
  });
  expect(result.originalShadow).toBeLessThan(140);
  expect(Math.min(...result.paper)).toBeGreaterThan(238);
  expect(Math.min(...result.pencilContrast)).toBeGreaterThan(14);
  expect(result.blue[2] - result.blue[0]).toBeGreaterThan(90);
  expect(result.red[0] - result.red[2]).toBeGreaterThan(90);
  expect(result.charcoal).toBeLessThan(100);
  expect(result.greyFill).toBeLessThan(190);
});

test('a folded detection cannot become a plausible looking but mirrored scan', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const source = document.createElement('canvas'); source.width = 600; source.height = 800;
    const point = (x, y) => ({ x, y });
    const frame = {
      confidence: 1,
      paths: {
        top: [point(100, 20), point(300, 20), point(500, 20)],
        bottom: [point(100, 780), point(300, 780), point(500, 780)],
        left: [point(100, 20), point(100, 400), point(100, 780)],
        right: [point(500, 20), point(40, 400), point(500, 780)]
      }
    };
    try {
      await ScannerPro.Lightweight.warp(source,
        [point(0, 0), point(600, 0), point(600, 800), point(0, 800)], frame, { maxDimension: 256 });
      return 'accepted';
    } catch (error) { return error.message; }
  });
  expect(result).toContain('overlaps itself');
});

test('cold and warm illumination are removed without bleaching pencil or coloured writing', async ({ page }) => {
  const results = await page.evaluate(async () => {
    const reports = [];
    for (const gains of [[.62, .80, 1], [1, .80, .62]]) {
      const canvas = document.createElement('canvas');
      canvas.width = 630; canvas.height = 891;
      const context = canvas.getContext('2d', { willReadFrequently: true });
      context.fillStyle = '#ededed'; context.fillRect(0, 0, 630, 891);
      context.fillStyle = '#aaaaaa'; context.fillRect(70, 420, 490, 2);
      context.fillStyle = '#143eb0'; context.fillRect(80, 650, 180, 3);
      context.fillStyle = '#bf2929'; context.fillRect(370, 650, 180, 3);
      context.fillStyle = '#3c3c3c'; context.fillRect(90, 710, 120, 70);
      const pixels = context.getImageData(0, 0, 630, 891);
      for (let y = 0; y < 891; y++) for (let x = 0; x < 630; x++) {
        const shade = .65 + .35 * x / 629;
        for (let c = 0; c < 3; c++) pixels.data[(y * 630 + x) * 4 + c] *= gains[c] * shade;
      }
      context.putImageData(pixels, 0, 0);
      await ScannerPro.Lightweight.correctColors(canvas, { useStencil: false });
      const at = (x, y) => Array.from(context.getImageData(x, y, 1, 1).data).slice(0, 3);
      const paper = [90, 180, 320, 520].map(x => at(x, 320));
      reports.push({
        paper, pencilContrast: [90, 180, 320, 520].map((x, i) => paper[i][0] - at(x, 420)[0]),
        blue: at(160, 651), red: at(460, 651), charcoal: at(150, 745)
      });
    }
    return reports;
  });
  for (const result of results) {
    expect(Math.min(...result.paper.flat()), JSON.stringify(result)).toBeGreaterThan(238);
    expect(Math.max(...result.paper.map(rgb => Math.max(...rgb) - Math.min(...rgb)))).toBeLessThan(12);
    expect(Math.min(...result.pencilContrast)).toBeGreaterThan(14);
    expect(result.blue[2] - result.blue[0]).toBeGreaterThan(90);
    expect(result.red[0] - result.red[2]).toBeGreaterThan(90);
    expect(Math.max(...result.charcoal)).toBeLessThan(100);
  }
});

test('template cleanup preserves isolated punctuation even where it crosses a printed dot', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const canvas = document.createElement('canvas'); canvas.width = 630; canvas.height = 891;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    context.fillStyle = '#fff'; context.fillRect(0, 0, canvas.width, canvas.height);
    context.fillStyle = '#c8c8c8';
    for (let column = 0; column < 35; column += 1) for (let row = 0; row < 51; row += 1) {
      context.beginPath(); context.arc((2 + column * .5) * 30, (1.93 + row * .5) * 30, 1, 0, Math.PI * 2); context.fill();
    }
    const x = 285, y = 388;
    context.fillStyle = '#646464'; context.beginPath(); context.arc(x, y, 1.8, 0, Math.PI * 2); context.fill();
    await ScannerPro.Lightweight.correctColors(canvas, { useStencil: true, preciseStencil: true });
    const pixels = context.getImageData(x - 1, y - 1, 3, 3).data;
    return Math.min(...Array.from({ length: 9 }, (_, index) => pixels[index * 4]));
  });
  expect(result).toBeLessThan(180);
});

test('synthetic white warp margins do not create a grey halo or erase nearby writing', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const canvas = document.createElement('canvas'); canvas.width = 630; canvas.height = 891;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    context.fillStyle = '#fff'; context.fillRect(0, 0, 630, 891);
    context.fillStyle = '#aaaaaa'; context.fillRect(45, 43, 540, 810);
    context.fillStyle = '#646464'; context.fillRect(565, 300, 2, 80);
    await ScannerPro.Lightweight.correctColors(canvas, { useStencil: true, preciseStencil: true });
    return {
      paper: [50, 60, 570, 578].map(x => context.getImageData(x, 400, 1, 1).data[0]),
      ink: context.getImageData(565, 330, 1, 1).data[0]
    };
  });
  expect(Math.min(...result.paper)).toBeGreaterThan(248);
  expect(result.ink).toBeLessThan(180);
});
