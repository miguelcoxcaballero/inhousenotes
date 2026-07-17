// Document snapshots in IndexedDB: one record per doc (order + meta) and
// one record per page. Loading replays any oplog entries that survived a
// crash — ops are idempotent, so replaying an already-compacted op is a
// no-op rather than a duplicate.

import type { Doc, Img, Page, Stroke } from '../core/model';
import { applyOp } from '../core/ops';
import { deserializeDoc } from '../core/serial';
import type { SerialPage, SerialStroke } from '../core/serial';
import { docKeyRange, req, txDone, STORE_ASSETS, STORE_DOCS, STORE_OPLOG, STORE_PAGES, STORE_VERSIONS } from './idb';
import type { BinaryAsset } from './assets';
import type { OplogRecord } from './oplog';

export interface DocRecord {
  id: string;
  rev: number;
  pageOrder: string[];
  pageTombstones: string[];
  meta: Doc['meta'];
  savedAt: number;
  /** Denormalized for the document list UI. */
  name: string;
  pageCount: number;
  /** Revision represented by the page records written with this metadata. */
  snapshotRev?: number;
}

type PageRecord = SerialPage & { docId: string; snapshotRev?: number };

export interface IncrementalSaveOptions {
  throughSeq?: number;
  dirtyPageIds: Iterable<string>;
  deletedPageIds?: Iterable<string>;
}

export async function saveDocSnapshot(db: IDBDatabase, doc: Doc, opts: { throughSeq?: number } = {}): Promise<void> {
  await saveDocRecords(db, doc, {
    throughSeq: opts.throughSeq,
    dirtyPageIds: doc.pageOrder,
    deleteUnknownPages: true
  });
}

/** Write only pages touched since the previous compaction. */
export async function saveDocChanges(
  db: IDBDatabase,
  doc: Doc,
  opts: IncrementalSaveOptions
): Promise<void> {
  await saveDocRecords(db, doc, {
    ...opts,
    deleteUnknownPages: false
  });
}

async function saveDocRecords(
  db: IDBDatabase,
  doc: Doc,
  opts: IncrementalSaveOptions & { deleteUnknownPages: boolean }
): Promise<void> {
  // Capture synchronously before opening the transaction. Ops created while
  // this write is pending remain in the oplog and in the next dirty set.
  const snapshotRev = doc.rev;
  const dirtyPages = [...new Set(opts.dirtyPageIds)]
    .map((id) => doc.pages.get(id))
    .filter((page): page is Page => !!page)
    .map((page) => toPageRecord(doc.id, snapshotRev, page));
  const deletedPageIds = new Set(opts.deletedPageIds ?? []);
  const stores: string[] = [STORE_DOCS, STORE_PAGES];
  const clearOplog = typeof opts.throughSeq === 'number';
  if (clearOplog) stores.push(STORE_OPLOG);

  const tx = db.transaction(stores, 'readwrite');
  const docsStore = tx.objectStore(STORE_DOCS);
  const pagesStore = tx.objectStore(STORE_PAGES);

  const record: DocRecord = {
    id: doc.id,
    rev: snapshotRev,
    pageOrder: [...doc.pageOrder],
    pageTombstones: [...doc.pageTombstones],
    meta: { ...doc.meta },
    savedAt: Date.now(),
    name: doc.meta.name,
    pageCount: doc.pageOrder.length,
    snapshotRev
  };
  docsStore.put(record);

  for (const page of dirtyPages) {
    pagesStore.put(page);
  }
  for (const pageId of deletedPageIds) {
    pagesStore.delete([doc.id, pageId]);
  }

  if (opts.deleteUnknownPages) {
    const liveIds = new Set(doc.pageOrder);
    const existingKeys = (await req(pagesStore.getAllKeys(docKeyRange(doc.id)))) as [string, string][];
    for (const key of existingKeys) {
      if (!liveIds.has(key[1])) pagesStore.delete(key);
    }
  }

  if (clearOplog) {
    tx.objectStore(STORE_OPLOG).delete(
      IDBKeyRange.bound([doc.id], [doc.id, opts.throughSeq], false, false)
    );
  }
  await txDone(tx);
}

