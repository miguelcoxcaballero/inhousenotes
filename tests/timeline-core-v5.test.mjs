import assert from 'node:assert/strict';
import fs from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const source = fs.readFileSync(new URL('../timeline-core-v5.js', import.meta.url), 'utf8');
const context = vm.createContext({ console });
vm.runInContext(source, context, { filename: 'timeline-core-v5.js' });
const core = context.InhouseTimelineCore;

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

function makePage(pageId, marker = 0) {
  return {
    pageId,
    strokes: [{ id: `${pageId}-stroke-${marker}`, tool: 'pen', points: [{ x: marker, y: marker + 1 }] }],
    images: [],
    deletedStrokeIds: [],
    deletedStrokeStamps: {},
    backgroundSource: 'template',
    templateKind: 'default',
    pageWidth: 210,
    pageHeight: 297,
    sidePanel: null,
    legacyCoverStrokes: null
  };
}

function makeHistory() {
  let pages = [makePage('page-a'), makePage('page-b'), makePage('page-c')];
  const history = [];
  for (let index = 0; index < 20; index += 1) {
    pages = clone(pages);
    pages[0].strokes.push({
      id: `live-${index}`,
      tool: 'pen',
      width: 2,
      color: '#123456',
      points: [{ x: index + 10, y: index + 20 }]
    });
    if (index === 4) pages = [pages[2], pages[0], pages[1]];
    if (index === 7) pages.splice(1, 0, makePage('page-added', index));
    if (index === 11) pages = pages.filter(page => page.pageId !== 'page-b');
    history.push({
      id: `version-${index}`,
      originId: `version-${index}`,
      ts: 1000 + index,
      author: { email: 'editor@example.com', name: 'Editor', photo: '' },
      summary: `Version ${index}`,
      kind: index === 6 ? 'manual' : 'autosave',
      isMilestone: index === 6,
      schemaVersion: 3,
      parentId: index ? `version-${index - 1}` : null,
      contentHash: `content-${index}`,
      deviceId: 'test-device',
      calendarPageConfig: index >= 14 ? { start: '2026-08-01', mode: 'week' } : null,
      exportName: index >= 16 ? 'Renamed document' : 'Document',
      pages
    });
  }
  return history;
}

test('incremental archive round-trips edits, page reorders, additions and deletions', () => {
  const history = makeHistory();
  const archive = core.createArchive(history, { checkpointInterval: 8 });
  const restored = core.materializeArchive(archive);
  assert.deepEqual(JSON.parse(JSON.stringify(restored)), history);
  assert.equal(archive.schemaVersion, 3);
  assert.equal(archive.entries[0].type, 'checkpoint');
  assert.equal(archive.entries[6].type, 'checkpoint', 'milestones are recovery checkpoints');
  assert.equal(archive.entries[8].type, 'checkpoint', 'periodic checkpoints bound replay work');
  assert.equal(archive.entries[9].type, 'delta');
});

test('incremental archive is materially smaller when only one page changes', () => {
  const history = makeHistory();
  const fullBytes = JSON.stringify(history).length;
  const archiveBytes = JSON.stringify(core.createArchive(history)).length;
  assert.ok(archiveBytes < fullBytes * 0.7, `expected ${archiveBytes} to be less than 70% of ${fullBytes}`);
});

test('legacy full-snapshot arrays remain readable without mutation', () => {
  const legacy = makeHistory().slice(0, 2);
  const restored = core.materializeArchive(legacy);
  assert.deepEqual(JSON.parse(JSON.stringify(restored)), legacy);
  restored[0].pages[0].strokes.length = 0;
  assert.notEqual(restored[0].pages[0].strokes.length, legacy[0].pages[0].strokes.length);
});

test('broken or tampered delta chains fail closed', () => {
  const archive = core.createArchive(makeHistory());
  const deltaIndex = archive.entries.findIndex(entry => entry.type === 'delta');
  archive.entries[deltaIndex].delta.upserts[0][1].pageWidth = 999;
  assert.throws(() => core.materializeArchive(archive), /integrity check failed/);

  const broken = core.createArchive(makeHistory());
  const brokenIndex = broken.entries.findIndex(entry => entry.type === 'delta');
  broken.entries[brokenIndex].baseHash = 'wrong-base';
  assert.throws(() => core.materializeArchive(broken), /chain is broken/);
});

test('archive bounds reject resource-exhaustion payloads', () => {
  const entry = makeHistory()[0];
  assert.throws(
    () => core.createArchive(Array.from({ length: 201 }, (_value, index) => ({ ...entry, id: `v-${index}` }))),
    /too many entries/
  );
  assert.equal(core.materializeArchive({ schema: 'other', schemaVersion: 3, entries: [] }), null);
});
