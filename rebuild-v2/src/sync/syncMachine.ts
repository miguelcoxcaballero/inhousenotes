// Single sync state machine. Manages the lifecycle of syncing a document
// to Google Drive, handling dirty state, conflicts, and PDF staleness.
//
// State diagram (based on rebuild plan):
//   idle ──local change──► localDirty ──start()──► syncing
//   syncing ──success──► idle
//   syncing ──remote newer──► conflict
//   syncing ──pdf modified locally──► pdfStale
//   conflict ──user resolves──► localDirty
//   pdfStale ──user resolves──► localDirty

import type { DocStore } from '../core/store';
import type { Doc } from '../core/model';
import type { PersistController } from '../persist/persistController';
import type { DriveClient, DriveFile } from './driveClient';
import { loadDoc, packDoc } from './sidecar';

const AUTO_SYNC_DELAY_MS = 1800;

// ── State types ───────────────────────────────────────────────────────────────

export type SyncState = 'idle' | 'localDirty' | 'syncing' | 'pdfStale' | 'conflict';

export interface SyncStatus {
  state: SyncState;
  saving: boolean;
  lastSaved: number | null;
  lastDriveSyncAt: number | null;
  error: string | null;
}

export type StatusChangeHandler = (status: SyncStatus) => void;

// ── SyncMachine ────────────────────────────────────────────────────────────────

export class SyncMachine {
  private state: SyncState = 'idle';
  private saving = false;
  private lastSaved: number | null = null;
  private lastDriveSyncAt: number | null = null;
  private error: string | null = null;
  private unsubscribers: (() => void)[] = [];
  private statusHandlers = new Set<StatusChangeHandler>();
  private docMeta: DocMeta;
  private disposed = false;
  private autoSyncTimer: ReturnType<typeof setTimeout> | null = null;
  private dirtyWhileSyncing = false;

  /** Store reference for collaborative edits and conflict resolution. */
  readonly store: DocStore;
  /** Persist reference for compaction triggers. */
  readonly persist: PersistController;
  /** Drive client for API operations. */
  readonly drive: DriveClient;

  constructor(
    store: DocStore,
    persist: PersistController,
    drive: DriveClient,
    docMeta: DocMeta
  ) {
    this.store = store;
    this.persist = persist;
    this.drive = drive;
    this.docMeta = docMeta;

    // Sync store changes to dirty state
    const unsubStore = store.subscribe((applied) => {
      if (applied.source === 'local' && !this.disposed) {
        this.markDirty();
      }
    });
    this.unsubscribers.push(unsubStore);

    // Forward persist events
    persist.onSaved = (savedAt) => {
      this.lastSaved = savedAt;
      this.emit();
    };
    persist.onError = (err) => {
      this.error = err instanceof Error ? err.message : String(err);
      this.emit();
    };
  }

  /** Start syncing. Transitions from idle to syncing, or from localDirty directly. */
  async start(): Promise<void> {
    if (this.disposed) return;
    this.clearAutoSyncTimer();
    if (this.state === 'idle') {
      this.setState('syncing');
    } else if (this.state === 'localDirty' || this.state === 'conflict' || this.state === 'pdfStale') {
      this.setState('syncing');
    }

    await this.syncToDrive();
  }

  /**
   * Mark the document as having local changes.
   * Called by the store subscriber when local ops are applied.
   */
  markDirty(): void {
    if (this.state === 'idle') {
      this.setState('localDirty');
      this.scheduleAutoSync();
    } else if (this.state === 'localDirty' || this.state === 'pdfStale') {
      this.scheduleAutoSync();
    } else if (this.state === 'syncing') {
      this.dirtyWhileSyncing = true;
    }
    // If already syncing, let the sync cycle handle it
  }

  /**
   * Register a handler for sync status changes.
   * Returns an unsubscribe function.
   */
  onStatusChange(handler: StatusChangeHandler): () => void {
    this.statusHandlers.add(handler);
    return () => this.statusHandlers.delete(handler);
  }

  /** Current sync status. */
  get status(): SyncStatus {
    return {
      state: this.state,
      saving: this.saving,
      lastSaved: this.lastSaved,
      lastDriveSyncAt: this.lastDriveSyncAt,
      error: this.error
    };
  }

  /** Clean up and stop syncing. */
  dispose(): void {
    this.disposed = true;
    this.clearAutoSyncTimer();
    for (const unsub of this.unsubscribers) unsub();
    this.unsubscribers = [];
    this.statusHandlers.clear();
  }

  // ── Private helpers ─────────────────────────────────────────────────────────

  private setState(state: SyncState): void {
    this.state = state;
    this.emit();
  }

  private emit(): void {
    const status = this.status;
    for (const handler of this.statusHandlers) {
      try {
        handler(status);
      } catch {
        // Ignore handler errors
      }
    }
  }

