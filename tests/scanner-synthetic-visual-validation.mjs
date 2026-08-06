import { chromium } from '@playwright/test';
import fs from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';

const root = path.resolve(import.meta.dirname, '..');
const outputDirectory = path.join(root, 'test-results', 'scanner-synthetic-visual');
const toPng = dataUrl => Buffer.from(dataUrl.replace(/^data:image\/png;base64,/, ''), 'base64');

let server = null;
try {
  const response = await fetch('http://127.0.0.1:4173/scanner/index.html');
  if (!response.ok) throw new Error(`Unexpected status ${response.status}`);
} catch {
  server = (await import('node:child_process')).spawn(
    process.execPath,
    [path.join(root, 'tests', 'e2e', 'server.mjs')],
    { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] }
  );
  await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('E2E server did not start')), 10_000);
    server.stdout.on('data', chunk => {
      if (!String(chunk).includes('listening')) return;
      clearTimeout(timeout);
      resolve();
    });
    server.once('error', reject);
  });
}

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
await page.goto('http://127.0.0.1:4173/scanner/index.html?embed=1&synthetic-validation=1', {
  waitUntil: 'domcontentloaded'
});
await page.addScriptTag({ url: '/tests/e2e/fixtures/scanner-synthetic.js' });
await fs.rm(outputDirectory, { recursive: true, force: true });
await fs.mkdir(outputDirectory, { recursive: true });

