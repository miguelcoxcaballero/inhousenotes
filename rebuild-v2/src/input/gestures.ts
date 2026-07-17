// Viewport gestures: touch pan, two-finger pinch zoom, wheel scroll
// (Ctrl+wheel zooms, matching the legacy app), inertia, and
// overscroll-past-the-last-page to create a new page.
//
// Pointer-based gestures react ONLY to touch: pen and mouse belong to the
// drawing pipeline (see router.ts). Touches the pipeline claimed for
// drawing — and any touch while a pen stroke is in progress (palm
// rejection) — are ignored here.

import type { CameraState } from '../viewport/camera';
import { clampZoom } from '../viewport/camera';
import { InputRouter } from './router';

export interface GestureCallbacks {
  onCameraChange(camera: CameraState): void;
  onOverscrollCreatePage(): void;
}

// Camera accessor — either a { get, set } pair or a simple getter.
// When only a getter is passed, the setter is derived from onCameraChange.
type CameraAccessor =
  | { get: () => CameraState; set: (c: CameraState) => void }
  | (() => CameraState);

interface TrackedTouch {
  clientX: number;
  clientY: number;
}

const OVERSCROLL_CREATE_PX = 120;

export class GestureHandler {
  private readonly el: HTMLElement;
  private readonly getCamera: () => CameraState;
  private readonly setCamera: (cam: CameraState) => void;
  private readonly getContentBounds: (() => { left: number; top: number; right: number; bottom: number }) | null;
  private readonly cb: GestureCallbacks;

  private touches = new Map<number, TrackedTouch>();

  // Pan state
  private panPointerId: number | null = null;
  private panStart = { x: 0, y: 0 };
  private cameraAtPanStart: CameraState = { panX: 0, panY: 0, zoom: 1 };
  private hasMoved = false;

  // Velocity / inertia
  private velX = 0;
  private velY = 0;
  private lastPanX = 0;
  private lastPanY = 0;
  private lastPanTime = 0;
  private rafId = 0;

  // Pinch state
  private isPinching = false;
  private pinchStartDist = 0;
  private pinchStartZoom = 1;
  private pinchCenterX = 0;
  private pinchCenterY = 0;

  // Overscroll
  private overscrollFired = false;

  constructor(
    viewportEl: HTMLElement,
    camera: CameraAccessor,
    onCameraChange: (camera: CameraState) => void,
    onOverscrollCreatePage: () => void,
    getContentBounds?: () => { left: number; top: number; right: number; bottom: number },
    private router: InputRouter = new InputRouter()
  ) {
    this.el = viewportEl;
    this.getContentBounds = getContentBounds ?? null;
    this.cb = { onCameraChange, onOverscrollCreatePage };

    if (typeof camera === 'function') {
      this.getCamera = camera;
      this.setCamera = (cam) => onCameraChange(cam);
    } else {
      this.getCamera = camera.get;
      this.setCamera = (cam) => camera.set(cam);
    }

    this.el.addEventListener('pointerdown', this.onPointerDown, { passive: false });
    this.el.addEventListener('pointermove', this.onPointerMove, { passive: false });
    this.el.addEventListener('pointerup', this.onPointerUp, { passive: false });
    this.el.addEventListener('pointercancel', this.onPointerUp, { passive: false });
    this.el.addEventListener('wheel', this.onWheel, { passive: false });
  }

  // ── Pointer events (touch only) ────────────────────────────────────────────

  private onPointerDown = (e: PointerEvent): void => {
    if (e.pointerType !== 'touch') return;
    if (this.router.penDrawing) return; // palm rejection
    this.touches.set(e.pointerId, { clientX: e.clientX, clientY: e.clientY });

    if (this.touches.size === 1) {
      this.stopInertia();
      this.panPointerId = e.pointerId;
      this.panStart = { x: e.clientX, y: e.clientY };
      this.cameraAtPanStart = { ...this.getCamera() };
      this.hasMoved = false;
      this.velX = 0;
      this.velY = 0;
      this.lastPanX = e.clientX;
      this.lastPanY = e.clientY;
      this.lastPanTime = performance.now();
      this.overscrollFired = false;
    } else if (this.touches.size === 2) {
      // Second finger: switch from pan to pinch. The pipeline cancels its
      // touch stroke on its own when this happens.
      this.panPointerId = null;
      this.isPinching = false; // re-initialized on first pinch move
    }
  };

  private onPointerMove = (e: PointerEvent): void => {
    if (e.pointerType !== 'touch') return;
    if (!this.touches.has(e.pointerId)) return;
    this.touches.set(e.pointerId, { clientX: e.clientX, clientY: e.clientY });

    // Drop touches the drawing pipeline claimed.
    if (this.router.isClaimed(e.pointerId)) {
      this.touches.delete(e.pointerId);
      this.panPointerId = null;
      return;
    }

    if (this.touches.size >= 2) {
      this.handlePinch();
      return;
    }

    if (this.panPointerId === e.pointerId) {
      const dx = e.clientX - this.panStart.x;
      const dy = e.clientY - this.panStart.y;
      if (!this.hasMoved) {
        if (Math.abs(dx) < 3 && Math.abs(dy) < 3) return;
        this.hasMoved = true;
      }
      e.preventDefault();

      const now = performance.now();
      const dt = now - this.lastPanTime;
      if (dt > 0 && dt < 100) {
        this.velX = (e.clientX - this.lastPanX) / dt;
        this.velY = (e.clientY - this.lastPanY) / dt;
      }
      this.lastPanX = e.clientX;
      this.lastPanY = e.clientY;
      this.lastPanTime = now;

      const cam = this.cameraAtPanStart;
      const newCam: CameraState = { panX: cam.panX + dx, panY: cam.panY + dy, zoom: cam.zoom };
      this.setCamera(newCam);
      this.checkOverscroll(newCam);
    }
  };

