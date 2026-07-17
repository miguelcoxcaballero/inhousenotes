// Pure stroke geometry: hit tests for the erasers and lasso, and the
// stroke-splitting used by the area eraser. Operates on packed point
// arrays ([x,y,p] triplets) — no DOM, fully unit-testable.

import { POINT_STRIDE } from './model';
import type { Stroke } from './model';

function distSqPointToSegment(px: number, py: number, ax: number, ay: number, bx: number, by: number): number {
  const abx = bx - ax;
  const aby = by - ay;
  const lenSq = abx * abx + aby * aby;
  let t = 0;
  if (lenSq > 0) {
    t = ((px - ax) * abx + (py - ay) * aby) / lenSq;
    t = Math.max(0, Math.min(1, t));
  }
  const cx = ax + t * abx;
  const cy = ay + t * aby;
  const dx = px - cx;
  const dy = py - cy;
  return dx * dx + dy * dy;
}

/** True when a circle touches the painted stroke (segments + width). */
export function strokeHitByCircle(stroke: Stroke, cx: number, cy: number, radius: number): boolean {
  const pts = stroke.points;
  const n = pts.length / POINT_STRIDE;
  const hitR = radius + stroke.width / 2;
  const hitSq = hitR * hitR;
  if (n === 1) {
    const dx = (pts[0] as number) - cx;
    const dy = (pts[1] as number) - cy;
    return dx * dx + dy * dy <= hitSq;
  }
  for (let i = 0; i < n - 1; i++) {
    const ax = pts[i * 3] as number;
    const ay = pts[i * 3 + 1] as number;
    const bx = pts[(i + 1) * 3] as number;
    const by = pts[(i + 1) * 3 + 1] as number;
    if (distSqPointToSegment(cx, cy, ax, ay, bx, by) <= hitSq) return true;
  }
  return false;
}

/**
 * Remove the part of a stroke inside a circle, returning the surviving
 * fragments as packed point arrays (the area-eraser split). A fragment
 * needs at least 2 points to remain drawable.
 */
export function splitStrokeByCircle(points: Float32Array, cx: number, cy: number, radius: number): Float32Array[] {
  const n = points.length / POINT_STRIDE;
  const rSq = radius * radius;
  const fragments: Float32Array[] = [];
  let current: number[] = [];

  const inside = (i: number): boolean => {
    const dx = (points[i * 3] as number) - cx;
    const dy = (points[i * 3 + 1] as number) - cy;
    return dx * dx + dy * dy <= rSq;
  };

  for (let i = 0; i < n; i++) {
    if (inside(i)) {
      if (current.length >= 2 * POINT_STRIDE) {
        fragments.push(new Float32Array(current));
      }
      current = [];
    } else {
      current.push(points[i * 3] as number, points[i * 3 + 1] as number, points[i * 3 + 2] as number);
    }
  }
  if (current.length >= 2 * POINT_STRIDE) {
    fragments.push(new Float32Array(current));
  }
  return fragments;
}

/** Whether the split would change anything (any point inside the circle). */
export function strokeIntersectsCircleArea(points: Float32Array, cx: number, cy: number, radius: number): boolean {
  const n = points.length / POINT_STRIDE;
  const rSq = radius * radius;
  for (let i = 0; i < n; i++) {
    const dx = (points[i * 3] as number) - cx;
    const dy = (points[i * 3 + 1] as number) - cy;
    if (dx * dx + dy * dy <= rSq) return true;
  }
  return false;
}

export interface Point {
  x: number;
  y: number;
}

export function pointInPolygon(x: number, y: number, polygon: ReadonlyArray<Point>): boolean {
  let inside = false;
  for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i++) {
    const xi = polygon[i]!.x;
    const yi = polygon[i]!.y;
    const xj = polygon[j]!.x;
    const yj = polygon[j]!.y;
    const intersects = yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi;
    if (intersects) inside = !inside;
  }
  return inside;
}

/** A stroke is lassoed when most of its points fall inside the polygon. */
export function strokeInLasso(stroke: Stroke, polygon: ReadonlyArray<Point>): boolean {
  const pts = stroke.points;
  const n = pts.length / POINT_STRIDE;
  if (n === 0 || polygon.length < 3) return false;
  let insideCount = 0;
  for (let i = 0; i < n; i++) {
    if (pointInPolygon(pts[i * 3] as number, pts[i * 3 + 1] as number, polygon)) insideCount++;
  }
  return insideCount / n > 0.5;
}

/** Translate packed points by (dx, dy). Returns a new array. */
export function translatePoints(points: Float32Array, dx: number, dy: number): Float32Array {
  const out = new Float32Array(points);
  for (let i = 0; i < out.length; i += POINT_STRIDE) {
    out[i] = (out[i] as number) + dx;
    out[i + 1] = (out[i + 1] as number) + dy;
  }
  return out;
}

/** Scale packed points around (originX, originY). Returns a new array. */
export function scalePoints(
  points: Float32Array,
  originX: number,
  originY: number,
  scaleX: number,
  scaleY: number
): Float32Array {
  const out = new Float32Array(points);
  for (let i = 0; i < out.length; i += POINT_STRIDE) {
    out[i] = originX + ((out[i] as number) - originX) * scaleX;
    out[i + 1] = originY + ((out[i + 1] as number) - originY) * scaleY;
  }
  return out;
}

/**
 * Ramer–Douglas–Peucker simplification on packed points, preserving
 * pressure. Tolerance in page units; ~0.4 keeps handwriting shape intact
 * while dropping redundant samples from high-rate styluses.
 */
export function simplifyPoints(points: Float32Array, tolerance = 0.4): Float32Array {
  const n = points.length / POINT_STRIDE;
  if (n <= 2) return points;
  const keep = new Uint8Array(n);
  keep[0] = 1;
  keep[n - 1] = 1;
  const tolSq = tolerance * tolerance;
  const stack: [number, number][] = [[0, n - 1]];
  while (stack.length > 0) {
    const [start, end] = stack.pop()!;
    if (end - start < 2) continue;
    const ax = points[start * 3] as number;
    const ay = points[start * 3 + 1] as number;
    const bx = points[end * 3] as number;
    const by = points[end * 3 + 1] as number;
    let maxDistSq = -1;
    let maxIdx = -1;
    for (let i = start + 1; i < end; i++) {
      const d = distSqPointToSegment(points[i * 3] as number, points[i * 3 + 1] as number, ax, ay, bx, by);
      if (d > maxDistSq) {
        maxDistSq = d;
        maxIdx = i;
      }
    }
    if (maxDistSq > tolSq && maxIdx > 0) {
      keep[maxIdx] = 1;
      stack.push([start, maxIdx], [maxIdx, end]);
    }
  }
  let kept = 0;
  for (let i = 0; i < n; i++) if (keep[i]) kept++;
  const out = new Float32Array(kept * POINT_STRIDE);
  let w = 0;
  for (let i = 0; i < n; i++) {
    if (!keep[i]) continue;
    out[w++] = points[i * 3] as number;
    out[w++] = points[i * 3 + 1] as number;
    out[w++] = points[i * 3 + 2] as number;
  }
  return out;
}
