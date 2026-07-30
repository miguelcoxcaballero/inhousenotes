import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import test from 'node:test';

const require = createRequire(import.meta.url);
const {
  ihnCanonicalDocumentHash,
  ihnChooseMaterializableFallbackPageId,
  ihnChooseConcurrentItem,
  ihnCompareCollabStamps,
  ihnComputeReconnectDelay,
  ihnDeletionWinsItem,
  ihnFieldMetaHash,
  ihnMergeDeletionStamps,
  ihnMergeFieldMeta,
  ihnMergeStructureMeta,
  ihnNormalizeDeletionStamps,
  ihnNormalizeFieldMeta,
  ihnNormalizeStructureMeta,
  ihnRecordFieldValue,
  ihnRecordPageAdded,
  ihnRecordPageDeleted,
  ihnRecordPageMoved,
  ihnRecordStructureReplacement,
  ihnResolveFieldValue,
  ihnStableStringify
} = require('../collaboration-core-v5.js');

const ids = meta => Object.keys(meta.entries);
const actorA = 'peer-a:tab-a';
const actorB = 'peer-b:tab-b';

test('structure merge is commutative and idempotent for independent page moves', () => {
  const baseOrder = ['a', 'b', 'c', 'd', 'e'];
  const base = ihnNormalizeStructureMeta(null, baseOrder);

  const orderA = ['a', 'e', 'b', 'c', 'd'];
  const replicaA = ihnRecordPageMoved(base, orderA, 'e', 1, actorA, 100);

  const orderB = ['a', 'b', 'd', 'c', 'e'];
  const replicaB = ihnRecordPageMoved(base, orderB, 'd', 2, actorB, 100);

  const ab = ihnMergeStructureMeta(replicaA, replicaB, orderA, orderB);
  const ba = ihnMergeStructureMeta(replicaB, replicaA, orderB, orderA);

  assert.deepEqual(ab.orderedPageIds, ba.orderedPageIds);
  assert.equal(ihnStableStringify(ab.meta), ihnStableStringify(ba.meta));
  assert.ok(ab.orderedPageIds.indexOf('e') < ab.orderedPageIds.indexOf('b'));
  assert.ok(ab.orderedPageIds.indexOf('d') < ab.orderedPageIds.indexOf('c'));

  const again = ihnMergeStructureMeta(ab.meta, ab.meta, ab.orderedPageIds, ab.orderedPageIds);
  assert.deepEqual(again.orderedPageIds, ab.orderedPageIds);
  assert.equal(ihnStableStringify(again.meta), ihnStableStringify(ab.meta));
});

test('same-page concurrent moves have a deterministic Lamport/actor winner', () => {
  const baseOrder = ['a', 'b', 'c', 'd'];
  const base = ihnNormalizeStructureMeta(null, baseOrder);
  const leftOrder = ['b', 'a', 'c', 'd'];
  const rightOrder = ['a', 'c', 'd', 'b'];
  const left = ihnRecordPageMoved(base, leftOrder, 'b', 0, actorA, 500);
  const right = ihnRecordPageMoved(base, rightOrder, 'b', 3, actorB, 500);

  const leftRight = ihnMergeStructureMeta(left, right, leftOrder, rightOrder);
  const rightLeft = ihnMergeStructureMeta(right, left, rightOrder, leftOrder);
  assert.deepEqual(leftRight.orderedPageIds, rightLeft.orderedPageIds);
  assert.equal(leftRight.orderedPageIds.at(-1), 'b', 'actor-b wins an equal-clock tie');
});

