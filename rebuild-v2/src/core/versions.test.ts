import { describe, expect, it } from 'vitest';
import { createDoc, makeStroke, packPoints } from './model';
import { addStrokeOp, applyOp } from './ops';
import { serializeStroke } from './serial';
import { VersionLog } from './versions';
import type { VersionAuthor } from './versions';

const ALICE: VersionAuthor = { name: 'Alice', email: 'a@x.com', photo: '' };
const BOB: VersionAuthor = { name: 'Bob', email: 'b@x.com', photo: '' };

function draw(doc: ReturnType<typeof createDoc>, x = 0): string {
  const pageId = doc.pageOrder[0]!;
  const stroke = makeStroke({ tool: 'pen', color: '#000', width: 2, points: packPoints([{ x, y: 0 }, { x, y: 10 }]) });
  applyOp(doc, addStrokeOp(pageId, serializeStroke(stroke)));
  return stroke.id;
}

describe('VersionLog', () => {
  it('coalesces rapid captures by the same author', () => {
    const doc = createDoc();
    const log = new VersionLog();
    draw(doc, 1);
    log.capture(doc, ALICE);
    draw(doc, 2);
    log.capture(doc, ALICE);
    expect(log.entries).toHaveLength(1);
    expect(log.entries[0]!.snapshot.pages[0]!.strokes).toHaveLength(2);
  });

  it('does not coalesce across authors or milestones', () => {
    const doc = createDoc();
    const log = new VersionLog();
    draw(doc, 1);
    log.capture(doc, ALICE);
    draw(doc, 2);
    log.capture(doc, BOB);
    draw(doc, 3);
    log.capture(doc, BOB, { milestone: true, kind: 'restore', summary: 'Restored' });
    draw(doc, 4);
    log.capture(doc, BOB);
    expect(log.entries).toHaveLength(4);
    expect(log.entries[2]!.isMilestone).toBe(true);
  });

  it('summarizes the delta against the previous entry', () => {
    const doc = createDoc();
    const log = new VersionLog();
    draw(doc, 1);
    log.capture(doc, ALICE);
    draw(doc, 2);
    draw(doc, 3);
    log.capture(doc, BOB);
    expect(log.entries[1]!.summary).toBe('+2 strokes');
  });

  it('restoreOp rolls back and tombstones discarded strokes', () => {
    const doc = createDoc();
    const log = new VersionLog();
    const keptId = draw(doc, 1);
    log.capture(doc, ALICE);
    const entry = log.entries[0]!;

    const discardedId = draw(doc, 2);
    const op = log.restoreOp(doc, entry);
    applyOp(doc, op);

    const page = doc.pages.get(doc.pageOrder[0]!)!;
    expect(page.strokes.has(keptId)).toBe(true);
    expect(page.strokes.has(discardedId)).toBe(false);
    expect(page.tombstones.has(discardedId)).toBe(true);
    expect(page.tombstones.has(keptId)).toBe(false);
  });

  it('evicts oldest non-milestones first when over the entry cap', () => {
    const doc = createDoc();
    const log = new VersionLog();
    draw(doc, 0);
    const milestone = log.capture(doc, ALICE, { milestone: true, kind: 'restore', summary: 'M' });
    for (let i = 1; i <= 105; i++) {
      draw(doc, i);
      // Alternate authors to defeat coalescing.
      log.capture(doc, i % 2 === 0 ? ALICE : BOB);
      log.entries[log.entries.length - 1]!.ts -= 10 * 60 * 1000; // age it out of the coalesce window
    }
    expect(log.entries.length).toBeLessThanOrEqual(100);
    expect(log.entries.some((e) => e.id === milestone.id)).toBe(true);
  });
});
