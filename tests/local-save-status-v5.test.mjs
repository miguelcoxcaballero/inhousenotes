import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
import { createRequire } from 'node:module';
const runtime = createRequire(import.meta.url)('../runtime-core-v5.js');

const app = readFileSync(new URL('../app-v5.js', import.meta.url), 'utf8');
const section = (start, end) => app.slice(app.indexOf(start), app.indexOf(end, app.indexOf(start)));
const source = [
  section('        function getLocalSaveStatus()', '        function isDriveSyncPending()'),
  section('        function hasPendingLocalSave()', '        function hasPendingDriveSave()'),
  section('        const localCheckpointQueue =', '        function flushOnClose()'),
  section('        async function runStrokeOpsQueueFlush(', '        async function flushStrokeOpsQueue(')
].join('\n');

function harness() {
  const timers = new Map();
  let nextTimer = 0;
  const context = vm.createContext({
    console: { error() {} },
    window: { InhouseRuntimeCore: runtime },
    state: { isReadOnly: false, lastSavedAt: null },
    session: 1,
    saveInProgress: false,
    saveQueued: false,
    activeLocalSaveController: null,
    localSaveError: '',
    localSaveErrorSource: '',
    lastLocalSaveAt: null,
    localSaveRetryTimer: null,
    localSaveRetryAttempts: 0,
    strokeOpsFlushInProgress: false,
    strokeOpsFlushScheduled: false,
    strokeOpsQueue: [],
    pageDirty: new Set(),
    collabPageOrderMutationInProgress: false,
    collabPageStoreRewritePending: false,
    STROKE_OP_FLUSH_BATCH_SIZE: 12,
    hasPendingLegacyPageMigration: () => false,
    hasSmoothInteraction: () => false,
    isDriveSyncPending: () => false,
    recordSaveDebug() {},
    showPendingSaveStatus() {},
    showStatus() {},
    reconcileSavedIndicator() {},
    updateSaveTimestamp() {},
    refreshSaveIndicatorState() {},
    setTimeout(fn, delay) {
      const id = ++nextTimer;
      timers.set(id, { fn, delay });
      return id;
    },
    clearTimeout(id) { timers.delete(id); },
    saveToStorage: async () => ({ ok: true, savedAt: 100 }),
    appendStrokeOpsToIndexedDb: async () => true,
    yieldToMainThread: async () => {},
    flushStrokeOpsQueue() {}
  });
  context.captureDocumentPersistenceContext = () => ({ session: context.session, sessionToken: context.session });
  context.isDocumentPersistenceContextCurrent = captured => captured.session === context.session;
  context.getDocumentSessionToken = () => context.session;
  context.isDocumentSessionTokenValid = token => token === context.session;
  context.markPageDirty = index => context.pageDirty.add(index);
  vm.runInContext(source, context);
  return { context, timers };
}

test('Drive success cannot clear a local checkpoint failure or replace its timestamp', async () => {
  const { context, timers } = harness();
  context.confirmLocalSave(100);
  context.saveToStorage = async () => ({ ok: false, savedAt: null });
  await context.queueSave();
  context.state.lastSavedAt = 5000; // The existing Drive status path writes this field.
  assert.ok(context.getLocalSaveStatus().error);
  assert.equal(context.getLocalSaveStatus().savedAt, 100);

  context.confirmLocalSave(200, 'operations');
  assert.ok(context.getLocalSaveStatus().error, 'a stroke append cannot repair failed metadata');
  context.saveToStorage = async () => ({ ok: true, savedAt: 300 });
  await context.queueSave();
  assert.equal(context.getLocalSaveStatus().error, '');
  assert.equal(context.getLocalSaveStatus().savedAt, 300);
  assert.equal(timers.size, 0);
});

