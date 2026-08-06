import { expect, test } from '@playwright/test';

test('production app boots under CSP with external runtime modules', async ({ page }) => {
  const violations = [];
  const pageErrors = [];
  page.on('console', message => {
    if (/Refused to (load|execute|connect|frame|apply)/i.test(message.text())) violations.push(message.text());
  });
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  await expect(page.locator('#welcome-view')).toBeVisible();
  await expect(page.locator('[data-app-version]').first()).toHaveText('v5.11.8');
  expect(await page.evaluate(() => !!(window.pdfjsLib && window.PDFLib && window.jspdf))).toBe(true);
  expect(violations).toEqual([]);
  expect(pageErrors).toEqual([]);
});

test('manage pages opens the embedded Inhouse Scanner below Photo', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(async () => {
    await window.__IHN_TEST_API__.ready();
    await window.__IHN_TEST_API__.resetLocalDocument(1, 'Scanner UI test');
    await window.__IHN_TEST_API__.showEditorForTest();
  });

  await page.locator('#btn-edit-pages').click();
  await expect(page.locator('#btn-set-cover')).toBeVisible();
  await expect(page.locator('#btn-scan-page')).toBeVisible();
  expect(await page.evaluate(() => {
    const photo = document.getElementById('btn-set-cover');
    const scan = document.getElementById('btn-scan-page');
    return !!(photo.compareDocumentPosition(scan) & Node.DOCUMENT_POSITION_FOLLOWING);
  })).toBe(true);

  await page.locator('#btn-scan-page').click();
  await expect(page.locator('#scanner-editor-overlay')).toHaveClass(/visible/);
  await expect(page.locator('#scanner-editor-frame')).toHaveAttribute('src', /scanner\/index\.html.*embed=1/);
  const closed = await page.evaluate(() => window.__IHN_TEST_API__.closeScannerForTest());
  expect(closed).toBe(true);
});

test('multi-page scanner insertion is ordered, durable, and preserves shifted pages', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  const result = await page.evaluate(async () => {
    const api = window.__IHN_TEST_API__;
    await api.ready();
    await api.resetLocalDocument(2, 'Scanner insertion test');
    const shiftedStrokeId = await api.addSyntheticStroke(1, 'shifted-page');
    const insertion = await api.addScannedPagesForTest(2);
    const checkpoint = await api.checkpointBeforeLeaving();
    return { shiftedStrokeId, insertion, checkpoint };
  });

  expect(result.insertion.inserted).toBe(true);
  expect(result.insertion.snapshot.pages).toHaveLength(4);
  expect(result.insertion.snapshot.pages.slice(1, 3).every(pageData => (
    pageData.backgroundSource === 'custom'
    && /^data:image\/jpeg/.test(pageData.backgroundImage || '')
  ))).toBe(true);
  expect(result.insertion.snapshot.pages[3].strokes.map(stroke => stroke.id))
    .toContain(result.shiftedStrokeId);
  expect(new Set(result.insertion.snapshot.pages.map(pageData => pageData.pageId)).size).toBe(4);
  expect(result.checkpoint.snapshot.dirtyPages).toEqual([]);
});

test('embedded scanner exposes its complete document workflow', async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=e2e-scanner', { waitUntil: 'domcontentloaded' });
  await expect(page.locator('#landingPage')).toBeHidden();
  await expect(page.locator('#appContainer')).toBeVisible();
  await expect(page.locator('#closeEmbedBtn')).toBeVisible();
  await expect(page.locator('#btnDesktopAdd')).toBeAttached();
  await expect(page.locator('#cropBtn')).toBeAttached();
  await expect(page.locator('#stencilBtn')).toBeAttached();
  await expect(page.locator('#pageList')).toBeAttached();
  await expect(page.locator('#addToDocumentBtn')).toBeVisible();
  await expect(page.locator('#exportBtn')).toBeVisible();
  await expect(page.locator('#downloadEmbedStencilBtn')).toBeVisible();
});

