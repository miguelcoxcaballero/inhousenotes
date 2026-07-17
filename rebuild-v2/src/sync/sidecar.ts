// Google Drive sidecar format. Version 3 references source PDFs stored as
// separate Drive files, so normal note edits upload only the small JSON.

import { newId } from '../core/ids';
import type { Doc } from '../core/model';
import type { JsonDoc } from '../core/serial';
import { fromJsonDoc, toJsonDoc } from '../core/serial';
import { getPdfAsset } from '../pdf/pdfAssets';
import type { BinaryAsset } from '../persist/assets';
import pako from 'pako';

interface SidecarAsset {
  id: string;
  name: string;
  mimeType: string;
  createdAt: number;
  data?: string;
  fileId?: string;
  resourceKey?: string;
}

export interface SidecarAssetReference {
  id: string;
  name: string;
  mimeType: string;
  createdAt: number;
  fileId: string;
  resourceKey?: string;
}

export interface SidecarDocument {
  version: 1 | 2 | 3;
  docId: string;
  rev: number;
  pageOrder: string[];
  pages: JsonDoc['pages'];
  pageTombstones: string[];
  meta: JsonDoc['meta'];
  savedAt: number;
  sidecarVersion: number;
  assets: SidecarAsset[];
  writeId: string;
  parentWriteId: string | null;
}

export interface SidecarBundle {
  doc: JsonDoc;
  assets: BinaryAsset[];
  assetRefs: SidecarAssetReference[];
  writeId: string;
  parentWriteId: string | null;
  savedAt: number;
}

export interface PackOptions {
  assets?: Iterable<BinaryAsset>;
  assetRefs?: Iterable<SidecarAssetReference>;
  inlineAssets?: boolean;
  writeId?: string;
  parentWriteId?: string | null;
}

export const SIDECAR_VERSION = 3;

export function sidecarFilename(docName: string): string {
  const safe = docName.replace(/[<>:"/\\|?*]/g, '_').trim() || 'notebook';
  return `${safe}.ihn.json`;
}

export function serializeSidecar(
  doc: Doc,
  savedAt: number,
  options: PackOptions = {}
): SidecarDocument {
  const json = toJsonDoc(doc);
  return {
    version: 3,
    docId: doc.id,
    rev: doc.rev,
    pageOrder: json.pageOrder,
    pages: json.pages,
    pageTombstones: json.pageTombstones,
    meta: json.meta,
    savedAt,
    sidecarVersion: SIDECAR_VERSION,
    assets: collectAssetEntries(doc, options),
    writeId: options.writeId ?? newId(),
    parentWriteId: options.parentWriteId ?? null
  };
}

export function deserializeSidecar(data: unknown): SidecarDocument | null {
  if (!data || typeof data !== 'object') return null;
  const value = data as Record<string, unknown>;
  if (value.version !== 1 && value.version !== 2 && value.version !== 3) return null;
  if (typeof value.docId !== 'string' || !value.docId) return null;
  if (typeof value.rev !== 'number') return null;
  if (!Array.isArray(value.pageOrder) || !Array.isArray(value.pages)) return null;
  if (typeof value.meta !== 'object' || value.meta === null) return null;
  if (typeof value.savedAt !== 'number') return null;
  if (typeof value.sidecarVersion === 'number' && value.sidecarVersion > SIDECAR_VERSION) return null;
  if (value.pageTombstones !== undefined && !Array.isArray(value.pageTombstones)) return null;

  for (const page of value.pages as unknown[]) {
    if (!page || typeof page !== 'object') return null;
    const record = page as Record<string, unknown>;
    if (typeof record.id !== 'string') return null;
    if (!Array.isArray(record.strokes) || !Array.isArray(record.images)) return null;
  }

  const assets = parseAssets(value.assets);
  if (!assets) return null;
  const meta = value.meta as Partial<JsonDoc['meta']>;
  return {
    version: value.version,
    docId: value.docId,
    rev: value.rev,
    pageOrder: value.pageOrder as string[],
    pages: value.pages as JsonDoc['pages'],
    pageTombstones: (value.pageTombstones as string[] | undefined) ?? [],
    meta: {
      name: typeof meta.name === 'string' ? meta.name : 'cuaderno',
      calendarPageConfig: meta.calendarPageConfig ?? null
    },
    savedAt: value.savedAt,
    sidecarVersion: typeof value.sidecarVersion === 'number' ? value.sidecarVersion : 1,
    assets,
    writeId: typeof value.writeId === 'string' && value.writeId
      ? value.writeId
      : `legacy-${value.docId}-${value.savedAt}`,
    parentWriteId: typeof value.parentWriteId === 'string' ? value.parentWriteId : null
  };
}

export function compress(data: unknown): string {
  return base64Encode(pako.deflate(JSON.stringify(data)));
}

export function decompress(value: string): unknown {
  try {
    return JSON.parse(new TextDecoder().decode(pako.inflate(base64Decode(value))));
  } catch {
    return null;
  }
}

export function packDoc(doc: Doc, savedAt: number, options: PackOptions = {}): string {
  const sidecar = serializeSidecar(doc, savedAt, options);
  const json = JSON.stringify(sidecar);
  const deflated = `IHN3:${compress(sidecar)}`;
  return deflated.length < json.length ? deflated : json;
}

export function unpackDoc(value: string): JsonDoc | null {
  return unpackBundle(value)?.doc ?? null;
}

export function unpackBundle(value: string): SidecarBundle | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(value);
  } catch {
    parsed = decompress(value.startsWith('IHN3:') ? value.slice(5) : value);
  }
  const sidecar = deserializeSidecar(parsed);
  if (!sidecar) return null;
  try {
    return {
      doc: {
        id: sidecar.docId,
        rev: sidecar.rev,
        pageOrder: sidecar.pageOrder,
        pages: sidecar.pages,
        meta: sidecar.meta,
        pageTombstones: sidecar.pageTombstones
      },
      assets: sidecar.assets.filter(hasInlineData).map(assetFromSidecar),
      assetRefs: sidecar.assets.filter(hasFileId).map(assetReferenceFromSidecar),
      writeId: sidecar.writeId,
      parentWriteId: sidecar.parentWriteId,
      savedAt: sidecar.savedAt
    };
  } catch {
    return null;
  }
}

