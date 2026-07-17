// Camera math: zoom/pan state, clamping, pinch and inertia stepping.
// Pure (no DOM) — the transformApplier turns this into CSS.

export interface CameraState {
  zoom: number;
  panX: number;
  panY: number;
}

export interface ViewportSize {
  width: number;
  height: number;
}

export interface ContentBounds {
  /** Content extents in page space (unscaled). */
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export const MIN_ZOOM = 0.5;
export const MAX_ZOOM = 3;
/** Pixels of page that must stay visible when clamping pans. */
const MIN_VISIBLE = 80;

export const INERTIA_DECAY = 0.94;
export const INERTIA_STOP = 0.015;
/** Cap a single inertia step so a stalled frame can't teleport the view. */
const MAX_PAN_STEP = 90;

export function clampZoom(zoom: number): number {
  return Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, zoom));
}

export function clampPan(camera: CameraState, viewport: ViewportSize, bounds: ContentBounds): CameraState {
  const minPanX = MIN_VISIBLE - bounds.right * camera.zoom;
  const maxPanX = viewport.width - MIN_VISIBLE - bounds.left * camera.zoom;
  const minPanY = MIN_VISIBLE - bounds.bottom * camera.zoom;
  const maxPanY = viewport.height - MIN_VISIBLE - bounds.top * camera.zoom;
  return {
    zoom: camera.zoom,
    panX: minPanX > maxPanX ? (minPanX + maxPanX) / 2 : Math.max(minPanX, Math.min(camera.panX, maxPanX)),
    panY: minPanY > maxPanY ? (minPanY + maxPanY) / 2 : Math.max(minPanY, Math.min(camera.panY, maxPanY))
  };
}

/** Zoom around a fixed screen point (pinch center / cursor). */
export function zoomAround(camera: CameraState, screenX: number, screenY: number, newZoom: number): CameraState {
  const zoom = clampZoom(newZoom);
  const scale = zoom / camera.zoom;
  return {
    zoom,
    panX: screenX - (screenX - camera.panX) * scale,
    panY: screenY - (screenY - camera.panY) * scale
  };
}

export function screenToPage(camera: CameraState, screenX: number, screenY: number): { x: number; y: number } {
  return {
    x: (screenX - camera.panX) / camera.zoom,
    y: (screenY - camera.panY) / camera.zoom
  };
}

export interface InertiaState {
  vx: number;
  vy: number;
}

/** One inertia frame. Returns null when motion has stopped. */
export function stepInertia(
  inertia: InertiaState,
  dtMs: number
): { dx: number; dy: number; next: InertiaState } | null {
  const decay = Math.pow(INERTIA_DECAY, dtMs / 16.67);
  const vx = inertia.vx * decay;
  const vy = inertia.vy * decay;
  if (Math.abs(vx) < INERTIA_STOP && Math.abs(vy) < INERTIA_STOP) return null;
  const cap = (v: number) => Math.max(-MAX_PAN_STEP, Math.min(MAX_PAN_STEP, v));
  return { dx: cap(vx * dtMs), dy: cap(vy * dtMs), next: { vx, vy } };
}

/** Fit a page into the viewport (used by "go to today"). */
export function fitPageZoom(
  pageWidth: number,
  pageHeight: number,
  viewport: ViewportSize,
  padding = 24
): number {
  const fitX = viewport.width / (pageWidth + padding * 2);
  const fitY = viewport.height / (pageHeight + padding * 2);
  return clampZoom(Math.min(fitX, fitY));
}
