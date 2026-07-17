// Glue between the DocStore and IndexedDB:
//  - every applied op → oplog (microtask, crash-safe)
//  - debounced compaction → doc/page snapshot + oplog trim + version capture
//
// Compaction is deliberately cheap to trigger: it runs off a debounce, never
// during a burst of ops, and the snapshot write is one transaction.

import type { DocStore } from '../core/store';
import { VersionLog } from '../core/versions';
import type { VersionAuthor, VersionEntry } from '../core/versions';
import { docKeyRange, req, txDone, STORE_VERSIONS } from './idb';
import { OplogWriter } from './oplog';
import { saveDocSnapshot } from './docRepo';

const COMPACT_DEBOUNCE_MS = 800;
const COMPACT_MAX_DELAY_MS = 5000;

export class PersistController {
  readonly versions = new VersionLog();
  private oplog: OplogWriter;
  private unsubscribe: (() => void) | null = null;
  private debounceTimer: ReturnType<typeof setTimeout> | null = null;
  private maxTimer: ReturnType<typeof setTimeout> | null = null;
  private compactInProgress = false;
  private compactQueued = false;
  private disposed = false;

  /** Last successful snapshot time, for the "Saved" status UI. */
  lastSavedAt: number | null = null;
  onSaved: ((savedAt: number) => void) | null = null;
  onError: ((err: unknown) => void) | null = null;

  getAuthor: () => VersionAuthor = () => ({ name: 'You', email: '', photo: '' });
  /** One-shot hint consumed by the next version capture (restores). */
  pendingVersionHint: Parameters<VersionLog['capture']>[2] = null;

  constructor(
    private db: IDBDatabase,
    private store: DocStore
  ) {
    this.oplog = new OplogWriter(db, store.doc.id);
    this.oplog.onError = (err) => this.onError?.(err);
  }

  async start(): Promise<void> {
    await this.oplog.init();
    await this.loadVersions();
    this.unsubscribe = this.store.subscribe((applied) => {
      this.oplog.append(applied.op);
      this.scheduleCompact();
    });
  }

  scheduleCompact(): void {
    if (this.disposed) return;
    if (this.debounceTimer) clearTimeout(this.debounceTimer);
    this.debounceTimer = setTimeout(() => void this.compact(), COMPACT_DEBOUNCE_MS);
    if (!this.maxTimer) {
      this.maxTimer = setTimeout(() => void this.compact(), COMPACT_MAX_DELAY_MS);
    }
  }

  /** Flush oplog + write snapshot + capture a version. Safe to call anytime. */
  async compact(): Promise<boolean> {
    if (this.disposed) return false;
    if (this.compactInProgress) {
      this.compactQueued = true;
      return true;
    }
    this.compactInProgress = true;
    if (this.debounceTimer) {
      clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
    if (this.maxTimer) {
      clearTimeout(this.maxTimer);
      this.maxTimer = null;
    }
    try {
      await this.oplog.flush();
      const throughSeq = this.oplog.writtenThrough();
      await saveDocSnapshot(this.db, this.store.doc, { throughSeq });

      const hint = this.pendingVersionHint;
      this.pendingVersionHint = null;
      const entry = this.versions.capture(this.store.doc, this.getAuthor(), hint);
      await this.persistVersions(entry);

      this.lastSavedAt = Date.now();
      this.onSaved?.(this.lastSavedAt);
      return true;
    } catch (err) {
      this.onError?.(err);
      return false;
    } finally {
      this.compactInProgress = false;
      if (this.compactQueued) {
        this.compactQueued = false;
        this.scheduleCompact();
      }
    }
  }

  async dispose(): Promise<void> {
    this.unsubscribe?.();
    this.unsubscribe = null;
    if (this.debounceTimer) clearTimeout(this.debounceTimer);
    if (this.maxTimer) clearTimeout(this.maxTimer);
    if (!this.disposed) {
      // Final compaction so closing the doc never loses the tail.
      await this.compact();
    }
    this.disposed = true;
  }

  private async loadVersions(): Promise<void> {
    const tx = this.db.transaction(STORE_VERSIONS, 'readonly');
    const entries = (await req(
      tx.objectStore(STORE_VERSIONS).getAll(docKeyRange(this.store.doc.id))
    )) as (VersionEntry & { docId: string })[];
    entries.sort((a, b) => a.ts - b.ts);
    this.versions.entries = entries.map(({ docId: _docId, ...entry }) => entry);
  }

  private async persistVersions(latest: VersionEntry): Promise<void> {
    const docId = this.store.doc.id;
    const tx = this.db.transaction(STORE_VERSIONS, 'readwrite');
    const store = tx.objectStore(STORE_VERSIONS);
    store.put({ ...latest, docId });
    // Drop persisted entries the in-memory log has evicted.
    const liveIds = new Set(this.versions.entries.map((e) => e.id));
    const keys = (await req(store.getAllKeys(docKeyRange(docId)))) as [string, string][];
    for (const key of keys) {
      if (!liveIds.has(key[1])) store.delete(key);
    }
    await txDone(tx);
  }
}