test('concurrent page additions both survive with canonical ordering', () => {
  const baseOrder = ['p1', 'p2'];
  const base = ihnNormalizeStructureMeta(null, baseOrder);
  const orderA = ['p1', 'add-a', 'p2'];
  const orderB = ['p1', 'add-b', 'p2'];
  const replicaA = ihnRecordPageAdded(base, orderA, 'add-a', 1, actorA, 1000);
  const replicaB = ihnRecordPageAdded(base, orderB, 'add-b', 1, actorB, 1000);

  const ab = ihnMergeStructureMeta(replicaA, replicaB, orderA, orderB);
  const ba = ihnMergeStructureMeta(replicaB, replicaA, orderB, orderA);
  assert.deepEqual(ab.orderedPageIds, ba.orderedPageIds);
  assert.deepEqual(new Set(ab.orderedPageIds), new Set(['p1', 'p2', 'add-a', 'add-b']));
  assert.equal(ab.orderedPageIds.filter(id => id === 'add-a').length, 1);
  assert.equal(ab.orderedPageIds.filter(id => id === 'add-b').length, 1);
});

test('page deletion is durable and does not erase an unrelated concurrent add', () => {
  const baseOrder = ['p1', 'p2'];
  const base = ihnNormalizeStructureMeta(null, baseOrder);
  const deleted = ihnRecordPageDeleted(base, ['p1'], 'p2', actorA, 2000);
  const addedOrder = ['p1', 'p2', 'p3'];
  const added = ihnRecordPageAdded(base, addedOrder, 'p3', 2, actorB, 2000);

  const result = ihnMergeStructureMeta(deleted, added, ['p1'], addedOrder);
  assert.deepEqual(result.orderedPageIds, ['p1', 'p3']);
  assert.ok(result.tombstonedPageIds.includes('p2'));

  const staleReplica = ihnNormalizeStructureMeta(null, ['p1', 'p2']);
  const afterStale = ihnMergeStructureMeta(result.meta, staleReplica, result.orderedPageIds, ['p1', 'p2']);
  assert.deepEqual(afterStale.orderedPageIds, ['p1', 'p3']);
});

test('Timeline structure replacement causally restores a previously deleted page', () => {
  const baseOrder = ['p1'];
  const base = ihnNormalizeStructureMeta(null, baseOrder);
  const timelineOrder = ['p1', 'timeline-page'];
  const added = ihnRecordPageAdded(
    base,
    timelineOrder,
    'timeline-page',
    1,
    actorA,
    100
  );
  const deleted = ihnRecordPageDeleted(
    added,
    baseOrder,
    'timeline-page',
    actorB,
    200
  );
  const restored = ihnRecordStructureReplacement(
    deleted,
    baseOrder,
    timelineOrder,
    actorA,
    300
  );

  assert.deepEqual(
    restored.entries['timeline-page'].createdStamp,
    { clock: 300, actor: actorA },
    'explicit restoration is a new causal creation, not the original stale add'
  );
  assert.equal(restored.tombstones['timeline-page'], undefined);

  const restoredDeleted = ihnMergeStructureMeta(
    restored,
    deleted,
    timelineOrder,
    baseOrder
  );
  const deletedRestored = ihnMergeStructureMeta(
    deleted,
    restored,
    baseOrder,
    timelineOrder
  );
  assert.deepEqual(restoredDeleted.orderedPageIds, timelineOrder);
  assert.deepEqual(deletedRestored.orderedPageIds, timelineOrder);
  assert.equal(
    ihnStableStringify(restoredDeleted.meta),
    ihnStableStringify(deletedRestored.meta),
    'the restore wins the stale delete regardless of arrival order'
  );
  assert.ok(!restoredDeleted.tombstonedPageIds.includes('timeline-page'));

  const again = ihnMergeStructureMeta(
    restoredDeleted.meta,
    restoredDeleted.meta,
    timelineOrder,
    timelineOrder
  );
  assert.deepEqual(again.orderedPageIds, timelineOrder);
  assert.equal(
    ihnStableStringify(again.meta),
    ihnStableStringify(restoredDeleted.meta),
    'the restored state remains idempotent'
  );
});

