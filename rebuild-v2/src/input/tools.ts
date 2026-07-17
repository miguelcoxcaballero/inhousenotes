// Drawing tools. Each tool receives page-space positions from the pointer
// pipeline and produces exactly one op per gesture (one undo step, one
// oplog record, collab-safe ids).
//
// Live-rendering scheme (port of the legacy incremental finalize):
//  - pen: committed prefix is painted straight onto the ink layer as the
//    stroke grows; only the last few points (the "tail") repaint on the
//    live layer each frame.
//  - highlighter: drawn fully on the live layer (alpha), committed once.

import type { DocStore } from '../core/store';
import type { DocRenderer } from '../render/docRenderer';
import { makeStroke, packPoints, POINT_STRIDE } from '../core/model';
import type { Tool as StrokeToolKind, Stroke, Img, Box } from '../core/model';
import {
  simplifyPoints,
  splitStrokeByCircle,
  strokeHitByCircle,
  strokeInLasso,
  strokeIntersectsCircleArea,
  translatePoints
} from '../core/geometry';
import { drawPathSegment, drawStroke, setupStrokeContext } from '../render/strokePainter';
import { serializeStroke } from '../core/serial';
import { newId } from '../core/ids';
import type { PageId, StrokeId } from '../core/ids';

export interface ToolPoint {
  x: number;
  y: number;
  pressure: number;
}

export interface ToolEnv {
  store: DocStore;
  renderer: DocRenderer;
}

export interface DrawTool {
  begin(pageId: PageId, point: ToolPoint): void;
  move(points: ToolPoint[], predicted?: ToolPoint[]): void;
  end(point: ToolPoint | null): void;
  cancel(): void;
}

const LIVE_TAIL_POINTS = 6;
const FINALIZE_STEP = 6;

// ── Pen / highlighter ──────────────────────────────────────────────────────

export class PenTool implements DrawTool {
  private pageId: PageId | null = null;
  private points: number[] = [];
  private finalizedIndex = -1;
  private lastMid: { x: number; y: number } | null = null;

  constructor(
    private env: ToolEnv,
    private getStyle: () => { tool: StrokeToolKind; color: string; width: number }
  ) {}

  begin(pageId: PageId, point: ToolPoint): void {
    this.pageId = pageId;
    this.points = [point.x, point.y, point.pressure];
    this.finalizedIndex = -1;
    this.lastMid = null;
    this.drawLiveTail([]);
  }

  move(points: ToolPoint[], predicted: ToolPoint[] = []): void {
    if (!this.pageId) return;
    const style = this.getStyle();
    for (const p of points) {
      if (style.tool === 'highlighter') {
        // Min-distance filter keeps highlighter bands smooth.
        const n = this.points.length / POINT_STRIDE;
        if (n > 0) {
          const lx = this.points[(n - 1) * 3]!;
          const ly = this.points[(n - 1) * 3 + 1]!;
          const minDist = Math.max(0.6, style.width * 0.12);
          if (Math.hypot(p.x - lx, p.y - ly) < minDist) {
            this.points[(n - 1) * 3] = p.x;
            this.points[(n - 1) * 3 + 1] = p.y;
            this.points[(n - 1) * 3 + 2] = p.pressure;
            continue;
          }
        }
      }
      this.points.push(p.x, p.y, p.pressure);
    }

    if (style.tool === 'pen') this.finalizeIfNeeded();
    this.drawLiveTail(predicted);
  }

