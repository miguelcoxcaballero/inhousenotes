// Serialization between the in-memory model (Maps/Sets/Float32Array) and
// plain structured-clone/JSON-safe records.
//
// SerialDoc uses number[] for points throughout:
//   - IndexedDB: structured clone converts Float32Array → number[] automatically.
//   - JSON/Drive sidecar: JSON.parse always returns number[].
// The in-memory Stroke model always keeps Float32Array; conversion happens at
// the serialize/deserialize boundary.

import type { Background, Doc, DocMeta, Img, Page, SidePanelConfig, Stroke } from './model';
import { computeBbox } from './model';

export interface SerialStroke {
  id: string;
  tool: Stroke['tool'];
  color: string;
  width: number;
  points: number[] | Float32Array;
}

export interface SerialPage {
  id: string;
  width: number;
  height: number;
  background: Background;
  strokes: SerialStroke[]; // in draw order
  images: Img[]; // in draw order
  tombstones: string[];
  sidePanel: SidePanelConfig | null;
}

export interface SerialDoc {
  id: string;
  rev: number;
  pageOrder: string[];
  pages: SerialPage[];
  pageTombstones: string[];
  meta: DocMeta;
}

export function serializeStroke(s: Stroke): SerialStroke {
  return { id: s.id, tool: s.tool, color: s.color, width: s.width, points: [...s.points] };
}

export function deserializeStroke(s: SerialStroke): Stroke {
  const points = s.points instanceof Float32Array ? s.points : new Float32Array(s.points);
  return { id: s.id, tool: s.tool, color: s.color, width: s.width, points, bbox: computeBbox(points) };
}

export function serializePage(page: Page): SerialPage {
  return {
    id: page.id,
    width: page.width,
    height: page.height,
    background: page.background,
    strokes: page.strokeOrder
      .map((id) => page.strokes.get(id))
      .filter((s): s is Stroke => !!s)
      .map(serializeStroke),
    images: page.imageOrder
      .map((id) => page.images.get(id))
      .filter((img): img is Img => !!img)
      .map((img) => ({ ...img })),
    tombstones: [...page.tombstones],
    sidePanel: page.sidePanel ? { ...page.sidePanel, dateKeys: [...page.sidePanel.dateKeys] } : null
  };
}

export function deserializePage(s: SerialPage): Page {
  const strokes = s.strokes.map(deserializeStroke);
  return {
    id: s.id,
    width: s.width,
    height: s.height,
    background: s.background,
    strokeOrder: strokes.map((st) => st.id),
    strokes: new Map(strokes.map((st) => [st.id, st])),
    imageOrder: s.images.map((img) => img.id),
    images: new Map(s.images.map((img) => [img.id, { ...img }])),
    tombstones: new Set(s.tombstones),
    sidePanel: s.sidePanel ? { ...s.sidePanel, dateKeys: [...s.sidePanel.dateKeys] } : null
  };
}

export function serializeDoc(doc: Doc): SerialDoc {
  return {
    id: doc.id,
    rev: doc.rev,
    pageOrder: [...doc.pageOrder],
    pages: doc.pageOrder
      .map((id) => doc.pages.get(id))
      .filter((p): p is Page => !!p)
      .map(serializePage),
    pageTombstones: [...doc.pageTombstones],
    meta: { ...doc.meta }
  };
}

export function deserializeDoc(s: SerialDoc): Doc {
  const pages = s.pages.map(deserializePage);
  return {
    id: s.id,
    rev: s.rev,
    pageOrder: [...s.pageOrder],
    pages: new Map(pages.map((p) => [p.id, p])),
    pageTombstones: new Set(s.pageTombstones ?? []),
    meta: { ...s.meta }
  };
}

// ── JSON (number[] points) for the Drive sidecar and fixtures ─────────────
// After the Float32Array → number[] refactor, SerialDoc already uses number[],
// so JsonDoc/JsonStroke/JsonPage are type aliases over SerialDoc.
// toJsonDoc / fromJsonDoc are kept for backward compatibility.

export type JsonStroke = SerialStroke;
export type JsonPage = SerialPage;
export type JsonDoc = SerialDoc;

export function toJsonDoc(doc: Doc): JsonDoc {
  const serial = serializeDoc(doc);
  return {
    ...serial,
    pages: serial.pages.map((page) => ({
      ...page,
      strokes: page.strokes.map((stroke) => ({
        ...stroke,
        points: Array.from(stroke.points)
      }))
    }))
  };
}

export function fromJsonDoc(j: JsonDoc): Doc {
  return deserializeDoc(j);
}
