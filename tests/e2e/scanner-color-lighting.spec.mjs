import { expect, test } from '@playwright/test';

// The colour references are rendered by the production template, then passed
// through a camera response. These are recoverable illumination changes: no
// clipped channel is expected to reconstruct information it never recorded.
async function measure(page, cases) {
  return page.evaluate(async definitions => {
    const rows = [];
    for (const camera of definitions) {
      const canvas = document.createElement('canvas');
      canvas.width = 840; canvas.height = 1188;
      const context = canvas.getContext('2d', { willReadFrequently: true });
      ScannerPro.ScanTemplate.render(context, canvas.width, canvas.height, { background: true });
      if (camera.hideYellow || camera.greyYellow) {
        // The photograph has an already verified warp, but the thin yellow
        // border was washed out. The footer colour discs remain observable.
        context.fillStyle = '#fff';
        for (const [x, y, width, height] of [[55, 52, 730, 10], [55, 1132, 730, 10], [55, 52, 10, 1090], [775, 52, 10, 1090]]) {
          context.fillRect(x, y, width, height);
        }
      }
      if (camera.greyYellow) {
        // A neutral ridge can confirm geometry, but must not masquerade as
        // yellow pigment and distort a valid four-disc camera response.
        context.strokeStyle = '#aaa';
        context.lineWidth = 2.4;
        context.strokeRect(1.5 * 40, 1.43 * 40, 18 * 40, 27 * 40);
      }
      if (camera.missingMarker || camera.corruptMarker) {
        context.fillStyle = camera.corruptMarker || '#fff';
        context.beginPath(); context.arc(10.625 * 40, 27.68 * 40, 5.5, 0, Math.PI * 2); context.fill();
      }
      for (const [colour, y] of [['#d52b36', 290], ['#254acb', 440], ['#414141', 590], ['#aaa', 740]]) {
        context.fillStyle = colour; context.fillRect(140, y, 210, 12);
      }
      const pixels = context.getImageData(0, 0, canvas.width, canvas.height);
      const gains = camera.gains || [1, 1, 1];
      const flare = camera.flare || 0;
      const exposure = camera.exposure || 1;
      for (let y = 0; y < canvas.height; y++) for (let x = 0; x < canvas.width; x++) {
        const offset = (y * canvas.width + x) * 4;
        const shade = (.94 - .1 * x / canvas.width)
          * (camera.footerShadow && y > 1000 ? .48 : 1);
        const source = [pixels.data[offset], pixels.data[offset + 1], pixels.data[offset + 2]];
        for (let channel = 0; channel < 3; channel++) {
          const localGain = camera.localGains ? 1 + (camera.localGains[channel] - 1) * Math.exp(-(((y - 580) / 290) ** 4)) : 1;
          pixels.data[offset + channel] = exposure * shade * gains[channel] * localGain
            * (flare + (camera.matrix
              ? camera.matrix[channel].reduce((sum, gain, c) => sum + gain * source[c], 0)
              : source[channel]) * (1 - flare / 255));
        }
      }
      context.putImageData(pixels, 0, 0);
      const started = performance.now();
      const colors = await ScannerPro.Lightweight.correctColors(canvas, { useStencil: true, preciseStencil: true, disableRust: camera.disableRust });
      const elapsedMs = performance.now() - started;
      const result = context.getImageData(0, 0, canvas.width, canvas.height).data;
      const rgb = (x, y) => Array.from(result.slice((y * canvas.width + x) * 4, (y * canvas.width + x) * 4 + 3));
      const pens={};
      if(camera.localGains) for(const [name,y] of [['red',290],['blue',440],['black',590],['pencil',740]]) {
        const crop=context.getImageData(125,y-3,240,18);
        InhouseRust.separateInk(crop.data);
        const ink=await ScannerPro.InkVectorizer.fromPixels(crop,{forceJavaScript:camera.disableRust});
        pens[name]=[...new Set(ink.strokes.map(s=>s.color))];
      }
      rows.push({ name: camera.name, pens, calibrated: colors.calibrated, calibration: colors.calibration,
        references: colors.references, elapsedMs, backend: colors.pixelProcessing.backend,
        red: rgb(240, 296), blue: rgb(240, 446), black: rgb(240, 596), pencil: rgb(240, 746),
        paper: rgb(280, 370), outsideRed: rgb(247, 287), outsideBlue: rgb(247, 437) });
    }
    return rows;
  }, cases);
}

