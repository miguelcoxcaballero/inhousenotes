import { beforeEach, describe, expect, it } from 'vitest';
import { IDBFactory, IDBKeyRange as FakeKeyRange } from 'fake-indexeddb';
import {
  LEGACY_DB_NAME,
  LEGACY_OPS_STORE,
  LEGACY_PAGE_STORE,
  LEGACY_SNAPSHOT_STORE,
  legacyPayloadFromPdfKeywords,
  legacyPayloadToDoc,
  migrateLegacyData,
  readLegacyIdb
} from './migrate';
import type { LegacyPayload } from './migrate';

(globalThis as Record<string, unknown>).IDBKeyRange = FakeKeyRange;

let factory: IDBFactory;

beforeEach(() => {
  factory = new IDBFactory();
});

const LEGACY_PAYLOAD: LegacyPayload = {
  savedAt: 1700000000000,
  calendarPageConfig: { mode: 'day', startDateKey: '2026-01-05', startPage: 2, nextDateKey: null },
  pages: [
    {
      pageId: 'page-a',
      pageWidth: 794,
      pageHeight: 1123,
      templateKind: 'agenda',
      deletedStrokeIds: ['gone-1'],
      sidePanel: { mode: 'day', dateKeys: ['2026-01-05'] },
      strokes: [
        { id: 's1', tool: 'pen', color: '#002FD9', width: 2, points: [{ x: 1, y: 2, p: 0.7 }, { x: 3, y: 4 }] },
        { id: 's2', tool: 'highlighter', color: '#ff0', width: 12, points: [{ x: 0, y: 0 }, { x: 9, y: 9 }] },
        { id: 's3', tool: 'eraser-area', width: 12, points: [{ x: 5, y: 5 }] }
      ],
      images: [{ id: 'img1', src: 'data:image/png;base64,xyz', x: 10, y: 20, width: 50, height: 40, rotation: 0 }]
    },
    {
      pdfPageIndex: 3,
      pageWidth: 612,
      pageHeight: 792,
      strokes: []
    }
  ]
};

function mockStorage(entries: Record<string, string>): Pick<Storage, 'getItem'> {
  return { getItem: (key: string) => entries[key] ?? null };
}

