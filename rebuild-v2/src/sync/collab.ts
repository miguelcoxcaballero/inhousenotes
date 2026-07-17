// Collaborative editing via polling Drive for remote changes.
// Merges incoming changes with local changes using the existing merge.ts logic.

import type { JsonDoc } from '../core/serial';
import { toJsonDoc, fromJsonDoc, serializeDoc } from '../core/serial';
import { mergeDocs } from '../core/merge';
import type { DocStore } from '../core/store';
import type { SyncMachine } from './syncMachine';
import type { DriveClient } from './driveClient';
import { unpackDoc } from './sidecar';

// ── Presence handler types ────────────────────────────────────────────────────

export interface PresenceInfo {
  userId: string;
  name: string;
  avatar?: string;
  cursorPage?: string;
  lastSeen: number;
}

export type PresenceHandler = (presences: PresenceInfo[]) => void;

// ── CollabClient ──────────────────────────────────────────────────────────────

const POLL_INTERVAL_MS = 5000;

export class CollabClient {
  private pollTimer: ReturnType<typeof setInterval> | null = null;
  private lastRemoteRev = 0;
  private baseSnapshot: JsonDoc | null = null;
  private disposed = false;
  private presenceHandlers = new Set<PresenceHandler>();
  private drive: DriveClient;
  private fileId: string | null = null;
  private docMeta: { name: string } | null = null;

  onRemoteChange: ((doc: JsonDoc) => void) | null = null;
  /** Called when a merge conflict cannot be automatically resolved. */
  onConflict: (() => void) | null = null;

  constructor(
    private store: DocStore,
    private sync: SyncMachine,
    drive: DriveClient,
    private docId: string,
    fileId?: string,
    docName?: string
  ) {
    this.drive = drive;
    this.fileId = fileId ?? null;
    this.docMeta = docName ? { name: docName } : null;
    this.lastRemoteRev = store.doc.rev;
    this.baseSnapshot = serializeDoc(store.doc);
  }

  /** Start polling for remote changes. */
  start(): void {
    if (this.disposed) return;

    // Get the file ID from the doc meta if not provided
    this.resolveFileId();

    this.stop(); // Clear any existing poll
    this.poll();
    this.pollTimer = setInterval(() => this.poll(), POLL_INTERVAL_MS);
  }

  /** Stop polling and clean up. */
  stop(): void {
    if (this.pollTimer) {
      clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
  }

  /** Register a handler for presence updates. */
  onPresence(handler: PresenceHandler): () => void {
    this.presenceHandlers.add(handler);
    return () => this.presenceHandlers.delete(handler);
  }

  /** Clean up and release resources. */
  dispose(): void {
    this.disposed = true;
    this.stop();
    this.presenceHandlers.clear();
  }

  // ── Private helpers ─────────────────────────────────────────────────────────

  private async resolveFileId(): Promise<void> {
    if (this.fileId || !this.docMeta) {
      // File ID was provided at construction
      return;
    }

    try {
      const folder = await this.drive.getOrCreateAppFolder();
      const fileName = `${this.docMeta.name}.ihn.json`;
      const result = await this.drive.listFiles({
        folderId: folder.id,
        mimeType: 'application/json'
      });

      const file = result.files.find((f) => f.name === fileName);
      if (file) {
        this.fileId = file.id;
      }
    } catch {
      // Will retry on next poll
    }
  }

  private async poll(): Promise<void> {
    if (this.disposed) return;
    if (!this.fileId) {
      await this.resolveFileId();
      if (!this.fileId) return;
    }

    try {
      // Download the current remote document
      const blob = await this.drive.downloadMedia(this.fileId);
      const text = await blob.text();
      const remoteDoc = unpackDoc(text);

      if (!remoteDoc) return;

      // Skip if no change
      if (remoteDoc.rev <= this.lastRemoteRev) return;

      // Check if there are local uncommitted changes
      const localDoc = toJsonDoc(this.store.doc);
      const hasLocalChanges = localDoc.rev > this.lastRemoteRev;

      if (hasLocalChanges) {
        // Three-way merge
        const base = this.lastRemoteRev > 0 ? await this.getBaseSnapshot() : null;
        const result = mergeDocs(
          serializeDoc(this.store.doc),
          remoteDoc,
          base
        );

        if (result.changedFromRemote || result.changedFromLocal) {
          // Apply merged document
          const merged = fromJsonDoc(result.merged);
          this.store.reset(merged);
          this.onRemoteChange?.(result.merged);
          this.baseSnapshot = result.merged;
        }
      } else {
        // No local changes — just adopt remote
        const doc = fromJsonDoc(remoteDoc);
        this.store.reset(doc);
        this.onRemoteChange?.(remoteDoc);
        this.baseSnapshot = remoteDoc;
      }

      this.lastRemoteRev = remoteDoc.rev;
    } catch (err) {
      // Network errors are non-fatal for collaborative editing
      console.warn('Collab poll failed:', err);
    }
  }

  private async getBaseSnapshot(): Promise<JsonDoc | null> {
    return this.baseSnapshot;
  }

  /** Broadcast current presence state to Drive file properties. */
  async broadcastPresence(pageId?: string): Promise<void> {
    if (this.disposed || !this.fileId) return;

    try {
      await this.drive.patch(`/files/${this.fileId}`, {
        properties: {
          lastEditor: 'user', // Would be actual user info
          lastEditPage: pageId ?? ''
        }
      });
    } catch {
      // Ignore presence broadcast errors
    }
  }
}