export function loadDoc(value: string): Doc | null {
  return loadBundle(value)?.doc ?? null;
}

export function loadBundle(value: string): {
  doc: Doc;
  assets: BinaryAsset[];
  assetRefs: SidecarAssetReference[];
  writeId: string;
  parentWriteId: string | null;
  savedAt: number;
} | null {
  const bundle = unpackBundle(value);
  if (!bundle) return null;
  try {
    return {
      doc: fromJsonDoc(bundle.doc),
      assets: bundle.assets,
      assetRefs: bundle.assetRefs,
      writeId: bundle.writeId,
      parentWriteId: bundle.parentWriteId,
      savedAt: bundle.savedAt
    };
  } catch {
    return null;
  }
}

function collectAssetEntries(doc: Doc, options: PackOptions): SidecarAsset[] {
  const supplied = new Map<string, BinaryAsset>();
  if (options.assets) {
    for (const asset of options.assets) supplied.set(asset.id, asset);
  }
  const references = new Map<string, SidecarAssetReference>();
  if (options.assetRefs) {
    for (const reference of options.assetRefs) references.set(reference.id, reference);
  }
  const result: SidecarAsset[] = [];
  const seen = new Set<string>();
  for (const pageId of doc.pageOrder) {
    const background = doc.pages.get(pageId)?.background;
    if (background?.kind !== 'pdf' || !background.sourceId || seen.has(background.sourceId)) continue;
    seen.add(background.sourceId);
    const reference = references.get(background.sourceId);
    if (reference) {
      result.push({ ...reference });
      continue;
    }
    if (options.inlineAssets === false) continue;
    const asset = supplied.get(background.sourceId) ?? getPdfAsset(background.sourceId);
    if (asset) result.push(assetToSidecar(asset));
  }
  return result;
}

function assetToSidecar(asset: BinaryAsset): SidecarAsset {
  return {
    id: asset.id,
    name: asset.name,
    mimeType: asset.mimeType,
    createdAt: asset.createdAt,
    data: base64Encode(asset.bytes)
  };
}

function assetFromSidecar(asset: SidecarAsset): BinaryAsset {
  return {
    id: asset.id,
    name: asset.name,
    mimeType: asset.mimeType,
    createdAt: asset.createdAt,
    bytes: base64Decode(asset.data!)
  };
}

function assetReferenceFromSidecar(asset: SidecarAsset): SidecarAssetReference {
  return {
    id: asset.id,
    name: asset.name,
    mimeType: asset.mimeType,
    createdAt: asset.createdAt,
    fileId: asset.fileId!,
    ...(asset.resourceKey ? { resourceKey: asset.resourceKey } : {})
  };
}

function hasInlineData(asset: SidecarAsset): asset is SidecarAsset & { data: string } {
  return typeof asset.data === 'string';
}

function hasFileId(asset: SidecarAsset): asset is SidecarAsset & { fileId: string } {
  return typeof asset.fileId === 'string' && asset.fileId.length > 0;
}

function parseAssets(value: unknown): SidecarAsset[] | null {
  if (value === undefined) return [];
  if (!Array.isArray(value)) return null;
  const result: SidecarAsset[] = [];
  for (const item of value) {
    if (!item || typeof item !== 'object') return null;
    const asset = item as Record<string, unknown>;
    if (
      typeof asset.id !== 'string' || !asset.id ||
      typeof asset.name !== 'string' ||
      typeof asset.mimeType !== 'string' ||
      typeof asset.createdAt !== 'number' ||
      (typeof asset.data !== 'string' && (typeof asset.fileId !== 'string' || !asset.fileId)) ||
      (asset.resourceKey !== undefined && typeof asset.resourceKey !== 'string')
    ) return null;
    result.push(asset as unknown as SidecarAsset);
  }
  return result;
}

function base64Encode(bytes: Uint8Array): string {
  const chunks: string[] = [];
  const chunkSize = 0x8000;
  for (let offset = 0; offset < bytes.byteLength; offset += chunkSize) {
    chunks.push(String.fromCharCode(...bytes.subarray(offset, offset + chunkSize)));
  }
  return btoa(chunks.join('')).replace(/\+/g, '-').replace(/\//g, '_').replace(/=/g, '');
}

function base64Decode(value: string): Uint8Array {
  const standard = value.replace(/-/g, '+').replace(/_/g, '/');
  const padded = standard + '==='.slice(0, (4 - (standard.length % 4)) % 4);
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}
