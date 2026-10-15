import { expect, test } from '@playwright/test';

test('photographed cursive keeps the shape of small letters, hooks and counters', async ({ page }) => {
  await page.goto('/scanner/index.html', { waitUntil: 'domcontentloaded' });
  const report = await page.evaluate(async () => {
    const width = 1588, height = 2246, scale = 2;
    const truth = document.createElement('canvas'); truth.width = width; truth.height = height;
    const pen = truth.getContext('2d');
    pen.scale(scale, scale); pen.lineCap = pen.lineJoin = 'round'; pen.strokeStyle = '#000';
    for (let row = 0; row < 4; row++) for (let column = 0; column < 12; column++) {
      const x = 100 + column * 45, y = 150 + row * 70;
      pen.lineWidth = [1, 1.4, 2, 2.6][row];
      // Single handwritten e/a/l shapes, not a font or an OCR target. The
      // silhouette is known before lens softness and JPEG damage occur.
      const path = new Path2D(); path.moveTo(x, y);
      path.bezierCurveTo(x + 12, y - 2, x + 7, y - 9, x + 3, y - 5);
      path.bezierCurveTo(x - 2, y + 1, x + 6, y + 7, x + 12, y + 1);
      path.bezierCurveTo(x + 17, y - 6, x + 20, y - 6, x + 21, y);
      path.bezierCurveTo(x + 20, y + 7, x + 14, y + 6, x + 15, y);
      path.bezierCurveTo(x + 15, y - 4, x + 21, y - 6, x + 22, y - 3);
      path.lineTo(x + 22, y + 5);
      path.quadraticCurveTo(x + 27, y + 6, x + 28, y - 7);
      path.bezierCurveTo(x + 30, y - 15, x + 33, y - 12, x + 31, y - 5);
      path.bezierCurveTo(x + 27, y + 3, x + 26, y + 7, x + 34, y + 2);
      pen.stroke(path);
    }
    const camera = document.createElement('canvas'); camera.width = width; camera.height = height;
    const context = camera.getContext('2d');
    context.fillStyle = '#fff'; context.fillRect(0, 0, width, height);
    context.filter = 'blur(1.5px)'; context.drawImage(truth, 0, 0); context.filter = 'none';
    const bitmap = await createImageBitmap(await new Promise(resolve => camera.toBlob(resolve, 'image/jpeg', .85)));
    context.drawImage(bitmap, 0, 0); bitmap.close();
    const photo = context.getImageData(0, 0, width, height), coverage = new Uint8Array(width * height);
    for (let i = 0; i < coverage.length; i++) {
      coverage[i] = 255 - photo.data[i * 4];
      photo.data.set([0, 49, 225, coverage[i]], i * 4);
    }
    const expected = pen.getImageData(0, 0, width, height).data, results = [];
    for (const forceJavaScript of [false, true]) {
      const start = performance.now();
      const ink = await ScannerPro.InkVectorizer.fromPixels(photo, { coverage, forceJavaScript });
      const elapsedMs = performance.now() - start;
      context.clearRect(0, 0, width, height); ScannerPro.InkVectorizer.render(context, ink);
      const actual = context.getImageData(0, 0, width, height).data, rows = [];
      for (let row = 0; row < 4; row++) {
        let intersection = 0, extra = 0, absent = 0, missed = 0, total = 0;
        for (let y = (130 + row * 70) * scale; y < (165 + row * 70) * scale; y++) {
          for (let x = 90 * scale; x < 650 * scale; x++) {
            const at = (y * width + x) * 4, original = expected[at + 3] > 100, recovered = actual[at + 3] > 100;
            if (original && recovered) intersection++; else if (original) absent++; else if (recovered) extra++;
            if (!original) continue;
            total++; let nearby = false;
            for (let dy = -1; dy <= 1; dy++) for (let dx = -1; dx <= 1; dx++) {
              if (actual[((y + dy) * width + x + dx) * 4 + 3] > 100) nearby = true;
            }
            if (!nearby) missed++;
          }
        }
        rows.push({ f1: 2 * intersection / (2 * intersection + extra + absent), missing: missed / total });
      }
      results.push({ backend: ink.backend, elapsedMs, rows, strokes: ink.strokes.length, repairCount: ink.repairCount, repairBackend: ink.repairBackend, points: ink.strokes.reduce((sum, stroke) => sum + stroke.points.length, 0), shape: JSON.stringify(ink.strokes) });
    }
    const parity = results[0].shape === results[1].shape;
    for (const result of results) delete result.shape;
    return { parity, results };
  });
  console.log('HANDWRITING_FIDELITY', JSON.stringify(report));
  expect(report.parity).toBe(true);
  for (const result of report.results) {
    // F1 penalizes both lost ink and invented/thickened ink. A centre-line
    // recall test alone would accept filled counters and oversized letters.
    const minimumF1 = [.73, .85, .92, .93], maximumMissing = [.005, .003, .008, .008];
    result.rows.forEach((row, i) => {
      expect(row.f1, `${result.backend}, pen ${i}: silhouette`).toBeGreaterThan(minimumF1[i]);
      expect(row.missing, `${result.backend}, pen ${i}: missing ink`).toBeLessThan(maximumMissing[i]);
    });
    // Keep the original bounds on pen counts. The envelope follows the full
    // silhouette, so it uses more points than isolated two-point repairs.
    expect(result.strokes - result.repairCount, 'no explosion of traced branches').toBeLessThan(550);
    expect(result.repairCount, 'bounded repairs for 48 handwritten glyphs').toBeLessThan(48 * 16);
    expect(result.strokes).toBeLessThan(550 + 48 * 16);
    expect(result.points, 'full envelopes remain bounded for native editing').toBeLessThan(20000);
  }
});
