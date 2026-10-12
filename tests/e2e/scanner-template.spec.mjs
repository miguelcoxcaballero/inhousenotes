import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.route('**/template-harness.html', route => route.fulfill({
    contentType: 'text/html', body: '<!doctype html><title>Printed scan template</title>'
  }));
  await page.goto('/template-harness.html');
  await page.addScriptTag({ url: '/scanner/processing/scan-template.js' });
});

test('the scan overlay matches the printable A4 bullet journal, calibration boxes and colour markers', async ({ page }) => {
  const result = await page.evaluate(() => {
    const canvas = document.createElement('canvas'); canvas.width = 1260; canvas.height = 1782;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    context.fillStyle = '#fff'; context.fillRect(0, 0, canvas.width, canvas.height);
    ScannerPro.ScanTemplate.render(context, canvas.width, canvas.height);
    const pixel = (x, y) => Array.from(context.getImageData(Math.floor(x * 6), Math.floor(y * 6), 1, 1).data).slice(0, 3);
    const bulletCenters = [];
    for (let row = 0; row < 51; row++) for (let column = 0; column < 35; column++) {
      bulletCenters.push(pixel(20 + column * 5, 19.3 + row * 5));
    }
    const subdivisions = [];
    for (const start of [20, 110]) for (let i = 1; i < 16; i++) subdivisions.push(pixel(start + 5 * i, 276.8));
    const printable = new DOMParser().parseFromString(ScannerPro.ScanTemplate.svg({ background: true }), 'image/svg+xml');
    const pattern = printable.querySelector('pattern');
    return {
      bulletCenters,
      paper: [[10, 10], [22.5, 21.8], [187.5, 271.8], [200, 288]].map(([x, y]) => pixel(x, y)),
      frame: [[15, 150], [195, 150], [100, 14.3], [100, 284.3]].map(([x, y]) => pixel(x, y)),
      subdivisions,
      markers: [101.25, 103.75, 106.25, 108.75].map(x => pixel(x, 276.8)),
      printedPage: printable.documentElement.getAttribute('viewBox'),
      printedPattern: ['x', 'y', 'width', 'height'].map(name => pattern.getAttribute(name)),
      printedDot: ['cx', 'cy', 'r', 'fill'].map(name => pattern.querySelector('circle').getAttribute(name))
    };
  });
  expect(result.printedPage).toBe('0 0 21 29.7');
  expect(result.printedPattern).toEqual(['1.75', '1.68', '0.5', '0.5']);
  expect(result.printedDot).toEqual(['0.25', '0.25', '0.035', '#a8a8a8']);
  expect(result.bulletCenters).toHaveLength(1785);
  for (const point of result.bulletCenters) {
    expect(Math.max(...point)).toBeLessThan(240);
    expect(Math.max(...point) - Math.min(...point)).toBe(0);
  }
  for (const pixel of result.paper) expect(pixel).toEqual([255, 255, 255]);
  for (const pixel of [...result.frame, ...result.subdivisions]) expect(pixel).toEqual([240, 219, 76]);
  expect(result.markers).toEqual([[255, 0, 0], [0, 0, 0], [0, 0, 255], [110, 255, 18]]);
});

test('a transparent template keeps pen and pencil strokes, including ink crossing a printed bullet', async ({ page }) => {
  const result = await page.evaluate(() => {
    const canvas = document.createElement('canvas'); canvas.width = 1260; canvas.height = 1782;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    context.fillStyle = '#fff'; context.fillRect(0, 0, canvas.width, canvas.height);
    const strokes = [
      { x: 100, y: 119.3, color: '#646464' },
      { x: 80, y: 84.3, color: '#123ca0' },
      { x: 15, y: 150, color: '#151515' },
      { x: 50, y: 100, color: '#aa3030' }
    ];
    for (const { x, y, color } of strokes) {
      context.fillStyle = color; context.fillRect(x * 6 - 2, y * 6 - 2, 4, 4);
    }
    const sample = () => strokes.map(({ x, y }) => Array.from(context.getImageData(Math.floor(x * 6), Math.floor(y * 6), 1, 1).data));
    const before = sample();
    context.globalAlpha = .4; context.globalCompositeOperation = 'screen';
    context.setTransform(2, 0, 0, 2, 7, 9);
    ScannerPro.ScanTemplate.render(context, canvas.width, canvas.height);
    const after = sample();
    const transform = context.getTransform();
    const transparent = document.createElement('canvas'); transparent.width = 630; transparent.height = 891;
    const transparentContext = transparent.getContext('2d');
    ScannerPro.ScanTemplate.render(transparentContext, 630, 891);
    return { before, after,
      alpha: context.globalAlpha, blend: context.globalCompositeOperation,
      transform: [transform.a, transform.b, transform.c, transform.d, transform.e, transform.f],
      untouchedAlpha: transparentContext.getImageData(300, 300, 1, 1).data[3],
      transparentSVG: ScannerPro.ScanTemplate.svg().includes('width="21" height="29.7" fill="white"')
    };
  });
  for (let stroke = 0; stroke < result.before.length; stroke++) {
    for (let channel = 0; channel < 3; channel++) expect(result.after[stroke][channel]).toBeLessThanOrEqual(result.before[stroke][channel]);
    expect(result.after[stroke][3]).toBe(255);
  }
  expect(result.after[3]).toEqual(result.before[3]);
  expect(result.alpha).toBe(.4);
  expect(result.blend).toBe('screen');
  expect(result.transform).toEqual([2, 0, 0, 2, 7, 9]);
  expect(result.untouchedAlpha).toBe(0);
  expect(result.transparentSVG).toBe(false);
});

test('worker and document canvases produce the same complete template without an image decoder', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const canvas = document.createElement('canvas'); canvas.width = 630; canvas.height = 891;
    const context = canvas.getContext('2d');
    ScannerPro.ScanTemplate.render(context, 630, 891);
    const expected = context.getImageData(0, 0, 630, 891).data;
    const workerURL = URL.createObjectURL(new Blob([`
      importScripts(${JSON.stringify(`${location.origin}/scanner/processing/scan-template.js`)});
      const canvas = new OffscreenCanvas(630, 891);
      ScannerPro.ScanTemplate.render(canvas.getContext('2d'), 630, 891);
      const data = canvas.getContext('2d').getImageData(0, 0, 630, 891).data;
      postMessage(data, [data.buffer]);
    `], { type: 'application/javascript' }));
    const worker = new Worker(workerURL);
    try {
      const actual = await new Promise((resolve, reject) => {
        worker.onmessage = event => resolve(event.data);
        worker.onerror = reject;
      });
      let mismatches = 0;
      for (let i = 0; i < actual.length; i++) if (actual[i] !== expected[i]) mismatches++;
      return { mismatches, bytes: actual.length };
    } finally { worker.terminate(); URL.revokeObjectURL(workerURL); }
  });
  expect(result.bytes).toBe(630 * 891 * 4);
  expect(result.mismatches).toBe(0);
});