async function seedLegacyIdb(meta: LegacyPayload | null, pagesByIndex: Record<number, unknown>, ops: unknown[] = []) {
  const db = await new Promise<IDBDatabase>((resolve, reject) => {
    const request = factory.open(LEGACY_DB_NAME, 3);
    request.onupgradeneeded = () => {
      const d = request.result;
      d.createObjectStore(LEGACY_SNAPSHOT_STORE, { keyPath: 'key' });
      d.createObjectStore(LEGACY_PAGE_STORE);
      d.createObjectStore(LEGACY_OPS_STORE, { keyPath: 'id', autoIncrement: true });
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const tx = db.transaction([LEGACY_SNAPSHOT_STORE, LEGACY_PAGE_STORE, LEGACY_OPS_STORE], 'readwrite');
  if (meta) tx.objectStore(LEGACY_SNAPSHOT_STORE).put({ key: 'latest', data: meta, savedAt: meta.savedAt });
  for (const [index, page] of Object.entries(pagesByIndex)) {
    tx.objectStore(LEGACY_PAGE_STORE).put(page, Number(index));
  }
  for (const op of ops) tx.objectStore(LEGACY_OPS_STORE).put(op);
  await new Promise<void>((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
  db.close();
}

describe('legacyPayloadToDoc', () => {
  it('converts pages, strokes, images, tombstones and calendar config', () => {
    const doc = legacyPayloadToDoc(LEGACY_PAYLOAD);
    expect(doc.pageOrder).toHaveLength(2);

    const pageA = doc.pages.get('page-a')!;
    expect(pageA.background).toEqual({ kind: 'template', template: 'agenda' });
    // Eraser pseudo-stroke dropped, pen + highlighter kept.
    expect(pageA.strokeOrder).toEqual(['s1', 's2']);
    expect([...pageA.strokes.get('s1')!.points]).toEqual([1, 2, Math.fround(0.7), 3, 4, 0.5]);
    expect(pageA.strokes.get('s2')!.tool).toBe('highlighter');
    expect(pageA.tombstones.has('gone-1')).toBe(true);
    expect(pageA.images.get('img1')!.width).toBe(50);
    expect(pageA.sidePanel).toEqual({ mode: 'day', dateKeys: ['2026-01-05'] });

    const pageB = doc.pages.get(doc.pageOrder[1]!)!;
    expect(pageB.background).toEqual({ kind: 'pdf', pdfPageIndex: 3 });
    expect(pageB.width).toBe(612);

    expect(doc.meta.calendarPageConfig).toEqual({
      mode: 'day',
      startDateKey: '2026-01-05',
      startPage: 2,
      nextDateKey: null
    });
  });

  it('keeps PDF page index 0 as a PDF background', () => {
    const doc = legacyPayloadToDoc({ pages: [{ pdfPageIndex: 0, pageWidth: 612, pageHeight: 792 }] });
    const page = doc.pages.get(doc.pageOrder[0]!)!;
    expect(page.background).toEqual({ kind: 'pdf', pdfPageIndex: 0 });
  });
});

describe('legacyPayloadFromPdfKeywords', () => {
  it('decodes STROKES_DATA metadata', async () => {
    const encoded = btoa(unescape(encodeURIComponent(JSON.stringify(LEGACY_PAYLOAD))));
    const payload = await legacyPayloadFromPdfKeywords(`STROKES_DATA:${encoded};IH_CAL:ignored`);
    expect(payload!.pages).toHaveLength(2);
    expect(payload!.pages![0]!.strokes![0]!.id).toBe('s1');
  });

  it('decodes STROKES_Z metadata when deflate streams are available', async () => {
    if (typeof CompressionStream === 'undefined' || typeof DecompressionStream === 'undefined') return;
    const stream = new Blob([JSON.stringify(LEGACY_PAYLOAD)]).stream().pipeThrough(new CompressionStream('deflate'));
    const bytes = new Uint8Array(await new Response(stream).arrayBuffer());
    let binary = '';
    for (const byte of bytes) binary += String.fromCharCode(byte);
    const payload = await legacyPayloadFromPdfKeywords(`STROKES_Z:${btoa(binary)};IH_OVERLAY_REFS:p0=1`);
    expect(payload!.pages).toHaveLength(2);
    expect(payload!.pages![0]!.images![0]!.id).toBe('img1');
  });
});

describe('readLegacyIdb', () => {
  it('returns null when there is no legacy database', async () => {
    expect(await readLegacyIdb(factory)).toBeNull();
  });

  it('joins meta payload with full page records and pending stroke-ops', async () => {
    const meta: LegacyPayload = {
      savedAt: 1700000001000,
      pages: [
        { pageId: 'page-a', strokes: null, templateKind: 'agenda', pageWidth: 794, pageHeight: 1123 },
        { pageId: 'page-b', strokes: null, pageWidth: 794, pageHeight: 1123 }
      ]
    };
    await seedLegacyIdb(
      meta,
      {
        0: { pageId: 'page-a', strokes: [{ id: 's1', tool: 'pen', points: [{ x: 0, y: 0 }, { x: 1, y: 1 }] }] },
        1: { pageId: 'page-b', strokes: [] }
      },
      [
        {
          pageIndex: 1,
          type: 'add-stroke',
          stroke: { id: 'crash-stroke', tool: 'pen', points: [{ x: 7, y: 7 }, { x: 8, y: 8 }] }
        }
      ]
    );

    const payload = await readLegacyIdb(factory);
    expect(payload).not.toBeNull();
    expect(payload!.pages).toHaveLength(2);
    expect(payload!.pages![0]!.strokes!.map((s) => s.id)).toEqual(['s1']);
    // The crash-recovery op landed on page-b.
    expect(payload!.pages![1]!.strokes!.map((s) => s.id)).toEqual(['crash-stroke']);
    // Meta fields survive even though the full record lacked them.
    expect(payload!.pages![0]!.templateKind).toBe('agenda');
  });
});

describe('migrateLegacyData', () => {
  it('prefers the IDB copy (full strokes) over older localStorage', async () => {
    await seedLegacyIdb(LEGACY_PAYLOAD, {
      0: LEGACY_PAYLOAD.pages![0],
      1: LEGACY_PAYLOAD.pages![1]
    });
    const storage = mockStorage({
      'notebook-data-v3': JSON.stringify({ savedAt: 1, pages: [{ strokes: null }] })
    });
    const result = await migrateLegacyData(storage, factory);
    expect(result!.source).toBe('idb');
    expect(result!.doc.pages.get('page-a')!.strokeOrder).toContain('s1');
  });

  it('uses localStorage when it is newer and carries strokes', async () => {
    const storage = mockStorage({ 'notebook-data-v3': JSON.stringify(LEGACY_PAYLOAD) });
    const result = await migrateLegacyData(storage, factory);
    expect(result!.source).toBe('localStorage');
    expect(result!.doc.pages.get('page-a')!.strokeOrder).toEqual(['s1', 's2']);
  });

  it('returns null when there is nothing to migrate', async () => {
    expect(await migrateLegacyData(mockStorage({}), factory)).toBeNull();
  });
});
