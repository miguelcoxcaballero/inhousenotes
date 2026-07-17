// Drive synchronization with read-before-write merging. The sidecar is the
// source of truth; PDFs are derived export artifacts.

import { newId } from '../core/ids';
import { mergeDocs } from '../core/merge';
import { serializeDoc, type SerialDoc } from '../core/serial';
import type { DocStore } from '../core/store';
import { getPdfAsset, getPdfAssetsForDoc } from '../pdf/pdfAssets';
import type { BinaryAsset } from '../persist/assets';
import type { PersistController } from '../persist/persistController';
import type { DriveClient, DriveFile } from './driveClient';
import {
  packDoc,
  unpackBundle,
  type SidecarAssetReference,
  type SidecarBundle
} from './sidecar';

const AUTO_SYNC_DELAY_MS = 1800;

export type SyncState = 'idle' | 'localDirty' | 'syncing' | 'pdfStale' | 'conflict';

export interface SyncStatus {
  state: SyncState;
  saving: boolean;
  driveEnabled: boolean;
  lastSaved: number | null;
  lastDriveSyncAt: number | null;
  error: string | null;
}

export type StatusChangeHandler = (status: SyncStatus) => void;
export type RemoteAssetHandler = (assets: BinaryAsset[]) => void | Promise<void>;

export class SyncMachine {
  private state: SyncState = 'idle';
  private saving = false;
  private lastSaved: number | null = null;
  private lastDriveSyncAt: number | null = null;
  private error: string | null = null;
  private unsubscribers: (() => void)[] = [];
  private statusHandlers = new Set<StatusChangeHandler>();
  private disposed = false;
  private autoSyncTimer: ReturnType<typeof setTimeout> | null = null;
  private dirtyWhileSyncing = false;
  private syncPromise: Promise<void> | null = null;
  private baseSnapshot: SerialDoc | null;
  private lastRemoteWriteId: string | null;
  private pendingRemote: SidecarBundle | null = null;
  private overwriteRemoteOnce = false;
  private assetRefs = new Map<string, SidecarAssetReference>();

  readonly store: DocStore;
  readonly persist: PersistController;
  readonly drive: DriveClient;

  constructor(
    store: DocStore,
    persist: PersistController,
    drive: DriveClient,
    private docMeta: DocMeta,
    private onRemoteAssets: RemoteAssetHandler = () => undefined
  ) {
    this.store = store;
    this.persist = persist;
    this.drive = drive;
    this.lastSaved = persist.lastSavedAt;
    this.lastRemoteWriteId = docMeta.writeId ?? null;
    this.baseSnapshot = docMeta.writeId ? serializeDoc(store.doc) : null;

    this.unsubscribers.push(store.subscribe((applied) => {
      if (applied.source === 'local' && !this.disposed) this.markDirty();
    }));
    persist.onSaved = (savedAt) => {
      this.lastSaved = savedAt;
      this.emit();
    };
    persist.onError = (err) => {
      this.error = err instanceof Error ? err.message : String(err);
      this.emit();
    };
  }

  async start(): Promise<void> {
    if (this.disposed || this.state === 'conflict' || !this.drive.isSignedIn()) return;
    this.clearAutoSyncTimer();
    if (this.syncPromise) return this.syncPromise;
    this.setState('syncing');
    this.syncPromise = this.syncToDrive().finally(() => {
      this.syncPromise = null;
    });
    return this.syncPromise;
  }

  markDirty(): void {
    if (this.state === 'syncing') {
      this.dirtyWhileSyncing = true;
      return;
    }
    if (this.state !== 'conflict') {
      this.setState('localDirty');
      this.scheduleAutoSync();
    }
  }

  onStatusChange(handler: StatusChangeHandler): () => void {
    this.statusHandlers.add(handler);
    return () => this.statusHandlers.delete(handler);
  }

  get status(): SyncStatus {
    return {
      state: this.state,
      saving: this.saving,
      driveEnabled: this.drive.isSignedIn(),
      lastSaved: this.lastSaved,
      lastDriveSyncAt: this.lastDriveSyncAt,
      error: this.error
    };
  }

  dispose(): void {
    this.disposed = true;
    this.clearAutoSyncTimer();
    for (const unsubscribe of this.unsubscribers) unsubscribe();
    this.unsubscribers = [];
    this.statusHandlers.clear();
  }

