// Applies CSS transform to the canvas container based on camera state.
// During gestures: only updates CSS transform, no canvas reallocation.
// After zoom settles: triggers re-rasterization via debounced scheduleRaster.

import { CameraState } from './camera';

export interface TransformApplierOptions {
  /** Container element to apply CSS transform to. */
  container: HTMLElement;
  /** Called when zoom settles and canvas should be re-rasterized. */
  onRasterize?: (scale: number) => void;
  /** Debounce delay in ms after zoom settles (default ~220ms). */
  rasterDebounceMs?: number;
}

const DEFAULT_RASTER_DEBOUNCE_MS = 220;

export class TransformApplier {
  private container: HTMLElement;
  private onRasterize?: (scale: number) => void;
  private rasterDebounceMs: number;

  private lastZoom: number = 1;
  private rafId: number | null = null;
  private rasterTimeoutId: ReturnType<typeof setTimeout> | null = null;

  constructor(options: TransformApplierOptions) {
    this.container = options.container;
    this.onRasterize = options.onRasterize;
    this.rasterDebounceMs = options.rasterDebounceMs ?? DEFAULT_RASTER_DEBOUNCE_MS;
  }

  /**
   * Apply camera state to the container's CSS transform.
   * Updates only the CSS transform (no rasterization during gestures).
   */
  apply(camera: CameraState): void {
    this.cancelScheduledRaster();

    const { zoom, panX, panY } = camera;
    const transform = `scale(${zoom}) translate(${panX / zoom}px, ${panY / zoom}px)`;
    this.container.style.transform = transform;

    if (zoom !== this.lastZoom) {
      this.scheduleRaster(zoom);
      this.lastZoom = zoom;
    }
  }

  /**
   * Schedule a rasterization call after zoom settles.
   * Debounces to avoid re-rasterizing during active zoom gestures.
   */
  scheduleRaster(scale: number): void {
    this.cancelScheduledRaster();

    this.rasterTimeoutId = setTimeout(() => {
      this.rasterTimeoutId = null;
      this.rafId = requestAnimationFrame(() => {
        this.rafId = null;
        this.onRasterize?.(scale);
      });
    }, this.rasterDebounceMs);
  }

  private cancelScheduledRaster(): void {
    if (this.rasterTimeoutId !== null) {
      clearTimeout(this.rasterTimeoutId);
      this.rasterTimeoutId = null;
    }
    if (this.rafId !== null) {
      cancelAnimationFrame(this.rafId);
      this.rafId = null;
    }
  }

  /** Immediately trigger rasterization if scheduled, useful for forced redraws. */
  forceRaster(scale: number): void {
    this.cancelScheduledRaster();
    this.onRasterize?.(scale);
  }

  dispose(): void {
    this.cancelScheduledRaster();
  }
}