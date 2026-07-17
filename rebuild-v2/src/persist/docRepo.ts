// Document snapshots in IndexedDB: one record per doc (order + meta) and
// one record per page. Loading replays any oplog entries that survived a
// crash — ops are idempotent, so replaying an already-compacted op is a
// no-op rather than a duplicate.

import type { Doc } from '../core/model';
import { applyOp } from '../core/ops';
import type { SerialPage } from '../core/serial';
import { deserializeDoc, serializeDoc } from '../core/serial';
import { docKeyRange, req, txDone, STORE_DOCS, STORE_OPLOG, STORE_PAGES } from './idb';
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
}

type PageRecord = SerialPage & { docId: string };

export async function saveDocSnapshot(db: IDBDatabase, doc: Doc, opts: { throughSeq?: number } = {}): Promise<void> {
  const serial = serializeDoc(doc);
  const stores: string[] = [STORE_DOCS, STORE_PAGES];
  const clearOplog = typeof opts.throughSeq === 'number';
  if (clearOplog) stores.push(STORE_OPLOG);

  const tx = db.transaction(stores, 'readwrite');
  const docsStore = tx.objectStore(STORE_DOCS);
  const pagesStore = tx.objectStore(STORE_PAGES);

  const record: DocRecord = {
    id: serial.id,
    rev: serial.rev,
    pageOrder: serial.pageOrder,
    pageTombstones: serial.pageTombstones,
    meta: serial.meta,
    savedAt: Date.now(),
    name: serial.meta.name,
    pageCount: serial.pages.length
  };
  docsStore.put(record);

  const liveIds = new Set(serial.pageOrder);
  for (const page of serial.pages) {
    pagesStore.put({ ...page, docId: serial.id } satisfies PageRecord);
  }
  // Remove pages that no longer exist in the doc.
  const existingKeys = (await req(pagesStore.getAllKeys(docKeyRange(serial.id)))) as [string, string][];
  for (const key of existingKeys) {
    if (!liveIds.has(key[1])) pagesStore.delete(key);
  }

  if (clearOplog) {
    tx.objectStore(STORE_OPLOG).delete(
      IDBKeyRange.bound([serial.id], [serial.id, opts.throughSeq], false, false)
    );
  }
  await txDone(tx);
}

export interface LoadedDoc {
  doc: Doc;
  savedAt: number;
  /** Oplog entries replayed on top of the snapshot (crash recovery). */
  replayed: number;
}

export async function loadDoc(db: IDBDatabase, docId: string): Promise<LoadedDoc | null> {
  const tx = db.transaction([STORE_DOCS, STORE_PAGES, STORE_OPLOG], 'readonly');
  const record = (await req(tx.objectStore(STORE_DOCS).get(docId))) as DocRecord | undefined;
  if (!record) return null;
  const pageRecords = (await req(tx.objectStore(STORE_PAGES).getAll(docKeyRange(docId)))) as PageRecord[];
  const opRecords = (await req(tx.objectStore(STORE_OPLOG).getAll(docKeyRange(docId)))) as OplogRecord[];

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
  return { doc, savedAt: record.savedAt, replayed };
}

export async function listDocs(db: IDBDatabase): Promise<DocRecord[]> {
  const tx = db.transaction(STORE_DOCS, 'readonly');
  const all = (await req(tx.objectStore(STORE_DOCS).getAll())) as DocRecord[];
  return all.sort((a, b) => b.savedAt - a.savedAt);
}

export async function deleteDoc(db: IDBDatabase, docId: string): Promise<void> {
  const tx = db.transaction([STORE_DOCS, STORE_PAGES, STORE_OPLOG], 'readwrite');
  tx.objectStore(STORE_DOCS).delete(docId);
  tx.objectStore(STORE_PAGES).delete(docKeyRange(docId));
  tx.objectStore(STORE_OPLOG).delete(docKeyRange(docId));
  await txDone(tx);
}
