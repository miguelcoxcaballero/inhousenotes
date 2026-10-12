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

test('soft photographed dots are removed across the whole page, including sites missing from geometry', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const canvas = document.createElement('canvas'); canvas.width = 840; canvas.height = 1188;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    context.fillStyle = '#fff'; context.fillRect(0, 0, canvas.width, canvas.height);
    const actual = [], observed = [];
    context.filter = 'blur(1.4px)'; context.fillStyle = '#a8a8a8';
    for (let row = 0; row < 51; row++) for (let col = 0; col < 35; col++) {
      const x = (20 + col * 5) * 4 + 1.8 * Math.sin(row * .43 + col * .14);
      const y = (19.3 + row * 5) * 4 + 1.5 * Math.cos(col * .39 + row * .17);
      context.beginPath(); context.arc(x, y, 1.2 + .2 * Math.sin(row + col), 0, Math.PI * 2); context.fill();
      const point = { x, y, row, col }; actual.push(point);
      if ((row * 7 + col * 11) % 5) observed.push(point);
    }
    context.filter = 'none';
    context.strokeStyle = '#c6c6c6'; context.lineWidth = 1.4;
    context.beginPath(); context.moveTo(110, 579); context.lineTo(735, 579); context.stroke();
    const punctuation = actual.find(point => point.row === 25 && point.col === 17);
    context.fillStyle = '#a6a6a6'; context.beginPath();
    context.arc(punctuation.x + 4, punctuation.y + 1, 1.1, 0, Math.PI * 2); context.fill();
    const before = context.getImageData(0, 0, canvas.width, canvas.height).data;
    const processed = await ScannerPro.Lightweight.correctColors(canvas, {
      useStencil: true, preciseStencil: true, dotObservations: observed
    });
    const after = context.getImageData(0, 0, canvas.width, canvas.height).data;
    // Known handwriting regions are excluded, but NO sites are excluded for
    // being unobserved, poorly registered or too faint for the detector.
    const allCleanSites = actual.filter(point => point.row < 23 || point.row > 27);
    const darkness = data => {
      let sum = 0, count = 0;
      for (const point of allCleanSites) for (let dy = -5; dy <= 5; dy++) for (let dx = -5; dx <= 5; dx++) {
        const at = ((Math.floor(point.y) + dy) * canvas.width + Math.floor(point.x) + dx) * 4;
        sum += 255 - data[at]; count++;
      }
      return sum / count;
    };
    const darkest = (x, y) => {
      let value = 255;
      for (let dy = -2; dy <= 2; dy++) for (let dx = -2; dx <= 2; dx++)
        value = Math.min(value, after[((Math.floor(y) + dy) * canvas.width + Math.floor(x) + dx) * 4]);
      return value;
    };
    return { before: darkness(before), after: darkness(after), sites: allCleanSites.length,
      observed: observed.length, cleanup: processed.stencilCleanup,
      pencil: darkest(330, 579), punctuation: darkest(punctuation.x + 4, punctuation.y + 1) };
  });
  console.log('soft-grid cleanup', JSON.stringify(result));
  expect(result.sites).toBeGreaterThan(result.observed);
  expect(result.after, 'the photographed lattice cannot remain underneath the native template').toBeLessThan(result.before * .08);
  expect(result.cleanup.isolatedRemoved).toBeGreaterThan(1300);
  expect(result.pencil).toBeLessThan(215);
  expect(result.punctuation).toBeLessThan(190);
});

test('a faint handwritten i-dot near a missing printed site is not mistaken for recovered print', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const canvas = document.createElement('canvas'); canvas.width = 840; canvas.height = 1188;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    context.fillStyle = '#fff'; context.fillRect(0, 0, canvas.width, canvas.height);
    const marks = [180, 195, 210, 225, 235].map((tone, index) => ({ tone, row: 25, col: 10 + index * 3 }));
    const observed = [];
    context.filter = 'blur(1.4px)'; context.fillStyle = '#a8a8a8';
    for (let row = 0; row < 51; row++) for (let col = 0; col < 35; col++) {
      if (marks.some(mark => mark.row === row && mark.col === col)) continue;
      const x = (20 + col * 5) * 4, y = (19.3 + row * 5) * 4;
      context.beginPath(); context.arc(x, y, 1.2, 0, Math.PI * 2); context.fill();
      observed.push({ x, y, row, col });
    }
    context.filter = 'none';
    for (const mark of marks) {
      mark.x = (20 + mark.col * 5) * 4 + 3; mark.y = (19.3 + mark.row * 5) * 4;
      context.fillStyle = `rgb(${mark.tone}, ${mark.tone}, ${mark.tone})`;
      context.beginPath(); context.arc(mark.x, mark.y, 1.1, 0, Math.PI * 2); context.fill();
      context.fillRect(mark.x - .5, mark.y + 8, 1, 8);
    }
    const control = document.createElement('canvas'); control.width = canvas.width; control.height = canvas.height;
    control.getContext('2d').drawImage(canvas, 0, 0);
    await ScannerPro.Lightweight.correctColors(control, { useStencil: true, preciseStencil: true,
      dotObservations: observed, skipStencilCleanup: true });
    const processed = await ScannerPro.Lightweight.correctColors(canvas, {
      useStencil: true, preciseStencil: true, dotObservations: observed
    });
    const darkest = (image, mark) => {
      const pixels = image.getContext('2d').getImageData(Math.floor(mark.x) - 2, Math.floor(mark.y) - 2, 5, 5).data;
      let minimum = 255;
      for (let i = 0; i < pixels.length; i += 4) minimum = Math.min(minimum, pixels[i]);
      return minimum;
    };
    return { marks: marks.map(mark => ({ tone: mark.tone, before: darkest(control, mark), after: darkest(canvas, mark) })),
      cleanup: processed.stencilCleanup };
  });
  console.log('faint punctuation protection', JSON.stringify(result));
  for (const mark of result.marks) {
    expect(mark.after, `preserve the handwritten dot at pencil tone ${mark.tone}`).toBeLessThanOrEqual(mark.before + 3);
  }
});
