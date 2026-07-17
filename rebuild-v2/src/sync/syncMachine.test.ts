import { describe, expect, it } from 'vitest';
import { createDoc, makeStroke, packPoints } from '../core/model';
import { addStrokeOp, applyOp } from '../core/ops';
import { deserializeDoc, serializeDoc, serializeStroke } from '../core/serial';
import { DocStore } from '../core/store';
import type { PersistController } from '../persist/persistController';
import type { DriveClient, DriveFile } from './driveClient';
import { packDoc, unpackDoc } from './sidecar';
import { SyncMachine } from './syncMachine';
import { registerPdfAssets, unregisterPdfAssets } from '../pdf/pdfAssets';

class FakeDrive {
  remoteText = '';
  remoteFile: DriveFile = {
    id: 'remote-file',
    name: 'Notebook.ihn.json',
    mimeType: 'application/json',
    modifiedTime: new Date(0).toISOString()
  };
  updated = 0;
  created = 0;
  assetCreated = 0;
  createdName = '';
  hasRemote = true;

  isSignedIn(): boolean { return true; }
  async getOrCreateAppFolder(): Promise<DriveFile> {
    return { ...this.remoteFile, id: 'folder', name: 'Inhouse Notes' };
  }
  async get<T>(): Promise<T> { return this.remoteFile as T; }
  async listFiles(): Promise<{ files: DriveFile[] }> { return { files: this.hasRemote ? [this.remoteFile] : [] }; }
  async downloadMedia(): Promise<Blob> { return new Blob([this.remoteText]); }
  async updateFileMedia(_id: string, blob: Blob): Promise<DriveFile> {
    this.updated++;
    this.remoteText = await blob.text();
    return this.remoteFile;
  }
  async createFile(name: string, blob: Blob, mimeType: string): Promise<DriveFile> {
    if (mimeType !== 'application/json') {
      this.assetCreated++;
      return { ...this.remoteFile, id: `asset-file-${this.assetCreated}`, name, mimeType };
    }
    this.created++;
    this.createdName = name;
    this.remoteText = await blob.text();
    this.hasRemote = true;
    this.remoteFile = { ...this.remoteFile, id: 'created-file', name };
    return this.remoteFile;
  }
  async patch<T>(): Promise<T> { return this.remoteFile as T; }
}

function persistStub(): PersistController {
  return { onSaved: null, onError: null } as unknown as PersistController;
}

function addTestStroke(store: DocStore, x: number): string {
  const stroke = makeStroke({
    tool: 'pen',
    color: '#000000',
    width: 2,
    points: packPoints([{ x, y: 0 }, { x, y: 20 }])
  });
  store.apply(addStrokeOp(store.doc.pageOrder[0]!, serializeStroke(stroke)));
  return stroke.id;
}

describe('safe Drive synchronization', () => {
  it('merges concurrent local and remote strokes before uploading', async () => {
    const base = createDoc({ name: 'Notebook' });
    const localStore = new DocStore(deserializeDoc(serializeDoc(base)));
    const remote = deserializeDoc(serializeDoc(base));
    const drive = new FakeDrive();
    const sync = new SyncMachine(
      localStore,
      persistStub(),
      drive as unknown as DriveClient,
      { name: 'Notebook', fileId: drive.remoteFile.id, writeId: 'base-write' }
    );

    const localId = addTestStroke(localStore, 10);
    const remoteStroke = makeStroke({
      tool: 'pen',
      color: '#ff0000',
      width: 2,
      points: packPoints([{ x: 90, y: 0 }, { x: 90, y: 20 }])
    });
    applyOp(remote, addStrokeOp(remote.pageOrder[0]!, serializeStroke(remoteStroke)));
    drive.remoteText = packDoc(remote, 10, { writeId: 'remote-write', parentWriteId: 'base-write' });

    await sync.start();
    const uploaded = unpackDoc(drive.remoteText)!;
    const uploadedStrokes = uploaded.pages[0]!.strokes.map((stroke) => stroke.id);
    expect(uploadedStrokes).toContain(localId);
    expect(uploadedStrokes).toContain(remoteStroke.id);
    expect(drive.updated).toBe(1);
    sync.dispose();
  });

  it('creates a separate file when the matching name belongs to another doc', async () => {
    const local = createDoc({ name: 'Notebook' });
    const unrelated = createDoc({ name: 'Notebook' });
    const drive = new FakeDrive();
    drive.remoteText = packDoc(unrelated, 10, { writeId: 'other-write' });
    const sync = new SyncMachine(
      new DocStore(local),
      persistStub(),
      drive as unknown as DriveClient,
      { name: 'Notebook' }
    );

    await sync.start();
    expect(drive.updated).toBe(0);
    expect(drive.created).toBe(1);
    expect(drive.createdName).not.toBe('Notebook.ihn.json');
    expect(unpackDoc(drive.remoteText)!.id).toBe(local.id);
    sync.dispose();
  });

  it('uploads a source PDF once and keeps later sidecars small', async () => {
    const local = createDoc({ name: 'PDF notebook' });
    const page = local.pages.get(local.pageOrder[0]!)!;
    page.background = { kind: 'pdf', sourceId: 'pdf-asset', pdfPageIndex: 0 };
    registerPdfAssets([{
      id: 'pdf-asset',
      name: 'source.pdf',
      mimeType: 'application/pdf',
      bytes: new Uint8Array(200_000).fill(7),
      createdAt: 12
    }]);
    const drive = new FakeDrive();
    drive.hasRemote = false;
    const store = new DocStore(local);
    const sync = new SyncMachine(
      store,
      persistStub(),
      drive as unknown as DriveClient,
      { name: 'PDF notebook' }
    );

    await sync.start();
    const firstSidecar = drive.remoteText;
    const bundle = unpackDoc(firstSidecar)!;
    expect(bundle.pages[0]!.background).toEqual(page.background);
    expect(drive.assetCreated).toBe(1);
    expect(firstSidecar.length).toBeLessThan(20_000);

    addTestStroke(store, 30);
    await sync.start();
    expect(drive.assetCreated).toBe(1);
    expect(drive.remoteText.length).toBeLessThan(20_000);
    sync.dispose();
    unregisterPdfAssets(['pdf-asset']);
  });
});
