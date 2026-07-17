import { describe, expect, it } from 'vitest';
import { createDoc, createPage, makeStroke, packPoints } from './model';
import type { Doc } from './model';
import { addStrokeOp, applyOp, invertOp, removeStrokesOp } from './ops';
import type { Op } from './ops';
import { serializeDoc, serializeStroke } from './serial';
import { DocStore } from './store';

function docWithStrokes(count: number): { doc: Doc; pageId: string; strokeIds: string[] } {
  const doc = createDoc();
  const pageId = doc.pageOrder[0]!;
  const strokeIds: string[] = [];
  for (let i = 0; i < count; i++) {
    const stroke = makeStroke({
      tool: 'pen',
      color: '#000',
      width: 2,
      points: packPoints([
        { x: i, y: i },
        { x: i + 10, y: i + 10 }
      ])
    });
    applyOp(doc, addStrokeOp(pageId, serializeStroke(stroke)));
    strokeIds.push(stroke.id);
  }
  return { doc, pageId, strokeIds };
}

/**
 * Content snapshot for round-trip checks. Excludes `rev` (bumps on every op
 * by design) and tombstones (deliberately monotone: undoing an *add* leaves
 * a tombstone so the deletion propagates through sync — asserted separately).
 */
function snapshot(doc: Doc): string {
  const s = serializeDoc(doc);
  return JSON.stringify({
    id: s.id,
    pageOrder: s.pageOrder,
    meta: s.meta,
    pages: s.pages.map((p) => ({
      ...p,
      tombstones: undefined,
      strokes: p.strokes.map((st) => ({ ...st, points: [...st.points] }))
    }))
  });
}

/** Apply op, then its inverse, and check the doc round-trips exactly. */
function expectRoundTrip(doc: Doc, op: Op): void {
  const before = snapshot(doc);
  const inverse = invertOp(doc, op);
  applyOp(doc, op);
  applyOp(doc, inverse);
  expect(snapshot(doc)).toBe(before);
}

describe('applyOp / invertOp round-trips', () => {
  it('add stroke', () => {
    const { doc, pageId } = docWithStrokes(2);
    const stroke = makeStroke({ tool: 'pen', color: '#f00', width: 3, points: packPoints([{ x: 1, y: 2 }, { x: 3, y: 4 }]) });
    expectRoundTrip(doc, addStrokeOp(pageId, serializeStroke(stroke)));
  });

  it('remove strokes from the middle preserves z-order on undo', () => {
    const { doc, pageId, strokeIds } = docWithStrokes(5);
    const op = removeStrokesOp(pageId, [strokeIds[1]!, strokeIds[3]!]);
    const inverse = invertOp(doc, op);
    applyOp(doc, op);
    expect(doc.pages.get(pageId)!.strokeOrder).toEqual([strokeIds[0], strokeIds[2], strokeIds[4]]);
    applyOp(doc, inverse);
    expect(doc.pages.get(pageId)!.strokeOrder).toEqual(strokeIds);
  });

  it('remove strokes adds tombstones; re-add clears them', () => {
    const { doc, pageId, strokeIds } = docWithStrokes(3);
    const op = removeStrokesOp(pageId, [strokeIds[0]!]);
    const inverse = invertOp(doc, op);
    applyOp(doc, op);
    expect(doc.pages.get(pageId)!.tombstones.has(strokeIds[0]!)).toBe(true);
    applyOp(doc, inverse);
    expect(doc.pages.get(pageId)!.tombstones.has(strokeIds[0]!)).toBe(false);
  });

  it('transform strokes', () => {
    const { doc, pageId, strokeIds } = docWithStrokes(2);
    const op: Op = {
      type: 'transform-strokes',
      pageId,
      entries: [{ id: strokeIds[0]!, points: packPoints([{ x: 100, y: 100 }, { x: 110, y: 110 }]) }]
    };
    expectRoundTrip(doc, op);
  });

  it('transform updates bbox', () => {
    const { doc, pageId, strokeIds } = docWithStrokes(1);
    applyOp(doc, {
      type: 'transform-strokes',
      pageId,
      entries: [{ id: strokeIds[0]!, points: packPoints([{ x: 100, y: 200 }, { x: 110, y: 210 }]) }]
    });
    const stroke = doc.pages.get(pageId)!.strokes.get(strokeIds[0]!)!;
    expect(stroke.bbox).toEqual({ x0: 100, y0: 200, x1: 110, y1: 210 });
  });

  it('add / remove / move page', () => {
    const { doc } = docWithStrokes(1);
    const page = createPage();
    const addOp: Op = { type: 'add-page', page: { ...serializeDoc(createDoc({ pages: [page] })).pages[0]! }, index: 1 };
    expectRoundTrip(doc, addOp);

    applyOp(doc, addOp);
    expectRoundTrip(doc, { type: 'move-page', pageId: page.id, toIndex: 0 });
    expectRoundTrip(doc, { type: 'remove-page', pageId: page.id });
  });

  it('remove-page records a doc-level tombstone', () => {
    const doc = createDoc({ pages: [createPage(), createPage()] });
    const pageId = doc.pageOrder[1]!;
    applyOp(doc, { type: 'remove-page', pageId });
    expect(doc.pageTombstones.has(pageId)).toBe(true);
  });

  it('set background / side panel / meta', () => {
    const { doc, pageId } = docWithStrokes(1);
    expectRoundTrip(doc, { type: 'set-page-background', pageId, background: { kind: 'template', template: 'agenda' } });
    expectRoundTrip(doc, { type: 'set-page-side-panel', pageId, sidePanel: { mode: 'day', dateKeys: ['2026-06-11'] } });
    expectRoundTrip(doc, { type: 'set-meta', meta: { name: 'otro' } });
  });

  it('replace-doc round-trips and tombstones dropped pages', () => {
    const { doc } = docWithStrokes(2);
    const droppedPageId = doc.pageOrder[0]!;
    const replacement = serializeDoc(createDoc({ pages: [createPage()] }));
    const op: Op = { type: 'replace-doc', pages: replacement.pages };
    const inverse = invertOp(doc, op);
    const before = snapshot(doc);
    applyOp(doc, op);
    expect(doc.pageTombstones.has(droppedPageId)).toBe(true);
    applyOp(doc, inverse);
    expect(snapshot(doc)).toBe(before);
  });

  it('ops are tolerant of missing ids (post-merge replay)', () => {
    const { doc, pageId } = docWithStrokes(1);
    applyOp(doc, removeStrokesOp(pageId, ['nonexistent']));
    applyOp(doc, { type: 'transform-strokes', pageId, entries: [{ id: 'nope', points: packPoints([{ x: 0, y: 0 }]) }] });
    applyOp(doc, { type: 'move-page', pageId: 'ghost', toIndex: 0 });
    expect(doc.pages.get(pageId)!.strokeOrder.length).toBe(1);
  });
});

describe('DocStore', () => {
  it('emits applied ops with inverses and bumps rev', () => {
    const { doc, pageId } = docWithStrokes(0);
    const store = new DocStore(doc);
    const events: string[] = [];
    store.subscribe((applied) => events.push(`${applied.op.type}:${applied.source}`));
    const stroke = makeStroke({ tool: 'pen', color: '#000', width: 2, points: packPoints([{ x: 0, y: 0 }, { x: 1, y: 1 }]) });
    const rev = store.doc.rev;
    store.apply(addStrokeOp(pageId, serializeStroke(stroke)));
    expect(events).toEqual(['splice-strokes:local']);
    expect(store.doc.rev).toBe(rev + 1);
  });
});