test('scanner starts offline and processes a stencil without OpenCV', async ({ page }) => {
  await page.route(/^https:\/\//, route => route.abort());
  const pageErrors = [];
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.goto('/scanner/index.html?embed=1&session=e2e-light-scanner', { waitUntil: 'domcontentloaded' });
  await expect(page.locator('#appLoading')).toBeHidden();
  await page.evaluate(() => {
    window.__scannerProcessingPhases = [];
    addEventListener('scanner-processing-phase', event => {
      window.__scannerProcessingPhases.push(event.detail);
    });
  });

  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="840" height="1188" viewBox="0 0 210 297">
    <rect width="210" height="297" fill="white"/>
    <rect x="15" y="14.3" width="180" height="270" fill="none" stroke="#f0db4c" stroke-width="1.2"/>
    <path d="M15 274.3 H195 M15 279.3 H195 M15 274.3 V279.3 M20 274.3 V279.3 M25 274.3 V279.3 M30 274.3 V279.3 M35 274.3 V279.3 M40 274.3 V279.3 M45 274.3 V279.3 M50 274.3 V279.3 M55 274.3 V279.3 M60 274.3 V279.3 M65 274.3 V279.3 M70 274.3 V279.3 M75 274.3 V279.3 M80 274.3 V279.3 M85 274.3 V279.3 M90 274.3 V279.3 M95 274.3 V279.3 M100 274.3 V279.3 M105 274.3 V279.3 M110 274.3 V279.3 M115 274.3 V279.3 M120 274.3 V279.3 M125 274.3 V279.3 M130 274.3 V279.3 M135 274.3 V279.3 M140 274.3 V279.3 M145 274.3 V279.3 M150 274.3 V279.3 M155 274.3 V279.3 M160 274.3 V279.3 M165 274.3 V279.3 M170 274.3 V279.3 M175 274.3 V279.3 M180 274.3 V279.3 M185 274.3 V279.3 M190 274.3 V279.3 M195 274.3 V279.3" fill="none" stroke="#f0db4c" stroke-width=".6"/>
    <path d="M45 70 C75 40 105 105 150 62 M52 130 L160 170" fill="none" stroke="#165dde" stroke-width="2.4"/>
    <g stroke="#f0db4c" stroke-width=".6">
      <circle cx="101.25" cy="276.8" r="1.25" fill="#f00"/>
      <circle cx="103.75" cy="276.8" r="1.25" fill="#000"/>
      <circle cx="106.25" cy="276.8" r="1.25" fill="#00f"/>
      <circle cx="108.75" cy="276.8" r="1.25" fill="#6eff12"/>
    </g>
  </svg>`;
  await page.locator('#fileInput').setInputFiles({
    name: 'stencil-page.svg',
    mimeType: 'image/svg+xml',
    buffer: Buffer.from(svg)
  });

  await expect(page.locator('#exportBtn')).toBeEnabled({ timeout: 12_000 });
  await expect(page.locator('.page-card')).toHaveCount(1);
  await expect(page.locator('#processAnimationLayer')).toBeHidden();
  const processingPhases = await page.evaluate(() => window.__scannerProcessingPhases);
  expect(processingPhases.map(entry => entry.phase)).toEqual([
    'corners', 'frame', 'mesh', 'warp', 'color', 'complete'
  ]);
  expect(processingPhases.find(entry => entry.phase === 'corners')).toMatchObject({
    cornerCount: 4
  });
  expect(processingPhases.find(entry => entry.phase === 'corners').yellowBoxPoints).toBeGreaterThan(40);
  expect(processingPhases.find(entry => entry.phase === 'mesh')).toMatchObject({
    rows: 12,
    columns: 8,
    curved: true
  });
  expect(processingPhases.find(entry => entry.phase === 'frame')).toMatchObject({
    detected: true
  });
  expect(processingPhases.find(entry => entry.phase === 'warp')).toMatchObject({
    realGeometry: true
  });
  expect(processingPhases.find(entry => entry.phase === 'color')).toMatchObject({
    realBeforeAfter: true
  });
  expect(processingPhases.find(entry => entry.phase === 'complete').yellowBoxPoints).toBeGreaterThan(40);
  expect(pageErrors).toEqual([]);
  expect(await page.evaluate(() => typeof window.cv)).toBe('undefined');
});

test('scanner isolates a photographed page from a warm background without washing it out', async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=e2e-photo-detection', { waitUntil: 'domcontentloaded' });
  const result = await page.evaluate(async () => {
    const canvas = document.createElement('canvas');
    canvas.width = 960;
    canvas.height = 1280;
    const context = canvas.getContext('2d');
    // This warm wood colour matched the old yellow predicate and made the
    // detector crop the desk instead of the sheet.
    context.fillStyle = '#aa7547';
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.fillStyle = '#d5d0c8';
    context.beginPath();
    context.moveTo(92, 76);
    context.lineTo(864, 132);
    context.lineTo(902, 1202);
    context.lineTo(62, 1148);
    context.closePath();
    context.fill();
    context.strokeStyle = '#f0db4c';
    context.lineWidth = 6;
    context.beginPath();
    context.moveTo(144, 135);
    context.lineTo(814, 181);
    context.lineTo(846, 1133);
    context.lineTo(118, 1085);
    context.closePath();
    context.stroke();
    context.strokeStyle = '#2854a8';
    context.lineWidth = 5;
    context.beginPath();
    context.moveTo(220, 390);
    context.bezierCurveTo(410, 250, 520, 610, 735, 420);
    context.stroke();

    const started = performance.now();
    const detection = ScannerPro.Lightweight.detectPage(canvas);
    const output = await ScannerPro.Lightweight.warp(canvas, detection.pageQuad, detection.frame);
    const preciseStencil = detection.method === 'stencil' || detection.method === 'marker-guided';
    const correction = await ScannerPro.Lightweight.correctColors(output, {
      useStencil: preciseStencil,
      preciseStencil
    });
    const center = Array.from(output.getContext('2d').getImageData(
      Math.floor(output.width / 2), Math.floor(output.height / 2), 1, 1
    ).data.slice(0, 3));
    return {
      method: detection.method,
      confidence: detection.confidence,
      frameSupport: detection.frame?.support || 0,
      areaRatio: detection.pageQuad.reduce((area, point, index, points) => {
        const next = points[(index + 1) % points.length];
        return area + point.x * next.y - next.x * point.y;
      }, 0) / (2 * canvas.width * canvas.height),
      center,
      correction: { calibrated: correction.calibrated, samples: correction.samples },
      elapsed: performance.now() - started
    };
  });

  expect(result.method).toBe('stencil');
  expect(result.confidence).toBeGreaterThan(0.5);
  expect(result.frameSupport).toBeGreaterThan(0.45);
  expect(Math.abs(result.areaRatio)).toBeGreaterThan(0.55);
  expect(Math.abs(result.areaRatio)).toBeLessThan(0.9);
  expect(Math.min(...result.center)).toBeGreaterThan(195);
  expect(Math.max(...result.center)).toBeLessThanOrEqual(255);
  expect(Math.min(...result.center)).toBeGreaterThan(245);
  expect(Math.max(...result.center) - Math.min(...result.center)).toBeLessThan(18);
  expect(result.elapsed).toBeLessThan(3000);
});

test('scanner calibrates paper and ink from the four dots and yellow frame', async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=e2e-reference-colours', { waitUntil: 'domcontentloaded' });
  const result = await page.evaluate(async () => {
    const canvas = document.createElement('canvas');
    canvas.width = 840;
    canvas.height = 1188;
    const context = canvas.getContext('2d');
    const pxPerCm = canvas.width / 21;
    context.fillStyle = 'rgb(218,213,205)';
    context.fillRect(0, 0, canvas.width, canvas.height);

    const yellow = 'rgb(195,170,55)';
    context.strokeStyle = yellow;
    context.lineWidth = 5;
    context.strokeRect(1.5 * pxPerCm, 1.43 * pxPerCm, 18 * pxPerCm, 27 * pxPerCm);

    const references = [
      [10.125, 'rgb(165,55,50)'],
      [10.375, 'rgb(42,42,42)'],
      [10.625, 'rgb(45,70,160)'],
      [10.875, 'rgb(75,145,55)']
    ];
    for (const [x, colour] of references) {
      context.fillStyle = colour;
      context.beginPath();
      context.arc(x * pxPerCm, 27.68 * pxPerCm, pxPerCm * 0.1, 0, Math.PI * 2);
      context.fill();
    }

    const swatches = {
      red: ['rgb(165,55,50)', 5],
      black: ['rgb(42,42,42)', 8],
      blue: ['rgb(45,70,160)', 11],
      green: ['rgb(75,145,55)', 14],
      yellow: [yellow, 17]
    };
    for (const [colour, x] of Object.values(swatches)) {
      context.fillStyle = colour;
      context.fillRect(x * pxPerCm, 12 * pxPerCm, pxPerCm, pxPerCm);
    }

    const correction = await ScannerPro.Lightweight.correctColors(canvas, {
      useStencil: true,
      preciseStencil: true
    });
    const sample = (x, y) => Array.from(context.getImageData(
      Math.round(x * pxPerCm), Math.round(y * pxPerCm), 1, 1
    ).data.slice(0, 3));
    return {
      calibrated: correction.calibrated,
      samples: correction.samples,
      paper: sample(10.5, 9),
      red: sample(5.5, 12.5),
      black: sample(8.5, 12.5),
      blue: sample(11.5, 12.5),
      green: sample(14.5, 12.5),
      yellow: sample(17.5, 12.5)
    };
  });

  expect(result.calibrated).toBe(true);
  expect(result.samples).toBe(4);
  expect(result.paper.every(channel => channel >= 250)).toBe(true);
  expect(result.red).toEqual([232, 16, 16]);
  expect(result.black).toEqual([77, 77, 77]);
  expect(result.blue).toEqual([0, 47, 217]);
  expect(result.green).toEqual([110, 255, 18]);
  expect(result.yellow).toEqual([255, 222, 0]);
});

test('scanner follows a curved yellow frame and straightens it with a real mesh', async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=e2e-curved-frame', { waitUntil: 'domcontentloaded' });
  const result = await page.evaluate(async () => {
    const source = document.createElement('canvas');
    source.width = 900;
    source.height = 1250;
    const context = source.getContext('2d');
    context.fillStyle = '#8b684b';
    context.fillRect(0, 0, source.width, source.height);
    context.fillStyle = '#ece9e2';
    context.beginPath();
    context.moveTo(82, 64);
    context.lineTo(822, 79);
    context.lineTo(850, 1191);
    context.lineTo(58, 1170);
    context.closePath();
    context.fill();
    context.strokeStyle = '#efd84a';
    context.lineWidth = 7;
    context.lineCap = 'round';
    context.beginPath();
    context.moveTo(125, 145);
    context.quadraticCurveTo(455, 92, 780, 151);
    context.quadraticCurveTo(824, 620, 797, 1110);
    context.quadraticCurveTo(452, 1162, 106, 1092);
    context.quadraticCurveTo(70, 618, 125, 145);
    context.stroke();
    // The square row is a separate, parallel observation of the bent lower
    // border. Dense vertical teeth must never be mistaken for the page edge.
    const curvedBottomPoint = t => ({
      x: (1 - t) * (1 - t) * 797 + 2 * (1 - t) * t * 452 + t * t * 106,
      y: (1 - t) * (1 - t) * 1110 + 2 * (1 - t) * t * 1162 + t * t * 1092
    });
    context.beginPath();
    context.moveTo(797, 1074);
    context.quadraticCurveTo(452, 1126, 106, 1056);
    context.moveTo(797, 1092);
    context.quadraticCurveTo(452, 1144, 106, 1074);
    for (let step = 0; step <= 24; step += 1) {
      const point = curvedBottomPoint(step / 24);
      context.moveTo(point.x, point.y - 36);
      context.lineTo(point.x, point.y - 18);
    }
    context.stroke();
    // Strong yellow geometry from neighbouring material is deliberately close
    // to the photo edges and must be rejected as a frame candidate.
    context.beginPath();
    context.moveTo(22, 42);
    context.lineTo(875, 58);
    context.moveTo(20, 1218);
    context.lineTo(882, 1228);
    context.stroke();
    context.strokeStyle = '#315cab';
    context.lineWidth = 5;
    context.beginPath();
    context.moveTo(220, 410);
    context.bezierCurveTo(390, 350, 520, 510, 690, 425);
    context.stroke();

    const detection = ScannerPro.Lightweight.detectPage(source);
    const output = await ScannerPro.Lightweight.warp(source, detection.pageQuad, detection.frame);
    const data = output.getContext('2d').getImageData(0, 0, output.width, output.height).data;
    const expectedY = Math.round(output.height * (1.43 / 29.7));
    const rows = [];
    for (const ratio of [0.18, 0.32, 0.5, 0.68, 0.82]) {
      const x = Math.round(output.width * ratio);
      let best = null;
      for (let y = expectedY - 22; y <= expectedY + 22; y += 1) {
        const offset = (y * output.width + x) * 4;
        const r = data[offset], g = data[offset + 1], b = data[offset + 2];
        if (r > 150 && g > 135 && b < 145 && r + g - b * 2 > 130) {
          if (best == null || Math.abs(y - expectedY) < Math.abs(best - expectedY)) best = y;
        }
      }
      rows.push(best);
    }
    return {
      method: detection.method,
      support: detection.frame?.support || 0,
      curvature: detection.frame?.curvature || 0,
      boxSupport: detection.frame?.box?.support || 0,
      rows
    };
  });

  expect(result.method).toBe('stencil');
  expect(result.support).toBeGreaterThan(0.45);
  expect(result.curvature).toBeGreaterThan(0.012);
  // One of the two narrow box rails may merge into the vertical teeth under
  // strong curvature; the other rail must still remain a strong observation.
  expect(result.boxSupport).toBeGreaterThan(0.45);
  expect(result.rows.every(Number.isFinite)).toBe(true);
  expect(Math.max(...result.rows) - Math.min(...result.rows)).toBeLessThanOrEqual(4);
});

test('scanner calibration strip recovers a rotated stencil when ordinary corners are ambiguous', async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=e2e-marker-strip', { waitUntil: 'domcontentloaded' });
  const result = await page.evaluate(() => {
    const source = document.createElement('canvas');
    source.width = 1280;
    source.height = 960;
    const sourceContext = source.getContext('2d');
    sourceContext.fillStyle = '#a96f43';
    sourceContext.fillRect(0, 0, source.width, source.height);

    const sheet = document.createElement('canvas');
    sheet.width = 600;
    sheet.height = 849;
    const context = sheet.getContext('2d');
    context.fillStyle = '#dedad2';
    context.fillRect(0, 0, sheet.width, sheet.height);
    context.strokeStyle = '#f0db4c';
    context.lineWidth = 4;
    context.strokeRect(43, 41, 514, 772);
    // Repeated boxes are intentionally stronger than the partly hidden frame.
    const stripTop = 783;
    const boxWidth = 18;
    for (let x = 43; x < 557; x += boxWidth) context.strokeRect(x, stripTop, boxWidth, 18);
    const markerY = 792;
    const markerX = 289;
    const colours = ['#d72f2f', '#202020', '#285bd7', '#68c92b'];
    colours.forEach((colour, index) => {
      context.fillStyle = colour;
      context.beginPath();
      context.arc(markerX + index * 7, markerY, 4, 0, Math.PI * 2);
      context.fill();
    });
    // Hide two long frame segments as overlapping sheets would in a real photo.
    context.fillStyle = '#dedad2';
    context.fillRect(39, 70, 9, 640);
    context.fillRect(130, 37, 360, 9);

    sourceContext.save();
    sourceContext.translate(115, 790);
    sourceContext.rotate(-Math.PI / 2);
    sourceContext.drawImage(sheet, 0, 0);
    sourceContext.restore();

    const started = performance.now();
    const detection = ScannerPro.Lightweight.detectPage(source);
    const expected = [
      { x: 115, y: 790 },
      { x: 115, y: 190 },
      { x: 964, y: 190 },
      { x: 964, y: 790 }
    ];
    return {
      method: detection.method,
      confidence: detection.confidence,
      elapsed: performance.now() - started,
      cornerErrors: detection.pageQuad.map((corner, index) => Math.hypot(
        corner.x - expected[index].x,
        corner.y - expected[index].y
      ))
    };
  });

  expect(result.method).toBe('marker-guided');
  expect(result.confidence).toBeGreaterThan(0.7);
  expect(Math.max(...result.cornerErrors)).toBeLessThan(90);
  expect(result.elapsed).toBeLessThan(1000);
});

test('blocked IndexedDB cannot trap startup', async ({ page }) => {
  await page.addInitScript(() => {
    const blockedDatabase = {
      open() {
        const request = {};
        setTimeout(() => request.onblocked?.({ target: request }), 0);
        return request;
      }
    };
    Object.defineProperty(window, 'indexedDB', {
      configurable: true,
      value: blockedDatabase
    });
  });
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__, null, { timeout: 8000 });
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  await expect(page.locator('#welcome-view')).toBeVisible();
});

test('failed IndexedDB page transactions return a bounded error without freezing', async ({ page }) => {
  await page.addInitScript(() => {
    const originalTransaction = IDBDatabase.prototype.transaction;
    IDBDatabase.prototype.transaction = function transaction(storeNames, mode, options) {
      const names = Array.isArray(storeNames) ? storeNames : [storeNames];
      if (mode === 'readwrite' && names.includes('pages')) {
        throw new DOMException('Injected write failure', 'InvalidStateError');
      }
      return originalTransaction.call(this, storeNames, mode, options);
    };
  });
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  const result = await page.evaluate(async () => {
    await window.__IHN_TEST_API__.ready();
    await window.__IHN_TEST_API__.resetLocalDocument(1, 'Failed transaction test');
    await window.__IHN_TEST_API__.addSyntheticStroke(0, 'must-not-hang');
    const started = performance.now();
    try {
      await window.__IHN_TEST_API__.checkpointBeforeLeaving();
      return { failed: false, elapsed: performance.now() - started };
    } catch (error) {
      return { failed: true, elapsed: performance.now() - started, message: error.message };
    }
  });
  expect(result.failed).toBe(true);
  expect(result.elapsed).toBeLessThan(3000);
  expect(result.message).toMatch(/save every changed page/i);
  await expect(page.locator('#welcome-view')).toBeVisible();
});

test('an edit followed immediately by leaving is durable on reload', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  const strokeId = await page.evaluate(async () => {
    await window.__IHN_TEST_API__.resetLocalDocument(2, 'Immediate close test');
    return window.__IHN_TEST_API__.addSyntheticStroke(0, 'last-second');
  });
  const checkpoint = await page.evaluate(() => window.__IHN_TEST_API__.checkpointBeforeLeaving());
  expect(checkpoint.elapsedMs).toBeLessThan(5000);
  expect(checkpoint.snapshot.dirtyPages).toEqual([]);
  const beforeReload = await page.evaluate(async () => {
    const db = await new Promise((resolve, reject) => {
      const request = indexedDB.open('notebook-data-db-v1', 4);
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    const read = (storeName, key) => new Promise(resolve => {
      const request = db.transaction(storeName, 'readonly').objectStore(storeName).get(key);
      request.onsuccess = () => resolve(request.result || null);
      request.onerror = () => resolve(null);
    });
    return {
      metadata: await read('snapshots', 'latest'),
      page: await read('pages', 0),
      backup: localStorage.getItem('notebook-data-v1')
    };
  });
  expect(beforeReload.page?.strokes?.map(stroke => stroke.id)).toContain(strokeId);
  expect(beforeReload.metadata?.data?.pages?.length).toBe(2);

  await page.reload({ waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  await page.evaluate(() => window.__IHN_TEST_API__.loadPage(0));
  const restoredIds = await page.evaluate(() => (
    window.__IHN_TEST_API__.snapshot().pages.flatMap(pageData => pageData.strokes.map(stroke => stroke.id))
  ));
  expect(restoredIds).toContain(strokeId);
});

test('concurrent local and out-of-order peer strokes survive a stale snapshot', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  const result = await page.evaluate(async () => {
    const api = window.__IHN_TEST_API__;
    await api.ready();
    await api.resetLocalDocument(1, 'Concurrent stroke convergence');
    const prepared = await api.prepareLiveDocument('e2e-concurrent-live-file');
    const pageId = prepared.pages[0].pageId;
    const localStrokeId = await api.addSyntheticStroke(0, 'local-concurrent');
    const stalePages = prepared.pages;
    const basePacket = {
      v: 1,
      type: 'live-stroke',
      fileId: 'e2e-concurrent-live-file',
      actorId: 'e2e-remote-peer:e2e-remote-tab',
      strokeId: 'e2e-remote-concurrent-stroke',
      pageId,
      tool: 'pen',
      color: '#654321',
      width: 3,
      syncStamp: { clock: 900, actor: 'e2e-remote-peer:e2e-remote-tab' },
      finalBatch: true,
      totalPoints: 4,
      cancel: false,
      sentAt: Date.now()
    };
    const tail = {
      ...basePacket,
      sequence: 102,
      offset: 2,
      points: [{ x: 40, y: 40, p: 0.5 }, { x: 50, y: 50, p: 0.5 }],
      final: true
    };
    const head = {
      ...basePacket,
      sequence: 101,
      offset: 0,
      points: [{ x: 20, y: 20, p: 0.5 }, { x: 30, y: 30, p: 0.5 }],
      final: false
    };
    const afterPeer = await api.receiveRemoteLiveStrokePackets([tail, head]);
    const afterDuplicate = await api.receiveRemoteLiveStrokePackets([head, tail]);
    const afterStaleMerge = await api.mergeRemotePagesForTest(stalePages);
    const ids = snapshot => snapshot.pages[0].strokes.map(stroke => stroke.id);
    return {
      localStrokeId,
      remoteStrokeId: basePacket.strokeId,
      afterPeerIds: ids(afterPeer),
      afterDuplicateIds: ids(afterDuplicate),
      afterStaleMergeIds: ids(afterStaleMerge.snapshot)
    };
  });

  expect(result.afterPeerIds).toEqual(expect.arrayContaining([
    result.localStrokeId,
    result.remoteStrokeId
  ]));
  expect(result.afterDuplicateIds.filter(id => id === result.remoteStrokeId)).toHaveLength(1);
  expect(result.afterStaleMergeIds).toEqual(expect.arrayContaining([
    result.localStrokeId,
    result.remoteStrokeId
  ]));
});

test('untrusted shared HTML and hostile PDF sources fail closed', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.InhouseSecurityCore);
  const result = await page.evaluate(async () => {
    const core = window.InhouseSecurityCore;
    const malicious = [
      '<div class="sp-day-section evil" onclick="window.pwned=1">',
      '<script>window.pwned=1</script>',
      '<div class="sp-events-list">',
      '<button class="sp-event" data-sp-event="same" style="--sp-event-color:#112233;background:url(javascript:alert(1))">A</button>',
      '<button class="sp-event" data-sp-event="same">duplicate</button>',
      '</div></div>'
    ].join('');
    const sanitized = core.sanitizeCalendarPanelHtml(malicious, document);
    const blocked = ['javascript:alert(1)', 'data:application/pdf;base64,AA==', 'file:///tmp/bad.pdf']
      .map(url => {
        try {
          core.hardenPdfDocumentParams({ url }, { locationHref: location.href });
          return false;
        } catch (error) {
          return true;
        }
      });
    let largeBlocked = false;
    try {
      core.hardenPdfDocumentParams({ data: new ArrayBuffer(core.MAX_PDF_BYTES + 1) });
    } catch (error) {
      largeBlocked = true;
    }
    const safeParams = core.hardenPdfDocumentParams({ url: 'blob:http://127.0.0.1/test' });
    const malformedStarted = performance.now();
    let malformedBlocked = false;
    try {
      await core.loadPdfDocument(window.pdfjsLib, { data: new Uint8Array([0, 1, 2, 3]) });
    } catch (error) {
      malformedBlocked = true;
    }
    return {
      sanitized,
      blocked,
      largeBlocked,
      malformedBlocked,
      malformedElapsed: performance.now() - malformedStarted,
      evalAllowed: safeParams.isEvalSupported
    };
  });
  expect(result.sanitized).not.toMatch(/script|onclick|javascript:|evil/);
  expect((result.sanitized.match(/data-sp-event="same"/g) || []).length).toBe(1);
  expect(result.blocked).toEqual([true, true, true]);
  expect(result.largeBlocked).toBe(true);
  expect(result.malformedBlocked).toBe(true);
  expect(result.malformedElapsed).toBeLessThan(2000);
  expect(result.evalAllowed).toBe(false);
});

test('slow network work is bounded instead of hanging the app', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.InhouseRuntimeCore);
  const result = await page.evaluate(async () => {
    const started = performance.now();
    try {
      await window.InhouseRuntimeCore.fetchWithDeadline('/__slow?delay=3000', {}, 1000);
      return { timedOut: false, elapsed: performance.now() - started };
    } catch (error) {
      return { timedOut: error.name === 'TimeoutError', elapsed: performance.now() - started };
    }
  });
  expect(result.timedOut).toBe(true);
  expect(result.elapsed).toBeLessThan(2000);
});