  end(point: ToolPoint | null): void {
    if (!this.pageId) return;
    const pageId = this.pageId;
    const style = this.getStyle();
    const view = this.env.renderer.view(pageId);

    if (point) {
      const n = this.points.length / POINT_STRIDE;
      const lx = n > 0 ? this.points[(n - 1) * 3]! : NaN;
      const ly = n > 0 ? this.points[(n - 1) * 3 + 1]! : NaN;
      if (!Number.isFinite(lx) || Math.hypot(point.x - lx, point.y - ly) > 0.5) {
        this.points.push(point.x, point.y, point.pressure);
      }
    }

    view?.clearLive();
    const packed = new Float32Array(this.points);
    this.reset();
    if (packed.length === 0) return;

    const simplified = style.tool === 'pen' ? simplifyPoints(packed) : packed;
    const stroke = makeStroke({ tool: style.tool, color: style.color, width: style.width, points: simplified });

    if (style.tool === 'pen' && view) {
      // Finish painting the tail onto ink with the ORIGINAL points (the
      // pixels for the prefix are already there), then skip the repaint
      // the renderer would do when the op lands.
      const ctx = view.inkContext();
      if (ctx) {
        const n = packed.length / POINT_STRIDE;
        setupStrokeContext(ctx, style.tool, style.color, style.width);
        if (n === 1) {
          drawStroke(ctx, { ...stroke, points: packed });
        } else {
          const mid = drawPathSegment(ctx, packed, Math.max(0, this.lastFinalized), n - 1, this.lastMidSnapshot);
          if (mid) {
            ctx.beginPath();
            ctx.moveTo(mid.x, mid.y);
            ctx.lineTo(packed[(n - 1) * 3]!, packed[(n - 1) * 3 + 1]!);
            ctx.stroke();
          }
          ctx.globalAlpha = 1;
        }
        view.markPainted(stroke.id);
      }
    }

    this.env.store.apply({
      type: 'splice-strokes',
      pageId,
      remove: [],
      add: [{ stroke: serializeStroke(stroke), index: Number.MAX_SAFE_INTEGER }]
    });
  }

  cancel(): void {
    if (this.pageId) {
      const view = this.env.renderer.view(this.pageId);
      view?.clearLive();
      // Prefix segments may already be on the ink layer — repaint cleanly.
      view?.fullRedraw();
    }
    this.reset();
  }

  // Snapshots taken before reset() so end() can finish the tail.
  private lastFinalized = 0;
  private lastMidSnapshot: { x: number; y: number } | null = null;

  private reset(): void {
    this.lastFinalized = Math.max(0, this.finalizedIndex);
    this.lastMidSnapshot = this.lastMid;
    this.pageId = null;
    this.points = [];
    this.finalizedIndex = -1;
    this.lastMid = null;
  }

  private finalizeIfNeeded(): void {
    if (!this.pageId) return;
    const n = this.points.length / POINT_STRIDE;
    const pending = n - 1 - this.finalizedIndex;
    if (pending <= LIVE_TAIL_POINTS + FINALIZE_STEP) return;
    const view = this.env.renderer.view(this.pageId);
    const ctx = view?.inkContext();
    if (!ctx) return;
    const style = this.getStyle();
    const target = n - 1 - LIVE_TAIL_POINTS;
    setupStrokeContext(ctx, style.tool, style.color, style.width);
    const packed = new Float32Array(this.points);
    this.lastMid = drawPathSegment(ctx, packed, Math.max(0, this.finalizedIndex), target, this.lastMid);
    ctx.globalAlpha = 1;
    this.finalizedIndex = target;
  }

