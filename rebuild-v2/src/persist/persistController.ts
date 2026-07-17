// Glue between the DocStore and IndexedDB:
//  - every applied op → oplog (microtask, crash-safe)
//  - debounced compaction → doc/page snapshot + oplog trim + version capture
//
// Compaction is deliberately cheap to trigger: it runs off a debounce, never
// during a burst of ops, and the snapshot write is one transaction.

import type { AppliedOp, DocStore } from '../core/store';
import { VersionLog } from '../core/versions';
import type { VersionAuthor, VersionEntry } from '../core/versions';
import { docKeyRange, req, txDone, STORE_VERSIONS } from './idb';
import { OplogWriter } from './oplog';
import { saveDocChanges } from './docRepo';

const COMPACT_DEBOUNCE_MS = 800;
const COMPACT_MAX_DELAY_MS = 5000;
const VERSION_DEBOUNCE_MS = 30_000;
const VERSION_MAX_DELAY_MS = 120_000;

export class PersistController {
  readonly versions = new VersionLog();
  private oplog: OplogWriter;
  private unsubscribe: (() => void) | null = null;
  private debounceTimer: ReturnType<typeof setTimeout> | null = null;
  private maxTimer: ReturnType<typeof setTimeout> | null = null;
  private versionDebounceTimer: ReturnType<typeof setTimeout> | null = null;
  private versionMaxTimer: ReturnType<typeof setTimeout> | null = null;
  private compactInProgress = false;
  private compactQueued = false;
  private compactQueuedCaptureVersion = false;
  private disposed = false;
  private dirtyPageIds = new Set<string>();
  private deletedPageIds = new Set<string>();
  private hasPendingOps = false;
  private changeGeneration = 0;
  private removeLifecycleListeners: (() => void) | null = null;

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

  async start(recoveredPageIds?: Iterable<string>): Promise<void> {
    await this.oplog.init();
    await this.loadVersions();
    this.unsubscribe = this.store.subscribe((applied) => {
      this.hasPendingOps = true;
      this.changeGeneration++;
      this.oplog.append(applied.op);
      this.trackDirtyPages(applied);
      this.scheduleCompact();
      this.scheduleVersionCapture();
    });
    this.installLifecycleListeners();
    if (recoveredPageIds) {
      for (const pageId of recoveredPageIds) this.dirtyPageIds.add(pageId);
      this.hasPendingOps = true;
      this.changeGeneration++;
      this.scheduleCompact();
    }
  }

  scheduleCompact(): void {
    if (this.disposed) return;
    if (this.debounceTimer) clearTimeout(this.debounceTimer);
    this.debounceTimer = setTimeout(() => void this.compact(false), COMPACT_DEBOUNCE_MS);
    if (!this.maxTimer) {
      this.maxTimer = setTimeout(() => void this.compact(false), COMPACT_MAX_DELAY_MS);
    }
  }