test('concurrent last-page deletes recover one deterministic page', () => {
  const baseOrder = ['p1', 'p2'];
  const base = ihnNormalizeStructureMeta(null, baseOrder);
  const left = ihnRecordPageDeleted(base, ['p1'], 'p2', actorA, 3000);
  const right = ihnRecordPageDeleted(base, ['p2'], 'p1', actorB, 3000);
  const merged = ihnMergeStructureMeta(left, right, ['p1'], ['p2']);
  assert.equal(merged.orderedPageIds.length, 1);
  assert.equal(merged.orderedPageIds[0], 'p1');
  assert.equal(merged.recoveredFallbackPageId, 'p1');
  assert.ok(
    merged.meta.tombstones.p1,
    'the projected fallback must retain its causal tombstone inside the join'
  );
});

test('an unavailable historical fallback chooses a deterministic materializable page body', () => {
  assert.equal(
    ihnChooseMaterializableFallbackPageId('p1', ['p2'], ['p3']),
    'p2'
  );
  assert.equal(
    ihnChooseMaterializableFallbackPageId('p1', ['p3'], ['p2']),
    'p2',
    'arrival order must not affect the recovered body'
  );
  assert.equal(
    ihnChooseMaterializableFallbackPageId('p1', [], []),
    'p1',
    'the recovered id remains usable for a deterministic blank shell when no body survives'
  );
});

test('structure merge stays associative when an all-deleted fallback meets a concurrent add', () => {
  const baseOrder = ['p1', 'p2'];
  const base = ihnNormalizeStructureMeta(null, baseOrder);
  const deletedP2 = ihnRecordPageDeleted(base, ['p1'], 'p2', actorA, 3000);
  const deletedP1 = ihnRecordPageDeleted(base, ['p2'], 'p1', actorB, 3000);
  const addedP3 = ihnRecordPageAdded(
    ihnNormalizeStructureMeta(null, []),
    [],
    'p3',
    0,
    'device-c',
    3000
  );

  const leftIntermediate = ihnMergeStructureMeta(
    deletedP2,
    deletedP1,
    ['p1'],
    ['p2']
  );
  const leftGrouped = ihnMergeStructureMeta(
    leftIntermediate.meta,
    addedP3,
    leftIntermediate.orderedPageIds,
    ['p3']
  );
  const rightIntermediate = ihnMergeStructureMeta(
    deletedP1,
    addedP3,
    ['p2'],
    ['p3']
  );
  const rightGrouped = ihnMergeStructureMeta(
    deletedP2,
    rightIntermediate.meta,
    ['p1'],
    rightIntermediate.orderedPageIds
  );

  assert.equal(ihnStructureHash(leftGrouped.meta), ihnStructureHash(rightGrouped.meta));
  assert.deepEqual(leftGrouped.orderedPageIds, ['p3']);
  assert.deepEqual(rightGrouped.orderedPageIds, ['p3']);
});

test('canonical document hash ignores arrival order and derived panel HTML', () => {
  const first = {
    pageId: 'p1',
    strokes: [{ id: 's2', points: [{ x: 2, y: 2 }] }, { id: 's1', points: [{ x: 1, y: 1 }] }],
    images: [{ id: 'i2', src: 'b' }, { id: 'i1', src: 'a' }],
    deletedStrokeIds: ['gone-b', 'gone-a'],
    backgroundSource: 'template',
    sidePanel: { title: 'Today', dateKeys: ['2026-07-30'], html: '<p>device A cache</p>' }
  };
  const second = {
    ...first,
    strokes: [...first.strokes].reverse(),
    images: [...first.images].reverse(),
    deletedStrokeIds: [...first.deletedStrokeIds].reverse(),
    sidePanel: { ...first.sidePanel, html: '<p>device B cache</p>' }
  };
  const structure = ihnNormalizeStructureMeta(null, ['p1']);
  assert.equal(
    ihnCanonicalDocumentHash([first], structure, null, 'Doc'),
    ihnCanonicalDocumentHash([second], structure, null, 'Doc')
  );
  const customOnDeviceA = {
    ...first,
    backgroundSource: 'custom',
    backgroundImage: 'blob:https://device-a/local-a',
    pdfPageIndex: 2
  };
  const customOnDeviceB = {
    ...customOnDeviceA,
    backgroundImage: 'blob:https://device-b/local-b',
    pdfPageIndex: 7
  };
  assert.equal(
    ihnCanonicalDocumentHash([customOnDeviceA], structure, null, 'Doc'),
    ihnCanonicalDocumentHash([customOnDeviceB], structure, null, 'Doc'),
    'local blob URLs and physical PDF locators must not create convergence loops'
  );
  const causallyDeleted = {
    ...first,
    deletedStrokeStamps: {
      'gone-a': { clock: 12, actor: actorA },
      'gone-b': { clock: 0, actor: '' }
    }
  };
  assert.notEqual(
    ihnCanonicalDocumentHash([first], structure, null, 'Doc'),
    ihnCanonicalDocumentHash([causallyDeleted], structure, null, 'Doc'),
    'deletion clocks are semantic state and must trigger live delivery'
  );
});

