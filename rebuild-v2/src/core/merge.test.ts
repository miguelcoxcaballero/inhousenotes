import { describe, expect, it } from 'vitest';
import { createDoc, createPage, makeStroke, packPoints } from './model';
import type { Doc } from './model';
import { addStrokeOp, applyOp, removeStrokesOp } from './ops';
import { deserializeDoc, serializeDoc, serializeStroke } from './serial';
import type { SerialDoc } from './serial';
import { mergeDocs } from './merge';

function makeBaseDoc(): { doc: Doc; pageId: string; strokeIds: string[] } {
  const doc = createDoc({ id: 'doc1' });
  const pageId = doc.pageOrder[0]!;
  const strokeIds: string[] = [];
  for (let i = 0; i < 3; i++) {
    const stroke = makeStroke({
      tool: 'pen',
      color: '#000',
      width: 2,
      points: packPoints([{ x: i, y: 0 }, { x: i, y: 10 }])
    });
    applyOp(doc, addStrokeOp(pageId, serializeStroke(stroke)));
    strokeIds.push(stroke.id);
  }
  return { doc, pageId, strokeIds };
}

function clone(doc: Doc): Doc {
  // Deep clone through serialization (same path used by persistence).
  const s = serializeDoc(doc);
  const json = JSON.stringify({
    ...s,
    pages: s.pages.map((p) => ({ ...p, strokes: p.strokes.map((st) => ({ ...st, points: [...st.points] })) }))
  });
  const parsed = JSON.parse(json);
  parsed.pages = parsed.pages.map((p: { strokes: { points: number[] }[] }) => ({
    ...p,
    strokes: p.strokes.map((st) => ({ ...st, points: new Float32Array(st.points) }))
  }));
  return deserializeDoc(parsed);
}

function pageStrokeIds(d: SerialDoc, pageIdx = 0): string[] {
  return d.pages[pageIdx]!.strokes.map((s) => s.id);
}

describe('mergeDocs', () => {
  it('unions strokes drawn on different devices', () => {
    const { doc: base, pageId } = makeBaseDoc();
    const local = clone(base);
    const remote = clone(base);

    const localStroke = makeStroke({ tool: 'pen', color: '#00f', width: 2, points: packPoints([{ x: 50, y: 0 }, { x: 50, y: 9 }]) });
    applyOp(local, addStrokeOp(pageId, serializeStroke(localStroke)));
    const remoteStroke = makeStroke({ tool: 'pen', color: '#f00', width: 2, points: packPoints([{ x: 60, y: 0 }, { x: 60, y: 9 }]) });
    applyOp(remote, addStrokeOp(pageId, serializeStroke(remoteStroke)));

    const result = mergeDocs(serializeDoc(local), serializeDoc(remote), serializeDoc(base));
    const ids = pageStrokeIds(result.merged);
    expect(ids).toContain(localStroke.id);
    expect(ids).toContain(remoteStroke.id);
    expect(result.changedFromLocal).toBe(true);
    expect(result.changedFromRemote).toBe(true);
  });

  it('a deletion on one device wins over the copy on the other', () => {
    const { doc: base, pageId, strokeIds } = makeBaseDoc();
    const local = clone(base);
    const remote = clone(base);
    applyOp(remote, removeStrokesOp(pageId, [strokeIds[1]!]));

    const result = mergeDocs(serializeDoc(local), serializeDoc(remote), serializeDoc(base));
    expect(pageStrokeIds(result.merged)).toEqual([strokeIds[0], strokeIds[2]]);
    expect(result.changedFromLocal).toBe(true);
  });

  it('with a base, a remote transform lands when local did not touch the stroke', () => {
    const { doc: base, pageId, strokeIds } = makeBaseDoc();
    const local = clone(base);
    const remote = clone(base);
    applyOp(remote, {
      type: 'transform-strokes',
      pageId,
      entries: [{ id: strokeIds[0]!, points: packPoints([{ x: 99, y: 99 }, { x: 99, y: 109 }]) }]
    });

    const result = mergeDocs(serializeDoc(local), serializeDoc(remote), serializeDoc(base));
    const moved = result.merged.pages[0]!.strokes.find((s) => s.id === strokeIds[0]);
    expect(moved!.points[0]).toBe(99);
  });

  it('when both sides transform the same stroke, local wins', () => {
    const { doc: base, pageId, strokeIds } = makeBaseDoc();
    const local = clone(base);
    const remote = clone(base);
    applyOp(local, {
      type: 'transform-strokes',
      pageId,
      entries: [{ id: strokeIds[0]!, points: packPoints([{ x: 11, y: 0 }, { x: 11, y: 10 }]) }]
    });
    applyOp(remote, {
      type: 'transform-strokes',
      pageId,
      entries: [{ id: strokeIds[0]!, points: packPoints([{ x: 99, y: 0 }, { x: 99, y: 10 }]) }]
    });

    const result = mergeDocs(serializeDoc(local), serializeDoc(remote), serializeDoc(base));
    const stroke = result.merged.pages[0]!.strokes.find((s) => s.id === strokeIds[0]);
    expect(stroke!.points[0]).toBe(11);
  });

  it('pages added remotely appear; pages deleted remotely disappear', () => {
    const { doc: base } = makeBaseDoc();
    const local = clone(base);
    const remote = clone(base);

    const newPage = createPage();
    applyOp(remote, { type: 'add-page', page: serializeDoc(createDoc({ pages: [newPage] })).pages[0]!, index: 1 });
    const deletedId = base.pageOrder[0]!;
    applyOp(remote, { type: 'remove-page', pageId: deletedId });

    const result = mergeDocs(serializeDoc(local), serializeDoc(remote), serializeDoc(base));
    expect(result.merged.pageOrder).toEqual([newPage.id]);
    expect(result.merged.pageTombstones).toContain(deletedId);
  });

  it('local-only pages keep their position relative to their predecessor', () => {
    const { doc: base } = makeBaseDoc();
    const local = clone(base);
    const remote = clone(base);

    const localPage = createPage();
    applyOp(local, { type: 'add-page', page: serializeDoc(createDoc({ pages: [localPage] })).pages[0]!, index: 1 });
    const remotePage = createPage();
    applyOp(remote, { type: 'add-page', page: serializeDoc(createDoc({ pages: [remotePage] })).pages[0]!, index: 1 });

    const result = mergeDocs(serializeDoc(local), serializeDoc(remote), serializeDoc(base));
    expect(result.merged.pageOrder[0]).toBe(base.pageOrder[0]);
    expect(result.merged.pageOrder).toContain(localPage.id);
    expect(result.merged.pageOrder).toContain(remotePage.id);
    expect(result.merged.pageOrder).toHaveLength(3);
  });

  it('identical docs merge to no changes', () => {
    const { doc: base } = makeBaseDoc();
    const result = mergeDocs(serializeDoc(base), serializeDoc(clone(base)), serializeDoc(base));
    expect(result.changedFromLocal).toBe(false);
    expect(result.changedFromRemote).toBe(false);
  });
});