  private drawLiveTail(predicted: ToolPoint[]): void {
    if (!this.pageId) return;
    const view = this.env.renderer.view(this.pageId);
    const ctx = view?.liveContext();
    if (!view || !ctx) return;
    view.clearLive();
    const style = this.getStyle();
    const tailPoints = [...this.points];
    for (const p of predicted) tailPoints.push(p.x, p.y, p.pressure);
    const packed = new Float32Array(tailPoints);
    const n = packed.length / POINT_STRIDE;
    if (n === 0) return;
    setupStrokeContext(ctx, style.tool, style.color, style.width);
    if (n === 1) {
      ctx.fillStyle = style.color;
      ctx.beginPath();
      ctx.arc(packed[0]!, packed[1]!, Math.max(0.7, style.width / 2), 0, Math.PI * 2);
      ctx.fill();
      ctx.globalAlpha = 1;
      return;
    }
    if (style.tool === 'highlighter') {
      drawStroke(ctx, makeStroke({ tool: style.tool, color: style.color, width: style.width, points: packed }));
      return;
    }
    const start = Math.max(0, this.finalizedIndex);
    const mid = drawPathSegment(ctx, packed, start, n - 1, this.lastMid ? { ...this.lastMid } : null);
    if (mid) {
      ctx.beginPath();
      ctx.moveTo(mid.x, mid.y);
      ctx.lineTo(packed[(n - 1) * 3]!, packed[(n - 1) * 3 + 1]!);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;
  }
}

// ── Erasers ────────────────────────────────────────────────────────────────

function drawEraserCursor(env: ToolEnv, pageId: PageId, x: number, y: number, radius: number): void {
  const view = env.renderer.view(pageId);
  const ctx = view?.liveContext();
  if (!view || !ctx) return;
  view.clearLive();
  ctx.strokeStyle = 'rgba(0,0,0,0.45)';
  ctx.lineWidth = 1.2;
  ctx.beginPath();
  ctx.arc(x, y, radius, 0, Math.PI * 2);
  ctx.stroke();
  ctx.fillStyle = 'rgba(255,255,255,0.25)';
  ctx.fill();
}

export class EraserStrokeTool implements DrawTool {
  private pageId: PageId | null = null;
  private hit: StrokeId[] = [];

  constructor(
    private env: ToolEnv,
    private getRadius: () => number
  ) {}

  begin(pageId: PageId, point: ToolPoint): void {
    this.pageId = pageId;
    this.hit = [];
    this.eraseAt(point);
  }

  move(points: ToolPoint[]): void {
    for (const p of points) this.eraseAt(p);
  }

  end(): void {
    if (!this.pageId) return;
    const pageId = this.pageId;
    const view = this.env.renderer.view(pageId);
    view?.clearLive();
    const ids = this.hit;
    this.pageId = null;
    this.hit = [];
    if (ids.length === 0) return;
    // Unhide first: the op removes them from the doc, so the repaint the
    // renderer performs will not include them anyway.
    if (view) for (const id of ids) view.hiddenStrokes.delete(id);
    this.env.store.apply({ type: 'splice-strokes', pageId, remove: ids, add: [] });
  }

  cancel(): void {
    if (this.pageId) {
      const view = this.env.renderer.view(this.pageId);
      if (view) {
        for (const id of this.hit) view.hiddenStrokes.delete(id);
        view.clearLive();
        view.fullRedraw();
      }
    }
    this.pageId = null;
    this.hit = [];
  }

  private eraseAt(point: ToolPoint): void {
    if (!this.pageId) return;
    const radius = this.getRadius();
    const page = this.env.store.doc.pages.get(this.pageId);
    const view = this.env.renderer.view(this.pageId);
    if (!page) return;
    const candidates = this.env.renderer.index(this.pageId).queryCircle(point.x, point.y, radius);
    let union: Box | null = null;
    for (const id of candidates) {
      if (view?.hiddenStrokes.has(id)) continue;
      const stroke = page.strokes.get(id);
      if (!stroke || !strokeHitByCircle(stroke, point.x, point.y, radius)) continue;
      this.hit.push(id);
      view?.hiddenStrokes.add(id);
      const box = {
        x0: stroke.bbox.x0 - stroke.width,
        y0: stroke.bbox.y0 - stroke.width,
        x1: stroke.bbox.x1 + stroke.width,
        y1: stroke.bbox.y1 + stroke.width
      };
      union = union
        ? { x0: Math.min(union.x0, box.x0), y0: Math.min(union.y0, box.y0), x1: Math.max(union.x1, box.x1), y1: Math.max(union.y1, box.y1) }
        : box;
    }
    if (view && union) view.redrawRect(union);
    drawEraserCursor(this.env, this.pageId, point.x, point.y, radius);
  }
}

export class EraserAreaTool implements DrawTool {
  private pageId: PageId | null = null;
  /** Working fragments per original stroke (live-split as the gesture moves). */
  private working = new Map<StrokeId, { original: Stroke; fragments: Float32Array[] }>();

