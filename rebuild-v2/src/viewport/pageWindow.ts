// Tracks which pages are currently in the viewport (with buffer).
// Fires onActivate/onDeactivate callbacks when pages enter/exit the viewport.

import { CameraState, ViewportSize } from './camera';
import { PageId } from '../core/ids';

export interface PageLayout {
  id: PageId;
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Returns all pages' layout info (id + page-space rect). */
export type LayoutGetter = () => PageLayout[];

/** Returns current viewport dimensions in screen space. */
export type ViewportSizeGetter = () => ViewportSize;

/** Returns current camera state. */
export type CameraGetter = () => CameraState;

export interface PageWindowOptions {
  layoutGetter: LayoutGetter;
  viewportSizeGetter: ViewportSizeGetter;
  cameraGetter: CameraGetter;
  /** Called when a page enters the viewport. */
  onActivate?: (pageId: PageId, scale: number) => void;
  /** Called when a page exits the viewport. */
  onDeactivate?: (pageId: PageId) => void;
  /** Extra pages to keep around on each side of the viewport (default 1). */
  bufferPages?: number;
}

/** Result of updateWindow() — pages that just entered or left the viewport. */
export interface WindowDelta {
  activated: PageId[];
  deactivated: PageId[];
}

export class PageWindow {
  private layoutGetter: LayoutGetter;
  private viewportSizeGetter: ViewportSizeGetter;
  private cameraGetter: CameraGetter;
  private onActivate?: (pageId: PageId, scale: number) => void;
  private onDeactivate?: (pageId: PageId) => void;
  private bufferPages: number;

  private visibleIds: Set<PageId> = new Set();

  constructor(options: PageWindowOptions) {
    this.layoutGetter = options.layoutGetter;
    this.viewportSizeGetter = options.viewportSizeGetter;
    this.cameraGetter = options.cameraGetter;
    this.onActivate = options.onActivate;
    this.onDeactivate = options.onDeactivate;
    this.bufferPages = options.bufferPages ?? 1;
  }

  /**
   * Returns all page IDs currently visible in the viewport (including buffer).
   */
  visiblePageIds(): PageId[] {
    return Array.from(this.visibleIds);
  }

  /**
   * Call after camera changes to update visible page set.
   * Returns which pages just entered or left the viewport.
   */
  updateWindow(): WindowDelta {
    const camera = this.cameraGetter();
    const viewport = this.viewportSizeGetter();
    const pages = this.layoutGetter();
    const { zoom, panX, panY } = camera;

    // Transform viewport from screen space to page space
    const viewLeft = -panX / zoom;
    const viewTop = -panY / zoom;
    const viewRight = (viewport.width - panX) / zoom;
    const viewBottom = (viewport.height - panY) / zoom;

    // Find all pages that intersect the (expanded) viewport
    const newVisible = new Set<PageId>();
    for (const page of pages) {
      if (
        page.x < viewRight &&
        page.x + page.width > viewLeft &&
        page.y < viewBottom &&
        page.y + page.height > viewTop
      ) {
        newVisible.add(page.id);
      }
    }

    // Determine delta
    const activated: PageId[] = [];
    const deactivated: PageId[] = [];

    for (const id of newVisible) {
      if (!this.visibleIds.has(id)) {
        activated.push(id);
      }
    }

    for (const id of this.visibleIds) {
      if (!newVisible.has(id)) {
        deactivated.push(id);
      }
    }

    // Fire callbacks
    for (const id of activated) {
      this.onActivate?.(id, zoom);
    }
    for (const id of deactivated) {
      this.onDeactivate?.(id);
    }

    this.visibleIds = newVisible;
    return { activated, deactivated };
  }

  /**
   * Manually activate a page (e.g., when programmatically focusing it).
   */
  activatePage(pageId: PageId): void {
    if (!this.visibleIds.has(pageId)) {
      this.visibleIds.add(pageId);
      this.onActivate?.(pageId, this.cameraGetter().zoom);
    }
  }

  /**
   * Manually deactivate a page (e.g., when programmatically unfocusing it).
   */
  deactivatePage(pageId: PageId): void {
    if (this.visibleIds.has(pageId)) {
      this.visibleIds.delete(pageId);
      this.onDeactivate?.(pageId);
    }
  }

  /** Check if a specific page is currently in the viewport. */
  isPageVisible(pageId: PageId): boolean {
    return this.visibleIds.has(pageId);
  }
}