  /** Flush oplog + write snapshot + capture a version. Safe to call anytime. */
  async compact(captureVersion = true): Promise<boolean> {
    if (this.disposed) return false;
    if (!this.hasPendingOps && !this.pendingVersionHint) return true;
    if (this.compactInProgress) {
      this.compactQueued = true;
      this.compactQueuedCaptureVersion ||= captureVersion;
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
    const dirtyPageIds = this.dirtyPageIds;
    const deletedPageIds = this.deletedPageIds;
    const compactGeneration = this.changeGeneration;
    this.dirtyPageIds = new Set();
    this.deletedPageIds = new Set();
    try {
      await this.oplog.flush();
      const throughSeq = this.oplog.writtenThrough();
      await withDocumentWriteLock(this.store.doc.id, () => saveDocChanges(this.db, this.store.doc, {
        throughSeq,
        dirtyPageIds,
        deletedPageIds
      }));

      if (captureVersion || this.pendingVersionHint) {
        const hint = this.pendingVersionHint;
        this.pendingVersionHint = null;
        const entry = this.versions.capture(this.store.doc, this.getAuthor(), hint);
        await this.persistVersions(entry);
        this.clearVersionTimers();
      }

      this.lastSavedAt = Date.now();
      if (this.changeGeneration === compactGeneration) this.hasPendingOps = false;
      this.onSaved?.(this.lastSavedAt);
      return true;
    } catch (err) {
      for (const pageId of dirtyPageIds) this.dirtyPageIds.add(pageId);
      for (const pageId of deletedPageIds) this.deletedPageIds.add(pageId);
      this.onError?.(err);
      return false;
    } finally {
      this.compactInProgress = false;
      if (this.compactQueued) {
        const captureQueued = this.compactQueuedCaptureVersion;
        this.compactQueued = false;
        this.compactQueuedCaptureVersion = false;
        if (captureQueued) {
          void this.compact(true);
        } else {
          this.scheduleCompact();
        }
      }
    }
  }

  async dispose(): Promise<void> {
    this.unsubscribe?.();
    this.unsubscribe = null;
    this.removeLifecycleListeners?.();
    this.removeLifecycleListeners = null;
    if (this.debounceTimer) clearTimeout(this.debounceTimer);
    if (this.maxTimer) clearTimeout(this.maxTimer);
    this.clearVersionTimers();
    if (!this.disposed) {
      // Final compaction so closing the doc never loses the tail.
      await this.compact();
    }
    this.disposed = true;
  }

  private scheduleVersionCapture(): void {
    if (this.disposed) return;
    if (this.versionDebounceTimer) clearTimeout(this.versionDebounceTimer);
    this.versionDebounceTimer = setTimeout(() => void this.compact(true), VERSION_DEBOUNCE_MS);
    if (!this.versionMaxTimer) {
      this.versionMaxTimer = setTimeout(() => void this.compact(true), VERSION_MAX_DELAY_MS);
    }
  }

  private installLifecycleListeners(): void {
    if (typeof document === 'undefined' || typeof window === 'undefined') return;
    const onVisibilityChange = () => {
      if (document.visibilityState === 'hidden') void this.compact(false);
    };
    const onPageHide = () => void this.compact(false);
    document.addEventListener('visibilitychange', onVisibilityChange);
    window.addEventListener('pagehide', onPageHide);
    this.removeLifecycleListeners = () => {
      document.removeEventListener('visibilitychange', onVisibilityChange);
      window.removeEventListener('pagehide', onPageHide);
    };
  }

  private clearVersionTimers(): void {
    if (this.versionDebounceTimer) clearTimeout(this.versionDebounceTimer);
    if (this.versionMaxTimer) clearTimeout(this.versionMaxTimer);
    this.versionDebounceTimer = null;
    this.versionMaxTimer = null;
  }

  private trackDirtyPages(applied: AppliedOp): void {
    const op = applied.op;
    switch (op.type) {
      case 'splice-strokes':
      case 'transform-strokes':
      case 'splice-images':
      case 'transform-images':
      case 'set-page-background':
      case 'set-page-side-panel':
        this.dirtyPageIds.add(op.pageId);
        this.deletedPageIds.delete(op.pageId);
        break;
      case 'add-page':
        this.dirtyPageIds.add(op.page.id);
        this.deletedPageIds.delete(op.page.id);
        break;
      case 'remove-page':
        this.dirtyPageIds.delete(op.pageId);
        this.deletedPageIds.add(op.pageId);
        break;
      case 'replace-doc': {
        const liveIds = new Set(op.pages.map((page) => page.id));
        for (const pageId of liveIds) {
          this.dirtyPageIds.add(pageId);
          this.deletedPageIds.delete(pageId);
        }
        if (applied.inverse.type === 'replace-doc') {
          for (const page of applied.inverse.pages) {
            if (!liveIds.has(page.id)) this.deletedPageIds.add(page.id);
          }
        }
        break;
      }
      case 'move-page':
      case 'set-meta':
        break;
    }
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

async function withDocumentWriteLock<T>(docId: string, task: () => Promise<T>): Promise<T> {
  const locks = typeof navigator === 'undefined' ? undefined : navigator.locks;
  if (!locks) return task();
  return locks.request(`inhouse-notes:${docId}`, { mode: 'exclusive' }, task);
}
