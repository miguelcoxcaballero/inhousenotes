// One-shot migration of legacy (v1) data into the v2 model.
//
// Legacy sources, in priority order (newest savedAt wins, as before):
//  1. localStorage 'notebook-data-v3' / 'notebook-data-v2' — meta payload,
//     possibly with inline strokes (very old saves).
//  2. IndexedDB 'notebook-data-db-v1':
//       snapshots['latest'] → meta payload (page list without strokes)
//       pages[index]        → full page records (strokes/images)
//       stroke-ops          → crash-recovery queue, appended on top.
//
// The reader stays in the codebase permanently: old Drive PDFs carry the
// same page shape inside their STROKES_Z keywords payload (Phase: sync).

import { newId } from '../core/ids';
import type { Background, Doc, Img, Tool } from '../core/model';
import { createDoc, createPage, makeStroke, packPoints } from '../core/model';
import type { Page } from '../core/model';

export const LEGACY_LOCALSTORAGE_KEYS = ['notebook-data-v3', 'notebook-data-v2'];
export const LEGACY_DB_NAME = 'notebook-data-db-v1';
export const LEGACY_SNAPSHOT_STORE = 'snapshots';
export const LEGACY_PAGE_STORE = 'pages';
export const LEGACY_OPS_STORE = 'stroke-ops';

interface LegacyPoint {
  x: number;
  y: number;
  p?: number;
}

export interface LegacyStroke {
  id?: string;
  tool?: string;
  color?: string;
  width?: number;
  points?: LegacyPoint[];
}

export interface LegacyImage {
  id?: string;
  src?: string;
  x?: number;
  y?: number;
  width?: number;
  height?: number;
  rotation?: number;
}

export interface LegacyPage {
  pageId?: string;
  strokes?: LegacyStroke[] | null;
  images?: LegacyImage[] | null;
  deletedStrokeIds?: string[];
  backgroundImage?: string | null;
  backgroundSource?: string;
  templateKind?: string;
  pdfPageIndex?: number | null;
  pageWidth?: number;
  pageHeight?: number;
  sidePanel?: { mode?: string; dateKeys?: string[] } | null;
}

export interface LegacyPayload {
  pages?: LegacyPage[];
  savedAt?: number;
  calendarPageConfig?: {
    mode?: string;
    startDateKey?: string | null;
    startPage?: number;
    nextDateKey?: string | null;
  } | null;
}

export async function legacyPayloadFromPdfKeywords(keywords: string): Promise<LegacyPayload | null> {
  const text = String(keywords || '');
  const strokesZ = extractKeywordValue(text, 'STROKES_Z:');
  if (strokesZ) {
    try {
      return JSON.parse(await inflateBase64(strokesZ)) as LegacyPayload;
    } catch {
      return null;
    }
  }

  const strokesData = extractKeywordValue(text, 'STROKES_DATA:');
  if (strokesData) {
    try {
      return JSON.parse(decodeURIComponent(escape(atob(strokesData)))) as LegacyPayload;
    } catch {
      return null;
    }
  }
  return null;
}

function extractKeywordValue(text: string, prefix: string): string | null {
  const start = text.indexOf(prefix);
  if (start < 0) return null;
  const valueStart = start + prefix.length;
  const semi = text.indexOf(';', valueStart);
  return text.slice(valueStart, semi >= 0 ? semi : undefined);
}

async function inflateBase64(encoded: string): Promise<string> {
  if (typeof DecompressionStream === 'undefined') {
    throw new Error('DecompressionStream unavailable');
  }
  const binary = atob(encoded);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  const stream = new Blob([bytes]).stream().pipeThrough(new DecompressionStream('deflate'));
  return new Response(stream).text();
}

function legacyTool(tool: string | undefined): Tool | null {
  if (tool === 'highlighter') return 'highlighter';
  if (!tool || tool === 'pen') return 'pen';
  // Legacy eraser pseudo-strokes were already applied destructively by the
  // old app (bakeLegacyEraserAreaStrokes); they carry no drawable content.
  if (tool.startsWith('eraser')) return null;
  return 'pen';
}

