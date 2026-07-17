// JSON document format stored in Google Drive.
// Documents are serialized, compressed (pako deflate + base64), and stored
// alongside the PDF in the app's Drive folder.

import type { Doc } from '../core/model';
import type { JsonDoc } from '../core/serial';
import { toJsonDoc, fromJsonDoc } from '../core/serial';

interface PakoRuntime {
  deflate(input: string): Uint8Array;
  inflate(input: Uint8Array): Uint8Array;
}

declare const pako: PakoRuntime | undefined;

// ── Sidecar document interface ─────────────────────────────────────────────────

export interface SidecarDocument {
  version: 1;
  docId: string;
  rev: number;
  pageOrder: string[];
  pages: JsonDoc['pages'];
  meta: JsonDoc['meta'];
  savedAt: number;
  sidecarVersion: number;
}

/** Current sidecar schema version — bump only for breaking changes. */
export const SIDECAR_VERSION = 1;

/** Filename for a sidecar document (no extension conflict with PDF). */
export function sidecarFilename(docName: string): string {
  // Strip problematic characters and ensure .ihn.json suffix
  const safe = docName.replace(/[<>:"/\\|?*]/g, '_').trim() || 'notebook';
  return `${safe}.ihn.json`;
}

// ── Serialization ─────────────────────────────────────────────────────────────

/**
 * Serialize a document to a sidecar structure, ready for compression.
 */
export function serializeSidecar(doc: Doc, savedAt: number): SidecarDocument {
  const j = toJsonDoc(doc);
  return {
    version: 1,
    docId: doc.id,
    rev: doc.rev,
    pageOrder: j.pageOrder,
    pages: j.pages,
    meta: j.meta,
    savedAt,
    sidecarVersion: SIDECAR_VERSION
  };
}

/**
 * Deserialize and validate a sidecar document.
 * Returns null if the data is invalid or from a future incompatible version.
 */
export function deserializeSidecar(data: unknown): SidecarDocument | null {
  if (!data || typeof data !== 'object') return null;

  const d = data as Record<string, unknown>;

  // Version check — only version 1 is supported
  if (d.version !== 1) return null;

  // Required fields
  if (typeof d.docId !== 'string' || !d.docId) return null;
  if (typeof d.rev !== 'number') return null;
  if (!Array.isArray(d.pageOrder)) return null;
  if (!Array.isArray(d.pages)) return null;
  if (typeof d.meta !== 'object' || d.meta === null) return null;
  if (typeof d.savedAt !== 'number') return null;

  // Validate pages structure
  for (const page of d.pages as unknown[]) {
    if (!page || typeof page !== 'object') return null;
    const p = page as Record<string, unknown>;
    if (typeof p.id !== 'string') return null;
    if (!Array.isArray(p.strokes)) return null;
    if (!Array.isArray(p.images)) return null;
  }

  const meta = d.meta as Partial<JsonDoc['meta']>;

  return {
    version: 1,
    docId: d.docId,
    rev: d.rev,
    pageOrder: d.pageOrder as string[],
    pages: d.pages as JsonDoc['pages'],
    meta: {
      name: typeof meta.name === 'string' ? meta.name : 'cuaderno',
      calendarPageConfig: meta.calendarPageConfig ?? null
    },
    savedAt: d.savedAt,
    sidecarVersion: (d.sidecarVersion as number | undefined) ?? 1
  };
}

// ── Compression (pako) ────────────────────────────────────────────────────────

/**
 * Compress JSON data: stringify → pako deflate → base64url.
 * Throws if pako is unavailable.
 */
export function compress(data: unknown): string {
  if (typeof pako === 'undefined') {
    throw new Error('pako not loaded — compression unavailable');
  }
  const json = JSON.stringify(data);
  const deflated = pako.deflate(json);
  return base64Encode(deflated);
}

/**
 * Decompress a compressed string: base64 → pako inflate → parse JSON.
 * Returns the parsed object or null if decompression/parsing fails.
 */
export function decompress(str: string): unknown {
  try {
    if (typeof pako === 'undefined') {
      throw new Error('pako not loaded — decompression unavailable');
    }
    const bytes = base64Decode(str);
    const inflated = pako.inflate(bytes);
    return JSON.parse(new TextDecoder().decode(inflated));
  } catch {
    return null;
  }
}

// ── Base64 helpers ────────────────────────────────────────────────────────────

function base64Encode(bytes: Uint8Array): string {
  let binary = '';
  for (let i = 0; i < bytes.byteLength; i++) {
    binary += String.fromCharCode(bytes[i]!);
  }
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=/g, '');
}

function base64Decode(str: string): Uint8Array {
  // Restore standard base64 padding and characters
  const std = str.replace(/-/g, '+').replace(/_/g, '/');
  const padded = std + '==='.slice(0, (4 - (std.length % 4)) % 4);
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

// ── Round-trip helpers ─────────────────────────────────────────────────────────

/**
 * Pack a doc into a compressed, Drive-ready string.
 */
export function packDoc(doc: Doc, savedAt: number): string {
  return JSON.stringify(serializeSidecar(doc, savedAt));
}

/**
 * Unpack a compressed Drive string into a JsonDoc.
 * Returns null on any parse error.
 */
export function unpackDoc(compressed: string): JsonDoc | null {
  let data: unknown = null;
  try {
    data = JSON.parse(compressed);
  } catch {
    data = decompress(compressed);
  }
  const sidecar = deserializeSidecar(data);
  if (!sidecar) return null;

  return {
    id: sidecar.docId,
    rev: sidecar.rev,
    pageOrder: sidecar.pageOrder,
    pages: sidecar.pages,
    meta: sidecar.meta,
    pageTombstones: []
  };
}

/**
 * Reconstruct a Doc from a compressed Drive string.
 * Returns null if unpacking or deserialization fails.
 */
export function loadDoc(compressed: string): Doc | null {
  const jdoc = unpackDoc(compressed);
  if (!jdoc) return null;
  try {
    return fromJsonDoc(jdoc);
  } catch {
    return null;
  }
}
