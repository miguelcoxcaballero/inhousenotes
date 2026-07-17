// One page on screen: a wrapper div with three stacked canvases.
//
//   bg   — template / custom image / (later) PDF raster. Rarely changes.
//   ink  — committed strokes + images. Updated incrementally: a finished
//          stroke is painted once; erases repaint only the dirty rect.
//   live — in-progress stroke tail, eraser cursor, lasso path, selection.
//
// Because layers are separate, the static ink layer is effectively the
// "baked" raster the legacy app wanted (and had to disable) — but vectors
// stay the source of truth, so there is no data-loss risk.

import type { Box, Page, Stroke } from '../core/model';
import { boxesIntersect, paintedBbox } from '../core/model';
import { drawStroke } from './strokePainter';
import { getTemplateCanvas } from './templates';
import { getCachedImage } from './imageCache';
import type { StrokeId } from '../core/ids';

/** Max backing-store pixels per dimension (memory guard on tablets). */
const MAX_BACKING_DIM = 4096;

export class PageView {
  readonly wrapper: HTMLDivElement;
  private bgCanvas: HTMLCanvasElement | null = null;
  private inkCanvas: HTMLCanvasElement | null = null;
  readonly liveCanvas: HTMLCanvasElement;
  private renderScale = 1;
  private allocated = false;
  /** Strokes temporarily hidden during an erase gesture. */
  readonly hiddenStrokes = new Set<StrokeId>();
  /** Stroke ids already painted by the live pen path — skip on commit. */
  private skipPaint = new Set<StrokeId>();

  constructor(
    public readonly pageId: string,
    private getPage: () => Page | undefined,
    private pageNumber: number
  ) {
    this.wrapper = document.createElement('div');
    this.wrapper.className = 'page-wrapper';
    this.wrapper.dataset.pageId = pageId;

    const number = document.createElement('div');
    number.className = 'page-number';
    number.textContent = String(pageNumber);
    this.wrapper.appendChild(number);

    this.liveCanvas = document.createElement('canvas');
    this.liveCanvas.className = 'live-layer';
    this.wrapper.appendChild(this.liveCanvas);
  }

  setPageNumber(n: number): void {
    if (n === this.pageNumber) return;
    this.pageNumber = n;
    const el = this.wrapper.querySelector('.page-number');
    if (el) el.textContent = String(n);
  }

  isAllocated(): boolean {
    return this.allocated;
  }

  /** Create canvases at the given resolution and paint everything. */
  allocate(renderScale: number): void {
    const page = this.getPage();
    if (!page) return;
    this.renderScale = this.clampScale(page, renderScale);
    if (!this.bgCanvas) {
      this.bgCanvas = document.createElement('canvas');
      this.bgCanvas.className = 'bg-layer';
      this.wrapper.insertBefore(this.bgCanvas, this.wrapper.firstChild);
    }
    if (!this.inkCanvas) {
      this.inkCanvas = document.createElement('canvas');
      this.inkCanvas.className = 'ink-layer';
      this.bgCanvas.after(this.inkCanvas);
    }
    this.resizeBacking(page);
    this.allocated = true;
    this.drawBackground(page);
    this.fullInkRedraw(page);
    this.wrapper.style.backgroundImage = '';
  }

  /** Drop canvas memory, leaving a thumbnail on the wrapper. */
  release(): void {
    const page = this.getPage();
    if (this.allocated && page) {
      const preview = this.renderPreview(page);
      if (preview) {
        this.wrapper.style.backgroundImage = `url("${preview}")`;
        this.wrapper.style.backgroundSize = 'cover';
      }
    }
    for (const canvas of [this.bgCanvas, this.inkCanvas]) {
      if (canvas) {
        canvas.width = 0;
        canvas.height = 0;
        canvas.remove();
      }
    }
    this.bgCanvas = null;
    this.inkCanvas = null;
    this.liveCanvas.width = 0;
    this.liveCanvas.height = 0;
    this.allocated = false;
  }

