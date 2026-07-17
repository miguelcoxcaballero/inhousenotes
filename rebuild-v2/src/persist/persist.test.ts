import { beforeEach, describe, expect, it } from 'vitest';
import { IDBFactory, IDBKeyRange as FakeKeyRange } from 'fake-indexeddb';
import { createDoc, createPage, makeStroke, packPoints } from '../core/model';
import { addStrokeOp, applyOp, removeStrokesOp } from '../core/ops';
import { serializeStroke } from '../core/serial';
import { DocStore } from '../core/store';
import { openDb, req, resetDbCache, STORE_PAGES } from './idb';
import { loadDoc, saveDocChanges, saveDocSnapshot, listDocs } from './docRepo';
import { OplogWriter } from './oplog';
import { PersistController } from './persistController';
import { saveAssets } from './assets';

// fake-indexeddb needs its own IDBKeyRange implementation globally.
(globalThis as Record<string, unknown>).IDBKeyRange = FakeKeyRange;

let factory: IDBFactory;

beforeEach(() => {
  factory = new IDBFactory();
  resetDbCache();
});

function drawOn(store: DocStore, x = 0): string {
  const pageId = store.doc.pageOrder[0]!;
  const stroke = makeStroke({
    tool: 'pen',
    color: '#000',
    width: 2,
    points: packPoints([{ x, y: 0 }, { x, y: 10 }])
  });
  store.apply(addStrokeOp(pageId, serializeStroke(stroke)));
  return stroke.id;
}

describe('doc snapshot round-trip', () => {
  it('saves and loads a document identically', async () => {
    const db = await openDb(factory);
    const doc = createDoc({ name: 'test', pages: [createPage(), createPage()] });
    const store = new DocStore(doc);
    const ids = [drawOn(store, 1), drawOn(store, 2)];

    await saveDocSnapshot(db, doc);
    const loaded = await loadDoc(db, doc.id);

    expect(loaded).not.toBeNull();
    expect(loaded!.doc.pageOrder).toEqual(doc.pageOrder);
    const page = loaded!.doc.pages.get(doc.pageOrder[0]!)!;
    expect(page.strokeOrder).toEqual(ids);
    expect([...page.strokes.get(ids[0]!)!.points]).toEqual([1, 0, 0.5, 1, 10, 0.5]);
  });

  it('a page removed from the doc disappears from the page store', async () => {
    const db = await openDb(factory);
    const doc = createDoc({ pages: [createPage(), createPage()] });
    await saveDocSnapshot(db, doc);

    const removedId = doc.pageOrder[1]!;
    applyOp(doc, { type: 'remove-page', pageId: removedId });
    await saveDocSnapshot(db, doc);

    const loaded = await loadDoc(db, doc.id);
    expect(loaded!.doc.pageOrder).toHaveLength(1);
    expect(loaded!.doc.pages.has(removedId)).toBe(false);
    expect(loaded!.doc.pageTombstones.has(removedId)).toBe(true);
  });

  it('lists documents most recent first', async () => {
    const db = await openDb(factory);
    const a = createDoc({ name: 'a' });
    const b = createDoc({ name: 'b' });
    await saveDocSnapshot(db, a);
    await new Promise((resolve) => setTimeout(resolve, 5));
    await saveDocSnapshot(db, b);
    const docs = await listDocs(db);
    expect(docs.map((d) => d.name)).toEqual(['b', 'a']);
  });

  it('rewrites only dirty pages during incremental compaction', async () => {
    const db = await openDb(factory);
    const doc = createDoc({ pages: [createPage(), createPage()] });
    const store = new DocStore(doc);
    await saveDocSnapshot(db, doc);
    const untouchedId = doc.pageOrder[1]!;
    const before = await pageRecord(db, doc.id, untouchedId);

    drawOn(store, 12);
    await saveDocChanges(db, doc, { dirtyPageIds: [doc.pageOrder[0]!] });

    const after = await pageRecord(db, doc.id, untouchedId);
    expect(after.snapshotRev).toBe(before.snapshotRev);
    expect(after).toEqual(before);
  });

  it('loads an original PDF asset with its page reference intact', async () => {
    const db = await openDb(factory);
    const page = createPage();
    page.background = { kind: 'pdf', sourceId: 'source-pdf', pdfPageIndex: 3 };
    const doc = createDoc({ pages: [page] });
    await saveAssets(db, doc.id, [{
      id: 'source-pdf',
      name: 'Samsung note.pdf',
      mimeType: 'application/pdf',
      bytes: new Uint8Array([37, 80, 68, 70]),
      createdAt: 123
    }]);
    await saveDocSnapshot(db, doc);

    const loaded = await loadDoc(db, doc.id);
    expect(loaded!.assets).toHaveLength(1);
    expect([...loaded!.assets[0]!.bytes]).toEqual([37, 80, 68, 70]);
    expect(loaded!.doc.pages.get(page.id)!.background).toEqual({
      kind: 'pdf',
      sourceId: 'source-pdf',
      pdfPageIndex: 3
    });
  });
});

