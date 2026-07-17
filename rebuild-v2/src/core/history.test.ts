import { describe, expect, it } from 'vitest';
import { createDoc, makeStroke, packPoints } from './model';
import { addStrokeOp } from './ops';
import { serializeStroke } from './serial';
import { DocStore } from './store';
import { History } from './history';

function setup() {
  const doc = createDoc();
  const store = new DocStore(doc);
  const history = new History();
  store.subscribe((applied) => history.record(applied));
  const pageId = doc.pageOrder[0]!;
  const draw = () => {
    const stroke = makeStroke({
      tool: 'pen',
      color: '#000',
      width: 2,
      points: packPoints([{ x: 0, y: 0 }, { x: 5, y: 5 }])
    });
    store.apply(addStrokeOp(pageId, serializeStroke(stroke)));
    return stroke.id;
  };
  return { store, history, pageId, draw };
}

describe('History', () => {
  it('undo removes the stroke, redo restores it', () => {
    const { store, history, pageId, draw } = setup();
    const strokeId = draw();
    const page = () => store.doc.pages.get(pageId)!;

    expect(page().strokes.has(strokeId)).toBe(true);
    expect(history.undo(store)).toBe(true);
    expect(page().strokes.has(strokeId)).toBe(false);
    expect(history.redo(store)).toBe(true);
    expect(page().strokes.has(strokeId)).toBe(true);
  });

  it('a new local op clears the redo stack', () => {
    const { store, history, draw } = setup();
    draw();
    history.undo(store);
    expect(history.canRedo()).toBe(true);
    draw();
    expect(history.canRedo()).toBe(false);
  });

  it('remote and history ops do not create undo entries', () => {
    const { store, history, pageId } = setup();
    const stroke = makeStroke({ tool: 'pen', color: '#000', width: 2, points: packPoints([{ x: 0, y: 0 }, { x: 1, y: 1 }]) });
    store.apply(addStrokeOp(pageId, serializeStroke(stroke)), 'remote');
    expect(history.canUndo()).toBe(false);
  });

  it('undo/redo of an undo replay does not duplicate entries', () => {
    const { store, history, draw } = setup();
    draw();
    draw();
    history.undo(store);
    history.undo(store);
    history.redo(store);
    history.redo(store);
    expect(history.canRedo()).toBe(false);
    expect(history.canUndo()).toBe(true);
    // Exactly two undos available again, not four.
    expect(history.undo(store)).toBe(true);
    expect(history.undo(store)).toBe(true);
    expect(history.undo(store)).toBe(false);
  });
});