  constructor(
    private env: ToolEnv,
    private getRadius: () => number
  ) {}

  begin(pageId: PageId, point: ToolPoint): void {
    this.pageId = pageId;
    this.working.clear();
    this.eraseAt(point);
  }

  move(points: ToolPoint[]): void {
    for (const p of points) this.eraseAt(p);
  }

  end(): void {
    if (!this.pageId) return;
    const pageId = this.pageId;
    const view = this.env.renderer.view(pageId);
    const page = this.env.store.doc.pages.get(pageId);
    view?.clearLive();
    const working = new Map(this.working);
    this.pageId = null;
    this.working.clear();
    if (working.size === 0) return;

    const remove: StrokeId[] = [];
    const add: { stroke: ReturnType<typeof serializeStroke>; index: number }[] = [];
    for (const [id, entry] of working) {
      remove.push(id);
      if (view) view.hiddenStrokes.delete(id);
      const zIndex = page ? page.strokeOrder.indexOf(id) : Number.MAX_SAFE_INTEGER;
      entry.fragments.forEach((points, i) => {
        const fragment = makeStroke({
          id: newId(),
          tool: entry.original.tool,
          color: entry.original.color,
          width: entry.original.width,
          points
        });
        add.push({ stroke: serializeStroke(fragment), index: (zIndex < 0 ? Number.MAX_SAFE_INTEGER : zIndex) + i });
      });
    }
    this.env.store.apply({ type: 'splice-strokes', pageId, remove, add });
  }

  cancel(): void {
    if (this.pageId) {
      const view = this.env.renderer.view(this.pageId);
      if (view) {
        for (const id of this.working.keys()) view.hiddenStrokes.delete(id);
        view.clearLive();
        view.fullRedraw();
      }
    }
    this.pageId = null;
    this.working.clear();
  }

  private eraseAt(point: ToolPoint): void {
    if (!this.pageId) return;
    const radius = this.getRadius();
    const page = this.env.store.doc.pages.get(this.pageId);
    const view = this.env.renderer.view(this.pageId);
    if (!page) return;

    const candidates = this.env.renderer.index(this.pageId).queryCircle(point.x, point.y, radius);
    let changed = false;
    let union: Box | null = null;

    for (const id of candidates) {
      const entry = this.working.get(id);
      if (entry) continue; // already split — handled below over fragments
      const stroke = page.strokes.get(id);
      if (!stroke || !strokeIntersectsCircleArea(stroke.points, point.x, point.y, radius)) continue;
      this.working.set(id, {
        original: stroke,
        fragments: splitStrokeByCircle(stroke.points, point.x, point.y, radius)
      });
      view?.hiddenStrokes.add(id);
      changed = true;
      const box = {
        x0: stroke.bbox.x0 - stroke.width,
        y0: stroke.bbox.y0 - stroke.width,
        x1: stroke.bbox.x1 + stroke.width,
        y1: stroke.bbox.y1 + stroke.width
      };
      union = union
        ? { x0: Math.min(union.x0, box.x0), y0: Math.min(union.y0, box.y0), x1: Math.max(union.x1, box.x1), y1: Math.max(union.y1, box.y1) }
        : box;
    }

    // Re-split existing fragments that the cursor touches.
    for (const entry of this.working.values()) {
      const next: Float32Array[] = [];
      let entryChanged = false;
      for (const fragment of entry.fragments) {
        if (strokeIntersectsCircleArea(fragment, point.x, point.y, radius)) {
          next.push(...splitStrokeByCircle(fragment, point.x, point.y, radius));
          entryChanged = true;
        } else {
          next.push(fragment);
        }
      }
      if (entryChanged) {
        entry.fragments = next;
        changed = true;
      }
    }

    if (view && union) view.redrawRect(union);
    if (view && (changed || true)) this.drawWorking(point, radius);
  }

