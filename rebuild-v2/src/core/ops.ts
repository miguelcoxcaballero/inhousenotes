// Every document mutation is an Op. Ops are plain data (structured-clone
// safe), apply in-place via applyOp, and each op has a computable inverse
// (invertOp, evaluated against the PRE-state) — undo/redo, the oplog and
// version deltas all reuse this single mechanism.
//
// Elements are addressed by id, never by array index. The only indices that
// appear are z-order insertion positions captured inside inverse ops.

import type { Doc, Background, Img, SidePanelConfig, DocMeta } from './model';
import { addTombstones, getPage } from './model';
import type { SerialPage, SerialStroke } from './serial';
import { deserializePage, deserializeStroke, serializePage, serializeStroke } from './serial';
import type { PageId, StrokeId, ImageId } from './ids';

export type StrokePointData = number[] | Float32Array;

export interface StrokeAt {
  stroke: SerialStroke;
  index: number;
}

export interface ImageAt {
  image: Img;
  index: number;
}

export type Op =
  | { type: 'splice-strokes'; pageId: PageId; remove: StrokeId[]; add: StrokeAt[] }
  | { type: 'transform-strokes'; pageId: PageId; entries: { id: StrokeId; points: StrokePointData }[] }
  | { type: 'splice-images'; pageId: PageId; remove: ImageId[]; add: ImageAt[] }
  | { type: 'transform-images'; pageId: PageId; entries: { id: ImageId; after: Img }[] }
  | { type: 'add-page'; page: SerialPage; index: number }
  | { type: 'remove-page'; pageId: PageId }
  | { type: 'move-page'; pageId: PageId; toIndex: number }
  | { type: 'set-page-background'; pageId: PageId; background: Background }
  | { type: 'set-page-side-panel'; pageId: PageId; sidePanel: SidePanelConfig | null }
  | { type: 'set-meta'; meta: Partial<DocMeta> }
  | { type: 'replace-doc'; pages: SerialPage[] };

// ── Helpers ───────────────────────────────────────────────────────────────

export function addStrokeOp(pageId: PageId, stroke: SerialStroke, index = Number.MAX_SAFE_INTEGER): Op {
  return { type: 'splice-strokes', pageId, remove: [], add: [{ stroke, index }] };
}

export function removeStrokesOp(pageId: PageId, strokeIds: StrokeId[]): Op {
  return { type: 'splice-strokes', pageId, remove: strokeIds, add: [] };
}

// ── Apply ─────────────────────────────────────────────────────────────────

export function applyOp(doc: Doc, op: Op): void {
  switch (op.type) {
    case 'splice-strokes': {
      const page = getPage(doc, op.pageId);
      if (op.remove.length > 0) {
        const removeSet = new Set(op.remove);
        const present = op.remove.filter((id) => page.strokes.has(id));
        for (const id of present) page.strokes.delete(id);
        if (present.length > 0) {
          page.strokeOrder = page.strokeOrder.filter((id) => !removeSet.has(id));
          addTombstones(page, present);
        }
      }
      if (op.add.length > 0) {
        const sorted = [...op.add].sort((a, b) => a.index - b.index);
        for (const { stroke, index } of sorted) {
          if (page.strokes.has(stroke.id)) continue; // idempotent on replay
          const live = deserializeStroke(stroke);
          page.strokes.set(live.id, live);
          page.strokeOrder.splice(Math.min(index, page.strokeOrder.length), 0, live.id);
          page.tombstones.delete(live.id);
        }
      }
      break;
    }
    case 'transform-strokes': {
      const page = getPage(doc, op.pageId);
      for (const { id, points } of op.entries) {
        const stroke = page.strokes.get(id);
        if (!stroke) continue;
        page.strokes.set(id, deserializeStroke({ ...serializeStroke(stroke), points: Array.from(points) }));
      }
      break;
    }
    case 'splice-images': {
      const page = getPage(doc, op.pageId);
      if (op.remove.length > 0) {
        const removeSet = new Set(op.remove);
        const present = op.remove.filter((id) => page.images.has(id));
        for (const id of present) page.images.delete(id);
        if (present.length > 0) {
          page.imageOrder = page.imageOrder.filter((id) => !removeSet.has(id));
          addTombstones(page, present);
        }
      }
      if (op.add.length > 0) {
        const sorted = [...op.add].sort((a, b) => a.index - b.index);
        for (const { image, index } of sorted) {
          if (page.images.has(image.id)) continue;
          page.images.set(image.id, { ...image });
          page.imageOrder.splice(Math.min(index, page.imageOrder.length), 0, image.id);
          page.tombstones.delete(image.id);
        }
      }
      break;
    }
    case 'transform-images': {
      const page = getPage(doc, op.pageId);
      for (const { id, after } of op.entries) {
        if (page.images.has(id)) page.images.set(id, { ...after });
      }
      break;
    }
    case 'add-page': {
      if (doc.pages.has(op.page.id)) break; // idempotent on replay
      const page = deserializePage(op.page);
      doc.pages.set(page.id, page);
      doc.pageOrder.splice(Math.min(op.index, doc.pageOrder.length), 0, page.id);
      doc.pageTombstones.delete(page.id);
      break;
    }
    case 'remove-page': {
      if (!doc.pages.has(op.pageId)) break;
      doc.pages.delete(op.pageId);
      doc.pageOrder = doc.pageOrder.filter((id) => id !== op.pageId);
      doc.pageTombstones.add(op.pageId);
      break;
    }
    case 'move-page': {
      const from = doc.pageOrder.indexOf(op.pageId);
      if (from < 0) break;
      doc.pageOrder.splice(from, 1);
      doc.pageOrder.splice(Math.min(op.toIndex, doc.pageOrder.length), 0, op.pageId);
      break;
    }
    case 'set-page-background': {
      getPage(doc, op.pageId).background = op.background;
      break;
    }
    case 'set-page-side-panel': {
      getPage(doc, op.pageId).sidePanel = op.sidePanel;
      break;
    }
    case 'set-meta': {
      doc.meta = { ...doc.meta, ...op.meta };
      break;
    }
    case 'replace-doc': {
      const pages = op.pages.map(deserializePage);
      const newIds = new Set(pages.map((p) => p.id));
      // Pages dropped by the replace are tombstoned so the collaborative
      // merge deletes them remotely instead of resurrecting them.
      for (const id of doc.pageOrder) {
        if (!newIds.has(id)) doc.pageTombstones.add(id);
      }
      for (const id of newIds) doc.pageTombstones.delete(id);
      doc.pages = new Map(pages.map((p) => [p.id, p]));
      doc.pageOrder = pages.map((p) => p.id);
      break;
    }
  }
  doc.rev++;
}

