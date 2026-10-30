// Adaptive Drive revision polling and presence heartbeat cadence.
//
// The editor used to GET the document metadata every 0.6 s and PATCH a
// presence heartbeat every 2 s for as long as a document was open, even when
// no other device could see or change it. These tests run the real app-v5.js
// scheduling code on a virtual clock and prove the idle cadence, the speed-up
// when another device appears, the online window that keeps slow heartbeats
// visible, and that every timer stops when collaboration stops.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const app = readFileSync(new URL('../app-v5.js', import.meta.url), 'utf8');
const section = (start, end) => {
  const from = app.indexOf(start);
  assert.notEqual(from, -1, `missing section start: ${start}`);
  const to = app.indexOf(end, from + start.length);
  assert.notEqual(to, -1, `missing section end: ${end}`);
  return app.slice(from, to);
};

const source = [
  section('        const COLLAB_POLL_INTERVAL =', '        function clearCollabPullRetry('),
  section('        function clearCollabFastCatchup()', '        async function checkRemoteUpdateAfterSave()'),
  section('        function startCollabPolling()', '        async function pollDriveRevision()'),
  section('        let _presencePingInterval = null;', '        function detectDeviceType()'),
  section('        function packPresenceRuntime(', '        function getPresenceKey()'),
  section('        // Heartbeat cadence for the current collaboration mode.', '        function updatePresenceFromProps('),
  `globalThis.__probe = {
     setCanHeartbeat(value) { _presenceCanHeartbeat = value; },
     get advertised() { return presenceAdvertisedHeartbeatMs; },
     get pollDueAt() { return collabPollDueAt; },
     get heartbeatTimer() { return _presenceHeartbeatInterval; },
     get pingTimer() { return _presencePingInterval; },
     getCollabPollDelay, getPresenceHeartbeatDelay, getCollabPresenceMode,
     noteCollabPresenceModeChange, noteRemoteRevisionActivity,
     scheduleCollabFastCatchup, startCollabPolling, stopCollabPolling,
     pauseCollabPollingForBackground, setPresenceSavePhase,
     packPresenceRuntime, parsePackedPresence, isPresenceCurrentlyOnline,
     getPresenceOnlineWindowMs, notePresencePreviewFramePublished, sendPresenceDepartureBeat
   };`
].join('\n');

