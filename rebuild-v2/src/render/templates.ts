// Page template backgrounds (port of the legacy native-canvas renderer).
// Rendered once per (kind, size) and cached as an offscreen canvas, then a
// single drawImage per page redraw.

import type { TemplateKind } from '../core/model';

const cache = new Map<string, HTMLCanvasElement>();

function renderDefaultTemplate(width: number, height: number): HTMLCanvasElement {
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext('2d')!;

  ctx.fillStyle = 'white';
  ctx.fillRect(0, 0, width, height);

  const u = width / 21;
  const strokeColor = '#f0db4c';
  const strokeW = 0.06 * u;
  const cornerR = 0.03 * u;

  // Outer frame
  ctx.strokeStyle = strokeColor;
  ctx.lineWidth = strokeW;
  ctx.beginPath();
  ctx.roundRect(1.5 * u, 1.43 * u, 18 * u, 27 * u, cornerR);
  ctx.stroke();

  // Dot grid stamped from a small radial-gradient sprite.
  const dotR = 0.035 * u;
  const blurSigma = 0.02 * u;
  const stampR = dotR + blurSigma;
  const stampSize = Math.ceil(stampR * 2) + 2;
  const stamp = document.createElement('canvas');
  stamp.width = stampSize;
  stamp.height = stampSize;
  const sCtx = stamp.getContext('2d')!;
  const sc = stampSize / 2;
  const grad = sCtx.createRadialGradient(sc, sc, 0, sc, sc, stampR);
  grad.addColorStop(0, '#a8a8a8');
  grad.addColorStop(Math.max(0, (dotR - blurSigma) / stampR), '#a8a8a8');
  grad.addColorStop(1, 'rgba(168,168,168,0)');
  sCtx.fillStyle = grad;
  sCtx.beginPath();
  sCtx.arc(sc, sc, stampR, 0, Math.PI * 2);
  sCtx.fill();

  for (let col = 0; col < 35; col++) {
    const cx = (2.0 + col * 0.5) * u;
    for (let row = 0; row < 51; row++) {
      const cy = (1.93 + row * 0.5) * u;
      ctx.drawImage(stamp, cx - sc, cy - sc);
    }
  }

  // Footer rectangles + ticks
  const lineY = 27.43 * u;
  const lineH = 0.5 * u;
  ctx.strokeStyle = strokeColor;
  ctx.lineWidth = strokeW;
  ctx.beginPath();
  ctx.roundRect(2.0 * u, lineY, 8.0 * u, lineH, cornerR);
  ctx.stroke();
  ctx.beginPath();
  ctx.roundRect(11.0 * u, lineY, 8.0 * u, lineH, cornerR);
  ctx.stroke();
  ctx.beginPath();
  for (let x = 2.5; x <= 9.5 + 1e-9; x += 0.5) {
    ctx.moveTo(x * u, lineY);
    ctx.lineTo(x * u, lineY + lineH);
  }
  for (let x = 11.5; x <= 18.5 + 1e-9; x += 0.5) {
    ctx.moveTo(x * u, lineY);
    ctx.lineTo(x * u, lineY + lineH);
  }
  ctx.stroke();

  return canvas;
}

function renderDiaryTemplate(width: number, height: number): HTMLCanvasElement {
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext('2d')!;
  ctx.fillStyle = 'white';
  ctx.fillRect(0, 0, width, height);
  ctx.strokeStyle = '#666';
  ctx.lineWidth = Math.max(0.7, width / 1100);
  const marginX = width * 0.093;
  ctx.beginPath();
  ctx.moveTo(width * 0.062, height * 0.124);
  ctx.lineTo(width * 0.938, height * 0.124);
  for (let y = height * 0.19; y <= height * 0.92; y += height * 0.0208) {
    ctx.moveTo(marginX, y);
    ctx.lineTo(width - marginX, y);
  }
  ctx.stroke();
  return canvas;
}

function renderAgendaTemplate(width: number, height: number): HTMLCanvasElement {
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext('2d')!;
  ctx.fillStyle = 'white';
  ctx.fillRect(0, 0, width, height);

  const black = '#000';
  const grey = '#666';
  const green = '#d9ead3';
  const leftX = width * 0.06;
  const rightX = width * 0.67;
  const leftW = width * 0.58;
  const rightW = width * 0.27;
  ctx.lineWidth = Math.max(0.7, width / 1100);
  ctx.strokeStyle = black;

  ctx.beginPath();
  ctx.moveTo(width * 0.063, height * 0.128);
  ctx.lineTo(width * 0.94, height * 0.128);
  ctx.moveTo(width * 0.063, height * 0.91);
  ctx.lineTo(width * 0.94, height * 0.91);
  ctx.stroke();

  ctx.fillStyle = green;
  roundRect(ctx, rightX, height * 0.158, rightW, height * 0.174, 10);
  ctx.fill();
  ctx.strokeStyle = black;
  ctx.stroke();

  const sections = [0.188, 0.363, 0.538];
  for (const top of sections) {
    ctx.strokeStyle = black;
    ctx.beginPath();
    ctx.moveTo(leftX, height * top);
    ctx.lineTo(leftX + leftW, height * top);
    ctx.stroke();
    ctx.strokeStyle = grey;
    ctx.beginPath();
    for (let i = 1; i <= 5; i++) {
      const y = height * top + i * height * 0.0208;
      ctx.moveTo(leftX + width * 0.02, y);
      ctx.lineTo(leftX + leftW - width * 0.02, y);
    }
    ctx.stroke();
  }

  const rightSections = [0.363, 0.538, 0.713];
  for (const top of rightSections) {
    ctx.strokeStyle = black;
    ctx.beginPath();
    ctx.moveTo(rightX, height * top);
    ctx.lineTo(rightX + rightW, height * top);
    ctx.stroke();
    ctx.strokeStyle = grey;
    ctx.beginPath();
    for (let i = 1; i <= 5; i++) {
      const y = height * top + i * height * 0.0208;
      ctx.moveTo(rightX + width * 0.025, y);
      ctx.lineTo(rightX + rightW - width * 0.025, y);
    }
    ctx.stroke();
  }
  return canvas;
}

export function getTemplateCanvas(kind: TemplateKind, width: number, height: number): HTMLCanvasElement {
  const key = `${kind}|${width}x${height}`;
  let canvas = cache.get(key);
  if (!canvas) {
    if (kind === 'agenda') canvas = renderAgendaTemplate(width, height);
    else if (kind === 'diary') canvas = renderDiaryTemplate(width, height);
    else canvas = renderDefaultTemplate(width, height);
    cache.set(key, canvas);
    // Bound the cache: sizes change with zoom re-rasters.
    if (cache.size > 12) {
      const first = cache.keys().next().value;
      if (first) cache.delete(first);
    }
  }
  return canvas;
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number): void {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.lineTo(x + w - r, y);
  ctx.quadraticCurveTo(x + w, y, x + w, y + r);
  ctx.lineTo(x + w, y + h - r);
  ctx.quadraticCurveTo(x + w, y + h, x + w - r, y + h);
  ctx.lineTo(x + r, y + h);
  ctx.quadraticCurveTo(x, y + h, x, y + h - r);
  ctx.lineTo(x, y + r);
  ctx.quadraticCurveTo(x, y, x + r, y);
}