function toPageRecord(docId: string, snapshotRev: number, page: Page): PageRecord {
  return {
    docId,
    snapshotRev,
    id: page.id,
    width: page.width,
    height: page.height,
    background: page.background,
    strokes: page.strokeOrder
      .map((id) => page.strokes.get(id))
      .filter((stroke): stroke is Stroke => !!stroke)
      .map((stroke): SerialStroke => ({
        id: stroke.id,
        tool: stroke.tool,
        color: stroke.color,
        width: stroke.width,
        points: stroke.points
      })),
    images: page.imageOrder
      .map((id) => page.images.get(id))
      .filter((image): image is Img => !!image)
      .map((image) => ({ ...image })),
    tombstones: [...page.tombstones],
    sidePanel: page.sidePanel ? { ...page.sidePanel, dateKeys: [...page.sidePanel.dateKeys] } : null
  };
}

export interface LoadedDoc {
  doc: Doc;
  savedAt: number;
  /** Oplog entries replayed on top of the snapshot (crash recovery). */
  replayed: number;
  assets: BinaryAsset[];
}

export async function loadDoc(db: IDBDatabase, docId: string): Promise<LoadedDoc | null> {
  const tx = db.transaction([STORE_DOCS, STORE_PAGES, STORE_OPLOG, STORE_ASSETS], 'readonly');
  const record = (await req(tx.objectStore(STORE_DOCS).get(docId))) as DocRecord | undefined;
  if (!record) return null;
  const pageRecords = (await req(tx.objectStore(STORE_PAGES).getAll(docKeyRange(docId)))) as PageRecord[];
  const opRecords = (await req(tx.objectStore(STORE_OPLOG).getAll(docKeyRange(docId)))) as OplogRecord[];
  const assetRecords = (await req(tx.objectStore(STORE_ASSETS).getAll(docKeyRange(docId)))) as (BinaryAsset & { docId: string })[];

  const byId = new Map(pageRecords.map((p) => [p.id, p]));
  const doc = deserializeDoc({
    id: record.id,
    rev: record.rev,
    pageOrder: record.pageOrder,
    pages: record.pageOrder.map((id) => byId.get(id)).filter((p): p is PageRecord => !!p),
    pageTombstones: record.pageTombstones,
    meta: record.meta
  });

  opRecords.sort((a, b) => a.seq - b.seq);
  let replayed = 0;
  for (const op of opRecords) {
    try {
      applyOp(doc, op.op);
      replayed++;
    } catch {
      // A malformed trailing record (e.g. interrupted write) must not block
      // opening the document.
    }
  }
  const assets = assetRecords.map(({ docId: _docId, ...asset }) => asset);
  return { doc, savedAt: record.savedAt, replayed, assets };
}

export async function listDocs(db: IDBDatabase): Promise<DocRecord[]> {
  const tx = db.transaction(STORE_DOCS, 'readonly');
  const all = (await req(tx.objectStore(STORE_DOCS).getAll())) as DocRecord[];
  return all.sort((a, b) => b.savedAt - a.savedAt);
}

export async function deleteDoc(db: IDBDatabase, docId: string): Promise<void> {
  const tx = db.transaction([STORE_DOCS, STORE_PAGES, STORE_OPLOG, STORE_VERSIONS, STORE_ASSETS], 'readwrite');
  tx.objectStore(STORE_DOCS).delete(docId);
  tx.objectStore(STORE_PAGES).delete(docKeyRange(docId));
  tx.objectStore(STORE_OPLOG).delete(docKeyRange(docId));
  tx.objectStore(STORE_VERSIONS).delete(docKeyRange(docId));
  tx.objectStore(STORE_ASSETS).delete(docKeyRange(docId));
  await txDone(tx);
}