  private drawWorking(point: ToolPoint, radius: number): void {
    if (!this.pageId) return;
    const view = this.env.renderer.view(this.pageId);
    const ctx = view?.liveContext();
    if (!view || !ctx) return;
    view.clearLive();
    for (const entry of this.working.values()) {
      for (const points of entry.fragments) {
        drawStroke(
          ctx,
          makeStroke({
            tool: entry.original.tool,
            color: entry.original.color,
            width: entry.original.width,
            points
          })
        );
      }
    }
    ctx.strokeStyle = 'rgba(0,0,0,0.45)';
    ctx.lineWidth = 1.2;
    ctx.globalAlpha = 1;
    ctx.beginPath();
    ctx.arc(point.x, point.y, radius, 0, Math.PI * 2);
    ctx.stroke();
  }
}

// ── Lasso ──────────────────────────────────────────────────────────────────

interface LassoSelection {
  pageId: PageId;
  strokeIds: StrokeId[];
  imageIds: string[];
  bbox: Box;
}

export class LassoTool implements DrawTool {
  private pageId: PageId | null = null;
  private polygon: { x: number; y: number }[] = [];
  selection: LassoSelection | null = null;
  private dragStart: { x: number; y: number } | null = null;
  private dragDelta = { x: 0, y: 0 };
  onSelectionChange: (() => void) | null = null;

  constructor(private env: ToolEnv) {}

  begin(pageId: PageId, point: ToolPoint): void {
    if (
      this.selection &&
      this.selection.pageId === pageId &&
      point.x >= this.selection.bbox.x0 &&
      point.x <= this.selection.bbox.x1 &&
      point.y >= this.selection.bbox.y0 &&
      point.y <= this.selection.bbox.y1
    ) {
      // Drag the existing selection.
      this.pageId = pageId;
      this.dragStart = { x: point.x, y: point.y };
      this.dragDelta = { x: 0, y: 0 };
      const view = this.env.renderer.view(pageId);
      if (view) {
        for (const id of this.selection.strokeIds) view.hiddenStrokes.add(id);
        view.redrawRect(this.selection.bbox);
      }
      this.drawDragPreview();
      return;
    }
    this.clearSelection();
    this.pageId = pageId;
    this.polygon = [{ x: point.x, y: point.y }];
    this.drawLassoPath();
  }

  move(points: ToolPoint[]): void {
    if (!this.pageId) return;
    if (this.dragStart) {
      const last = points[points.length - 1];
      if (last) {
        this.dragDelta = { x: last.x - this.dragStart.x, y: last.y - this.dragStart.y };
        this.drawDragPreview();
      }
      return;
    }
    for (const p of points) this.polygon.push({ x: p.x, y: p.y });
    this.drawLassoPath();
  }

