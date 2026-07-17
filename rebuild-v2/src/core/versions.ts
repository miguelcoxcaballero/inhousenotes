// Version timeline. Entries are full JSON checkpoints of the document,
// captured on oplog compaction (works offline — no dependency on a Drive
// upload succeeding, unlike the legacy app). Capped by total byte size,
// evicting the oldest non-milestone entries first.
//
// Restoring produces a single 'replace-doc' op whose pages carry tombstones
// for every element being discarded, so the restore propagates correctly
// through the collaborative merge (port of legacy addRestoreTombstones).

import { newId } from './ids';
import type { Doc } from './model';
import type { JsonDoc } from './serial';
import { toJsonDoc } from './serial';
import type { Op } from './ops';

export interface VersionAuthor {
  name: string;
  email: string;
  photo: string;
}

export type VersionKind = 'autosave' | 'restore' | 'revert-original';

export interface VersionEntry {
  id: string;
  ts: number;
  author: VersionAuthor;
  summary: string;
  kind: VersionKind;
  isMilestone: boolean;
  snapshot: JsonDoc;
  /** Approximate serialized size, used for the size cap. */
  bytes: number;
}

export interface CaptureHint {
  milestone?: boolean;
  kind?: VersionKind;
  summary?: string;
}

const COALESCE_MS = 5 * 60 * 1000;
const MAX_TOTAL_BYTES = 24 * 1024 * 1024; // raw JSON; compressed at rest by persist
const MAX_ENTRIES = 100;

function estimateBytes(snapshot: JsonDoc): number {
  let points = 0;
  let images = 0;
  for (const page of snapshot.pages) {
    for (const stroke of page.strokes) points += stroke.points.length;
    for (const img of page.images) images += img.src.length;
  }
  return points * 8 + images + 1024 * snapshot.pages.length;
}

function countElements(snapshot: JsonDoc): { strokes: number; images: number } {
  let strokes = 0;
  let images = 0;
  for (const page of snapshot.pages) {
    strokes += page.strokes.length;
    images += page.images.length;
  }
  return { strokes, images };
}

export function summarizeDelta(prev: JsonDoc | null, next: JsonDoc): string {
  if (!prev) return 'First version';
  const a = countElements(prev);
  const b = countElements(next);
  const parts: string[] = [];
  const strokeDelta = b.strokes - a.strokes;
  const imageDelta = b.images - a.images;
  const pageDelta = next.pages.length - prev.pages.length;
  if (strokeDelta > 0) parts.push(`+${strokeDelta} stroke${strokeDelta !== 1 ? 's' : ''}`);
  if (strokeDelta < 0) parts.push(`${strokeDelta} strokes`);
  if (imageDelta !== 0) parts.push(`${imageDelta > 0 ? '+' : ''}${imageDelta} image${Math.abs(imageDelta) !== 1 ? 's' : ''}`);
  if (pageDelta !== 0) parts.push(`${pageDelta > 0 ? '+' : ''}${pageDelta} page${Math.abs(pageDelta) !== 1 ? 's' : ''}`);
  return parts.length > 0 ? parts.join(', ') : 'Edited';
}

export class VersionLog {
  entries: VersionEntry[] = [];

  /** Capture the current doc state. Returns the entry (new or coalesced). */
  capture(doc: Doc, author: VersionAuthor, hint: CaptureHint | null = null): VersionEntry {
    const snapshot = toJsonDoc(doc);
    const ts = Date.now();
    const isMilestone = !!hint?.milestone;
    const last = this.entries[this.entries.length - 1];

    const canCoalesce =
      !isMilestone &&
      !!last &&
      !last.isMilestone &&
      last.author.email === author.email &&
      ts - last.ts < COALESCE_MS;

    const prevSnapshot = canCoalesce
      ? (this.entries[this.entries.length - 2]?.snapshot ?? null)
      : (last?.snapshot ?? null);

    const entry: VersionEntry = {
      id: canCoalesce ? last.id : newId(),
      ts,
      author,
      summary: hint?.summary ?? summarizeDelta(prevSnapshot, snapshot),
      kind: hint?.kind ?? 'autosave',
      isMilestone,
      snapshot,
      bytes: estimateBytes(snapshot)
    };

    if (canCoalesce) {
      this.entries[this.entries.length - 1] = entry;
    } else {
      this.entries.push(entry);
    }
    this.evict();
    return entry;
  }

  find(versionId: string): VersionEntry | null {
    return this.entries.find((e) => e.id === versionId) ?? null;
  }

  current(): VersionEntry | null {
    return this.entries[this.entries.length - 1] ?? null;
  }

  /**
   * Build the op that rolls the live doc back to `entry`. The restored pages
   * gain tombstones for every element of the current doc that the snapshot
   * does not contain, so other devices delete them too instead of merging
   * them back in.
   */
  restoreOp(doc: Doc, entry: VersionEntry): Op {
    const restoredIds = new Set<string>();
    for (const page of entry.snapshot.pages) {
      for (const stroke of page.strokes) restoredIds.add(stroke.id);
      for (const img of page.images) restoredIds.add(img.id);
    }

    const pages = entry.snapshot.pages.map((p) => ({
      ...p,
      strokes: p.strokes.map((s) => ({ ...s, points: [...s.points] })),
      tombstones: [...p.tombstones]
    }));
    const byId = new Map(pages.map((p) => [p.id, p]));
    const fallback = pages[pages.length - 1];

    for (const pageId of doc.pageOrder) {
      const livePage = doc.pages.get(pageId);
      if (!livePage) continue;
      const target = byId.get(pageId) ?? fallback;
      if (!target) continue;
      const purged: string[] = [];
      for (const id of livePage.strokeOrder) if (!restoredIds.has(id)) purged.push(id);
      for (const id of livePage.imageOrder) if (!restoredIds.has(id)) purged.push(id);
      if (purged.length > 0) {
        target.tombstones = [...new Set([...target.tombstones, ...purged])];
      }
    }

    return { type: 'replace-doc', pages };
  }

  totalBytes(): number {
    return this.entries.reduce((sum, e) => sum + e.bytes, 0);
  }

  private evict(): void {
    const overBudget = () => this.entries.length > MAX_ENTRIES || this.totalBytes() > MAX_TOTAL_BYTES;
    while (this.entries.length > 1 && overBudget()) {
      const idx = this.entries.findIndex((e) => !e.isMilestone);
      this.entries.splice(idx >= 0 ? idx : 0, 1);
    }
  }
}