function harness() {
  let now = 1_700_000_000_000;
  let nextId = 0;
  const timers = new Map();
  const requests = [];
  const RealDate = Date;
  class FakeDate extends RealDate {
    constructor(...args) { super(...(args.length ? args : [now])); }
    static now() { return now; }
  }
  const schedule = (fn, delay, repeat) => {
    const id = ++nextId;
    timers.set(id, { fn, at: now + Math.max(0, Number(delay) || 0), repeat: repeat ? Math.max(1, delay) : 0 });
    return id;
  };
  const context = {
    console: { log() {}, warn() {}, info() {} },
    Date: FakeDate,
    Math, Number, String, Object, Array, Map, Set, Promise, JSON, Error,
    setTimeout: (fn, delay) => schedule(fn, delay, false),
    clearTimeout: id => { timers.delete(id); },
    setInterval: (fn, delay) => schedule(fn, delay, true),
    clearInterval: id => { timers.delete(id); },
    window: {},
    document: { visibilityState: 'visible' },
    // Merged lifecycle guard (v5.17.22): the editor is open in these scenarios.
    isDocumentEditorActive: () => true,
    state: { driveFileId: 'doc-1', collabEnabled: false, collabPollInterval: null },
    driveAccessToken: 'token',
    driveUserProfile: { email: 'me@example.test', name: 'Me' },
    driveSaveInProgress: false,
    driveSaveQueued: false,
    driveDirty: false,
    deferredDriveSave: false,
    presenceSavePhase: '',
    presenceSavePhaseAt: 0,
    remotePresenceSyncActive: false,
    remotePresenceSyncDoneUntil: 0,
    remotePresenceSyncTimer: null,
    remotePresenceSyncSourceKeys: new Set(),
    PRESENCE_ACTIVE_PHASES: new Set(['processing', 'uploading']),
    latestPresenceUsers: [],
    liveOverview: { openPeerCount: 0, connectingPeerCount: 0 },
    directPeers: new Set(),
    collabPullInProgress: false,
    pollCount: 0,
    pollDriveRevision: async () => {
      context.pollCount += 1;
      requests.push({ at: now, method: 'GET', kind: 'poll' });
    },
    pingPresence: async () => {
      requests.push({ at: now, method: 'PATCH', kind: 'ping' });
      context.__probe.setCanHeartbeat(true);
    },
    driveFetch: async (url, options = {}) => {
      requests.push({ at: now, method: options.method || 'GET', kind: 'fetch', url, body: options.body, keepalive: !!options.keepalive });
      // Optional virtual network latency, to hold a heartbeat in flight.
      if (context.fetchDelayMs) await new Promise(resolve => schedule(resolve, context.fetchDelayMs, false));
      // Optional Drive rejection (driveFetch throws on any non-ok status).
      if (context.fetchStatus) throw Object.assign(new Error(`Drive API error ${context.fetchStatus}`), { status: context.fetchStatus });
      return { ok: true, json: async () => ({}) };
    },
    loadDriveProfile: async () => {},
    updateDriveProfileUI() {},
    startLiveCollaboration() {},
    stopLiveCollaboration() {},
    stopPublicSharedPolling() {},
    clearCollabPullRetry() {},
    getLiveCollaborationOverview: () => context.liveOverview,
    getPresenceConnectionInfo: user => ({ directConnected: context.directPeers.has(user?.c) }),
    isOwnPresenceRecord: user => user?.c === 'me',
    hasRecentPresenceSaveSignal: () => false,
    normalizePresenceSavePhase: phase => (['processing', 'uploading', 'saved'].includes(phase) ? phase : ''),
    getPresenceKey: () => 'p_me',
    getPresenceFieldKey: (prefix, suffix) => `${prefix}_${suffix}`
  };
  vm.createContext(context);
  vm.runInContext(source, context, { filename: 'app-v5.js#collab-cadence' });

  async function advance(ms) {
    const end = now + ms;
    for (;;) {
      let nextTimer = null;
      for (const [id, timer] of timers) {
        if (timer.at <= end && (!nextTimer || timer.at < nextTimer[1].at)) nextTimer = [id, timer];
      }
      if (!nextTimer) break;
      const [id, timer] = nextTimer;
      now = Math.max(now, timer.at);
      if (timer.repeat) timer.at = now + timer.repeat;
      else timers.delete(id);
      timer.fn();
      // Let the async poll / heartbeat chains settle before the next timer.
      for (let i = 0; i < 20; i++) await Promise.resolve();
    }
    now = end;
  }

  // Counts use a request index mark, so a timer that fired exactly at the
  // boundary before the mark is never attributed to the measured window.
  const mark = () => requests.length;
  const count = (predicate, from = 0) => requests.slice(from).filter(predicate).length;
  const heartbeats = from => count(r => r.kind === 'fetch' && r.method === 'PATCH', from);
  const polls = from => count(r => r.kind === 'poll', from);
  return { context, probe: context.__probe, advance, requests, heartbeats, polls, mark, timers, now: () => now };
}

const MINUTE = 60_000;

test('alone and idle: metadata polls and heartbeats every 20 s', async () => {
  const h = harness();
  h.probe.startCollabPolling();
  await h.advance(5_000); // first ping + startup beat
  const since = h.mark();
  await h.advance(10 * MINUTE);
  const pollsPerMinute = h.polls(since) / 10;
  const beatsPerMinute = h.heartbeats(since) / 10;
  // Was 100 polls/min (0.6 s) and 30 heartbeats/min (2 s).
  assert.ok(pollsPerMinute >= 2.9 && pollsPerMinute <= 3.1, `polls/min ${pollsPerMinute}`);
  assert.ok(beatsPerMinute >= 2.9 && beatsPerMinute <= 3.1, `heartbeats/min ${beatsPerMinute}`);
  assert.equal(h.probe.advertised, 20_000, 'solo beats advertise their 20 s interval');
  const lastBeat = h.requests.filter(r => r.kind === 'fetch' && r.method === 'PATCH').at(-1);
  assert.equal(JSON.parse(lastBeat.body).properties.p_me_r.split('|')[3], (20_000).toString(36), 'runtime field carries the interval in base 36');
});