  end(): void {
    if (!this.pageId) return;
    const pageId = this.pageId;
    const view = this.env.renderer.view(pageId);

    if (this.dragStart && this.selection) {
      // Commit the move as transform ops.
      const { strokeIds, imageIds } = this.selection;
      const { x: dx, y: dy } = this.dragDelta;
      const page = this.env.store.doc.pages.get(pageId);
      this.dragStart = null;
      if (view) for (const id of strokeIds) view.hiddenStrokes.delete(id);
      view?.clearLive();
      if (page && (dx !== 0 || dy !== 0)) {
        if (strokeIds.length > 0) {
          this.env.store.apply({
            type: 'transform-strokes',
            pageId,
            entries: strokeIds
              .map((id) => {
                const stroke = page.strokes.get(id);
                return stroke ? { id, points: translatePoints(stroke.points, dx, dy) } : null;
              })
              .filter((e): e is { id: StrokeId; points: Float32Array } => !!e)
          });
        }
        if (imageIds.length > 0) {
          this.env.store.apply({
            type: 'transform-images',
            pageId,
            entries: imageIds
              .map((id) => {
                const image = page.images.get(id);
                return image ? { id, after: { ...image, x: image.x + dx, y: image.y + dy } as Img } : null;
              })
              .filter((e): e is { id: string; after: Img } => !!e)
          });
        }
        this.selection = {
          ...this.selection,
          bbox: {
            x0: this.selection.bbox.x0 + dx,
            y0: this.selection.bbox.y0 + dy,
            x1: this.selection.bbox.x1 + dx,
            y1: this.selection.bbox.y1 + dy
          }
        };
      } else if (view) {
        view.fullRedraw();
      }
      this.dragDelta = { x: 0, y: 0 };
      this.pageId = null;
      this.drawSelectionOutline();
      this.onSelectionChange?.();
      return;
    }

    // Close the lasso and select.
    const polygon = this.polygon;
    this.polygon = [];
    this.pageId = null;
    view?.clearLive();
    if (polygon.length < 3) {
      this.clearSelection();
      return;
    }
    const page = this.env.store.doc.pages.get(pageId);
    if (!page) return;

    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const p of polygon) {
      x0 = Math.min(x0, p.x);
      y0 = Math.min(y0, p.y);
      x1 = Math.max(x1, p.x);
      y1 = Math.max(y1, p.y);
    }
    const candidates = this.env.renderer.index(pageId).queryBox(x0, y0, x1, y1);
    const strokeIds: StrokeId[] = [];
    let bbox: Box | null = null;
    for (const id of page.strokeOrder) {
      if (!candidates.has(id)) continue;
      const stroke = page.strokes.get(id);
      if (!stroke || !strokeInLasso(stroke, polygon)) continue;
      strokeIds.push(id);
      bbox = bbox
        ? { x0: Math.min(bbox.x0, stroke.bbox.x0), y0: Math.min(bbox.y0, stroke.bbox.y0), x1: Math.max(bbox.x1, stroke.bbox.x1), y1: Math.max(bbox.y1, stroke.bbox.y1) }
        : { ...stroke.bbox };
    }
    const imageIds: string[] = [];
    for (const id of page.imageOrder) {
      const image = page.images.get(id);
      if (!image) continue;
      const inside = polygon.length >= 3 && strokeInLasso(
        makeStroke({ tool: 'pen', color: '#000', width: 1, points: packPoints([{ x: image.x, y: image.y }]) }),
        polygon
      );
      if (!inside) continue;
      imageIds.push(id);
      const half = Math.max(image.width, image.height) / 2;
      const box = { x0: image.x - half, y0: image.y - half, x1: image.x + half, y1: image.y + half };
      bbox = bbox
        ? { x0: Math.min(bbox.x0, box.x0), y0: Math.min(bbox.y0, box.y0), x1: Math.max(bbox.x1, box.x1), y1: Math.max(bbox.y1, box.y1) }
        : box;
    }

    if ((strokeIds.length > 0 || imageIds.length > 0) && bbox) {
      this.selection = { pageId, strokeIds, imageIds, bbox };
      this.drawSelectionOutline();
    } else {
      this.selection = null;
    }
    this.onSelectionChange?.();
  }

  cancel(): void {
    if (this.pageId) {
      const view = this.env.renderer.view(this.pageId);
      if (view && this.selection && this.dragStart) {
        for (const id of this.selection.strokeIds) view.hiddenStrokes.delete(id);
        view.fullRedraw();
      }
      view?.clearLive();
    }
    this.pageId = null;
    this.polygon = [];
    this.dragStart = null;
  }

  deleteSelection(): void {
    if (!this.selection) return;
    const { pageId, strokeIds, imageIds } = this.selection;
    if (strokeIds.length > 0) {
      this.env.store.apply({ type: 'splice-strokes', pageId, remove: strokeIds, add: [] });
    }
    if (imageIds.length > 0) {
      this.env.store.apply({ type: 'splice-images', pageId, remove: imageIds, add: [] });
    }
    this.clearSelection();
  }

  clearSelection(): void {
    if (this.selection) {
      this.env.renderer.view(this.selection.pageId)?.clearLive();
    }
    this.selection = null;
    this.onSelectionChange?.();
  }