  private onPointerUp = (e: PointerEvent): void => {
    if (e.pointerType !== 'touch') return;
    this.touches.delete(e.pointerId);
    if (this.touches.size < 2) this.isPinching = false;
    if (e.pointerId === this.panPointerId) {
      this.panPointerId = null;
      if (this.hasMoved && (Math.abs(this.velX) > 0.05 || Math.abs(this.velY) > 0.05)) {
        this.startInertia();
      }
    }
  };

  // ── Pinch-to-zoom ──────────────────────────────────────────────────────────

  private handlePinch(): void {
    const pts = [...this.touches.values()];
    if (pts.length < 2) return;
    const [p1, p2] = pts.slice(-2) as [TrackedTouch, TrackedTouch];
    const dist = Math.hypot(p2.clientX - p1.clientX, p2.clientY - p1.clientY);
    if (dist <= 0) return;
    const rect = this.el.getBoundingClientRect();
    const cx = (p1.clientX + p2.clientX) / 2 - rect.left;
    const cy = (p1.clientY + p2.clientY) / 2 - rect.top;

    if (!this.isPinching) {
      this.isPinching = true;
      this.pinchStartDist = dist;
      this.pinchStartZoom = this.getCamera().zoom;
      this.pinchCenterX = cx;
      this.pinchCenterY = cy;
      this.stopInertia();
      return;
    }

    const newZoom = clampZoom(this.pinchStartZoom * (dist / this.pinchStartDist));
    const cam = this.getCamera();
    const relX = (this.pinchCenterX - cam.panX) / cam.zoom;
    const relY = (this.pinchCenterY - cam.panY) / cam.zoom;
    this.setCamera({
      panX: this.pinchCenterX - relX * newZoom,
      panY: this.pinchCenterY - relY * newZoom,
      zoom: newZoom
    });
  }

  // ── Wheel: scroll pans, Ctrl+wheel zooms (legacy behaviour) ────────────────

  private onWheel = (e: WheelEvent): void => {
    e.preventDefault();
    const cam = this.getCamera();

    if (e.ctrlKey || e.metaKey) {
      const rect = this.el.getBoundingClientRect();
      const cx = e.clientX - rect.left;
      const cy = e.clientY - rect.top;
      const newZoom = clampZoom(cam.zoom * (e.deltaY < 0 ? 1.1 : 1 / 1.1));
      const relX = (cx - cam.panX) / cam.zoom;
      const relY = (cy - cam.panY) / cam.zoom;
      this.setCamera({ panX: cx - relX * newZoom, panY: cy - relY * newZoom, zoom: newZoom });
      return;
    }

    const newCam: CameraState = {
      panX: cam.panX - e.deltaX,
      panY: cam.panY - e.deltaY,
      zoom: cam.zoom
    };
    this.setCamera(newCam);
    this.checkOverscroll(newCam, /*fromWheel*/ true);
  };

  // ── Overscroll-to-create-page ──────────────────────────────────────────────

  private checkOverscroll(cam: CameraState, fromWheel = false): void {
    const bounds = this.getContentBounds?.();
    if (!bounds) return;
    const viewportH = this.el.clientHeight;
    const contentBottom = bounds.bottom * cam.zoom + cam.panY;
    // Fire when the user drags the end of the document up past the bottom
    // of the viewport by the threshold (i.e. overscrolls beyond the end).
    if (contentBottom < viewportH - OVERSCROLL_CREATE_PX) {
      if (!this.overscrollFired) {
        this.overscrollFired = true;
        this.cb.onOverscrollCreatePage();
      }
    } else if (fromWheel) {
      // Wheel has no gesture end — re-arm once the user scrolls back.
      this.overscrollFired = false;
    }
  }

  // ── Inertia ────────────────────────────────────────────────────────────────

  private startInertia(): void {
    this.stopInertia();
    let lastTime = performance.now();
    const decay = 0.94;
    const minVel = 0.015;

    const step = (time: number): void => {
      const dt = Math.max(1, time - lastTime);
      lastTime = time;
      const factor = Math.pow(decay, dt / 16.67);
      this.velX *= factor;
      this.velY *= factor;
      if (Math.abs(this.velX) < minVel && Math.abs(this.velY) < minVel) {
        this.stopInertia();
        return;
      }
      const cam = this.getCamera();
      // Cap the step so a stalled frame cannot teleport the view.
      const cap = (v: number) => Math.max(-90, Math.min(90, v));
      this.setCamera({
        panX: cam.panX + cap(this.velX * dt),
        panY: cam.panY + cap(this.velY * dt),
        zoom: cam.zoom
      });
      this.rafId = requestAnimationFrame(step);
    };
    this.rafId = requestAnimationFrame(step);
  }

  private stopInertia(): void {
    if (this.rafId) {
      cancelAnimationFrame(this.rafId);
      this.rafId = 0;
    }
    this.velX = 0;
    this.velY = 0;
  }

  destroy(): void {
    this.stopInertia();
    this.el.removeEventListener('pointerdown', this.onPointerDown);
    this.el.removeEventListener('pointermove', this.onPointerMove);
    this.el.removeEventListener('pointerup', this.onPointerUp);
    this.el.removeEventListener('pointercancel', this.onPointerUp);
    this.el.removeEventListener('wheel', this.onWheel);
    this.touches.clear();
  }
}
