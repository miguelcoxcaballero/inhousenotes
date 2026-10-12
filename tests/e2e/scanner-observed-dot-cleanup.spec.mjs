import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=e2e-observed-cleanup', { waitUntil: 'domcontentloaded' });
});

test('measured bullet positions clean displaced print while preserving pencil and punctuation', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const canvas = document.createElement('canvas'); canvas.width = 840; canvas.height = 1188;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    context.fillStyle = '#fff'; context.fillRect(0, 0, canvas.width, canvas.height);
    const observations = [];
    context.fillStyle = '#b8b8b8';
    for (let col = 0; col < 35; col += 1) for (let row = 0; row < 51; row += 1) {
      const x = (2 + col * .5) * 40 + 4 * Math.sin(row * .23) * Math.sin(col * .12);
      const y = (1.93 + row * .5) * 40 + 3 * Math.sin(col * .28);
      context.beginPath(); context.arc(x, y, 1.2, 0, Math.PI * 2); context.fill();
      // Missing observations represent marks hidden by writing, not invented
      // geometry. The actual printed page still contains those dots.
      if ((row * 3 + col * 7) % 11) observations.push({ x, y, row, col });
    }
    const punctuation = observations.find(point => point.row === 25 && point.col === 17);
    context.fillStyle = '#646464'; context.beginPath();
    context.arc(punctuation.x, punctuation.y, 1.8, 0, Math.PI * 2); context.fill();
    context.strokeStyle = '#969696'; context.lineWidth = 1.5;
    context.beginPath(); context.moveTo(105, 627); context.lineTo(735, 627); context.stroke();
    context.strokeStyle = '#214fb0'; context.lineWidth = 2;
    context.beginPath(); context.moveTo(105, 700); context.lineTo(735, 700); context.stroke();
    const before = context.getImageData(0, 0, canvas.width, canvas.height).data;
    const processed = await ScannerPro.Lightweight.correctColors(canvas, {
      useStencil: true, preciseStencil: true, dotObservations: observations
    });
    const after = context.getImageData(0, 0, canvas.width, canvas.height).data;
    const darkest = (data, x, y) => {
      let minimum = 255;
      for (let dy = -2; dy <= 2; dy += 1) for (let dx = -2; dx <= 2; dx += 1) {
        minimum = Math.min(minimum, data[((Math.round(y) + dy) * canvas.width + Math.round(x) + dx) * 4]);
      }
      return minimum;
    };
    const isolated = observations.filter(point => point.row < 20 && point.col > 4 && point.col < 30);
    const darkness = data => isolated.reduce((sum, point) => sum + 255 - darkest(data, point.x, point.y), 0) / isolated.length;
    return {
      cleanup: processed.stencilCleanup,
      before: darkness(before), after: darkness(after),
      punctuation: darkest(after, punctuation.x, punctuation.y),
      pencil: darkest(after, 315, 627),
      blue: Array.from(after.slice((700 * canvas.width + 315) * 4, (700 * canvas.width + 315) * 4 + 3))
    };
  });
  expect(result.cleanup.method).toBe('observed-dots');
  expect(result.cleanup.confirmed).toBeGreaterThan(1000);
  expect(result.after).toBeLessThan(result.before * .5);
  expect(result.punctuation).toBeLessThan(180);
  expect(result.pencil).toBeLessThan(195);
  expect(result.blue[2] - result.blue[0]).toBeGreaterThan(80);
});

test('stale source-space dot coordinates do not replace photographed-grid recovery', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const canvas = document.createElement('canvas'); canvas.width = 630; canvas.height = 891;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    context.fillStyle = '#fff'; context.fillRect(0, 0, canvas.width, canvas.height);
    const dots = [];
    context.fillStyle = '#b8b8b8';
    for (let col = 0; col < 35; col += 1) for (let row = 0; row < 51; row += 1) {
      const x = (2 + col * .5) * 30, y = (1.93 + row * .5) * 30;
      context.beginPath(); context.arc(x, y, 1, 0, Math.PI * 2); context.fill();
      dots.push({ column: col, row, x: x + 40, y: y + 90 });
    }
    const processed = await ScannerPro.Lightweight.correctColors(canvas, {
      useStencil: true, preciseStencil: true, dotObservations: dots
    });
    return processed.stencilCleanup;
  });
  expect(result.method).toBe('recovered-dot-phase');
  expect(result.observations).toBe(0);
});

test('verified bullet centres survive nearby camera speckles without leaving a second dot pattern', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const canvas = document.createElement('canvas'); canvas.width = 1260; canvas.height = 1782;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    context.fillStyle = '#fff'; context.fillRect(0, 0, canvas.width, canvas.height);
    // A soft printed dot with a sharper, directionally varying camera speckle
    // nearby. The dark speckle must not replace its verified geometric centre.
    const stamp = document.createElement('canvas'); stamp.width = stamp.height = 16;
    const stampContext = stamp.getContext('2d');
    stampContext.fillStyle = '#b8b8b8'; stampContext.filter = 'blur(.8px)';
    stampContext.beginPath(); stampContext.arc(8, 8, 1.6, 0, Math.PI * 2); stampContext.fill();
    const observations = [];
    for (let col = 0; col < 35; col += 1) for (let row = 0; row < 51; row += 1) {
      const x = (2 + col * .5) * 60, y = (1.93 + row * .5) * 60;
      observations.push({ col, row, x, y });
      context.drawImage(stamp, x - 8, y - 8);
      const angle = ((row * 13 + col * 7) % 23) / 23 * Math.PI * 2;
      context.fillStyle = '#a0a0a0';
      context.fillRect(Math.floor(x + Math.cos(angle) * 3.3), Math.floor(y + Math.sin(angle) * 3.3), 1, 1);
    }
    context.strokeStyle = '#969696'; context.lineWidth = 1.5;
    context.beginPath(); context.moveTo(160, 1011); context.lineTo(1100, 1011); context.stroke();
    const punctuation = observations.find(point => point.col === 17 && point.row === 25);
    context.fillStyle = '#646464'; context.beginPath();
    context.arc(punctuation.x, punctuation.y, 1.8, 0, Math.PI * 2); context.fill();
    const processed = await ScannerPro.Lightweight.correctColors(canvas, {
      useStencil: true, preciseStencil: true, dotObservations: observations
    });
    const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
    let dark = 0, count = 0;
    for (const point of observations.filter(point => point.row < 20)) {
      for (let dy = -3; dy <= 3; dy += 1) for (let dx = -3; dx <= 3; dx += 1) {
        const offset = ((Math.floor(point.y) + dy) * canvas.width + Math.floor(point.x) + dx) * 4;
        if (pixels[offset] < 235) dark += 1;
        count += 1;
      }
    }
    const darkest = (x, y) => {
      let minimum = 255;
      for (let dy = -2; dy <= 2; dy += 1) for (let dx = -2; dx <= 2; dx += 1) {
        minimum = Math.min(minimum, pixels[((Math.floor(y) + dy) * canvas.width + Math.floor(x) + dx) * 4]);
      }
      return minimum;
    };
    return { cleanup: processed.stencilCleanup, darkFraction: dark / count,
      pencil: darkest(430, 1011), punctuation: darkest(punctuation.x, punctuation.y) };
  });
  expect(result.cleanup.method).toBe('observed-dots');
  expect(result.darkFraction).toBeLessThan(.025);
  expect(result.pencil).toBeLessThan(195);
  expect(result.punctuation).toBeLessThan(180);
});