test('an operation removed from the queue stays pending until IndexedDB commits', async () => {
  const { context } = harness();
  let commit;
  context.appendStrokeOpsToIndexedDb = () => new Promise(resolve => { commit = resolve; });
  context.strokeOpsQueue.push({ pageIndex: 0, savedAt: 200 });
  const flushing = context.runStrokeOpsQueueFlush();
  assert.equal(context.strokeOpsQueue.length, 0);
  assert.equal(context.getLocalSaveStatus().pending, true);
  assert.equal(context.getLocalSaveStatus().savedAt, null);
  commit(true);
  await flushing;
  assert.equal(context.getLocalSaveStatus().pending, false);
  assert.equal(context.getLocalSaveStatus().savedAt, 200);
});

test('a durable operation retry clears its own error without requiring compaction', async () => {
  const { context } = harness();
  context.noteLocalSaveFailure('operations');
  context.strokeOpsQueue.push({ pageIndex: 0, savedAt: 200 });
  await context.runStrokeOpsQueueFlush();
  assert.equal(context.getLocalSaveStatus().error, '');
  assert.equal(context.getLocalSaveStatus().savedAt, 200);
});

test('local retries back off and stop after three automatic attempts', async () => {
  const { context, timers } = harness();
  let attempts = 0;
  context.saveToStorage = async () => { attempts += 1; return { ok: false }; };
  await context.queueSave();
  const delays = [];
  while (timers.size) {
    const [id, timer] = timers.entries().next().value;
    timers.delete(id);
    delays.push(timer.delay);
    timer.fn();
    for (let turn = 0; turn < 8; turn += 1) await Promise.resolve();
    assert.ok(attempts <= 4);
  }
  assert.deepEqual(delays, [800, 1600, 3200]);
  assert.equal(attempts, 4);
  assert.ok(context.getLocalSaveStatus().error);
});

test('read mode still saves edits made before the mode changed', async () => {
  const { context } = harness();
  context.state.isReadOnly = true;
  context.pageDirty.add(0);
  let attempts = 0;
  context.saveToStorage = async () => {
    attempts += 1;
    context.pageDirty.clear();
    return { ok: true, savedAt: 400 };
  };
  await context.queueSave();
  assert.equal(attempts, 1);
  assert.equal(context.getLocalSaveStatus().pending, false);
  assert.equal(context.getLocalSaveStatus().savedAt, 400);
});

test('a late checkpoint and its scheduled retry cannot affect a different document', async () => {
  const { context, timers } = harness();
  let complete;
  context.saveToStorage = () => new Promise(resolve => { complete = resolve; });
  const saving = context.queueSave();
  context.session += 1;
  complete({ ok: false });
  await saving;
  assert.equal(context.getLocalSaveStatus().error, '');
  assert.equal(timers.size, 0);

  let attempts = 0;
  context.saveToStorage = async () => { attempts += 1; return { ok: false }; };
  await context.queueSave();
  context.session += 1;
  timers.values().next().value.fn();
  await Promise.resolve();
  assert.equal(attempts, 1);
});


test('a new document save queued behind an old attempt still completes', async () => {
  const { context, timers } = harness();
  let completeOld;
  let attempts = 0;
  context.saveToStorage = () => {
    attempts += 1;
    if (attempts === 1) return new Promise(resolve => { completeOld = resolve; });
    context.pageDirty.clear();
    return Promise.resolve({ ok: true, savedAt: 900 });
  };
  const oldSave = context.queueSave();
  context.session += 1;
  context.saveQueued = false; // beginDocumentSession drops the previous queue.
  context.state.isReadOnly = true;
  context.pageDirty.add(0);
  const newSave = context.queueSave();
  assert.equal(context.saveQueued, true);
  completeOld({ ok: false });
  await oldSave;
  await newSave;
  assert.equal(attempts, 2);
  assert.equal(context.getLocalSaveStatus().savedAt, 900);
  assert.equal(context.getLocalSaveStatus().error, '');
  assert.equal(context.getLocalSaveStatus().pending, false);
  assert.equal(timers.size, 0);
});