  /** Pull a newer Drive sidecar and merge it without uploading. */
  async refreshFromDrive(): Promise<boolean> {
    if (this.disposed || this.saving || !this.drive.isSignedIn()) return false;
    try {
      const folder = await this.drive.getOrCreateAppFolder();
      const existing = await this.findDocFile(folder.id);
      if (!existing) return false;
      const remote = await this.downloadBundle(existing.id);
      if (!remote || remote.writeId === this.lastRemoteWriteId) return false;
      if (remote.doc.id !== this.store.doc.id) return false;

      await this.acceptBundleAssets(remote);
      const result = mergeDocs(serializeDoc(this.store.doc), remote.doc, this.baseSnapshot);
      if (result.changedFromLocal) this.applySnapshot(result.merged);
      this.lastRemoteWriteId = remote.writeId;
      this.baseSnapshot = remote.doc;
      this.lastDriveSyncAt = remote.savedAt;
      this.error = null;

      if (result.changedFromRemote) {
        this.setState('localDirty');
        this.scheduleAutoSync();
      } else {
        this.setState('idle');
      }
      return result.changedFromLocal;
    } catch (err) {
      this.error = err instanceof Error ? err.message : String(err);
      this.emit();
      return false;
    }
  }

  async acceptRemote(): Promise<boolean> {
    if (this.state !== 'conflict') return false;
    try {
      const remote = this.pendingRemote ?? await this.fetchCurrentBundle();
      if (!remote || remote.doc.id !== this.store.doc.id) return false;
      await this.acceptBundleAssets(remote);
      this.applySnapshot(remote.doc);
      this.baseSnapshot = remote.doc;
      this.lastRemoteWriteId = remote.writeId;
      this.pendingRemote = null;
      this.error = null;
      this.setState('idle');
      return true;
    } catch {
      return false;
    }
  }

  async keepLocal(): Promise<boolean> {
    if (this.state !== 'conflict') return false;
    this.pendingRemote = null;
    this.overwriteRemoteOnce = true;
    this.error = null;
    this.setState('localDirty');
    await this.start();
    return this.state !== 'conflict';
  }

  async currentDriveFile(): Promise<DriveFile | null> {
    if (!this.drive.isSignedIn()) return null;
    await this.start();
    const folder = await this.drive.getOrCreateAppFolder();
    return this.findDocFile(folder.id);
  }

  driveAssetFileIds(): string[] {
    return [...this.assetRefs.values()].map((reference) => reference.fileId);
  }

  markPdfStale(): void {
    if (this.state === 'idle') this.setState('pdfStale');
  }

  resolvePdfStale(): void {
    if (this.state === 'pdfStale') this.markDirty();
  }

  private async syncToDrive(): Promise<void> {
    this.saving = true;
    this.error = null;
    this.dirtyWhileSyncing = false;
    this.emit();

    try {
      const folder = await this.drive.getOrCreateAppFolder();
      let existing = await this.findDocFile(folder.id);
      let parentWriteId: string | null = null;

      if (existing && !this.overwriteRemoteOnce) {
        const remote = await this.downloadBundle(existing.id);
        if (!remote) throw new Error('Drive document is damaged or uses an unsupported format');

        // A matching filename can belong to a different local notebook. Keep
        // both instead of overwriting either one.
        if (remote.doc.id !== this.store.doc.id) {
          existing = null;
          this.docMeta.fileId = undefined;
        } else {
          await this.acceptBundleAssets(remote);
          const result = mergeDocs(serializeDoc(this.store.doc), remote.doc, this.baseSnapshot);
          if (result.changedFromLocal) this.applySnapshot(result.merged);
          parentWriteId = remote.writeId;
          this.lastRemoteWriteId = remote.writeId;
        }
      } else if (existing) {
        parentWriteId = this.lastRemoteWriteId;
      }

      const now = Date.now();
      const writeId = newId();
      const uploadedSnapshot = serializeDoc(this.store.doc);
      const assetRefs = await this.ensureDriveAssetRefs(folder.id);
      const packed = packDoc(this.store.doc, now, {
        writeId,
        parentWriteId,
        assetRefs,
        inlineAssets: false
      });
      const blob = new Blob([packed], { type: 'application/json' });
      const fileName = existing
        ? `${this.docMeta.name}.ihn.json`
        : await this.availableFileName(folder.id);

      let file: DriveFile;
      if (existing) {
        file = await this.drive.updateFileMedia(existing.id, blob, 'application/json');
        if (file.name !== fileName) {
          file = await this.drive.patch<DriveFile>(`/files/${existing.id}`, { name: fileName }, {
            fields: DRIVE_FILE_FIELDS
          });
        }
      } else {
        file = await this.drive.createFile(fileName, blob, 'application/json', [folder.id]);
      }

      this.docMeta.fileId = file.id;
      this.docMeta.writeId = writeId;
      this.baseSnapshot = uploadedSnapshot;
      this.lastRemoteWriteId = writeId;
      this.pendingRemote = null;
      this.overwriteRemoteOnce = false;
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
      if (this.state !== 'conflict') this.setState('localDirty');
    } finally {
      this.saving = false;
      this.emit();
    }
  }

