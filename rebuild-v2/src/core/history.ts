// Undo/redo built on op inverses. Entries reference elements by id, so a
// concurrent remote merge can reorder or interleave content without
// corrupting undo (ops apply tolerantly: missing ids are skipped).

import type { AppliedOp } from './store';
import type { DocStore } from './store';
import type { Op } from './ops';

interface HistoryEntry {
  op: Op;
  inverse: Op;
}

/** Approximate memory cap, measured in stroke points kept alive. */
const MAX_HISTORY_POINTS = 200_000;
const MAX_HISTORY_ENTRIES = 100;

function opPoints(op: Op): number {
  let total = 0;
  if (op.type === 'splice-strokes') {
    for (const { stroke } of op.add) total += stroke.points.length / 3;
  } else if (op.type === 'transform-strokes') {
    for (const { points } of op.entries) total += points.length / 3;
  } else if (op.type === 'replace-doc') {
    for (const page of op.pages) {
      for (const stroke of page.strokes) total += stroke.points.length / 3;
    }
  }
  return total;
}

export class History {
  private undoStack: HistoryEntry[] = [];
  private redoStack: HistoryEntry[] = [];
  onChange: (() => void) | null = null;

  /** Feed every store event here; only 'local' ops create undo entries. */
  record(applied: AppliedOp): void {
    if (applied.source !== 'local') return;
    this.undoStack.push({ op: applied.op, inverse: applied.inverse });
    this.redoStack = [];
    this.trim();
    this.onChange?.();
  }

  canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  canRedo(): boolean {
    return this.redoStack.length > 0;
  }

  undo(store: DocStore): boolean {
    const entry = this.undoStack.pop();
    if (!entry) return false;
    store.apply(entry.inverse, 'history');
    this.redoStack.push(entry);
    this.onChange?.();
    return true;
  }

  redo(store: DocStore): boolean {
    const entry = this.redoStack.pop();
    if (!entry) return false;
    store.apply(entry.op, 'history');
    this.undoStack.push(entry);
    this.onChange?.();
    return true;
  }

  clear(): void {
    this.undoStack = [];
    this.redoStack = [];
    this.onChange?.();
  }

  private trim(): void {
    while (this.undoStack.length > MAX_HISTORY_ENTRIES) this.undoStack.shift();
    let points = 0;
    for (const entry of this.undoStack) {
      points += opPoints(entry.op) + opPoints(entry.inverse);
    }
    while (points > MAX_HISTORY_POINTS && this.undoStack.length > 1) {
      const dropped = this.undoStack.shift()!;
      points -= opPoints(dropped.op) + opPoints(dropped.inverse);
    }
  }
}