// ── Invert (call with doc in PRE state) ───────────────────────────────────

export function invertOp(doc: Doc, op: Op): Op {
  switch (op.type) {
    case 'splice-strokes': {
      const page = getPage(doc, op.pageId);
      const removed: StrokeAt[] = [];
      for (const id of op.remove) {
        const stroke = page.strokes.get(id);
        const index = page.strokeOrder.indexOf(id);
        if (stroke && index >= 0) removed.push({ stroke: serializeStroke(stroke), index });
      }
      return {
        type: 'splice-strokes',
        pageId: op.pageId,
        remove: op.add.map((a) => a.stroke.id),
        add: removed
      };
    }
    case 'transform-strokes': {
      const page = getPage(doc, op.pageId);
      const entries: { id: StrokeId; points: number[] }[] = [];
      for (const { id } of op.entries) {
        const stroke = page.strokes.get(id);
        if (stroke) entries.push({ id, points: [...stroke.points] });
      }
      return { type: 'transform-strokes', pageId: op.pageId, entries };
    }
    case 'splice-images': {
      const page = getPage(doc, op.pageId);
      const removed: ImageAt[] = [];
      for (const id of op.remove) {
        const image = page.images.get(id);
        const index = page.imageOrder.indexOf(id);
        if (image && index >= 0) removed.push({ image: { ...image }, index });
      }
      return {
        type: 'splice-images',
        pageId: op.pageId,
        remove: op.add.map((a) => a.image.id),
        add: removed
      };
    }
    case 'transform-images': {
      const page = getPage(doc, op.pageId);
      const entries: { id: ImageId; after: Img }[] = [];
      for (const { id } of op.entries) {
        const image = page.images.get(id);
        if (image) entries.push({ id, after: { ...image } });
      }
      return { type: 'transform-images', pageId: op.pageId, entries };
    }
    case 'add-page':
      return { type: 'remove-page', pageId: op.page.id };
    case 'remove-page': {
      const page = getPage(doc, op.pageId);
      return { type: 'add-page', page: serializePage(page), index: doc.pageOrder.indexOf(op.pageId) };
    }
    case 'move-page':
      return { type: 'move-page', pageId: op.pageId, toIndex: doc.pageOrder.indexOf(op.pageId) };
    case 'set-page-background':
      return {
        type: 'set-page-background',
        pageId: op.pageId,
        background: getPage(doc, op.pageId).background
      };
    case 'set-page-side-panel':
      return {
        type: 'set-page-side-panel',
        pageId: op.pageId,
        sidePanel: getPage(doc, op.pageId).sidePanel
      };
    case 'set-meta': {
      const before: Partial<DocMeta> = {};
      for (const key of Object.keys(op.meta) as (keyof DocMeta)[]) {
        (before as Record<string, unknown>)[key] = doc.meta[key];
      }
      return { type: 'set-meta', meta: before };
    }
    case 'replace-doc':
      return {
        type: 'replace-doc',
        pages: doc.pageOrder
          .map((id) => doc.pages.get(id))
          .filter((p): p is NonNullable<typeof p> => !!p)
          .map(serializePage)
      };
  }
}
