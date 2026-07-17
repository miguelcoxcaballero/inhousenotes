// Lightweight Drive polling. SyncMachine owns document merging so polling and
// manual sync cannot apply competing replacement paths.

import type { DocStore } from '../core/store';
import type { DriveClient } from './driveClient';
import type { SyncMachine } from './syncMachine';

export interface PresenceInfo {
  userId: string;
  name: string;
  avatar?: string;
  cursorPage?: string;
  lastSeen: number;
}

export type PresenceHandler = (presences: PresenceInfo[]) => void;

const POLL_INTERVAL_MS = 5000;

export class CollabClient {
  private pollTimer: ReturnType<typeof setInterval> | null = null;
  private disposed = false;
  private presenceHandlers = new Set<PresenceHandler>();
  private fileId: string | null;

  onRemoteChange: (() => void) | null = null;
  onConflict: (() => void) | null = null;

  constructor(
    private store: DocStore,
    private sync: SyncMachine,
    private drive: DriveClient,
    private docId: string,
    fileId?: string,
    private docName?: string
  ) {
    this.fileId = fileId ?? null;
  }

  start(): void {
    if (this.disposed) return;
    this.stop();
    void this.poll();
    this.pollTimer = setInterval(() => void this.poll(), POLL_INTERVAL_MS);
  }

  stop(): void {
    if (this.pollTimer) clearInterval(this.pollTimer);
    this.pollTimer = null;
  }

  onPresence(handler: PresenceHandler): () => void {
    this.presenceHandlers.add(handler);
    return () => this.presenceHandlers.delete(handler);
  }

  dispose(): void {
    this.disposed = true;
    this.stop();
    this.presenceHandlers.clear();
  }

  async broadcastPresence(pageId?: string): Promise<void> {
    if (this.disposed || !this.drive.isSignedIn()) return;
    await this.resolveFileId();
    if (!this.fileId) return;
    try {
      await this.drive.patch(`/files/${this.fileId}`, {
        properties: { lastEditor: 'user', lastEditPage: pageId ?? '' }
      });
    } catch {
      // Presence is best effort.
    }
  }

  private async poll(): Promise<void> {
    if (this.disposed || !this.drive.isSignedIn()) return;
    const changed = await this.sync.refreshFromDrive();
    if (changed) this.onRemoteChange?.();
    if (this.sync.status.state === 'conflict') this.onConflict?.();
  }

  private async resolveFileId(): Promise<void> {
    if (this.fileId || !this.docName || !this.drive.isSignedIn()) return;
    try {
      const folder = await this.drive.getOrCreateAppFolder();
      const result = await this.drive.listFiles({ folderId: folder.id, mimeType: 'application/json' });
      const file = result.files.find((entry) => entry.name === `${this.docName}.ihn.json`);
      this.fileId = file?.id ?? null;
    } catch {
      // Retried on the next presence update.
    }
  }
}
