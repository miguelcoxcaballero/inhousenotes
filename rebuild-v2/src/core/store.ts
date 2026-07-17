// DocStore: the single write path. UI/tools/sync never mutate the Doc
// directly — they build an Op and call apply(). Subscribers (renderer,
// persistence, sync, history) react to the emitted AppliedOp.

import type { Doc } from './model';
import type { Op } from './ops';
import { applyOp, invertOp } from './ops';

/**
 * Where an op came from:
 * - 'local'   user action → enters undo history, marks doc dirty for sync
 * - 'remote'  collaborative merge → no undo entry, no re-upload echo
 * - 'history' undo/redo replay → no new undo entry
 * - 'restore' version restore → no undo entry (timeline owns rollback)
 */
export type OpSource = 'local' | 'remote' | 'history' | 'restore';

export interface AppliedOp {
  op: Op;
  inverse: Op;
  source: OpSource;
  ts: number;
}

export type StoreListener = (applied: AppliedOp, doc: Doc) => void;

export class DocStore {
  doc: Doc;
  private listeners = new Set<StoreListener>();

  constructor(doc: Doc) {
    this.doc = doc;
  }

  apply(op: Op, source: OpSource = 'local'): AppliedOp {
    const inverse = invertOp(this.doc, op);
    applyOp(this.doc, op);
    const applied: AppliedOp = { op, inverse, source, ts: Date.now() };
    for (const listener of this.listeners) listener(applied, this.doc);
    return applied;
  }

  subscribe(listener: StoreListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Swap the whole document (open another file). Listeners are kept. */
  reset(doc: Doc): void {
    this.doc = doc;
  }
}