function diagnostics(row) {
  return { ...row, calibration: { method: row.calibration.method, matrixError: row.calibration.matrixError,
    references: row.calibration.references, sourcePaper: row.calibration.sourcePaper } };
}

function expectWriting(row) {
  expect(Math.min(...row.paper), `${row.name}: paper white`).toBeGreaterThan(245);
  expect(Math.max(...row.paper) - Math.min(...row.paper), `${row.name}: paper neutral`).toBeLessThan(8);
  expect(row.red[0] - Math.max(row.red[1], row.red[2]), `${row.name}: red remains saturated`).toBeGreaterThan(120);
  expect(row.blue[2] - Math.max(row.blue[0], row.blue[1]), `${row.name}: blue remains saturated`).toBeGreaterThan(115);
  expect(Math.max(...row.black), `${row.name}: black writing stays dark`).toBeLessThan(95);
  expect(Math.max(...row.black) - Math.min(...row.black), `${row.name}: neutral writing has no false colour`).toBeLessThan(15);
  expect(Math.max(...row.pencil), `${row.name}: faint pencil survives`).toBeLessThan(235);
  for (const paper of [row.outsideRed, row.outsideBlue]) {
    expect(Math.min(...paper), `${row.name}: no coloured halo beside ink`).toBeGreaterThan(245);
  }
}

test('four measured footer colours calibrate recoverable green, magenta, tungsten, dim and bright illumination', async ({ page }) => {
  test.setTimeout(60_000);
  await page.goto('/scanner/index.html', { waitUntil: 'domcontentloaded' });
  const rows = await measure(page, [
    { name: 'neutral' },
    { name: 'green', gains: [.55, 1, .62], flare: 12 },
    { name: 'magenta', gains: [1, .52, .88], flare: 12 },
    { name: 'tungsten', gains: [1, .66, .38], flare: 15 },
    { name: 'dim', exposure: .36, gains: [1, .9, .75], flare: 5 },
    { name: 'bright-flare', exposure: .98, flare: 55 },
    { name: 'footer-shadow', gains: [.7, .83, 1], flare: 18, footerShadow: true },
    { name: 'camera-crosstalk', gains: [1, .74, .58], flare: 20,
      matrix: [[.90, .07, .03], [.06, .90, .04], [.03, .09, .88]] }
  ]);
  console.log('COLOR_LIGHTING', JSON.stringify(rows.map(diagnostics)));
  for (const row of rows) {
    expect(row.calibrated, row.name).toBe(true);
    for (const name of ['red', 'black', 'blue', 'green']) expect(row.references[name].valid, `${row.name}: ${name} measured`).toBe(true);
    expect(row.calibration.matrix, `${row.name}: bounded fitted camera response`).not.toBeNull();
    expect(row.calibration.matrixError, `${row.name}: references fit their actual pigments`).toBeLessThan(.06);
    expectWriting(row);
    for (const colour of ['red', 'blue']) for (let c = 0; c < 3; c++) {
      expect(Math.abs(row[colour][c] - rows[0][colour][c]), `${row.name}: ${colour} consistency`).toBeLessThan(42);
    }
  }
});