  private scheduleAutoSync(): void {
    if (this.disposed || !this.drive.isSignedIn()) return;
    this.clearAutoSyncTimer();
    this.autoSyncTimer = setTimeout(() => {
      this.autoSyncTimer = null;
      void this.start();
    }, AUTO_SYNC_DELAY_MS);
  }

  private clearAutoSyncTimer(): void {
    if (this.autoSyncTimer) {
      clearTimeout(this.autoSyncTimer);
      this.autoSyncTimer = null;
    }
  }

  private async syncToDrive(): Promise<void> {
    if (this.disposed) return;

    this.saving = true;
    this.error = null;
    this.emit();

    try {
      this.dirtyWhileSyncing = false;
      // Get or create the app folder
      const folder = await this.drive.getOrCreateAppFolder();

      // Find existing file for this document
      const existing = await this.findDocFile(folder.id);
      const fileName = `${this.docMeta.name}.ihn.json`;

      // Serialize current doc
      const now = Date.now();
      const packed = packDoc(this.store.doc, now);

      let file: DriveFile;
      if (existing) {
        // Update existing file
        const blob = new Blob([packed], { type: 'application/json' });
        file = await this.drive.updateFileMedia(existing.id, blob, 'application/json');
        if (file.name !== fileName) {
          file = await this.drive.patch<DriveFile>(`/files/${existing.id}`, { name: fileName }, {
            fields: 'id,name,mimeType,modifiedTime,size,parents,shared,webContentLink,webViewLink,resourceKey'
          });
        }
      } else {
        // Create new file
        const blob = new Blob([packed], { type: 'application/json' });
        const folder = await this.drive.getOrCreateAppFolder();
        file = await this.drive.createFile(fileName, blob, 'application/json', [folder.id]);
      }

      this.docMeta.fileId = file.id;
      this.lastSaved = now;
      this.lastDriveSyncAt = now;
      if (this.dirtyWhileSyncing) {
        this.setState('localDirty');
        this.scheduleAutoSync();
      } else {
        this.setState('idle');
      }
    } catch (err) {
      this.error = err instanceof Error ? err.message : String(err);
      // Stay in current state on error — user can retry
      if (this.state === 'syncing') {
        // Try to recover to a sensible state
        this.setState('localDirty');
      }
    } finally {
      this.saving = false;
      this.emit();
    }
  }

  private async findDocFile(folderId: string): Promise<DriveFile | null> {
    if (this.docMeta.fileId) {
      try {
        return await this.drive.get<DriveFile>(`/files/${this.docMeta.fileId}`, {
          fields: 'id,name,mimeType,modifiedTime,size,parents,shared,webContentLink,webViewLink,resourceKey'
        });
      } catch {
        this.docMeta.fileId = undefined;
      }
    }
    const fileName = `${this.docMeta.name}.ihn.json`;
    const result = await this.drive.listFiles({
      folderId,
      mimeType: 'application/json',
      pageSize: 10
    });

    return result.files.find((f) => f.name === fileName) ?? null;
  }

  /**
   * Attempt to resolve a conflict by accepting the remote version.
   * Returns true if successful, false if resolution is not possible.
   */
  async acceptRemote(): Promise<boolean> {
    if (this.state !== 'conflict') return false;

    try {
      const folder = await this.drive.getOrCreateAppFolder();
      const existing = await this.findDocFile(folder.id);
      if (!existing) return false;

      const blob = await this.drive.downloadMedia(existing.id);
      const text = await blob.text();
      const remoteDoc = loadDoc(text);
      if (!remoteDoc) return false;

      // Replace local doc with remote
      this.store.reset(remoteDoc);
      this.setState('idle');
      return true;
    } catch {
      return false;
    }
  }

  /**
   * Attempt to resolve a conflict by pushing local changes.
   * Returns true if successful.
   */
  async keepLocal(): Promise<boolean> {
    if (this.state !== 'conflict') return false;
    this.setState('localDirty');
    return true;
  }

  /** Ensure the latest sidecar exists in Drive and return its file record. */
  async currentDriveFile(): Promise<DriveFile | null> {
    await this.start();
    const folder = await this.drive.getOrCreateAppFolder();
    return this.findDocFile(folder.id);
  }

  /**
   * Mark that the PDF has been regenerated and needs re-upload.
   */
  markPdfStale(): void {
    if (this.state === 'idle') {
      this.setState('pdfStale');
    }
  }

  /**
   * Acknowledge PDF stale state and mark it resolved.
   */
  resolvePdfStale(): void {
    if (this.state === 'pdfStale') {
      this.setState('localDirty');
    }
  }
}

// ── DocMeta ───────────────────────────────────────────────────────────────────

export interface DocMeta {
  name: string;
  fileId?: string;
}
