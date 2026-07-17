// Document renderer: owns the page views, the vertical layout, the camera
// CSS transform, the visible-page window and the op→repaint mapping.
//
// Repaints are incremental: a committed stroke paints once; an erase
// repaints only the union bbox of what disappeared (computed from the
// op's inverse — no full-page redraw per eraser pass).

import type { Doc, Box, Page } from '../core/model';
import { computeBbox } from '../core/model';
import type { AppliedOp, DocStore } from '../core/store';
import { deserializeStroke } from '../core/serial';
import type { CameraState, ContentBounds, ViewportSize } from '../viewport/camera';
import { clampPan, clampZoom } from '../viewport/camera';
import { PageView } from './pageView';
import { StrokesIndex } from './strokesIndex';
import type { PageId } from '../core/ids';

export const PAGE_GAP = 28;
export const PAGE_PADDING = 24;
/** Pages beyond the viewport kept allocated on each side. */
const WINDOW_BUFFER = 1;
/** Re-raster resolution = dpr × zoom, clamped. */
const MIN_RASTER = 1;
const MAX_RASTER = 3;

export interface PageLayout {
  pageId: PageId;
  x: number;
  y: number;
  width: number;
  height: number;
}

// Structural CSS the renderer depends on. The camera math assumes the
// container transforms from its top-left corner at the viewport origin,
// pages are absolutely positioned via translate(), and the three canvas
// layers of a page sit exactly on top of each other.
const RENDERER_CSS = `
.canvas-container {
  position: absolute;
  top: 0;
  left: 0;
  transform-origin: 0 0;
  will-change: transform;
}
.canvas-container .page-wrapper {
  position: absolute;
  top: 0;
  left: 0;
  background: #ffffff;
  border-radius: 2px;
  box-shadow: 0 2px 14px rgba(15, 23, 42, 0.18);
}
.canvas-container .page-wrapper canvas {
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  touch-action: none;
}
.canvas-container .page-number {
  position: absolute;
  top: 6px;
  right: 10px;
  font-size: 12px;
  color: rgba(15, 23, 42, 0.45);
  user-select: none;
  pointer-events: none;
  z-index: 1;
}
`;

let rendererCssInjected = false;

function ensureRendererCss(): void {
  if (rendererCssInjected) return;
  const style = document.createElement('style');
  style.dataset.ihnRendererCss = '1';
  style.textContent = RENDERER_CSS;
  document.head.appendChild(style);
  rendererCssInjected = true;
}

export class DocRenderer {
  readonly container: HTMLDivElement;
  camera: CameraState = { zoom: 1, panX: 0, panY: 0 };

  private views = new Map<PageId, PageView>();
  private indices = new Map<PageId, StrokesIndex>();
  private layout: PageLayout[] = [];
  private layoutById = new Map<PageId, PageLayout>();
  private transformRaf = 0;
  private rasterTimer: ReturnType<typeof setTimeout> | null = null;
  private unsubscribe: () => void;

  constructor(
    private viewportEl: HTMLElement,
    private store: DocStore
  ) {
    ensureRendererCss();
    this.container = document.createElement('div');
    this.container.className = 'canvas-container';
    viewportEl.appendChild(this.container);
    this.unsubscribe = store.subscribe((applied) => this.onOp(applied));
    this.rebuild();
  }

  get doc(): Doc {
    return this.store.doc;
  }

  dispose(): void {
    this.unsubscribe();
    for (const view of this.views.values()) view.release();
    this.views.clear();
    this.container.remove();
  }

  // ── Layout ───────────────────────────────────────────────────────────────

  /** Full rebuild: views in doc order, recomputed layout. */
  rebuild(): void {
    const doc = this.doc;
    const seen = new Set<PageId>();
    let pageNumber = 1;
    for (const pageId of doc.pageOrder) {
      seen.add(pageId);
      let view = this.views.get(pageId);
      if (!view) {
        view = new PageView(pageId, () => this.doc.pages.get(pageId), pageNumber);
        this.views.set(pageId, view);
      }
      view.setPageNumber(pageNumber++);
      this.container.appendChild(view.wrapper); // appendChild reorders in place
    }
    for (const [pageId, view] of this.views) {
      if (!seen.has(pageId)) {
        view.release();
        view.wrapper.remove();
        this.views.delete(pageId);
        this.indices.delete(pageId);
      }
    }
    this.relayout();
    this.updateWindow();
  }

