// Append-only op journal. Every applied op lands here within a microtask,
// so a crash between snapshot compactions loses at most the op in flight.
// (Direct successor of the legacy stroke-ops queue, generalized to all op
// types and keyed by docId instead of page index.)

import type { Op } from '../core/ops';
import { docKeyRange, req, txDone, STORE_OPLOG } from './idb';

export interface OplogRecord {
  docId: string;
  seq: number;
  op: Op;
  ts: number;
}

const FLUSH_RETRY_MS = 250;

export class OplogWriter {
  private queue: OplogRecord[] = [];
  private nextSeq = 1;
  private flushScheduled = false;
  private flushInProgress = false;
  private flushWaiters: ((ok: boolean) => void)[] = [];
  private lastWrittenSeq = 0;
  /** Surfaced to the UI when local writes fail (e.g. quota). */
  onError: ((err: unknown) => void) | null = null;

  constructor(
    private db: IDBDatabase,
    private docId: string
  ) {}

  /** Continue the sequence after whatever is already in the store. */
  async init(): Promise<void> {
    const tx = this.db.transaction(STORE_OPLOG, 'readonly');
    const keys = (await req(tx.objectStore(STORE_OPLOG).getAllKeys(docKeyRange(this.docId)))) as [string, number][];
    const maxSeq = keys.reduce((max, key) => Math.max(max, key[1]), 0);
    this.nextSeq = maxSeq + 1;
    this.lastWrittenSeq = maxSeq;
  }

  append(op: Op): void {
    this.queue.push({ docId: this.docId, seq: this.nextSeq++, op, ts: Date.now() });
    if (!this.flushScheduled) {
      this.flushScheduled = true;
      queueMicrotask(() => {
        this.flushScheduled = false;
        void this.flush();
      });
    }
  }

  /** Highest seq known to be durable; pass to saveDocSnapshot(throughSeq). */
  writtenThrough(): number {
    return this.lastWrittenSeq;
  }

  hasPending(): boolean {
    return this.queue.length > 0 || this.flushInProgress;
  }

  async flush(): Promise<boolean> {
    if (this.flushInProgress) {
      return new Promise((resolve) => this.flushWaiters.push(resolve));
    }
    if (this.queue.length === 0) return true;
    this.flushInProgress = true;
    let ok = true;
    try {
      // Drain until stable. Ops appended while an IDB transaction is in
      // flight are picked up by the next loop before any waiter resolves.
      while (this.queue.length > 0) {
        const batch = this.queue;
        this.queue = [];
        try {
          const tx = this.db.transaction(STORE_OPLOG, 'readwrite');
          const store = tx.objectStore(STORE_OPLOG);
          for (const record of batch) store.put(record);
          await txDone(tx);
          const last = batch[batch.length - 1];
          if (last) this.lastWrittenSeq = Math.max(this.lastWrittenSeq, last.seq);
        } catch (err) {
          this.queue = [...batch, ...this.queue];
          this.onError?.(err);
          ok = false;
          setTimeout(() => void this.flush(), FLUSH_RETRY_MS);
          break;
        }
      }
      return ok;
    } finally {
      this.flushInProgress = false;
      const waiters = this.flushWaiters;
      this.flushWaiters = [];
      for (const resolve of waiters) resolve(ok);
      if (ok && this.queue.length > 0 && !this.flushScheduled) {
        this.flushScheduled = true;
        queueMicrotask(() => {
          this.flushScheduled = false;
          void this.flush();
        });
      }
    }
  }
}