function legacyBackground(page: LegacyPage): Background {
  const source =
    page.backgroundSource ?? (Number.isFinite(page.pdfPageIndex) ? 'pdf' : page.backgroundImage ? 'custom' : 'template');
  if (source === 'pdf' && Number.isFinite(page.pdfPageIndex)) {
    return { kind: 'pdf', pdfPageIndex: Number(page.pdfPageIndex) };
  }
  if (source === 'custom' && page.backgroundImage) {
    return { kind: 'custom', src: page.backgroundImage };
  }
  const template = page.templateKind === 'agenda' || page.templateKind === 'diary' ? page.templateKind : 'default';
  return { kind: 'template', template };
}

export function legacyPageToPage(legacy: LegacyPage): Page {
  const page = createPage({
    id: legacy.pageId ?? newId(),
    width: typeof legacy.pageWidth === 'number' && legacy.pageWidth > 0 ? legacy.pageWidth : undefined,
    height: typeof legacy.pageHeight === 'number' && legacy.pageHeight > 0 ? legacy.pageHeight : undefined,
    background: legacyBackground(legacy),
    sidePanel:
      legacy.sidePanel && (legacy.sidePanel.mode === 'day' || legacy.sidePanel.mode === 'week')
        ? { mode: legacy.sidePanel.mode, dateKeys: legacy.sidePanel.dateKeys ?? [] }
        : null,
    tombstones: new Set(legacy.deletedStrokeIds ?? [])
  });

  for (const raw of legacy.strokes ?? []) {
    const tool = legacyTool(raw.tool);
    if (!tool) continue;
    const points = raw.points ?? [];
    if (points.length === 0) continue;
    const stroke = makeStroke({
      id: raw.id ?? newId(),
      tool,
      color: raw.color ?? '#002FD9',
      width: typeof raw.width === 'number' && raw.width > 0 ? raw.width : 2,
      points: packPoints(points)
    });
    page.strokes.set(stroke.id, stroke);
    page.strokeOrder.push(stroke.id);
  }

  for (const raw of legacy.images ?? []) {
    if (!raw.src) continue;
    const img: Img = {
      id: raw.id ?? newId(),
      src: raw.src,
      x: raw.x ?? 0,
      y: raw.y ?? 0,
      width: raw.width ?? 100,
      height: raw.height ?? 100,
      rotation: raw.rotation ?? 0
    };
    page.images.set(img.id, img);
    page.imageOrder.push(img.id);
  }

  return page;
}

export function legacyPayloadToDoc(payload: LegacyPayload, name = 'cuaderno'): Doc {
  const pages = (payload.pages ?? []).map(legacyPageToPage);
  const doc = createDoc({ name, pages: pages.length > 0 ? pages : undefined });
  const cfg = payload.calendarPageConfig;
  if (cfg && (cfg.mode === 'day' || cfg.mode === 'week')) {
    doc.meta.calendarPageConfig = {
      mode: cfg.mode,
      startDateKey: cfg.startDateKey ?? null,
      startPage: typeof cfg.startPage === 'number' ? cfg.startPage : 0,
      nextDateKey: cfg.nextDateKey ?? null
    };
  }
  return doc;
}

export function readLegacyLocalStorage(storage: Pick<Storage, 'getItem'>): LegacyPayload | null {
  for (const key of LEGACY_LOCALSTORAGE_KEYS) {
    try {
      const raw = storage.getItem(key);
      if (!raw) continue;
      const parsed = JSON.parse(raw) as LegacyPayload;
      if (parsed && Array.isArray(parsed.pages)) return parsed;
    } catch {
      // Corrupt legacy entry — try the next key.
    }
  }
  return null;
}

interface LegacyOpRecord {
  pageIndex?: number;
  type?: string;
  stroke?: LegacyStroke;
}

function reqP<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

/**
 * Read the entire legacy IndexedDB into a LegacyPayload. Returns null when
 * the database does not exist or holds no pages.
 */
