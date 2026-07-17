// Domain model. No DOM, no IO — every mutation goes through ops.ts.
//
// Stroke points are packed as Float32Array [x0,y0,p0, x1,y1,p1, ...]:
// 3 floats per point. This is ~60% smaller than {x,y,p} object arrays,
// serializes structurally into IndexedDB, and iterates cache-friendly.

import { newId } from './ids';
import type { DocId, PageId, StrokeId, ImageId } from './ids';

export const POINT_STRIDE = 3;

export type Tool = 'pen' | 'highlighter';

export interface Box {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

export interface Stroke {
  id: StrokeId;
  tool: Tool;
  color: string;
  width: number;
  /** Packed [x,y,p] triplets. Treat as immutable once committed. */
  points: Float32Array;
  bbox: Box;
}

export interface Img {
  id: ImageId;
  src: string;
  x: number;
  y: number;
  width: number;
  height: number;
  rotation: number;
}

export type TemplateKind = 'default' | 'agenda' | 'diary';

export type Background =
  | { kind: 'template'; template: TemplateKind }
  | { kind: 'pdf'; pdfPageIndex: number }
  | { kind: 'custom'; src: string };

export interface SidePanelConfig {
  mode: 'day' | 'week';
  dateKeys: string[];
}

export interface Page {
  id: PageId;
  width: number;
  height: number;
  background: Background;
  /** Draw order (z-order) of strokes; entries index into `strokes`. */
  strokeOrder: StrokeId[];
  strokes: Map<StrokeId, Stroke>;
  imageOrder: ImageId[];
  images: Map<ImageId, Img>;
  /** Ids deleted on this device — used by the collaborative merge so a
   *  deletion wins over a concurrent remote copy of the same element. */
  tombstones: Set<string>;
  sidePanel: SidePanelConfig | null;
}

export interface CalendarPageConfig {
  mode: 'day' | 'week';
  startDateKey: string | null;
  startPage: number;
  nextDateKey: string | null;
}

export interface DocMeta {
  name: string;
  calendarPageConfig: CalendarPageConfig | null;
}

export interface Doc {
  id: DocId;
  /** Monotonic local revision; bumps on every applied op. */
  rev: number;
  pageOrder: PageId[];
  pages: Map<PageId, Page>;
  /** Ids of pages deleted on this device (for the collaborative merge). */
  pageTombstones: Set<PageId>;
  meta: DocMeta;
}

export const A4_WIDTH = 794;
export const A4_HEIGHT = 1123;
export const TOMBSTONE_CAP = 4000;

// ── Constructors ──────────────────────────────────────────────────────────

export function createPage(init: Partial<Omit<Page, 'strokes' | 'images'>> = {}): Page {
  return {
    id: init.id ?? newId(),
    width: init.width ?? A4_WIDTH,
    height: init.height ?? A4_HEIGHT,
    background: init.background ?? { kind: 'template', template: 'default' },
    strokeOrder: init.strokeOrder ?? [],
    strokes: new Map(),
    imageOrder: init.imageOrder ?? [],
    images: new Map(),
    tombstones: init.tombstones ?? new Set(),
    sidePanel: init.sidePanel ?? null
  };
}

export function createDoc(init: { id?: DocId; name?: string; pages?: Page[] } = {}): Doc {
  const pages = init.pages ?? [createPage()];
  return {
    id: init.id ?? newId(),
    rev: 0,
    pageOrder: pages.map((p) => p.id),
    pages: new Map(pages.map((p) => [p.id, p])),
    pageTombstones: new Set(),
    meta: { name: init.name ?? 'cuaderno', calendarPageConfig: null }
  };
}

// ── Points & bbox ─────────────────────────────────────────────────────────

export function packPoints(points: ReadonlyArray<{ x: number; y: number; p?: number }>): Float32Array {
  const out = new Float32Array(points.length * POINT_STRIDE);
  for (let i = 0; i < points.length; i++) {
    const pt = points[i]!;
    out[i * 3] = pt.x;
    out[i * 3 + 1] = pt.y;
    out[i * 3 + 2] = pt.p ?? 0.5;
  }
  return out;
}

export function pointCount(points: Float32Array): number {
  return points.length / POINT_STRIDE;
}

export function computeBbox(points: Float32Array): Box {
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (let i = 0; i < points.length; i += POINT_STRIDE) {
    const x = points[i] as number;
    const y = points[i + 1] as number;
    if (x < x0) x0 = x;
    if (y < y0) y0 = y;
    if (x > x1) x1 = x;
    if (y > y1) y1 = y;
  }
  if (!Number.isFinite(x0)) return { x0: 0, y0: 0, x1: 0, y1: 0 };
  return { x0, y0, x1, y1 };
}

/** Stroke bbox inflated by half the stroke width (the painted extent). */
export function paintedBbox(stroke: Stroke): Box {
  const r = stroke.width / 2 + 1;
  return {
    x0: stroke.bbox.x0 - r,
    y0: stroke.bbox.y0 - r,
    x1: stroke.bbox.x1 + r,
    y1: stroke.bbox.y1 + r
  };
}

export function boxesIntersect(a: Box, b: Box): boolean {
  return a.x0 <= b.x1 && a.x1 >= b.x0 && a.y0 <= b.y1 && a.y1 >= b.y0;
}

export function makeStroke(init: {
  id?: StrokeId;
  tool: Tool;
  color: string;
  width: number;
  points: Float32Array;
}): Stroke {
  return {
    id: init.id ?? newId(),
    tool: init.tool,
    color: init.color,
    width: init.width,
    points: init.points,
    bbox: computeBbox(init.points)
  };
}

export function cloneStroke(s: Stroke): Stroke {
  return { ...s, points: new Float32Array(s.points), bbox: { ...s.bbox } };
}

export function addTombstones(page: Page, ids: Iterable<string>): void {
  for (const id of ids) page.tombstones.add(id);
  // Cap so long-lived documents don't accumulate unbounded delete records.
  // Oldest entries are dropped first (Set preserves insertion order).
  if (page.tombstones.size > TOMBSTONE_CAP) {
    const excess = page.tombstones.size - TOMBSTONE_CAP;
    let i = 0;
    for (const id of page.tombstones) {
      if (i++ >= excess) break;
      page.tombstones.delete(id);
    }
  }
}

// ── Lookups ───────────────────────────────────────────────────────────────

export function getPage(doc: Doc, pageId: PageId): Page {
  const page = doc.pages.get(pageId);
  if (!page) throw new Error(`Page not found: ${pageId}`);
  return page;
}

export function pageIndex(doc: Doc, pageId: PageId): number {
  return doc.pageOrder.indexOf(pageId);
}