  relayout(): void {
    const doc = this.doc;
    let maxWidth = 0;
    for (const pageId of doc.pageOrder) {
      const page = doc.pages.get(pageId);
      if (page) maxWidth = Math.max(maxWidth, page.width);
    }
    let y = PAGE_GAP;
    this.layout = [];
    this.layoutById.clear();
    for (const pageId of doc.pageOrder) {
      const page = doc.pages.get(pageId);
      if (!page) continue;
      const x = PAGE_PADDING + (maxWidth - page.width) / 2;
      const entry: PageLayout = { pageId, x, y, width: page.width, height: page.height };
      this.layout.push(entry);
      this.layoutById.set(pageId, entry);
      const view = this.views.get(pageId);
      if (view) {
        view.wrapper.style.transform = `translate(${x}px, ${y}px)`;
        view.wrapper.style.width = `${page.width}px`;
        view.wrapper.style.height = `${page.height}px`;
      }
      y += page.height + PAGE_GAP;
    }
    this.container.style.width = `${maxWidth + PAGE_PADDING * 2}px`;
    this.container.style.height = `${y}px`;
  }

  contentBounds(): ContentBounds {
    const last = this.layout[this.layout.length - 1];
    const width = parseFloat(this.container.style.width) || 0;
    return {
      left: 0,
      top: 0,
      right: width,
      bottom: last ? last.y + last.height + PAGE_GAP : 0
    };
  }

  viewportSize(): ViewportSize {
    const rect = this.viewportEl.getBoundingClientRect();
    return { width: rect.width, height: rect.height };
  }

  // ── Camera ───────────────────────────────────────────────────────────────

  setCamera(camera: CameraState, opts: { clamp?: boolean } = {}): void {
    const zoomed = { ...camera, zoom: clampZoom(camera.zoom) };
    this.camera = opts.clamp === false ? zoomed : clampPan(zoomed, this.viewportSize(), this.contentBounds());
    if (!this.transformRaf) {
      this.transformRaf = requestAnimationFrame(() => {
        this.transformRaf = 0;
        this.applyTransform();
        this.updateWindow();
      });
    }
    this.scheduleRaster();
  }

  private applyTransform(): void {
    this.container.style.transform = `translate(${this.camera.panX}px, ${this.camera.panY}px) scale(${this.camera.zoom})`;
  }

  /** Re-rasterize visible pages once the zoom level settles. */
  private scheduleRaster(): void {
    if (this.rasterTimer) clearTimeout(this.rasterTimer);
    this.rasterTimer = setTimeout(() => {
      this.rasterTimer = null;
      const scale = this.targetRenderScale();
      for (const pageId of this.visiblePages(0)) {
        this.views.get(pageId)?.setResolution(scale);
      }
    }, 220);
  }

  targetRenderScale(): number {
    const dpr = typeof devicePixelRatio === 'number' ? devicePixelRatio : 1;
    return Math.max(MIN_RASTER, Math.min(MAX_RASTER, dpr * this.camera.zoom));
  }

  /** Page-space position under a viewport-relative screen point. */
  pageAt(screenX: number, screenY: number): { pageId: PageId; x: number; y: number } | null {
    const cx = (screenX - this.camera.panX) / this.camera.zoom;
    const cy = (screenY - this.camera.panY) / this.camera.zoom;
    for (const entry of this.layout) {
      if (
        cx >= entry.x &&
        cx <= entry.x + entry.width &&
        cy >= entry.y &&
        cy <= entry.y + entry.height
      ) {
        return { pageId: entry.pageId, x: cx - entry.x, y: cy - entry.y };
      }
    }
    return null;
  }

  layoutOf(pageId: PageId): PageLayout | undefined {
    return this.layoutById.get(pageId);
  }

  // ── Visible-page window ──────────────────────────────────────────────────

  private visiblePages(buffer = WINDOW_BUFFER): PageId[] {
    const viewport = this.viewportSize();
    const top = (0 - this.camera.panY) / this.camera.zoom;
    const bottom = (viewport.height - this.camera.panY) / this.camera.zoom;
    const visible: number[] = [];
    this.layout.forEach((entry, i) => {
      if (entry.y + entry.height >= top && entry.y <= bottom) visible.push(i);
    });
    if (visible.length === 0) return [];
    const first = Math.max(0, visible[0]! - buffer);
    const last = Math.min(this.layout.length - 1, visible[visible.length - 1]! + buffer);
    const out: PageId[] = [];
    for (let i = first; i <= last; i++) out.push(this.layout[i]!.pageId);
    return out;
  }

