// Pointer event pipeline: turns raw pointer events into page-space stroke
// callbacks, with coalesced + predicted event support.
//
// Routing (see router.ts): pen and left-button mouse always draw; a finger
// draws only on devices where a pen has never been seen, and hands over to
// the pinch gesture when a second finger lands. Coordinates are converted
// to viewport-relative client space — pageAt() expects them that way.

import type { PageId } from '../core/ids';
import { InputRouter, isUiEventTarget } from './router';

export interface PointerPoint {
  x: number;
  y: number;
  pressure: number;
}

export interface PointerPipelineCallbacks {
  onStrokeStart(pageId: PageId, point: PointerPoint): void;
  onStrokeMove(pageId: PageId, points: PointerPoint[], predicted?: PointerPoint[]): void;
  onStrokeEnd(pageId: PageId, point: PointerPoint | null): void;
  /** Discard the stroke in progress (e.g. a touch-draw became a pinch). */
  onStrokeCancel(): void;
}

type PageHit = { pageId: PageId; x: number; y: number } | null;

export class PointerPipeline {
  private readonly el: HTMLElement;
  private readonly cb: PointerPipelineCallbacks;
  private pageAtViewport: ((vx: number, vy: number) => PageHit) | null = null;

  private drawingPointerId: number | null = null;
  private drawingIsTouch = false;
  private activeTouchIds = new Set<number>();
  private lastPageId: PageId | null = null;

  constructor(
    viewportEl: HTMLElement,
    callbacks: PointerPipelineCallbacks,
    private router: InputRouter = new InputRouter()
  ) {
    this.el = viewportEl;
    this.cb = callbacks;
    this.el.addEventListener('pointerdown', this.onPointerDown, { passive: false });
    this.el.addEventListener('pointermove', this.onPointerMove, { passive: false });
    this.el.addEventListener('pointerup', this.onPointerUp, { passive: false });
    this.el.addEventListener('pointercancel', this.onPointerCancel, { passive: false });
    // Deliberately NO pointerleave handler: with pointer capture active the
    // stroke must survive brief excursions outside the viewport (legacy bug
    // C4 — strokes were cut when the stylus grazed the edge).
  }

  /** Supply the renderer hit-test after construction. */
  setRenderer(pageAt: (viewportX: number, viewportY: number) => PageHit): void {
    this.pageAtViewport = pageAt;
  }

  private hit(e: { clientX: number; clientY: number }): PageHit {
    if (!this.pageAtViewport) return null;
    const rect = this.el.getBoundingClientRect();
    return this.pageAtViewport(e.clientX - rect.left, e.clientY - rect.top);
  }

  private onPointerDown = (e: PointerEvent): void => {
    if (isUiEventTarget(e.target)) return;
    if (e.pointerType === 'touch') {
      this.activeTouchIds.add(e.pointerId);
      // A second finger while touch-drawing → discard the stroke and let
      // the gesture handler take over with a pinch.
      if (this.drawingIsTouch && this.activeTouchIds.size > 1) {
        this.cancelStroke();
        return;
      }
      if (this.router.penEverSeen) return; // fingers pan on pen devices
      if (this.activeTouchIds.size > 1) return;
    } else if (e.pointerType === 'pen') {
      this.router.notePen();
    } else if (e.button !== 0) {
      return; // mouse: left button only
    }
    if (this.drawingPointerId !== null) return;

    const result = this.hit(e);
    if (!result) return;

    this.drawingPointerId = e.pointerId;
    this.drawingIsTouch = e.pointerType === 'touch';
    this.lastPageId = result.pageId;
    this.router.claim(e.pointerId);
    if (e.pointerType === 'pen') this.router.penDrawing = true;
    try {
      this.el.setPointerCapture(e.pointerId);
    } catch {
      // Synthetic events (tests) have no active pointer to capture.
    }
    e.preventDefault();

    this.cb.onStrokeStart(result.pageId, { x: result.x, y: result.y, pressure: e.pressure || 0.5 });
  };

  private onPointerMove = (e: PointerEvent): void => {
    if (e.pointerId !== this.drawingPointerId || !this.lastPageId) return;
    const pageId = this.lastPageId;
    e.preventDefault();

    const coalesced = typeof e.getCoalescedEvents === 'function' ? e.getCoalescedEvents() : [];
    const events: { clientX: number; clientY: number; pressure: number }[] =
      coalesced.length > 0 ? coalesced : [e];
    const points: PointerPoint[] = [];
    for (const ev of events) {
      const r = this.hit(ev);
      if (r && r.pageId === pageId) {
        points.push({ x: r.x, y: r.y, pressure: ev.pressure || 0.5 });
      }
    }
    if (points.length === 0) return;

    const predicted: PointerPoint[] = [];
    if (typeof e.getPredictedEvents === 'function') {
      for (const ev of e.getPredictedEvents()) {
        const r = this.hit(ev);
        if (r && r.pageId === pageId) {
          predicted.push({ x: r.x, y: r.y, pressure: ev.pressure || 0.5 });
        }
      }
    }

    this.cb.onStrokeMove(pageId, points, predicted.length > 0 ? predicted : undefined);
  };

  private onPointerUp = (e: PointerEvent): void => {
    this.activeTouchIds.delete(e.pointerId);
    if (e.pointerId !== this.drawingPointerId || !this.lastPageId) return;
    const pageId = this.lastPageId;
    const result = this.hit(e);
    this.finishPointer(e.pointerId);
    this.cb.onStrokeEnd(
      pageId,
      result && result.pageId === pageId
        ? { x: result.x, y: result.y, pressure: e.pressure || 0.5 }
        : null
    );
  };

  private onPointerCancel = (e: PointerEvent): void => {
    this.activeTouchIds.delete(e.pointerId);
    if (e.pointerId !== this.drawingPointerId) return;
    this.cancelStroke();
  };

  private cancelStroke(): void {
    if (this.drawingPointerId === null) return;
    this.finishPointer(this.drawingPointerId);
    this.cb.onStrokeCancel();
  }

  private finishPointer(pointerId: number): void {
    this.router.release(pointerId);
    this.router.penDrawing = false;
    try {
      this.el.releasePointerCapture(pointerId);
    } catch {
      /* not captured */
    }
    this.drawingPointerId = null;
    this.drawingIsTouch = false;
    this.lastPageId = null;
  }

  destroy(): void {
    this.el.removeEventListener('pointerdown', this.onPointerDown);
    this.el.removeEventListener('pointermove', this.onPointerMove);
    this.el.removeEventListener('pointerup', this.onPointerUp);
    this.el.removeEventListener('pointercancel', this.onPointerCancel);
  }
}
