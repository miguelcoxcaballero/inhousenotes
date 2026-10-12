import { expect, test } from '@playwright/test';

test('photo import keeps rejected geometry original in preview and export, then allows a manual crop', async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=import-quality', { waitUntil: 'domcontentloaded' });
  await expect(page.locator('#appLoading')).toBeHidden();
  await page.evaluate(() => {
    ScannerPro.LiveCamera.close();
    window.importQuality = { warpCalls: 0, colorCalls: 0, exported: null };
    ScannerPro.Lightweight.detectPageAsync = async () => ({
      method: 'stencil', confidence: 1,
      pageQuad: [{ x: -600, y: -200 }, { x: 300, y: 0 }, { x: 350, y: 600 }, { x: -700, y: 800 }],
      quality: { valid: false, reason: 'refined-contour-disagreement' }
    });
    const warp = ScannerPro.Lightweight.warp;
    ScannerPro.Lightweight.warp = (...args) => { importQuality.warpCalls++; return warp(...args); };
    const color = ScannerPro.Lightweight.correctColors;
    ScannerPro.Lightweight.correctColors = (...args) => { importQuality.colorCalls++; return color(...args); };
    // Capture the bitmap passed to PDF output without depending on a CDN.
    window.jspdf = { jsPDF: class {
      internal = { pageSize: { getWidth: () => 210, getHeight: () => 297 } };
      addImage(image) { importQuality.exported = image; }
      save() {}
    } };
  });
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="840" height="600">
    <rect width="840" height="600" fill="white"/>
    <rect x="5" y="5" width="60" height="60" fill="#c02030"/>
    <rect x="775" y="535" width="60" height="60" fill="#194fb4"/>
    <path d="M150 210 L680 380" stroke="#202020" stroke-width="4"/>
  </svg>`;
  await page.locator('#fileInput').setInputFiles({ name: 'rejected-frame.svg', mimeType: 'image/svg+xml', buffer: Buffer.from(svg) });
  await expect(page.locator('#exportBtn')).toBeEnabled();
  await expect(page.locator('.page-meta')).toHaveText('Page 1 · Original photo');
  await expect(page.locator('#stageText')).toHaveText('Original photo · page correction unavailable');
  await expect(page.locator('#previewImg')).toBeVisible();
  await expect(page.locator('#stencilLayer')).toBeHidden();
  await expect(page.locator('#stencilBtn')).toBeDisabled();
  await expect(page.locator('#cropBtn')).toBeEnabled();
  expect(await page.locator('#previewImg').evaluate(image => [image.naturalWidth, image.naturalHeight])).toEqual([840, 600]);
  expect(await page.evaluate(() => [importQuality.warpCalls, importQuality.colorCalls])).toEqual([0, 0]);

  await page.locator('#exportBtn').click();
  await page.waitForFunction(() => !!importQuality.exported);
  const exported = await page.evaluate(async () => {
    const image = new Image(); image.src = importQuality.exported; await image.decode();
    const canvas = document.createElement('canvas'); canvas.width = image.naturalWidth; canvas.height = image.naturalHeight;
    const context = canvas.getContext('2d'); context.drawImage(image, 0, 0);
    const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
    let yellow = 0;
    for (let i = 0; i < pixels.length; i += 4) {
      if (pixels[i] > 170 && pixels[i + 1] > 130 && pixels[i + 2] < 100) yellow++;
    }
    return { width: canvas.width, height: canvas.height, yellow,
      red: Array.from(context.getImageData(30, 30, 1, 1).data.slice(0, 3)),
      blue: Array.from(context.getImageData(810, 570, 1, 1).data.slice(0, 3)) };
  });
  expect(exported.width).toBe(840);
  expect(exported.height).toBe(600);
  expect(exported.yellow).toBe(0);
  expect(exported.red[0]).toBeGreaterThan(170);
  expect(exported.blue[2]).toBeGreaterThan(150);

  await page.locator('#cropBtn').click();
  await expect(page.locator('#cropLayer')).toBeVisible();
  await page.locator('#cropBtn').click();
  await expect(page.locator('#exportBtn')).toBeEnabled({ timeout: 12000 });
  await expect(page.locator('.page-meta')).toHaveText('Page 1');
  await expect(page.locator('#stencilBtn')).toBeEnabled();
  expect(await page.evaluate(() => [importQuality.warpCalls, importQuality.colorCalls])).toEqual([1, 1]);
});