  private applySnapshot(snapshot: SerialDoc): void {
    this.store.apply({
      type: 'replace-doc',
      pages: snapshot.pages,
      meta: snapshot.meta,
      pageTombstones: snapshot.pageTombstones
    }, 'remote');
  }

  private async fetchCurrentBundle(): Promise<SidecarBundle | null> {
    const folder = await this.drive.getOrCreateAppFolder();
    const existing = await this.findDocFile(folder.id);
    return existing ? this.downloadBundle(existing.id) : null;
  }

  private async downloadBundle(fileId: string): Promise<SidecarBundle | null> {
    const blob = await this.drive.downloadMedia(fileId);
    return unpackBundle(await blob.text());
  }

  private async acceptBundleAssets(bundle: SidecarBundle): Promise<void> {
    for (const reference of bundle.assetRefs) this.assetRefs.set(reference.id, reference);
    const assets = [...bundle.assets];
    const inlineIds = new Set(assets.map((asset) => asset.id));
    for (const reference of bundle.assetRefs) {
      if (inlineIds.has(reference.id) || getPdfAsset(reference.id)) continue;
      const blob = await this.drive.downloadMedia(reference.fileId, {
        resourceKey: reference.resourceKey
      });
      assets.push({
        id: reference.id,
        name: reference.name,
        mimeType: reference.mimeType,
        createdAt: reference.createdAt,
        bytes: new Uint8Array(await blob.arrayBuffer())
      });
    }
    if (assets.length > 0) await this.onRemoteAssets(assets);
  }

  private async ensureDriveAssetRefs(folderId: string): Promise<SidecarAssetReference[]> {
    const result: SidecarAssetReference[] = [];
    for (const asset of getPdfAssetsForDoc(this.store.doc)) {
      let reference = this.assetRefs.get(asset.id);
      if (!reference) {
        const bytes = asset.bytes.buffer.slice(
          asset.bytes.byteOffset,
          asset.bytes.byteOffset + asset.bytes.byteLength
        ) as ArrayBuffer;
        const file = await this.drive.createFile(
          `${safeDriveName(this.docMeta.name)}.${asset.id}.source.pdf`,
          new Blob([bytes], { type: asset.mimeType }),
          asset.mimeType,
          [folderId]
        );
        reference = {
          id: asset.id,
          name: asset.name,
          mimeType: asset.mimeType,
          createdAt: asset.createdAt,
          fileId: file.id,
          ...(file.resourceKey ? { resourceKey: file.resourceKey } : {})
        };
        this.assetRefs.set(asset.id, reference);
      }
      result.push(reference);
    }
    return result;
  }

  private async findDocFile(folderId: string): Promise<DriveFile | null> {
    if (this.docMeta.fileId) {
      try {
        return await this.drive.get<DriveFile>(`/files/${this.docMeta.fileId}`, {
          fields: DRIVE_FILE_FIELDS
        });
      } catch {
        this.docMeta.fileId = undefined;
      }
    }
    const result = await this.drive.listFiles({
      folderId,
      mimeType: 'application/json',
      pageSize: 100
    });
    return result.files.find((file) => file.name === `${this.docMeta.name}.ihn.json`) ?? null;
  }

  private async availableFileName(folderId: string): Promise<string> {
    const base = `${this.docMeta.name}.ihn.json`;
    const result = await this.drive.listFiles({
      folderId,
      mimeType: 'application/json',
      pageSize: 100
    });
    if (!result.files.some((file) => file.name === base)) return base;
    return `${this.docMeta.name} (${this.store.doc.id.slice(-6)}).ihn.json`;
  }

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
        // Status UI failures must not affect persistence.
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
    if (this.autoSyncTimer) clearTimeout(this.autoSyncTimer);
    this.autoSyncTimer = null;
  }
}

const DRIVE_FILE_FIELDS = 'id,name,mimeType,modifiedTime,size,parents,shared,webContentLink,webViewLink,resourceKey';

function safeDriveName(value: string): string {
  return value.replace(/[<>:"/\\|?*]/g, '_').trim() || 'notebook';
}

export interface DocMeta {
  name: string;
  fileId?: string;
  writeId?: string;
}