test('same-ID content conflicts use stamps, then a stable fingerprint tie-break', () => {
  const older = { id: 'same', x: 1, syncStamp: { clock: 10, actor: actorA } };
  const newer = { id: 'same', x: 2, syncStamp: { clock: 11, actor: actorB } };
  assert.equal(ihnChooseConcurrentItem(older, newer), newer);
  assert.equal(ihnCompareCollabStamps(older.syncStamp, newer.syncStamp), -1);

  const tieA = { id: 'same', x: 1 };
  const tieB = { id: 'same', x: 2 };
  const winnerAB = ihnChooseConcurrentItem(tieA, tieB, 'a', 'b');
  const winnerBA = ihnChooseConcurrentItem(tieB, tieA, 'b', 'a');
  assert.equal(winnerAB.x, winnerBA.x);
});

test('legacy deletion IDs normalize to Lamport zero and keep newer causal stamps', () => {
  assert.deepEqual(
    ihnNormalizeDeletionStamps(['stroke-b', 'stroke-a', 'stroke-a', '']),
    {
      'stroke-a': { clock: 0, actor: '' },
      'stroke-b': { clock: 0, actor: '' }
    }
  );

  assert.deepEqual(
    ihnNormalizeDeletionStamps(
      { 'stroke-a': { clock: 7, actor: actorA } },
      ['stroke-a', 'stroke-c']
    ),
    {
      'stroke-a': { clock: 7, actor: actorA },
      'stroke-c': { clock: 0, actor: '' }
    },
    'legacy IDs supplement causal metadata without downgrading it'
  );
});

test('deletion-stamp merge is commutative and idempotent', () => {
  const left = {
    shared: { clock: 10, actor: actorA },
    left: { clock: 4, actor: actorA }
  };
  const right = {
    shared: { clock: 10, actor: actorB },
    right: { clock: 8, actor: actorB }
  };
  const leftRight = ihnMergeDeletionStamps(left, right);
  const rightLeft = ihnMergeDeletionStamps(right, left);

  assert.equal(ihnStableStringify(leftRight), ihnStableStringify(rightLeft));
  assert.deepEqual(leftRight.shared, { clock: 10, actor: actorB });
  assert.deepEqual(leftRight.left, left.left);
  assert.deepEqual(leftRight.right, right.right);
  assert.equal(
    ihnStableStringify(ihnMergeDeletionStamps(leftRight, leftRight)),
    ihnStableStringify(leftRight)
  );
});

test('a newer causal undo survives a remote deletion while an old item stays deleted', () => {
  const deletions = {
    stroke: { clock: 20, actor: actorB },
    legacy: { clock: 0, actor: '' }
  };
  const oldItem = {
    id: 'stroke',
    syncStamp: { clock: 19, actor: actorA }
  };
  const equalItem = {
    id: 'stroke',
    syncStamp: { clock: 20, actor: actorB }
  };
  const undoneItem = {
    id: 'stroke',
    syncStamp: { clock: 21, actor: actorA }
  };

  assert.equal(ihnDeletionWinsItem(deletions, oldItem), true);
  assert.equal(ihnDeletionWinsItem(deletions, equalItem), true, 'delete wins equal stamp');
  assert.equal(
    ihnDeletionWinsItem(deletions, undoneItem),
    false,
    'a causally newer local undo keeps the recreated item'
  );
  assert.equal(
    ihnDeletionWinsItem(['legacy'], { id: 'legacy' }),
    true,
    'a legacy zero-stamp tombstone still removes an unstamped legacy item'
  );
  assert.equal(ihnDeletionWinsItem(deletions, { id: 'unrelated' }), false);
});