  /** Re-rasterize at a new resolution (after zoom settles). */
  setResolution(renderScale: number): void {
    const page = this.getPage();
    if (!page || !this.allocated) return;
    const clamped = this.clampScale(page, renderScale);
    if (Math.abs(clamped - this.renderScale) < 0.05) return;
    this.renderScale = clamped;
    this.resizeBacking(page);
    this.drawBackground(page);
    this.fullInkRedraw(page);
  }

  getRenderScale(): number {
    return this.renderScale;
  }

  inkContext(): CanvasRenderingContext2D | null {
    if (!this.inkCanvas) return null;
    const ctx = this.inkCanvas.getContext('2d')!;
    ctx.setTransform(this.renderScale, 0, 0, this.renderScale, 0, 0);
    return ctx;
  }

  liveContext(): CanvasRenderingContext2D | null {
    const page = this.getPage();
    if (!page || !this.allocated) return null;
    const ctx = this.liveCanvas.getContext('2d')!;
    ctx.setTransform(this.renderScale, 0, 0, this.renderScale, 0, 0);
    return ctx;
  }

  clearLive(): void {
    const ctx = this.liveCanvas.getContext('2d');
    if (!ctx) return;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, this.liveCanvas.width, this.liveCanvas.height);
  }

  /** Paint a newly committed stroke on top of the ink layer (no clear). */
  paintCommittedStroke(stroke: Stroke): void {
    if (this.skipPaint.delete(stroke.id)) return;
    const ctx = this.inkContext();
    if (!ctx) return;
    drawStroke(ctx, stroke);
  }

  /** The live pen path already painted this stroke — don't repaint it. */
  markPainted(strokeId: StrokeId): void {
    this.skipPaint.add(strokeId);
  }

  fullRedraw(): void {
    const page = this.getPage();
    if (!page || !this.allocated) return;
    this.drawBackground(page);
    this.fullInkRedraw(page);
  }

  /** Repaint only the given page-space rect of the ink layer. */
  redrawRect(box: Box): void {
    const page = this.getPage();
    if (!page || !this.allocated) return;
    const ctx = this.inkContext();
    if (!ctx) return;
    const pad = 2;
    const x = Math.max(0, box.x0 - pad);
    const y = Math.max(0, box.y0 - pad);
    const w = Math.min(page.width, box.x1 + pad) - x;
    const h = Math.min(page.height, box.y1 + pad) - y;
    if (w <= 0 || h <= 0) return;

    ctx.save();
    ctx.beginPath();
    ctx.rect(x, y, w, h);
    ctx.clip();
    ctx.clearRect(x, y, w, h);
    const clip: Box = { x0: x, y0: y, x1: x + w, y1: y + h };
    this.drawImages(ctx, page, clip);
    for (const id of page.strokeOrder) {
      if (this.hiddenStrokes.has(id)) continue;
      const stroke = page.strokes.get(id);
      if (!stroke) continue;
      if (!boxesIntersect(paintedBbox(stroke), clip)) continue;
      drawStroke(ctx, stroke);
    }
    ctx.restore();
  }

  // ── internals ──────────────────────────────────────────────────────────

  private clampScale(page: Page, scale: number): number {
    const maxScale = Math.min(MAX_BACKING_DIM / page.width, MAX_BACKING_DIM / page.height);
    return Math.max(0.5, Math.min(scale, maxScale));
  }

  private resizeBacking(page: Page): void {
    const w = Math.round(page.width * this.renderScale);
    const h = Math.round(page.height * this.renderScale);
    for (const canvas of [this.bgCanvas!, this.inkCanvas!, this.liveCanvas]) {
      canvas.width = w;
      canvas.height = h;
    }
  }

  private drawBackground(page: Page): void {
    if (!this.bgCanvas) return;
    const ctx = this.bgCanvas.getContext('2d')!;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, this.bgCanvas.width, this.bgCanvas.height);
    if (page.background.kind === 'template') {
      const tpl = getTemplateCanvas(page.background.template, this.bgCanvas.width, this.bgCanvas.height);
      ctx.drawImage(tpl, 0, 0);
    } else if (page.background.kind === 'custom') {
      ctx.fillStyle = 'white';
      ctx.fillRect(0, 0, this.bgCanvas.width, this.bgCanvas.height);
      const img = getCachedImage(page.background.src, () => this.drawBackground(page));
      if (img.complete && img.naturalWidth > 0) {
        ctx.drawImage(img, 0, 0, this.bgCanvas.width, this.bgCanvas.height);
      }
    } else {
      // PDF backgrounds arrive in the sync phase (rendered rasters).
      ctx.fillStyle = 'white';
      ctx.fillRect(0, 0, this.bgCanvas.width, this.bgCanvas.height);
    }
    this.drawSidePanel(ctx, page);
  }

  private drawSidePanel(ctx: CanvasRenderingContext2D, page: Page): void {
    if (!page.sidePanel || page.sidePanel.dateKeys.length === 0) return;
    const scale = this.renderScale;
    const panelWidth = Math.min(220, page.width * 0.28) * scale;
    const x = this.bgCanvas!.width - panelWidth;
    ctx.save();
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.fillStyle = 'rgba(248,250,252,0.92)';
    ctx.fillRect(x, 0, panelWidth, this.bgCanvas!.height);
    ctx.strokeStyle = '#cbd5e1';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(x + 0.5, 0);
    ctx.lineTo(x + 0.5, this.bgCanvas!.height);
    ctx.stroke();
    ctx.fillStyle = '#0f172a';
    ctx.font = `${Math.max(14, 18 * scale)}px system-ui, sans-serif`;
    ctx.fillText(page.sidePanel.mode === 'week' ? 'Week' : 'Day', x + 16 * scale, 34 * scale);
    ctx.fillStyle = '#475569';
    ctx.font = `${Math.max(12, 14 * scale)}px system-ui, sans-serif`;
    page.sidePanel.dateKeys.slice(0, 7).forEach((key, index) => {
      ctx.fillText(key, x + 16 * scale, (64 + index * 24) * scale);
    });
    ctx.restore();
  }

  private fullInkRedraw(page: Page): void {
    const ctx = this.inkContext();
    if (!ctx) return;
    ctx.save();
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, this.inkCanvas!.width, this.inkCanvas!.height);
    ctx.restore();
    this.drawImages(ctx, page, null);
    for (const id of page.strokeOrder) {
      if (this.hiddenStrokes.has(id)) continue;
      const stroke = page.strokes.get(id);
      if (stroke) drawStroke(ctx, stroke);
    }
  }

  private drawImages(ctx: CanvasRenderingContext2D, page: Page, clip: Box | null): void {
    for (const id of page.imageOrder) {
      const image = page.images.get(id);
      if (!image) continue;
      if (clip) {
        const half = Math.max(image.width, image.height) / 2;
        const box: Box = {
          x0: image.x - half,
          y0: image.y - half,
          x1: image.x + half,
          y1: image.y + half
        };
        if (!boxesIntersect(box, clip)) continue;
      }
      const img = getCachedImage(image.src, () => this.redrawRect({
        x0: image.x - image.width,
        y0: image.y - image.height,
        x1: image.x + image.width,
        y1: image.y + image.height
      }));
      if (!img.complete || img.naturalWidth === 0) continue;
      ctx.save();
      ctx.translate(image.x, image.y);
      ctx.rotate(image.rotation || 0);
      ctx.globalAlpha = 1;
      ctx.globalCompositeOperation = 'source-over';
      ctx.drawImage(img, -image.width / 2, -image.height / 2, image.width, image.height);
      ctx.restore();
    }
  }

  private renderPreview(page: Page): string | null {
    if (!this.inkCanvas || !this.bgCanvas) return null;
    try {
      const scale = 160 / page.width;
      const canvas = document.createElement('canvas');
      canvas.width = Math.max(1, Math.round(page.width * scale));
      canvas.height = Math.max(1, Math.round(page.height * scale));
      const ctx = canvas.getContext('2d')!;
      ctx.drawImage(this.bgCanvas, 0, 0, canvas.width, canvas.height);
      ctx.drawImage(this.inkCanvas, 0, 0, canvas.width, canvas.height);
      return canvas.toDataURL('image/jpeg', 0.6);
    } catch {
      return null;
    }
  }
}