test('another online device switches to the fast cadence immediately', async () => {
  const h = harness();
  h.probe.startCollabPolling();
  await h.advance(MINUTE);
  const beatsBefore = h.heartbeats(0);
  h.context.latestPresenceUsers = [{ c: 'tablet', isOnline: true }];
  h.probe.noteCollabPresenceModeChange();
  await h.advance(10);
  assert.equal(h.heartbeats(0), beatsBefore + 1, 'beats at once so the newcomer sees this device online');
  assert.equal(h.probe.advertised, 2_000);
  const since = h.mark();
  await h.advance(MINUTE);
  assert.ok(h.polls(since) >= 95, `shared polls/min ${h.polls(since)}`);
  assert.ok(h.heartbeats(since) >= 29, `shared heartbeats/min ${h.heartbeats(since)}`);

  // Linked peer-to-peer: Drive is only a safety net, commit hints poll directly.
  h.context.directPeers.add('tablet');
  h.context.liveOverview = { openPeerCount: 1, connectingPeerCount: 0 };
  h.probe.noteCollabPresenceModeChange();
  await h.advance(5_000);
  const linkedSince = h.mark();
  await h.advance(MINUTE);
  assert.ok(h.polls(linkedSince) >= 19 && h.polls(linkedSince) <= 21, `linked polls/min ${h.polls(linkedSince)}`);
  assert.ok(h.heartbeats(linkedSince) >= 9 && h.heartbeats(linkedSince) <= 11, `linked heartbeats/min ${h.heartbeats(linkedSince)}`);

  // The other device leaves: back to the idle cadence.
  h.context.directPeers.clear();
  h.context.liveOverview = { openPeerCount: 0, connectingPeerCount: 0 };
  h.context.latestPresenceUsers = [{ c: 'tablet', isOnline: false }];
  h.probe.noteCollabPresenceModeChange();
  await h.advance(35_000);
  const soloSince = h.mark();
  await h.advance(5 * MINUTE);
  assert.ok(h.polls(soloSince) <= 16, `solo polls ${h.polls(soloSince)}`);
  assert.ok(h.heartbeats(soloSince) <= 16, `solo heartbeats ${h.heartbeats(soloSince)}`);
});

test('a P2P peer that connects before its presence is polled counts as present', async () => {
  const h = harness();
  h.probe.startCollabPolling();
  await h.advance(MINUTE);
  h.context.liveOverview = { openPeerCount: 0, connectingPeerCount: 1 };
  assert.equal(h.probe.getCollabPresenceMode(), 'shared');
  h.context.liveOverview = { openPeerCount: 1, connectingPeerCount: 0 };
  assert.equal(h.probe.getCollabPresenceMode(), 'linked');
  assert.equal(h.probe.getCollabPollDelay(), 3_000);
});

test('a remote revision keeps polling fast for a minute, then relaxes', async () => {
  const h = harness();
  h.probe.startCollabPolling();
  await h.advance(MINUTE);
  h.probe.noteRemoteRevisionActivity();
  const since = h.mark();
  await h.advance(30_000);
  assert.ok(h.polls(since) >= 45, `polls after remote change ${h.polls(since)}`);
  await h.advance(31_000);
  const relaxedSince = h.mark();
  await h.advance(2 * MINUTE);
  assert.ok(h.polls(relaxedSince) <= 7, `relaxed polls ${h.polls(relaxedSince)}`);
});

test('a save confirms once when alone and keeps the burst when shared', async () => {
  const h = harness();
  h.probe.startCollabPolling();
  // Regular solo polls land at 0, 20, 40, 60 s...: measure from 65 s so
  // only the catch-up reads fall inside the 4 s window.
  await h.advance(MINUTE + 5_000);
  const since = h.mark();
  h.probe.scheduleCollabFastCatchup();
  await h.advance(4_000);
  assert.equal(h.polls(since), 1, 'solo save: one confirmation read');

  h.context.latestPresenceUsers = [{ c: 'tablet', isOnline: true }];
  assert.equal(h.probe.getCollabPresenceMode(), 'shared');
  h.probe.stopCollabPolling();
  h.context.state.collabEnabled = false;
  const sharedSince = h.mark();
  h.probe.scheduleCollabFastCatchup();
  await h.advance(4_000);
  assert.equal(h.polls(sharedSince), 3, 'shared save: 0.9 / 1.8 / 3.5 s burst');
});