test('reconnect backoff is deterministic, jittered and bounded', () => {
  const values = Array.from({ length: 10 }, (_, attempt) => ihnComputeReconnectDelay(attempt, 'peer-z'));
  assert.equal(values[0], ihnComputeReconnectDelay(0, 'peer-z'));
  assert.ok(values[0] >= 800 && values[0] <= 1200);
  assert.ok(values.at(-1) >= 24_000 && values.at(-1) <= 36_000);
  assert.ok(values.every(value => value <= 36_000));
});

test('normalized metadata keeps every referenced page exactly once', () => {
  const meta = ihnNormalizeStructureMeta(null, ['a', 'a', '', 'b']);
  assert.deepEqual(ids(meta), ['a', 'b']);
});

test('field metadata merge is commutative and idempotent', () => {
  const base = ihnNormalizeFieldMeta(null);
  const left = ihnRecordFieldValue(base, 'doc:name', 'Notes from A', actorA, 100);
  const right = ihnRecordFieldValue(base, 'doc:name', 'Notes from B', actorB, 100);

  const leftRight = ihnMergeFieldMeta(left, right);
  const rightLeft = ihnMergeFieldMeta(right, left);
  assert.equal(ihnStableStringify(leftRight), ihnStableStringify(rightLeft));
  assert.equal(
    ihnResolveFieldValue(leftRight, 'doc:name', 'fallback'),
    'Notes from B',
    'actor-b wins an equal Lamport clock tie'
  );

  const again = ihnMergeFieldMeta(leftRight, leftRight);
  assert.equal(ihnStableStringify(again), ihnStableStringify(leftRight));
});

test('field and document hashes track semantic records but ignore the allocator clock', () => {
  const base = ihnRecordFieldValue(
    ihnNormalizeFieldMeta(null),
    'doc:exportName',
    'Alpha',
    actorA,
    100
  );
  const allocatorOnly = { ...base, clock: base.clock + 10_000 };
  assert.equal(
    ihnFieldMetaHash(base),
    ihnFieldMetaHash(allocatorOnly),
    'the aggregate clock is not shared document state'
  );

  const changedValue = ihnRecordFieldValue(
    base,
    'doc:exportName',
    'Beta',
    actorB,
    101
  );
  assert.notEqual(ihnFieldMetaHash(base), ihnFieldMetaHash(changedValue));

  const sameValueNewStamp = ihnRecordFieldValue(
    base,
    'doc:exportName',
    'Alpha',
    actorB,
    102
  );
  assert.notEqual(
    ihnFieldMetaHash(base),
    ihnFieldMetaHash(sameValueNewStamp),
    'causal stamps must converge even when the visible value is unchanged'
  );

  const pages = [{ pageId: 'p1', strokes: [], images: [] }];
  const structure = ihnNormalizeStructureMeta(null, ['p1']);
  assert.notEqual(
    ihnCanonicalDocumentHash(pages, structure, null, 'Alpha', base),
    ihnCanonicalDocumentHash(pages, structure, null, 'Alpha', changedValue)
  );

  const left = ihnRecordFieldValue(base, 'page:p1:template', 'blank', actorA, 200);
  const right = ihnRecordFieldValue(base, 'page:p1:size', { pageWidth: 210, pageHeight: 297 }, actorB, 200);
  assert.equal(
    ihnFieldMetaHash(ihnMergeFieldMeta(left, right)),
    ihnFieldMetaHash(ihnMergeFieldMeta(right, left)),
    'arrival order cannot change the converged hash'
  );
});