  updateWindow(): void {
    const active = new Set(this.visiblePages());
    const scale = this.targetRenderScale();
    for (const [pageId, view] of this.views) {
      if (active.has(pageId)) {
        if (!view.isAllocated()) view.allocate(scale);
      } else if (view.isAllocated()) {
        view.release();
      }
    }
  }

  // ── Op → repaint mapping ─────────────────────────────────────────────────

  view(pageId: PageId): PageView | undefined {
    return this.views.get(pageId);
  }

  index(pageId: PageId): StrokesIndex {
    let index = this.indices.get(pageId);
    if (!index) {
      const page = this.doc.pages.get(pageId);
      index = page ? StrokesIndex.fromPage(page) : new StrokesIndex();
      this.indices.set(pageId, index);
    }
    return index;
  }

  private onOp(applied: AppliedOp): void {
    const { op, inverse } = applied;
    switch (op.type) {
      case 'splice-strokes': {
        const view = this.views.get(op.pageId);
        const page = this.doc.pages.get(op.pageId);
        const index = this.indices.get(op.pageId);

        // Removed strokes: their geometry is in the inverse op.
        if (inverse.type === 'splice-strokes' && inverse.add.length > 0) {
          let union: Box | null = null;
          for (const { stroke } of inverse.add) {
            index?.remove(stroke.id);
            const live = deserializeStroke(stroke);
            const box = {
              x0: live.bbox.x0 - live.width,
              y0: live.bbox.y0 - live.width,
              x1: live.bbox.x1 + live.width,
              y1: live.bbox.y1 + live.width
            };
            union = union ? unionBox(union, box) : box;
          }
          if (view && union) view.redrawRect(union);
        }
        // Added strokes: top-of-stack paints incrementally, otherwise rect.
        if (page && op.add.length > 0) {
          for (const { stroke } of op.add) {
            const live = page.strokes.get(stroke.id);
            if (!live) continue;
            index?.add(live);
            if (!view) continue;
            const isTop = page.strokeOrder[page.strokeOrder.length - 1] === stroke.id;
            if (isTop) {
              view.paintCommittedStroke(live);
            } else {
              view.redrawRect({
                x0: live.bbox.x0 - live.width,
                y0: live.bbox.y0 - live.width,
                x1: live.bbox.x1 + live.width,
                y1: live.bbox.y1 + live.width
              });
            }
          }
        }
        break;
      }
      case 'transform-strokes': {
        const view = this.views.get(op.pageId);
        const page = this.doc.pages.get(op.pageId);
        const index = this.indices.get(op.pageId);
        let union: Box | null = null;
          const extend = (points: ArrayLike<number>, width: number) => {
            const b = computeBbox(points instanceof Float32Array ? points : new Float32Array(Array.from(points)));
          const box = { x0: b.x0 - width, y0: b.y0 - width, x1: b.x1 + width, y1: b.y1 + width };
          union = union ? unionBox(union, box) : box;
        };
        for (const entry of op.entries) {
          const stroke = page?.strokes.get(entry.id);
          if (stroke) {
            index?.add(stroke);
            extend(stroke.points, stroke.width);
          }
        }
        if (inverse.type === 'transform-strokes') {
          for (const entry of inverse.entries) extend(entry.points, 8);
        }
        if (view && union) view.redrawRect(union);
        break;
      }
      case 'splice-images':
      case 'transform-images':
        this.views.get(op.pageId)?.fullRedraw();
        break;
      case 'set-page-background':
        this.views.get(op.pageId)?.fullRedraw();
        break;
      case 'add-page':
      case 'remove-page':
      case 'move-page':
        this.indices.delete(op.type === 'add-page' ? op.page.id : op.pageId);
        this.rebuild();
        break;
      case 'replace-doc':
        this.indices.clear();
        for (const view of this.views.values()) {
          if (view.isAllocated()) view.fullRedraw();
        }
        this.rebuild();
        break;
      case 'set-page-side-panel':
      case 'set-meta':
        break;
    }
  }
}

function unionBox(a: Box, b: Box): Box {
  return {
    x0: Math.min(a.x0, b.x0),
    y0: Math.min(a.y0, b.y0),
    x1: Math.max(a.x1, b.x1),
    y1: Math.max(a.y1, b.y1)
  };
}

export function pageDirtyBox(page: Page): Box {
  return { x0: 0, y0: 0, x1: page.width, y1: page.height };
}
