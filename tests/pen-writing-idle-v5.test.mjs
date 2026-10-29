// Pen hot path: work that used to run between the strokes of a writing burst
// (whole-document offline copy, Drive PDF save, layout polling) now waits for
// the burst to pause. These tests run the real app-v5.js functions with a fake
// clock and fake timers.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const app = readFileSync(new URL('../app-v5.js', import.meta.url), 'utf8');
const section = (start, end) => {
  const from = app.indexOf(start);
  assert.ok(from >= 0, `missing ${start}`);
  const to = app.indexOf(end, from + start.length);
  assert.ok(to > from, `missing ${end}`);
  return app.slice(from, to);
};
const constant = name => {
  const match = app.match(new RegExp(`const ${name} = (\\d+);`));
  assert.ok(match, `missing ${name}`);
  return Number(match[1]);
};
const PEN_BURST_WINDOW_MS = constant('PEN_BURST_WINDOW_MS');
const PEN_BURST_QUIET_MS = constant('PEN_BURST_QUIET_MS');

function harness(extra = {}) {
  const timers = new Map();
  let nextTimer = 0;
  const calls = [];
  const context = vm.createContext({
    console,
    Math, Number, Array, Map, Set, Object, Promise,
    PEN_BURST_WINDOW_MS, PEN_BURST_QUIET_MS,
    penBurstLastStrokeAt: 0,
    penBurstPreviousStrokeAt: 0,
    lastPenActivityAt: 0,
    lastUserInteractionAt: 0,
    afterPenWritingTasks: new Map(),
    forceFlushingForExit: false,
    document: { hidden: false },
    clock: 10_000,
    window: { InhouseOfflineLibrary: { scheduleCapture: () => calls.push('capture') } },
    setTimeout(fn, delay) { const id = ++nextTimer; timers.set(id, { fn, delay }); return id; },
    clearTimeout(id) { timers.delete(id); },
    calls,
    ...extra
  });
  vm.runInContext('Date = { now: () => clock };', context);
  vm.runInContext([
    'let penBurstOfflineCaptureTimer = null;',
    section('        function notePenStrokeStart(', '        let penBurstOfflineCaptureTimer = null;'),
    section('        function scheduleOfflineCaptureAfterPenBurst()', '        function notePenActivity()')
  ].join('\n'), context);
  const runDueTimers = () => {
    // Fire timers in order of their delay, as long as one is pending.
    let fired = 0;
    while (timers.size) {
      const [id, timer] = [...timers].sort((a, b) => a[1].delay - b[1].delay)[0];
      timers.delete(id);
      context.clock += timer.delay;
      timer.fn();
      if (++fired > 50) throw new Error('timer loop');
    }
    return fired;
  };
  return { context, timers, calls, runDueTimers };
}

test('an isolated stroke never waits; a writing burst waits for its quiet period', () => {
  const { context } = harness();
  context.notePenStrokeStart(10_000);
  assert.equal(context.getPenBurstQuietRemaining(10_100), 0, 'first stroke is not a burst');
  context.notePenStrokeStart(10_400);
  context.lastPenActivityAt = 10_700; // last move of the second stroke
  assert.equal(context.getPenBurstQuietRemaining(10_800), PEN_BURST_QUIET_MS - 100);
  assert.equal(context.getPenBurstQuietRemaining(10_700 + PEN_BURST_QUIET_MS), 0);
  // Two strokes further apart than the burst window are not a burst.
  context.notePenStrokeStart(20_000);
  context.notePenStrokeStart(20_000 + PEN_BURST_WINDOW_MS + 1);
  assert.equal(context.getPenBurstQuietRemaining(20_000 + PEN_BURST_WINDOW_MS + 2), 0);
});

test('leaving the editor or hiding the app never waits for a burst', () => {
  const { context } = harness();
  context.notePenStrokeStart(10_000);
  context.notePenStrokeStart(10_300);
  assert.ok(context.getPenBurstQuietRemaining(10_350) > 0);
  context.forceFlushingForExit = true;
  assert.equal(context.getPenBurstQuietRemaining(10_350), 0);
  context.forceFlushingForExit = false;
  context.document.hidden = true;
  assert.equal(context.getPenBurstQuietRemaining(10_350), 0);
});