test('a heartbeat skipped during a save retries within 4 s instead of a full interval', async () => {
  const h = harness();
  h.probe.startCollabPolling();
  await h.advance(MINUTE);
  const last = h.requests.filter(r => r.kind === 'fetch' && r.method === 'PATCH').at(-1).at;
  h.context.driveSaveInProgress = true;
  await h.advance(last + 20_000 - h.now() + 10); // the due beat is skipped
  h.context.driveSaveInProgress = false;
  const skippedAt = h.now();
  await h.advance(4_100);
  const beats = h.requests.filter(r => r.kind === 'fetch' && r.method === 'PATCH' && r.at > skippedAt);
  assert.equal(beats.length, 1, 'retry beat sent');
  assert.ok(beats[0].at - last <= 24_100, 'gap stays inside the 27.2 s solo online window');
});

test('alone, save phase rings are not pushed to Drive; shared they are', async () => {
  const h = harness();
  h.probe.startCollabPolling();
  await h.advance(MINUTE);
  const before = h.heartbeats(0);
  await h.probe.setPresenceSavePhase('processing');
  await h.probe.setPresenceSavePhase('saved');
  assert.equal(h.heartbeats(0), before);
  h.context.latestPresenceUsers = [{ c: 'tablet', isOnline: true }];
  await h.probe.setPresenceSavePhase('uploading');
  assert.equal(h.requests.filter(r => r.kind === 'fetch' && r.method === 'PATCH').length, before + 1);
});

test('stopping or pausing collaboration stops every poll and heartbeat', async () => {
  for (const stop of ['stopCollabPolling', 'pauseCollabPollingForBackground']) {
    const h = harness();
    h.probe.startCollabPolling();
    h.context.latestPresenceUsers = [{ c: 'tablet', isOnline: true }];
    h.probe.noteCollabPresenceModeChange();
    await h.advance(10_000);
    h.probe[stop]();
    const since = h.mark();
    await h.advance(10 * MINUTE);
    assert.equal(h.polls(since), 0, `${stop}: no polls`);
    assert.deepEqual(h.requests.slice(since).map(r => [r.kind, r.method]), [], `${stop}: no requests at all`);
    assert.equal(h.timers.size, 0, `${stop}: no timers left`);
    assert.equal(h.context.state.collabEnabled, false);
  }
});

test('online window follows the advertised interval and stays compatible', () => {
  const h = harness();
  const { parsePackedPresence, isPresenceCurrentlyOnline, packPresenceRuntime } = h.probe;
  const t = 1_700_000_000_000;
  const identity = 'acct|tablet|w@example.test|Writer|t|Tablet';

  // Legacy / fast record: no fourth slot, original 2 s x 6 + 1.2 s window.
  const legacy = parsePackedPresence({ q: identity, r: `${t.toString(36)}||` });
  assert.equal(legacy.i, 2_000);
  assert.equal(isPresenceCurrentlyOnline(legacy, t + 13_200), true);
  assert.equal(isPresenceCurrentlyOnline(legacy, t + 13_300), false);
  // A fast beat never writes the extra slot, so it is byte-identical to before.
  assert.equal(packPresenceRuntime(t, 2_000), `${t.toString(36)}||`);

  // Solo record: 20 s interval plus a 7.2 s grace (27.2 s).
  const slow = parsePackedPresence({ q: identity, r: packPresenceRuntime(t, 20_000) });
  assert.equal(slow.i, 20_000);
  assert.equal(slow.h, t);
  assert.equal(isPresenceCurrentlyOnline(slow, t + 27_200), true);
  assert.equal(isPresenceCurrentlyOnline(slow, t + 27_300), false);

  // Linked record: 6 s interval keeps the original 13.2 s window.
  const linked = parsePackedPresence({ q: identity, r: packPresenceRuntime(t, 6_000) });
  assert.equal(isPresenceCurrentlyOnline(linked, t + 13_200), true);
  assert.equal(isPresenceCurrentlyOnline(linked, t + 13_300), false);

  // A forged huge interval is capped at 30 s: no record, whatever it
  // advertises, looks online for more than 37.2 s after its last beat.
  const forged = parsePackedPresence({ q: identity, r: `${t.toString(36)}|||${(10 * 3_600_000).toString(36)}` });
  assert.equal(forged.i, 30_000);
  assert.equal(isPresenceCurrentlyOnline(forged, t + 37_200), true);
  assert.equal(isPresenceCurrentlyOnline(forged, t + 37_300), false);
});

