import { chromium } from '@playwright/test';
import fs from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';

const root = path.resolve(import.meta.dirname, '..');
const outputDirectory = path.join(root, 'test-results', 'scanner-visual');
const inputs = process.argv.slice(2).filter(value => !value.startsWith('--'));
const debugPaths = process.argv.includes('--debug-paths');

if (!inputs.length) {
  process.stderr.write('Usage: node tests/scanner-visual-validation.mjs <photo.jpg> [...photo.jpg]\n');
  process.exit(2);
}

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
await page.goto('http://127.0.0.1:4173/scanner/index.html?embed=1&visual-validation=1', {
  waitUntil: 'domcontentloaded'
});
await page.waitForFunction(() => typeof ScannerPro?.Lightweight?.detectPage === 'function');
await fs.mkdir(outputDirectory, { recursive: true });

const manifest = [];
try {
  for (let index = 0; index < inputs.length; index += 1) {
    const input = path.resolve(inputs[index]);
    const encoded = await fs.readFile(input, 'base64');
    const extension = path.extname(input).toLowerCase();
    const mime = extension === '.png' ? 'image/png' : 'image/jpeg';
    const result = await page.evaluate(async ({ dataUrl, label, debugPaths }) => {
      const image = new Image();
      image.src = dataUrl;
      await image.decode();
      const source = document.createElement('canvas');
      source.width = image.naturalWidth;
      source.height = image.naturalHeight;
      source.getContext('2d', { willReadFrequently: true }).drawImage(image, 0, 0);
      const detection = ScannerPro.Lightweight.detectPage(source);
      const warped = await ScannerPro.Lightweight.warp(source, detection.pageQuad, detection.frame);
      const colourResult = await ScannerPro.Lightweight.correctColors(warped, {
        useStencil: detection.method === 'marker-guided' || detection.method === 'stencil',
        preciseStencil: detection.method === 'marker-guided' || detection.method === 'stencil'
      });

      const previewScale = Math.min(1, 1100 / Math.max(source.width, source.height));
      const overlay = document.createElement('canvas');
      overlay.width = Math.round(source.width * previewScale);
      overlay.height = Math.round(source.height * previewScale);
      const context = overlay.getContext('2d');
      context.drawImage(source, 0, 0, overlay.width, overlay.height);
      context.lineJoin = 'round';
      context.lineCap = 'round';
      const drawPath = (points, colour, width, closed = false) => {
        if (!points?.length) return;
        context.strokeStyle = colour;
        context.lineWidth = width;
        context.beginPath();
        context.moveTo(points[0].x * previewScale, points[0].y * previewScale);
        points.slice(1).forEach(point => context.lineTo(point.x * previewScale, point.y * previewScale));
        if (closed) context.closePath();
        context.stroke();
      };
      drawPath(detection.pageQuad, '#00e5ff', 4, true);
      drawPath(detection.stencilQuad, '#ff2bd6', 4, true);
      if (detection.frame?.paths) {
        Object.values(detection.frame.paths).forEach(points => drawPath(points, '#fff000', 5));
      }
      drawPath(detection.frame?.box?.top, '#53ff78', 4);
      drawPath(detection.frame?.box?.bottom, '#53ff78', 4);
      context.fillStyle = 'rgba(0,0,0,.76)';
      context.fillRect(8, 8, Math.min(overlay.width - 16, 520), 58);
      context.fillStyle = '#fff';
      context.font = '600 22px system-ui';
      context.fillText(`${label}: ${detection.method} ${(detection.confidence || 0).toFixed(3)}`, 20, 43);

      const warpedPreview = document.createElement('canvas');
      const warpedScale = Math.min(1, 1200 / warped.height);
      warpedPreview.width = Math.round(warped.width * warpedScale);
      warpedPreview.height = Math.round(warped.height * warpedScale);
      warpedPreview.getContext('2d').drawImage(warped, 0, 0, warpedPreview.width, warpedPreview.height);
      let debugWarped = null;
      if (debugPaths && detection.frame?.paths) {
        const marked = document.createElement('canvas');
        marked.width = source.width;
        marked.height = source.height;
        const markedContext = marked.getContext('2d');
        markedContext.drawImage(source, 0, 0);
        markedContext.lineWidth = Math.max(8, Math.max(source.width, source.height) * .006);
        markedContext.lineCap = 'round';
        markedContext.lineJoin = 'round';
        const mark = (points, colour) => {
          markedContext.strokeStyle = colour;
          markedContext.beginPath();
          markedContext.moveTo(points[0].x, points[0].y);
          points.slice(1).forEach(point => markedContext.lineTo(point.x, point.y));
          markedContext.stroke();
        };
        mark(detection.frame.paths.top, '#ff00ff');
        mark(detection.frame.paths.right, '#ff1800');
        mark(detection.frame.paths.bottom, '#00ff3c');
        mark(detection.frame.paths.left, '#00c8ff');
        if (detection.frame.box?.top) mark(detection.frame.box.top, '#ff7a00');
        if (detection.frame.box?.bottom) mark(detection.frame.box.bottom, '#8a2bff');
        const debugOutput = await ScannerPro.Lightweight.warp(marked, detection.pageQuad, detection.frame);
        const debugPreview = document.createElement('canvas');
        debugPreview.width = warpedPreview.width;
        debugPreview.height = warpedPreview.height;
        debugPreview.getContext('2d').drawImage(debugOutput, 0, 0, debugPreview.width, debugPreview.height);
        debugWarped = debugPreview.toDataURL('image/png');
      }
      return {
        width: source.width,
        height: source.height,
        method: detection.method,
        confidence: detection.confidence,
        pageQuad: detection.pageQuad,
        stencilQuad: detection.stencilQuad,
        frameSupport: detection.frame?.support || 0,
        boxSupport: detection.frame?.box?.support || 0,
        refinementMovement: detection.frame?.refinementMovement || 0,
        refinementArmSupport: detection.frame?.refinementArmSupport || 0,
        interiorTrace: detection.frame?.interiorTrace || null,
        markerCalibration: detection.markerCalibration || null,
        colourCalibrated: colourResult.calibrated,
        colourSamples: colourResult.samples,
        colourReferences: colourResult.references,
        overlay: overlay.toDataURL('image/png'),
        warped: warpedPreview.toDataURL('image/png'),
        debugWarped
      };
    }, { dataUrl: `data:${mime};base64,${encoded}`, label: path.basename(input), debugPaths });

    const stem = `${String(index + 1).padStart(2, '0')}-${path.basename(input, extension)}`;
    await fs.writeFile(path.join(outputDirectory, `${stem}-overlay.png`), toPng(result.overlay));
    await fs.writeFile(path.join(outputDirectory, `${stem}-warped.png`), toPng(result.warped));
    if (result.debugWarped) {
      await fs.writeFile(path.join(outputDirectory, `${stem}-warped-paths.png`), toPng(result.debugWarped));
    }
    delete result.overlay;
    delete result.warped;
    delete result.debugWarped;
    manifest.push({ input, ...result });
    process.stdout.write(`${stem}: ${result.method}, frame=${result.frameSupport.toFixed(3)}, box=${result.boxSupport.toFixed(3)}\n`);
  }
  await fs.writeFile(path.join(outputDirectory, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
} finally {
  await browser.close();
  server?.kill();
}

process.stdout.write(`${outputDirectory}\n`);