test('four footer colours still calibrate when the thin yellow border cannot be sampled', async ({ page }) => {
  await page.goto('/scanner/index.html', { waitUntil: 'domcontentloaded' });
  const [row] = await measure(page, [{ name: 'washed-yellow', hideYellow: true, gains: [1, .66, .46], flare: 18 }]);
  console.log('COLOR_NO_YELLOW', JSON.stringify(diagnostics(row)));
  expect(row.calibrated).toBe(true);
  expect(row.calibration.references).toEqual(expect.arrayContaining(['red', 'black', 'blue', 'green', 'paper']));
  expect(row.calibration.matrix).not.toBeNull();
  expect(row.references.yellow.valid, 'white paper is not a yellow pigment sample').toBe(false);
  expect(row.calibration.references).not.toContain('yellow');
  expectWriting(row);
});

test('missing or inconsistent footer pigment cannot be borrowed from a neighbouring disc', async ({ page }) => {
  await page.goto('/scanner/index.html', { waitUntil: 'domcontentloaded' });
  const rows = await measure(page, [
    { name: 'missing-blue', missingMarker: true, gains: [1, .82, .65], flare: 15 },
    { name: 'red-in-blue-position', corruptMarker: '#ff0000', gains: [1, .82, .65], flare: 15 },
    { name: 'yellow-in-blue-position', corruptMarker: '#f0db4c', gains: [1, .82, .65], flare: 15 },
    { name: 'grey-in-blue-position', corruptMarker: '#888', gains: [1, .82, .65], flare: 15 }
  ]);
  console.log('COLOR_MISSING_REFERENCE', JSON.stringify(rows.map(diagnostics)));
  for (const row of rows) {
    expect(row.references.blue.valid, row.name).toBe(false);
    expect(row.calibrated, row.name).toBe(false);
    expect(row.calibration.matrix, row.name).toBeNull();
    expectWriting(row);
  }
});


test('neutral frame ridges cannot override the four measured colour pigments', async ({ page }) => {
  await page.goto('/scanner/index.html', { waitUntil: 'domcontentloaded' });
  const [row] = await measure(page, [{ name: 'grey-frame', greyYellow: true, gains: [1, .66, .46], flare: 18 }]);
  console.log('COLOR_GREY_FRAME', JSON.stringify(diagnostics(row)));
  expect(row.calibrated).toBe(true);
  expect(row.calibration.references).toEqual(['red', 'black', 'blue', 'green', 'paper']);
  expect(row.calibration.matrix).not.toBeNull();
  expect(row.calibration.matrixError).toBeLessThan(.06);
  expect(row.references.yellow.valid, 'a grey geometric ridge is not a yellow colour reference').toBe(false);
  expectWriting(row);
});


test('mixed ambient light keeps black neutral without erasing blue and red ink', async ({ page }) => {
  test.setTimeout(60000);
  await page.goto('/scanner/index.html', { waitUntil: 'domcontentloaded' });
  const rows = await measure(page, [
    { name: 'local-blue-lamp', localGains: [.5, .72, 1], flare: 14 },
    { name: 'local-warm-lamp', localGains: [1, .65, .4], flare: 14 },
    { name: 'local-magenta-lamp', localGains: [1, .6, .9], flare: 14 },
    { name: 'local-blue-lamp-js', localGains: [.5, .72, 1], flare: 14, disableRust: true }
  ]);
  console.log('MIXED_LIGHT', JSON.stringify(rows.map(diagnostics)));
  for (const row of rows) {
    expectWriting(row);
    expect(row.pens.black,row.name).toEqual(['#4D4D4D']);
    expect(row.pens.pencil,row.name).toEqual(['#4D4D4D']);
    expect(row.pens.blue,row.name).toEqual(['#002FD9']);
    expect(row.pens.red,row.name).toEqual(['#E81010']);
  }
  expect(rows[0].backend).toBe('rust'); expect(rows.at(-1).backend).toBe('javascript');
  for(const colour of ['red','blue','black','pencil'])expect(rows[0][colour]).toEqual(rows.at(-1)[colour]);
});