// ---------------------------------------------------------------------------
// Review fixes: read-only viewers and departed solo devices.

const IDENTITY = 'acct|tablet|w@example.test|Writer|t|Tablet';
const beatRequests = (h, from = 0) => h.requests.slice(from).filter(r => r.kind === 'fetch' && r.method === 'PATCH');
const beatTime = request => parseInt(JSON.parse(request.body).properties.p_me_r.split('|')[0], 36);

test('the first live-preview frame after a quiet spell is followed by a heartbeat at once, alone or linked', async () => {
  // Read-only viewers cannot write presence and have no P2P link, so this
  // device stays 'solo' for them. Their preview reader sleeps after a 20 s
  // pause and wakes only on a new heartbeat from this device.
  const h = harness();
  h.probe.startCollabPolling();
  await h.advance(MINUTE + 3_000); // between two 20 s solo beats
  assert.equal(h.probe.getCollabPresenceMode(), 'solo');
  let since = h.mark();
  const frameAt = h.now();
  h.probe.notePresencePreviewFramePublished('doc-1');
  await h.advance(20);
  let wake = beatRequests(h, since);
  assert.equal(wake.length, 1, 'wake beat sent at once instead of up to 20 s later');
  assert.ok(beatTime(wake[0]) >= frameAt, 'the heartbeat is newer than the frame it announces');

  // Frames inside a writing burst (pauses under 10 s) add no requests.
  since = h.mark();
  for (let i = 0; i < 60; i++) {
    h.probe.notePresencePreviewFramePublished('doc-1');
    await h.advance(100);
  }
  await h.advance(9_000);
  h.probe.notePresencePreviewFramePublished('doc-1');
  await h.advance(20);
  assert.equal(beatRequests(h, since).length, 0, 'no extra beats inside a burst');

  // Another document's frame never beats for this one.
  since = h.mark();
  await h.advance(10_000); // quiet again (the 20 s solo beat may land here)
  since = h.mark();
  h.probe.notePresencePreviewFramePublished('other-doc');
  await h.advance(20);
  assert.equal(beatRequests(h, since).length, 0, 'frame of another document');

  // Linked: the 6 s cadence would also be too slow for a sleeping viewer.
  h.context.directPeers.add('tablet');
  h.context.liveOverview = { openPeerCount: 1, connectingPeerCount: 0 };
  h.context.latestPresenceUsers = [{ c: 'tablet', isOnline: true }];
  h.probe.noteCollabPresenceModeChange();
  await h.advance(15_000);
  assert.equal(h.probe.getCollabPresenceMode(), 'linked');
  await h.advance(3_000); // mid-way between two 6 s beats
  since = h.mark();
  h.probe.notePresencePreviewFramePublished('doc-1');
  await h.advance(20);
  wake = beatRequests(h, since);
  assert.equal(wake.length, 1, 'linked: wake beat sent at once');
});