async function pageRecord(db: IDBDatabase, docId: string, pageId: string): Promise<Record<string, unknown>> {
  const tx = db.transaction(STORE_PAGES, 'readonly');
  return await req(tx.objectStore(STORE_PAGES).get([docId, pageId])) as Record<string, unknown>;
}

describe('oplog crash recovery', () => {
  it('replays ops that were never compacted', async () => {
    const db = await openDb(factory);
    const doc = createDoc();
    const store = new DocStore(doc);
    await saveDocSnapshot(db, doc); // baseline snapshot, empty page

    const oplog = new OplogWriter(db, doc.id);
    await oplog.init();
    store.subscribe((applied) => oplog.append(applied.op));

    const strokeId = drawOn(store);
    await oplog.flush();
    // No compaction — simulate a crash here by just re-loading.

    const recovered = await loadDoc(db, doc.id);
    expect(recovered!.replayed).toBe(1);
    expect(recovered!.doc.pages.get(doc.pageOrder[0]!)!.strokes.has(strokeId)).toBe(true);
  });

  it('replaying already-compacted ops is harmless (idempotent)', async () => {
    const db = await openDb(factory);
    const doc = createDoc();
    const store = new DocStore(doc);
    const oplog = new OplogWriter(db, doc.id);
    await oplog.init();
    store.subscribe((applied) => oplog.append(applied.op));

    const strokeId = drawOn(store);
    await oplog.flush();
    // Compact WITHOUT clearing the oplog (throughSeq omitted) — the loader
    // will replay the op on top of a snapshot that already contains it.
    await saveDocSnapshot(db, doc);

    const recovered = await loadDoc(db, doc.id);
    const page = recovered!.doc.pages.get(doc.pageOrder[0]!)!;
    expect(page.strokeOrder.filter((id) => id === strokeId)).toHaveLength(1);
  });

  it('compaction with throughSeq clears the replayed tail', async () => {
    const db = await openDb(factory);
    const doc = createDoc();
    const store = new DocStore(doc);
    const oplog = new OplogWriter(db, doc.id);
    await oplog.init();
    store.subscribe((applied) => oplog.append(applied.op));

    drawOn(store);
    drawOn(store);
    await oplog.flush();
    await saveDocSnapshot(db, doc, { throughSeq: oplog.writtenThrough() });

    const recovered = await loadDoc(db, doc.id);
    expect(recovered!.replayed).toBe(0);
    expect(recovered!.doc.pages.get(doc.pageOrder[0]!)!.strokeOrder).toHaveLength(2);
  });

  it('compacts a crash-recovered journal after the document opens', async () => {
    const db = await openDb(factory);
    const doc = createDoc();
    await saveDocSnapshot(db, doc);
    const sourceStore = new DocStore(doc);
    const oplog = new OplogWriter(db, doc.id);
    await oplog.init();
    sourceStore.subscribe((applied) => oplog.append(applied.op));
    drawOn(sourceStore, 22);
    await oplog.flush();

    const recovered = await loadDoc(db, doc.id);
    expect(recovered!.replayed).toBe(1);
    const controller = new PersistController(db, new DocStore(recovered!.doc));
    await controller.start(recovered!.doc.pageOrder);
    await controller.compact(false);
    await controller.dispose();

    const cleanReload = await loadDoc(db, doc.id);
    expect(cleanReload!.replayed).toBe(0);
    expect(cleanReload!.doc.pages.get(doc.pageOrder[0]!)!.strokeOrder).toHaveLength(1);
  });
});

describe('PersistController', () => {
  it('end-to-end: ops persist, compaction captures a version', async () => {
    const db = await openDb(factory);
    const doc = createDoc({ name: 'ctrl' });
    const store = new DocStore(doc);
    const controller = new PersistController(db, store);
    await controller.start();

    const strokeId = drawOn(store);
    const erased = drawOn(store, 5);
    store.apply(removeStrokesOp(doc.pageOrder[0]!, [erased]));

    const ok = await controller.compact();
    expect(ok).toBe(true);
    expect(controller.versions.entries).toHaveLength(1);

    const loaded = await loadDoc(db, doc.id);
    const page = loaded!.doc.pages.get(doc.pageOrder[0]!)!;
    expect(page.strokes.has(strokeId)).toBe(true);
    expect(page.strokes.has(erased)).toBe(false);
    expect(page.tombstones.has(erased)).toBe(true);
    await controller.dispose();
  });

  it('reloads persisted versions on start', async () => {
    const db = await openDb(factory);
    const doc = createDoc({ name: 'versions' });
    const store = new DocStore(doc);
    const controller = new PersistController(db, store);
    await controller.start();
    drawOn(store);
    await controller.compact();
    await controller.dispose();

    const reloaded = await loadDoc(db, doc.id);
    const store2 = new DocStore(reloaded!.doc);
    const controller2 = new PersistController(db, store2);
    await controller2.start();
    expect(controller2.versions.entries).toHaveLength(1);
    await controller2.dispose();
  });
});