test('independent document fields survive concurrent edits', () => {
  const base = ihnNormalizeFieldMeta(null);
  const renamed = ihnRecordFieldValue(
    base,
    'doc:exportName',
    'Project Atlas',
    actorA,
    200
  );
  const calendarConfig = {
    mode: 'week',
    startDateKey: '2026-07-27',
    startPage: 2,
    nextDateKey: '2026-08-03'
  };
  const rescheduled = ihnRecordFieldValue(
    base,
    'doc:calendarPageConfig',
    calendarConfig,
    actorB,
    200
  );

  const merged = ihnMergeFieldMeta(renamed, rescheduled);
  assert.equal(
    ihnResolveFieldValue(merged, 'doc:exportName', 'Untitled'),
    'Project Atlas'
  );
  assert.deepEqual(
    ihnResolveFieldValue(merged, 'doc:calendarPageConfig', null),
    calendarConfig
  );
  assert.equal(
    ihnResolveFieldValue(merged, 'doc:missing', 'fallback'),
    'fallback',
    'an absent field uses its caller-provided fallback'
  );
});

test('production page fields preserve independent edits and atomic compound values', () => {
  const base = ihnNormalizeFieldMeta(null);
  const pdfBackground = {
    backgroundSource: 'pdf',
    backgroundImageHash: '',
    pdfPageIndex: 7,
  };
  const portraitSize = {
    pageWidth: 210,
    pageHeight: 297,
  };
  const left = ihnRecordFieldValue(
    base,
    'page:p1:background',
    pdfBackground,
    actorA,
    300
  );
  const right = ihnRecordFieldValue(
    base,
    'page:p1:size',
    portraitSize,
    actorB,
    300
  );

  const merged = ihnMergeFieldMeta(left, right);
  assert.deepEqual(
    ihnResolveFieldValue(merged, 'page:p1:background', null),
    pdfBackground,
    'source, image hash and PDF index move as one background value'
  );
  assert.deepEqual(
    ihnResolveFieldValue(merged, 'page:p1:size', null),
    portraitSize,
    'an unrelated concurrent size edit survives'
  );
});

test('side-panel removal and update obey the same LWW rule', () => {
  const base = ihnNormalizeFieldMeta(null);
  const updatedPanel = {
    title: 'This week',
    dateKeys: ['2026-07-27', '2026-07-28']
  };

  const olderUpdate = ihnRecordFieldValue(
    base,
    'page:p1:sidePanel',
    updatedPanel,
    actorA,
    400
  );
  const newerRemoval = ihnRecordFieldValue(
    base,
    'page:p1:sidePanel',
    null,
    actorB,
    401
  );
  assert.equal(
    ihnResolveFieldValue(
      ihnMergeFieldMeta(olderUpdate, newerRemoval),
      'page:p1:sidePanel',
      { title: 'fallback' }
    ),
    null,
    'null is a recorded removal, not an absent field'
  );

  const olderRemoval = ihnRecordFieldValue(
    base,
    'page:p1:sidePanel',
    null,
    actorA,
    500
  );
  const newerUpdate = ihnRecordFieldValue(
    base,
    'page:p1:sidePanel',
    updatedPanel,
    actorB,
    501
  );
  assert.deepEqual(
    ihnResolveFieldValue(
      ihnMergeFieldMeta(olderRemoval, newerUpdate),
      'page:p1:sidePanel',
      null
    ),
    updatedPanel,
    'a causally newer update can restore a removed panel'
  );

  const tiedUpdate = ihnRecordFieldValue(
    base,
    'page:p1:sidePanel',
    updatedPanel,
    actorA,
    600
  );
  const tiedRemoval = ihnRecordFieldValue(
    base,
    'page:p1:sidePanel',
    null,
    actorB,
    600
  );
  assert.equal(
    ihnResolveFieldValue(
      ihnMergeFieldMeta(tiedUpdate, tiedRemoval),
      'page:p1:sidePanel',
      updatedPanel
    ),
    null,
    'actor tie-breaking also applies to explicit removals'
  );
});