const manifest = [];
const thumbnails = [];
try {
  for (let index = 0; index < 15; index += 1) {
    const result = await page.evaluate(async fixtureIndex => {
      const definition = ScannerSyntheticFixtures.CASES[fixtureIndex];
      const rendered = ScannerSyntheticFixtures.renderCase(definition);
      const detection = ScannerPro.Lightweight.detectPage(rendered.source);
      const geometry = ScannerSyntheticFixtures.evaluateDetection(definition, detection);
      const warped = await ScannerPro.Lightweight.warp(rendered.source, detection.pageQuad, detection.frame);
      const overlay = document.createElement('canvas');
      overlay.width = rendered.source.width;
      overlay.height = rendered.source.height;
      const context = overlay.getContext('2d');
      context.drawImage(rendered.source, 0, 0);
      context.lineCap = 'round';
      context.lineJoin = 'round';
      const drawPath = (points, colour, width, closed = false, dash = []) => {
        if (!points?.length) return;
        context.strokeStyle = colour;
        context.lineWidth = width;
        context.setLineDash(dash);
        context.beginPath();
        context.moveTo(points[0].x, points[0].y);
        points.slice(1).forEach(point => context.lineTo(point.x, point.y));
        if (closed) context.closePath();
        context.stroke();
        context.setLineDash([]);
      };
      const map = ScannerSyntheticFixtures.mappingFor(definition);
      const truthCurve = (coordinate, horizontal) => Array.from({ length: 49 }, (_, sample) => {
        const amount = sample / 48;
        return horizontal ? map(amount, coordinate) : map(coordinate, amount);
      });
      drawPath([map(0, 0), map(1, 0), map(1, 1), map(0, 1)], '#ffffff', 7, true, [14, 10]);
      drawPath(truthCurve(ScannerSyntheticFixtures.FRAME.v0, true), '#ff7a00', 5, false, [11, 7]);
      drawPath(truthCurve(ScannerSyntheticFixtures.FRAME.v1, true), '#ff7a00', 5, false, [11, 7]);
      drawPath(truthCurve(ScannerSyntheticFixtures.FRAME.u0, false), '#ff7a00', 5, false, [11, 7]);
      drawPath(truthCurve(ScannerSyntheticFixtures.FRAME.u1, false), '#ff7a00', 5, false, [11, 7]);
      drawPath(truthCurve(ScannerSyntheticFixtures.FRAME.boxTop, true), '#ff7a00', 4, false, [9, 6]);
      drawPath(truthCurve(ScannerSyntheticFixtures.FRAME.boxBottom, true), '#ff7a00', 4, false, [9, 6]);
      drawPath(detection.pageQuad, '#00e5ff', 4, true);
      if (detection.frame?.paths) Object.values(detection.frame.paths)
        .forEach(points => drawPath(points, '#fff000', 3));
      drawPath(detection.frame?.box?.top, '#36ff78', 3);
      drawPath(detection.frame?.box?.bottom, '#36ff78', 3);
      context.fillStyle = 'rgba(0,0,0,.78)';
      context.fillRect(10, 10, Math.min(overlay.width - 20, 560), 70);
      context.fillStyle = '#fff';
      context.font = '600 24px system-ui';
      context.fillText(`${fixtureIndex + 1}/15 ${definition.name}`, 24, 42);
      context.font = '500 17px system-ui';
      context.fillText(`corner ${(geometry.cornerMaximumRatio * 100).toFixed(2)}% · curve ${(geometry.curveMaximumRatio * 100).toFixed(2)}%`, 24, 68);

      const colourResult = await ScannerPro.Lightweight.correctColors(warped, {
        useStencil: true,
        preciseStencil: true
      });

      const thumb = document.createElement('canvas');
      thumb.width = 250;
      thumb.height = 390;
      const thumbContext = thumb.getContext('2d');
      thumbContext.fillStyle = '#1f1f1f';
      thumbContext.fillRect(0, 0, thumb.width, thumb.height);
      const scale = Math.min(232 / warped.width, 340 / warped.height);
      const width = warped.width * scale;
      const height = warped.height * scale;
      thumbContext.drawImage(warped, (thumb.width - width) / 2, 34, width, height);
      thumbContext.fillStyle = '#fff';
      thumbContext.font = '600 13px system-ui';
      thumbContext.fillText(`${fixtureIndex + 1}. ${definition.name}`, 8, 21);
      return {
        name: definition.name,
        method: detection.method,
        confidence: detection.confidence,
        cornerMaximumRatio: geometry.cornerMaximumRatio,
        mappedCornerMaximumRatio: geometry.mappedCornerMaximumRatio,
        curveMeanRatio: geometry.curveMeanRatio,
        curveMaximumRatio: geometry.curveMaximumRatio,
        frameSupport: detection.frame?.support || 0,
        boxSupport: detection.frame?.box?.support || 0,
        colourCalibrated: colourResult.calibrated,
        colourSamples: colourResult.samples,
        colourReferences: colourResult.references,
        source: rendered.source.toDataURL('image/png'),
        overlay: overlay.toDataURL('image/png'),
        warped: warped.toDataURL('image/png'),
        thumbnail: thumb.toDataURL('image/png')
      };
    }, index);

    const stem = `${String(index + 1).padStart(2, '0')}-${result.name}`;
    await Promise.all(['source', 'overlay', 'warped'].map(kind =>
      fs.writeFile(path.join(outputDirectory, `${stem}-${kind}.png`), toPng(result[kind]))));
    thumbnails.push(result.thumbnail);
    delete result.source;
    delete result.overlay;
    delete result.warped;
    delete result.thumbnail;
    manifest.push(result);
    process.stdout.write(`${stem}: corner=${(result.cornerMaximumRatio * 100).toFixed(2)}%, curve=${(result.curveMaximumRatio * 100).toFixed(2)}%\n`);
  }

  const contactSheet = await page.evaluate(async dataUrls => {
    const images = await Promise.all(dataUrls.map(dataUrl => new Promise((resolve, reject) => {
      const image = new Image();
      image.onload = () => resolve(image);
      image.onerror = reject;
      image.src = dataUrl;
    })));
    const canvas = document.createElement('canvas');
    canvas.width = 5 * 250;
    canvas.height = 3 * 390;
    const context = canvas.getContext('2d');
    context.fillStyle = '#111';
    context.fillRect(0, 0, canvas.width, canvas.height);
    images.forEach((image, index) => context.drawImage(image, (index % 5) * 250, Math.floor(index / 5) * 390));
    return canvas.toDataURL('image/png');
  }, thumbnails);
  await fs.writeFile(path.join(outputDirectory, 'contact-sheet.png'), toPng(contactSheet));
  await fs.writeFile(path.join(outputDirectory, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
} finally {
  await browser.close();
  server?.kill();
}

process.stdout.write(`${outputDirectory}\n`);
