// Uniform-grid spatial index per page. The erasers and lasso query this
// instead of scanning every stroke (the legacy app's per-point linear scan
// was a main cause of eraser jank on dense pages).

import type { Page, Stroke } from '../core/model';
import { paintedBbox } from '../core/model';
import type { StrokeId } from '../core/ids';

const CELL_SIZE = 96;

export class StrokesIndex {
  private cells = new Map<string, Set<StrokeId>>();
  private strokeCells = new Map<StrokeId, string[]>();

  static fromPage(page: Page): StrokesIndex {
    const index = new StrokesIndex();
    for (const id of page.strokeOrder) {
      const stroke = page.strokes.get(id);
      if (stroke) index.add(stroke);
    }
    return index;
  }

  private static keysFor(stroke: Stroke): string[] {
    const box = paintedBbox(stroke);
    const keys: string[] = [];
    const cx0 = Math.floor(box.x0 / CELL_SIZE);
    const cy0 = Math.floor(box.y0 / CELL_SIZE);
    const cx1 = Math.floor(box.x1 / CELL_SIZE);
    const cy1 = Math.floor(box.y1 / CELL_SIZE);
    for (let cy = cy0; cy <= cy1; cy++) {
      for (let cx = cx0; cx <= cx1; cx++) {
        keys.push(`${cx},${cy}`);
      }
    }
    return keys;
  }

  add(stroke: Stroke): void {
    this.remove(stroke.id);
    const keys = StrokesIndex.keysFor(stroke);
    this.strokeCells.set(stroke.id, keys);
    for (const key of keys) {
      let cell = this.cells.get(key);
      if (!cell) {
        cell = new Set();
        this.cells.set(key, cell);
      }
      cell.add(stroke.id);
    }
  }

  remove(strokeId: StrokeId): void {
    const keys = this.strokeCells.get(strokeId);
    if (!keys) return;
    for (const key of keys) {
      const cell = this.cells.get(key);
      if (cell) {
        cell.delete(strokeId);
        if (cell.size === 0) this.cells.delete(key);
      }
    }
    this.strokeCells.delete(strokeId);
  }

  /** Candidate stroke ids whose painted bbox may touch the circle. */
  queryCircle(cx: number, cy: number, radius: number): Set<StrokeId> {
    return this.queryBox(cx - radius, cy - radius, cx + radius, cy + radius);
  }

  queryBox(x0: number, y0: number, x1: number, y1: number): Set<StrokeId> {
    const out = new Set<StrokeId>();
    const cx0 = Math.floor(x0 / CELL_SIZE);
    const cy0 = Math.floor(y0 / CELL_SIZE);
    const cx1 = Math.floor(x1 / CELL_SIZE);
    const cy1 = Math.floor(y1 / CELL_SIZE);
    for (let cy = cy0; cy <= cy1; cy++) {
      for (let cx = cx0; cx <= cx1; cx++) {
        const cell = this.cells.get(`${cx},${cy}`);
        if (cell) for (const id of cell) out.add(id);
      }
    }
    return out;
  }
}
