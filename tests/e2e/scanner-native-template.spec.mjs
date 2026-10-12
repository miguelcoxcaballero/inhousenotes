import { expect, test } from '@playwright/test';

async function bootEditor(page) {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(async () => {
    await __IHN_TEST_API__.ready();
    await __IHN_TEST_API__.resetLocalDocument(1, 'Native scan layers');
    await __IHN_TEST_API__.showEditorForTest();
    await __IHN_TEST_API__.openScannerForTest();
  });
  const iframe = page.frameLocator('#scanner-editor-frame');
  await expect(iframe.locator('#liveScanner')).toBeVisible();
  return iframe;
}

async function sendInkPage(iframe, mimeType = 'image/png') {
  return iframe.locator('body').evaluate(async (_body, type) => {
    const canvas = document.createElement('canvas'); canvas.width = 794; canvas.height = 1123;
    const context = canvas.getContext('2d');
    context.fillStyle = '#123ca0'; context.fillRect(250, 400, 150, 8);
    context.fillStyle = '#646464'; context.fillRect(250, 440, 150, 3);
    const blob = await new Promise(resolve => canvas.toBlob(resolve, type));
    const expectedInk = canvas.toDataURL('image/png');
    parent.postMessage({ source: 'inhouse-scanner', type: 'ihn-scanner-pages',
      sessionId: new URLSearchParams(location.search).get('session'),
      pages: [{ blob, width: 794, height: 1123, name: 'Native scan', inkLayer: true, templateKind: 'default' }]
    }, location.origin);
    return expectedInk;
  }, mimeType);
}

async function pageLayerCheck(page) {
  return page.evaluate(async () => {
    const api = __IHN_TEST_API__;
    await api.loadPage(1);
    await api.showEditorForTest();
    const snapshot = api.snapshot();
    const scan = snapshot.pages[1];
    const ink = new Image(); ink.src = scan.images[0].src; await ink.decode();
    const inkCanvas = document.createElement('canvas'); inkCanvas.width = ink.naturalWidth; inkCanvas.height = ink.naturalHeight;
    const inkContext = inkCanvas.getContext('2d'); inkContext.drawImage(ink, 0, 0);
    const pixel = (x, y) => [...inkContext.getImageData(x, y, 1, 1).data];
    return {
      pageCount: snapshot.pages.length, backgroundSource: scan.backgroundSource,
      backgroundImage: scan.backgroundImage, templateKind: scan.templateKind,
      images: scan.images.length, imageSource: scan.images[0].src,
      imageGeometry: ['x', 'y', 'width', 'height', 'rotation'].map(key => scan.images[0][key]),
      paperAlpha: pixel(50, 50)[3], markerAlpha: pixel(383, 1047)[3],
      pen: pixel(300, 404), pencil: pixel(300, 441)
    };
  });
}

async function nativeFooterCheck(page) {
  return page.evaluate(() => {
    const canvas = document.querySelector('.page-wrapper[data-page="1"] canvas.page-canvas');
    if (!canvas || canvas.dataset.hasPageDraw !== '1') return null;
    const context = canvas.getContext('2d');
    const expected = document.createElement('canvas'); expected.width = canvas.width; expected.height = canvas.height;
    InhouseBulletTemplate.render(expected.getContext('2d'), expected.width, expected.height);
    const a = context.getImageData(0, 0, canvas.width, canvas.height).data;
    const b = expected.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data;
    let footerDifferences = 0, gridDifferences = 0;
    for (let y = 0; y < canvas.height; y++) for (let x = 0; x < canvas.width; x++) {
      // The test handwriting is only in y=400..443 of an A4 page.
      const footer = y > canvas.height * .91;
      const blankGrid = y > canvas.height * .60 && y < canvas.height * .85;
      if (!footer && !blankGrid) continue;
      const offset = (y * canvas.width + x) * 4;
      for (let c = 0; c < 4; c++) if (a[offset + c] !== b[offset + c]) {
        if (footer) footerDifferences++;
        else gridDifferences++;
      }
    }
    const sampleX = Math.floor(300 / 794 * canvas.width), sampleY = Math.floor(404 / 1123 * canvas.height);
    return { footerDifferences, gridDifferences, pen: [...context.getImageData(sampleX, sampleY, 1, 1).data] };
  });
}

test('scanned ink uses the native editor template through insertion, local reload and a PDF round-trip', async ({ page }) => {
  test.setTimeout(120_000);
  const iframe = await bootEditor(page);
  const expectedInk = await sendInkPage(iframe);
  await expect(page.locator('#scanner-editor-overlay')).not.toHaveClass(/visible/);
  const inserted = await pageLayerCheck(page);
  expect(inserted).toMatchObject({ pageCount: 2, backgroundSource: 'template', backgroundImage: null,
    templateKind: 'default', images: 1, imageSource: expectedInk,
    imageGeometry: [397, 561.5, 794, 1123, 0], paperAlpha: 0, markerAlpha: 0,
    pen: [18, 60, 160, 255], pencil: [100, 100, 100, 255] });
  await expect.poll(() => nativeFooterCheck(page)).toEqual({ footerDifferences: 0, gridDifferences: 0, pen: [18, 60, 160, 255] });
  const saved = await page.evaluate(() => __IHN_TEST_API__.checkpointBeforeLeaving());
  expect(saved.snapshot.dirtyPages).toEqual([]);

  await page.reload({ waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => __IHN_TEST_API__.ready());
  expect(await pageLayerCheck(page)).toEqual(inserted);
  await expect.poll(() => nativeFooterCheck(page)).toEqual({ footerDifferences: 0, gridDifferences: 0, pen: [18, 60, 160, 255] });

  const exported = await page.evaluate(() => __IHN_TEST_API__.buildPdfBlobForTest({ includeBytes: true }));
  expect(exported.ok).toBe(true);
  expect(exported.size).toBeGreaterThan(1000);
  await page.evaluate(bytes => __IHN_TEST_API__.importPdfBlobForTest(bytes), exported.bytes);
  const reopened = await pageLayerCheck(page);
  expect(reopened).toMatchObject({ pageCount: 2, backgroundSource: 'template', backgroundImage: null,
    templateKind: 'default', images: 1, imageGeometry: inserted.imageGeometry,
    paperAlpha: 0, markerAlpha: 0, pen: inserted.pen, pencil: inserted.pencil });
  await expect.poll(() => nativeFooterCheck(page)).toEqual({ footerDifferences: 0, gridDifferences: 0, pen: [18, 60, 160, 255] });
});

test('a JPEG cannot masquerade as transparent native scan ink', async ({ page }) => {
  const iframe = await bootEditor(page);
  await iframe.locator('body').evaluate(() => {
    window.nativeInkRejection = null;
    addEventListener('message', event => {
      if (event.data?.type === 'ihn-scanner-result') window.nativeInkRejection = event.data;
    });
  });
  await sendInkPage(iframe, 'image/jpeg');
  await expect.poll(() => iframe.locator('body').evaluate(() => window.nativeInkRejection?.ok)).toBe(false);
  expect(await page.evaluate(() => __IHN_TEST_API__.snapshot().pages.length)).toBe(1);
  await expect(page.locator('#scanner-editor-overlay')).toHaveClass(/visible/);
});