test('a wake beat blocked by a save or by a beat already in flight follows within 0.5 s', async () => {
  const h = harness();
  h.probe.startCollabPolling();
  await h.advance(MINUTE + 3_000);

  // A save in progress skips heartbeats; the wake retries every 0.5 s.
  h.context.driveSaveInProgress = true;
  let since = h.mark();
  let frameAt = h.now();
  h.probe.notePresencePreviewFramePublished('doc-1');
  await h.advance(2_000);
  assert.equal(beatRequests(h, since).length, 0, 'no metadata PATCH during the save');
  h.context.driveSaveInProgress = false;
  const savedAt = h.now();
  await h.advance(600);
  let beats = beatRequests(h, since);
  assert.equal(beats.length, 1, 'wake beat after the save');
  assert.ok(beats[0].at - savedAt <= 500 && beatTime(beats[0]) >= frameAt);

  // A regular beat that started before the frame cannot announce it: a second
  // beat follows as soon as the first completes.
  await h.advance(15_000); // quiet spell
  h.context.fetchDelayMs = 1_000;
  const regularDue = beatRequests(h).at(-1).at + 20_000;
  await h.advance(regularDue - h.now() + 100); // regular beat now in flight
  since = h.mark();
  frameAt = h.now();
  h.probe.notePresencePreviewFramePublished('doc-1');
  await h.advance(2_000);
  beats = beatRequests(h, since);
  assert.equal(beats.length, 1, 'one more beat after the in-flight one');
  assert.ok(beatTime(beats[0]) >= frameAt, 'and it is newer than the frame');
  assert.ok(beats[0].at - frameAt <= 1_500, `wake beat ${beats[0].at - frameAt} ms after the frame`);
});

test('a wake beat that Drive rejects keeps the ordinary 4 s retry instead of 0.5 s', async () => {
  const h = harness();
  h.probe.startCollabPolling();
  await h.advance(MINUTE + 3_000);
  // Rate limited (or any failure): only a skipped beat may retry every 0.5 s.
  h.context.fetchStatus = 429;
  let since = h.mark();
  h.probe.notePresencePreviewFramePublished('doc-1');
  await h.advance(10_000);
  const attempts = beatRequests(h, since);
  assert.ok(attempts.length >= 2 && attempts.length <= 4, `attempts in 10 s ${attempts.length} (0.5 s retries would be ~20)`);
  for (let i = 1; i < attempts.length; i++) {
    assert.ok(attempts[i].at - attempts[i - 1].at >= 4_000, `retry gap ${attempts[i].at - attempts[i - 1].at} ms`);
  }
  // Once Drive accepts again the cadence returns to the solo beat.
  h.context.fetchStatus = 0;
  await h.advance(5_000);
  since = h.mark();
  await h.advance(MINUTE);
  assert.ok(h.heartbeats(since) <= 4, `solo beats after recovery ${h.heartbeats(since)}`);
});

test('going to the background while alone rewrites the heartbeat with the 2 s window', async () => {
  const h = harness();
  const { parsePackedPresence, getPresenceOnlineWindowMs, isPresenceCurrentlyOnline } = h.probe;
  h.probe.startCollabPolling();
  await h.advance(MINUTE + 5_000);
  assert.equal(h.probe.advertised, 20_000);
  const since = h.mark();
  const hiddenAt = h.now();
  h.probe.pauseCollabPollingForBackground();
  await h.advance(10);
  const departure = beatRequests(h, since);
  assert.equal(departure.length, 1, 'one departure beat');
  assert.equal(departure[0].keepalive, true, 'survives the page being closed');
  const runtime = JSON.parse(departure[0].body).properties.p_me_r;
  assert.equal(runtime.split('|').length, 3, 'no interval slot: byte-compatible 2 s record');
  const record = parsePackedPresence({ q: IDENTITY, r: runtime });
  assert.equal(getPresenceOnlineWindowMs(record), 13_200);
  assert.equal(isPresenceCurrentlyOnline(record, hiddenAt + 13_200), true);
  assert.equal(isPresenceCurrentlyOnline(record, hiddenAt + 13_300), false, 'offline 13.2 s after leaving, as before');
  await h.advance(10 * MINUTE);
  assert.equal(h.requests.length, since + 1, 'nothing else while in the background');
  assert.equal(h.timers.size, 0);

  // With fast beats the window is already 13.2 s: no extra request.
  const shared = harness();
  shared.probe.startCollabPolling();
  shared.context.latestPresenceUsers = [{ c: 'tablet', isOnline: true }];
  shared.probe.noteCollabPresenceModeChange();
  await shared.advance(10_000);
  assert.equal(shared.probe.advertised, 2_000);
  const sharedSince = shared.mark();
  shared.probe.pauseCollabPollingForBackground();
  await shared.advance(MINUTE);
  assert.equal(shared.requests.length, sharedSince, 'shared: no departure request');

  // A beat old enough to expire within 13.2 s anyway needs no departure.
  const late = harness();
  late.probe.startCollabPolling();
  await late.advance(MINUTE + 17_000); // last solo beat 15 s ago (beats at 2, 22, 42, 62 s)
  const lateSince = late.mark();
  late.probe.pauseCollabPollingForBackground();
  await late.advance(MINUTE);
  assert.equal(late.requests.length, lateSince, 'old beat: no departure request');

  // Never during a save upload; the 27.2 s solo window still bounds it.
  const saving = harness();
  saving.probe.startCollabPolling();
  await saving.advance(MINUTE + 5_000);
  saving.context.driveSaveInProgress = true;
  const savingSince = saving.mark();
  saving.probe.pauseCollabPollingForBackground();
  await saving.advance(MINUTE);
  assert.equal(saving.requests.length, savingSince, 'saving: no metadata PATCH');
});