export async function readLegacyIdb(factory: IDBFactory = indexedDB): Promise<LegacyPayload | null> {
  let db: IDBDatabase;
  try {
    db = await new Promise<IDBDatabase>((resolve, reject) => {
      const request = factory.open(LEGACY_DB_NAME);
      let existed = true;
      request.onupgradeneeded = () => {
        existed = false;
      };
      request.onsuccess = () => {
        if (!existed) {
          // We just created an empty DB by opening it — clean up and bail.
          request.result.close();
          factory.deleteDatabase(LEGACY_DB_NAME);
          reject(new Error('no legacy db'));
          return;
        }
        resolve(request.result);
      };
      request.onerror = () => reject(request.error);
    });
  } catch {
    return null;
  }

  try {
    const names = db.objectStoreNames;
    const meta: LegacyPayload | null = names.contains(LEGACY_SNAPSHOT_STORE)
      ? await (async () => {
          const tx = db.transaction(LEGACY_SNAPSHOT_STORE, 'readonly');
          const record = (await reqP(tx.objectStore(LEGACY_SNAPSHOT_STORE).get('latest'))) as
            | { data?: LegacyPayload | string; savedAt?: number }
            | undefined;
          if (!record?.data) return null;
          const data = typeof record.data === 'string' ? (JSON.parse(record.data) as LegacyPayload) : record.data;
          return { ...data, savedAt: record.savedAt ?? data.savedAt };
        })()
      : null;

    const fullPages = new Map<number, LegacyPage>();
    if (names.contains(LEGACY_PAGE_STORE)) {
      const tx = db.transaction(LEGACY_PAGE_STORE, 'readonly');
      const store = tx.objectStore(LEGACY_PAGE_STORE);
      const keys = (await reqP(store.getAllKeys())) as number[];
      const values = (await reqP(store.getAll())) as LegacyPage[];
      keys.forEach((key, i) => {
        const value = values[i];
        if (typeof key === 'number' && value) fullPages.set(key, value);
      });
    }

    let pendingOps: LegacyOpRecord[] = [];
    if (names.contains(LEGACY_OPS_STORE)) {
      const tx = db.transaction(LEGACY_OPS_STORE, 'readonly');
      pendingOps = (await reqP(tx.objectStore(LEGACY_OPS_STORE).getAll())) as LegacyOpRecord[];
    }

    const pageCount = Math.max(
      meta?.pages?.length ?? 0,
      fullPages.size > 0 ? Math.max(...fullPages.keys()) + 1 : 0
    );
    if (pageCount === 0) return null;

    const pages: LegacyPage[] = [];
    for (let i = 0; i < pageCount; i++) {
      const metaPage = meta?.pages?.[i] ?? {};
      const fullPage = fullPages.get(i);
      // Meta payload pages have strokes:null (they live in the page store);
      // very old payloads carry strokes inline. Full page records win.
      pages.push({ ...metaPage, ...(fullPage ?? {}) });
    }

    for (const op of pendingOps) {
      if (op?.type !== 'add-stroke' || !op.stroke || typeof op.pageIndex !== 'number') continue;
      const page = pages[op.pageIndex];
      if (!page) continue;
      page.strokes = [...(page.strokes ?? []), op.stroke];
    }

    return { pages, savedAt: meta?.savedAt, calendarPageConfig: meta?.calendarPageConfig ?? null };
  } finally {
    db.close();
  }
}

export interface MigrationResult {
  doc: Doc;
  source: 'idb' | 'localStorage';
}

/** Pick the freshest legacy source and convert it. Null → nothing to migrate. */
export async function migrateLegacyData(
  storage: Pick<Storage, 'getItem'>,
  factory: IDBFactory = indexedDB
): Promise<MigrationResult | null> {
  const fromLocal = readLegacyLocalStorage(storage);
  const fromIdb = await readLegacyIdb(factory);
  if (!fromLocal && !fromIdb) return null;

  const localTime = fromLocal?.savedAt ?? 0;
  const idbTime = fromIdb?.savedAt ?? 0;
  // The IDB copy carries full stroke data; prefer it unless localStorage is
  // strictly newer AND actually contains strokes.
  const localHasStrokes = !!fromLocal?.pages?.some((p) => Array.isArray(p.strokes) && p.strokes.length > 0);
  const useLocal = !!fromLocal && (!fromIdb || (localTime > idbTime && localHasStrokes));

  const payload = useLocal ? fromLocal! : fromIdb!;
  return {
    doc: legacyPayloadToDoc(payload),
    source: useLocal ? 'localStorage' : 'idb'
  };
}
