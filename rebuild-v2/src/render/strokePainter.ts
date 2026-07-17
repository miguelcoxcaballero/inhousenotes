// Canvas 2D stroke painting (port of the legacy smooth quadratic-midpoint
// renderer, adapted to packed point arrays). Shared by the static ink
// layer, the live preview and thumbnails.

import { POINT_STRIDE } from '../core/model';
import type { Stroke, Tool } from '../core/model';

export function setupStrokeContext(ctx: CanvasRenderingContext2D, tool: Tool, color: string, width: number): void {
  ctx.globalCompositeOperation = 'source-over';
  ctx.globalAlpha = tool === 'highlighter' ? 0.35 : 1;
  ctx.strokeStyle = color;
  ctx.lineWidth = width;
  ctx.lineCap = tool === 'highlighter' ? 'butt' : 'round';
  ctx.lineJoin = 'round';
}

function drawDot(ctx: CanvasRenderingContext2D, x: number, y: number, color: string, width: number): void {
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.arc(x, y, Math.max(0.7, width * 0.5), 0, Math.PI * 2);
  ctx.fill();
}

/**
 * Draw the path through points [from..to] (inclusive indices) using the
 * midpoint quadratic scheme. Assumes the context is already set up.
 * Returns the last midpoint, so live drawing can continue incrementally.
 */
export function drawPathSegment(
  ctx: CanvasRenderingContext2D,
  points: Float32Array,
  from: number,
  to: number,
  startMid: { x: number; y: number } | null
): { x: number; y: number } | null {
  const px = (i: number) => points[i * POINT_STRIDE] as number;
  const py = (i: number) => points[i * POINT_STRIDE + 1] as number;
  const n = to - from + 1;
  if (n <= 0) return startMid;
  if (n === 1 && !startMid) return null;

  ctx.beginPath();
  let mid = startMid;
  let i = from;
  if (!mid) {
    // First segment: line from p0 to the first midpoint.
    const mx = (px(from) + px(from + 1)) / 2;
    const my = (py(from) + py(from + 1)) / 2;
    ctx.moveTo(px(from), py(from));
    ctx.lineTo(mx, my);
    mid = { x: mx, y: my };
    i = from + 1;
  } else {
    ctx.moveTo(mid.x, mid.y);
  }
  for (; i < to; i++) {
    const mx = (px(i) + px(i + 1)) / 2;
    const my = (py(i) + py(i + 1)) / 2;
    ctx.quadraticCurveTo(px(i), py(i), mx, my);
    mid = { x: mx, y: my };
  }
  ctx.stroke();
  return mid;
}

/** Draw a full committed stroke. */
export function drawStroke(ctx: CanvasRenderingContext2D, stroke: Stroke): void {
  const n = stroke.points.length / POINT_STRIDE;
  if (n === 0) return;
  setupStrokeContext(ctx, stroke.tool, stroke.color, stroke.width);
  if (n === 1) {
    drawDot(ctx, stroke.points[0] as number, stroke.points[1] as number, stroke.color, stroke.width);
    ctx.globalAlpha = 1;
    return;
  }
  const mid = drawPathSegment(ctx, stroke.points, 0, n - 1, null);
  // Close the tail: line from last midpoint to the final point.
  if (mid) {
    ctx.beginPath();
    ctx.moveTo(mid.x, mid.y);
    ctx.lineTo(stroke.points[(n - 1) * 3] as number, stroke.points[(n - 1) * 3 + 1] as number);
    ctx.stroke();
  }
  ctx.globalAlpha = 1;
}