test('a solo device that vanished stops counting as online, and stops the newcomer fast cadence, 27.2 s after its last beat', async () => {
  const h = harness();
  const { parsePackedPresence, packPresenceRuntime, isPresenceCurrentlyOnline } = h.probe;
  const t = h.now();
  // Its process was killed 2 s after its last solo beat: no departure beat.
  const ghost = parsePackedPresence({ q: IDENTITY, r: packPresenceRuntime(t - 2_000, 20_000) });
  // Each metadata poll refreshes presence and the mode, as pollDriveRevision does.
  h.context.pollDriveRevision = async () => {
    h.context.pollCount += 1;
    h.requests.push({ at: h.now(), method: 'GET', kind: 'poll' });
    h.context.latestPresenceUsers = [{ ...ghost, isOnline: isPresenceCurrentlyOnline(ghost, h.now()) }];
    h.probe.noteCollabPresenceModeChange();
  };
  h.probe.startCollabPolling();
  await h.advance(100);
  assert.equal(h.probe.getCollabPresenceMode(), 'shared', 'within its window it may still be there');
  await h.advance(25_000);
  const lateSince = h.mark();
  await h.advance(2 * MINUTE);
  assert.equal(h.probe.getCollabPresenceMode(), 'solo');
  assert.ok(h.polls(lateSince) <= 8, `newcomer back to the idle cadence (${h.polls(lateSince)} polls in 2 min)`);
  const fastPolls = h.requests.filter(r => r.kind === 'poll' && r.at < t + 25_300).length;
  assert.ok(fastPolls <= 45, `fast cadence lasted only the 25.2 s left in the window (${fastPolls} polls)`);

  // A record silent for 50 s is offline whatever interval it advertises.
  for (const interval of [2_000, 6_000, 20_000, 30_000, 3_600_000]) {
    const silent = parsePackedPresence({ q: IDENTITY, r: packPresenceRuntime(t - 50_000, interval) });
    assert.equal(isPresenceCurrentlyOnline(silent, t), false, `advertised ${interval} ms`);
  }
});

// ---------------------------------------------------------------------------
// Review fixes: P2P handshakes. The live layer adds a peer it is answering, or
// applies the answer to its own offer, without calling refreshPresenceViews().
// Only a negotiation the other device has spoken in proves it is here; an
// unanswered offer of our own (a peer cached from an earlier session, or one
// that left and is retried for 90 s) is a guess.

const outboundOffer = () => ({ initiator: true, status: 'connecting', channel: null, pc: { remoteDescription: null } });
const inboundOffer = () => ({ initiator: false, status: 'connecting', channel: { readyState: 'connecting' }, pc: { remoteDescription: { type: 'offer' } } });
const answeredOffer = () => ({ initiator: true, status: 'connecting', channel: { readyState: 'connecting' }, pc: { remoteDescription: { type: 'answer' } } });

