import { expect, test } from '@playwright/test';

test('raw photographed dot alignment distinguishes coherent bow and phase errors from small local jitter', async ({ page }) => {
  await page.goto('/scanner/index.html?embed=1&session=dot-certification', { waitUntil: 'domcontentloaded' });
  const results = await page.evaluate(() => {
    const definitions = ['aligned', 'jitter', 'missing', 'bowed', 'wrong-phase', 'writing-only'];
    return definitions.map(kind => {
      const canvas = document.createElement('canvas');
      canvas.width = 900; canvas.height = Math.round(canvas.width * 297 / 210);
      const ctx = canvas.getContext('2d'), pitch = canvas.width * 5 / 210;
      ctx.fillStyle = '#f8f2df'; ctx.fillRect(0, 0, canvas.width, canvas.height);
      if (kind !== 'writing-only') {
        ctx.fillStyle = '#b8b3a2'; ctx.filter = 'blur(.4px)';
        for (let row = 0; row < 51; row++) for (let col = 0; col < 35; col++) {
          if (kind === 'missing' && ((col * 17 + row * 7) % 13 < 2 || (col > 14 && col < 20 && row > 22 && row < 30))) continue;
          let dx = 0, dy = 0;
          if (kind === 'jitter') { dx = pitch * .04 * Math.sin(row * 17 + col * 31); dy = pitch * .04 * Math.cos(row * 13 + col * 7); }
          if (kind === 'bowed') { const bow = Math.sin(Math.PI * row / 50) * Math.sin(Math.PI * col / 34); dx = pitch * .3 * bow; dy = pitch * .22 * bow; }
          if (kind === 'wrong-phase') { dx = pitch * .49; dy = pitch * .49; }
          ctx.beginPath(); ctx.arc((20 + col * 5) / 210 * canvas.width + dx, (19.3 + row * 5) / 297 * canvas.height + dy,
            pitch * .065, 0, Math.PI * 2); ctx.fill();
        }
        ctx.filter = 'none';
      } else {
        ctx.fillStyle = '#555'; ctx.font = '18px serif';
        for (let row = 0; row < 30; row++) ctx.fillText('Real writing stays intact; there is no printed dot pattern here.', 75, 90 + row * 34);
      }
      const before = canvas.toDataURL();
      const report = ScannerPro.MetricSurface.certifyRaster(canvas);
      return { kind, unchanged: before === canvas.toDataURL(), ...report };
    });
  });
  console.log(JSON.stringify(results));
  for (const report of results) {
    expect(report.unchanged, report.kind).toBe(true);
    const expected = ['bowed', 'wrong-phase'].includes(report.kind) ? 'failed' : report.kind === 'writing-only' ? 'not-applicable' : 'verified';
    expect(report.status, JSON.stringify(report)).toBe(expected);
    if (expected !== 'not-applicable') {
      expect(report.observations).toBeGreaterThan(500);
      expect(report.regularity).toBeGreaterThan(.6);
    }
  }
});