  private drawLassoPath(): void {
    if (!this.pageId) return;
    const view = this.env.renderer.view(this.pageId);
    const ctx = view?.liveContext();
    if (!view || !ctx) return;
    view.clearLive();
    if (this.polygon.length === 0) return;
    ctx.strokeStyle = '#002FD9';
    ctx.lineWidth = 1.5;
    ctx.setLineDash([6, 4]);
    ctx.beginPath();
    ctx.moveTo(this.polygon[0]!.x, this.polygon[0]!.y);
    for (let i = 1; i < this.polygon.length; i++) ctx.lineTo(this.polygon[i]!.x, this.polygon[i]!.y);
    ctx.stroke();
    ctx.setLineDash([]);
  }

  private drawSelectionOutline(): void {
    if (!this.selection) return;
    const view = this.env.renderer.view(this.selection.pageId);
    const ctx = view?.liveContext();
    if (!view || !ctx) return;
    view.clearLive();
    const b = this.selection.bbox;
    ctx.strokeStyle = '#002FD9';
    ctx.lineWidth = 1.5;
    ctx.setLineDash([6, 4]);
    ctx.strokeRect(b.x0 - 6, b.y0 - 6, b.x1 - b.x0 + 12, b.y1 - b.y0 + 12);
    ctx.setLineDash([]);
  }

  private drawDragPreview(): void {
    if (!this.pageId || !this.selection) return;
    const view = this.env.renderer.view(this.pageId);
    const ctx = view?.liveContext();
    if (!view || !ctx) return;
    const page = this.env.store.doc.pages.get(this.pageId);
    if (!page) return;
    view.clearLive();
    const { x: dx, y: dy } = this.dragDelta;
    for (const id of this.selection.strokeIds) {
      const stroke = page.strokes.get(id);
      if (!stroke) continue;
      drawStroke(ctx, { ...stroke, points: translatePoints(stroke.points, dx, dy) });
    }
    const b = this.selection.bbox;
    ctx.strokeStyle = '#002FD9';
    ctx.lineWidth = 1.5;
    ctx.globalAlpha = 1;
    ctx.setLineDash([6, 4]);
    ctx.strokeRect(b.x0 + dx - 6, b.y0 + dy - 6, b.x1 - b.x0 + 12, b.y1 - b.y0 + 12);
    ctx.setLineDash([]);
  }
}

// ── Tool definitions ──────────────────────────────────────────────────────────

import type { Tool as ToolKind } from '../core/model';

export interface ToolDefinition {
  id: string;
  name: string;
  icon: string;
  color: string;
  width: number;
  tool: ToolKind;
}

export const defaultTools: ToolDefinition[] = [
  {
    id: 'pen',
    name: 'Pen',
    icon: '✒️',
    color: '#1a1a1a',
    width: 2,
    tool: 'pen'
  },
  {
    id: 'highlighter',
    name: 'Highlighter',
    icon: '🖍️',
    color: '#FFE066',
    width: 12,
    tool: 'highlighter'
  },
  {
    id: 'eraserStroke',
    name: 'Eraser (Stroke)',
    icon: '⌫',
    color: '#cc2222',
    width: 8,
    tool: 'pen'
  },
  {
    id: 'eraserArea',
    name: 'Eraser (Area)',
    icon: '◻️',
    color: '#cc2222',
    width: 24,
    tool: 'pen'
  },
  {
    id: 'lasso',
    name: 'Lasso',
    icon: '⊝',
    color: '#002FD9',
    width: 1,
    tool: 'pen'
  }
];

/**
 * Applies a tool's visual properties to a stroke.
 * For highlighter, the alpha 0.35 is applied at render time via the stroke
 * painter (setupStrokeContext / drawStroke) using tool.tool === 'highlighter'.
 * This function exists for callers that need to annotate strokes with their
 * effective visual style before storage or transmission.
 */
export function applyToolToStroke(tool: ToolDefinition, stroke: { color: string }): void {
  // Highlighter strokes get an alpha applied at paint time.  The stored stroke
  // retains its base hex color; callers check tool.tool === 'highlighter' to
  // set ctx.globalAlpha = 0.35 before drawing.
}