test('an unanswered offer to a cached or departed peer keeps the solo cadence', async () => {
  const h = harness();
  // The real overview counts every non-open peer as connecting.
  h.context.ihnLivePeers = new Map([['cached-tablet', outboundOffer()]]);
  h.context.liveOverview = { openPeerCount: 0, connectingPeerCount: 1 };
  h.probe.startCollabPolling();
  assert.equal(h.probe.getCollabPresenceMode(), 'solo');
  await h.advance(15_000);
  // Was 35 polls and 11 heartbeats in the first 15 s of every open.
  assert.equal(h.polls(0), 1, 'only the opening poll');
  assert.ok(h.heartbeats(0) <= 2, `startup heartbeat only (${h.heartbeats(0)})`);

  // The P2P layer keeps retrying a collaborator that left (up to 90 s).
  const since = h.mark();
  for (let attempt = 0; attempt < 5; attempt++) {
    h.context.ihnLivePeers.set('cached-tablet', outboundOffer());
    await h.advance(18_000);
    h.context.ihnLivePeers.delete('cached-tablet');
    await h.advance(2_000);
  }
  assert.ok(h.polls(since) <= 6, `retries cost no fast polls (${h.polls(since)} in 100 s)`);
  assert.ok(h.heartbeats(since) <= 6, `retries cost no fast heartbeats (${h.heartbeats(since)} in 100 s)`);
});

test('a handshake the other device has spoken in retunes within a second, without any notification', async () => {
  for (const [label, peer] of [['inbound offer', inboundOffer], ['answer to our offer', answeredOffer]]) {
    const h = harness();
    h.context.ihnLivePeers = new Map();
    h.probe.startCollabPolling();
    await h.advance(MINUTE + 3_000); // between two solo polls
    assert.equal(h.probe.getCollabPresenceMode(), 'solo');
    // Silently, as ihnAcceptOffer / ihnApplyAnswer do.
    h.context.ihnLivePeers.set('tablet', peer());
    h.context.liveOverview = { openPeerCount: 0, connectingPeerCount: 1 };
    const since = h.mark();
    await h.advance(1_000);
    assert.equal(h.probe.getCollabPresenceMode(), 'shared', label);
    assert.equal(h.heartbeats(since), 1, `${label}: beats at once so the other device sees this one online`);
    await h.advance(14_000);
    assert.ok(h.polls(since) >= 20, `${label}: ${h.polls(since)} polls in 15 s (was 0)`);
    assert.ok(h.heartbeats(since) >= 7, `${label}: ${h.heartbeats(since)} heartbeats in 15 s`);

    // The channel opens: linked cadence (the layer does notify this one, but
    // the local check alone also picks it up).
    h.context.ihnLivePeers.get('tablet').channel.readyState = 'open';
    h.context.liveOverview = { openPeerCount: 1, connectingPeerCount: 0 };
    h.probe.noteCollabPresenceModeChange();
    assert.equal(h.probe.getCollabPresenceMode(), 'linked');
    h.probe.stopCollabPolling();
    assert.equal(h.timers.size, 0, `${label}: no timers after stop`);
  }
});

test('the local mode check makes no requests and stops with collaboration', async () => {
  for (const stop of ['stopCollabPolling', 'pauseCollabPollingForBackground']) {
    const h = harness();
    h.context.ihnLivePeers = new Map();
    h.probe.startCollabPolling();
    await h.advance(MINUTE + 5_000);
    const since = h.mark();
    await h.advance(10_000); // no regular poll or beat is due in this window
    assert.deepEqual(h.requests.slice(since).map(r => [r.kind, r.method]), [], 'solo: the mode check is local');
    h.probe[stop]();
    await h.advance(10 * MINUTE);
    assert.equal(h.timers.size, 0, `${stop} while solo: no timers left`);
  }
});

test('solo heartbeats are spaced from their start, so PATCH latency does not widen the gap', async () => {
  const h = harness();
  h.context.fetchDelayMs = 1_500;
  h.probe.startCollabPolling();
  await h.advance(3 * MINUTE);
  const stamps = beatRequests(h).map(beatTime).slice(-6);
  const gaps = stamps.slice(1).map((t, i) => t - stamps[i]);
  assert.ok(gaps.every(gap => gap === 20_000), `heartbeat gaps ${gaps.join(', ')}`);
});