test('the offline whole-document copy is requested once, after the burst pauses', () => {
  const { context, timers, calls, runDueTimers } = harness();
  // Outside a burst the request is immediate (unchanged behaviour).
  context.scheduleOfflineCaptureAfterPenBurst();
  assert.deepEqual(calls, ['capture']);
  calls.length = 0;
  context.notePenStrokeStart(10_000);
  context.notePenStrokeStart(10_300);
  for (let stroke = 0; stroke < 6; stroke++) {
    context.clock += 150;
    context.lastPenActivityAt = context.clock;
    context.scheduleOfflineCaptureAfterPenBurst();
  }
  assert.deepEqual(calls, [], 'nothing is copied between strokes');
  assert.equal(timers.size, 1, 'one timer for the whole burst, not one per stroke');
  // More ink extends the deadline: the timer re-arms instead of capturing.
  const [[firstId, first]] = timers;
  timers.delete(firstId);
  context.clock += 100;
  first.fn();
  assert.deepEqual(calls, []);
  runDueTimers();
  assert.deepEqual(calls, ['capture']);
  assert.ok(context.clock >= context.lastPenActivityAt + PEN_BURST_QUIET_MS);
});

test('deferred view work runs once when the pen lifts', () => {
  const { context } = harness();
  let visible = 0, resolution = 0;
  context.runAfterPenWriting('visible-pages', () => visible++);
  context.runAfterPenWriting('visible-pages', () => visible++);
  context.runAfterPenWriting('canvas-resolution', () => resolution++);
  context.flushAfterPenWritingTasks();
  context.flushAfterPenWritingTasks();
  assert.equal(visible, 1);
  assert.equal(resolution, 1);
});

test('visible-page tracking does not read layout or queue thumbnails while ink is drawn', () => {
  const { context } = harness();
  let layoutReads = 0, thumbnails = 0, scheduled = [];
  Object.assign(context, {
    state: { pages: [{}, {}] },
    isPenWriting: true,
    isViewportInteracting: () => true,
    queueScrollPagePreviews: () => thumbnails++,
    getVisiblePageRange: () => { layoutReads++; return { start: 0, end: 1 }; },
    scheduleVisiblePageUpdate: force => scheduled.push(force),
    visiblePageUpdateRetryHandle: null
  });
  vm.runInContext(section('        function updateVisiblePages(', '            if (visiblePageUpdateRetryHandle) {\n                clearTimeout')
    + '\n}', context);
  for (let frame = 0; frame < 20; frame++) context.updateVisiblePages(false);
  assert.equal(thumbnails, 0);
  assert.equal(layoutReads, 0);
  assert.equal(context.visiblePageUpdateRetryHandle, null, 'no 140 ms polling timer while writing');
  context.flushAfterPenWritingTasks();
  assert.deepEqual(scheduled, [undefined], 'one reconciliation after the pen lifts');
});

test('a non-forced Drive save during a burst is deferred before the offline copy', async () => {
  const { context } = harness();
  const retries = [], queued = [];
  Object.assign(context, {
    state: { driveAutosave: true, isReadOnly: false },
    driveSaveBlockedReason: '',
    driveContentVersion: 3,
    driveUploadedContentVersion: 2,
    driveDirty: false,
    deferredDriveSave: false,
    scheduleDeferredDriveSaveRetry: delay => retries.push(delay),
    captureDocumentPersistenceContext: () => ({ sessionToken: 1 }),
    driveCheckpointQueue: { request: (key, request) => { queued.push(request.force); return Promise.resolve('saved'); } }
  });
  vm.runInContext(section('        async function queueDriveSave(', '        async function queueDriveSaveOnline('), context);
  context.notePenStrokeStart(10_000);
  context.notePenStrokeStart(10_300);
  context.clock = 10_400;
  await context.queueDriveSave(false);
  assert.deepEqual(queued, [], 'the offline-copy + save queue is not entered');
  assert.equal(context.driveDirty, true);
  assert.equal(context.deferredDriveSave, true);
  assert.equal(retries.length, 1);
  assert.equal(retries[0], PEN_BURST_QUIET_MS - 100 + 90, 'retried right after the burst pauses');
  // Forced saves (leaving, explicit save) are never held back.
  assert.equal(await context.queueDriveSave(true), 'saved');
  assert.deepEqual(queued, [true]);
  // After the quiet period the normal path runs again.
  context.clock = 10_300 + PEN_BURST_QUIET_MS;
  assert.equal(await context.queueDriveSave(false), 'saved');
  assert.deepEqual(queued, [true, false]);
});

test('an empty overlay canvas is hidden and only touched when its state changes', () => {
  const { context } = harness();
  vm.runInContext(section('        function setOverlayCanvasInk(', '        function drawLiveHighlighterAppend('), context);
  let writes = 0;
  const style = {};
  const overlay = { style: new Proxy(style, { set(target, key, value) { writes++; target[key] = value; return true; } }) };
  context.setOverlayCanvasInk(overlay, false);
  assert.equal(style.visibility, 'hidden');
  for (let frame = 0; frame < 30; frame++) context.setOverlayCanvasInk(overlay, true);
  assert.equal(style.visibility, '');
  context.setOverlayCanvasInk(overlay, false);
  assert.equal(style.visibility, 'hidden');
  assert.equal(writes, 3, 'per-frame calls do not restyle the canvas');
});
