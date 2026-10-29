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
     getPresenceOnlineWindowMs
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
    presenceSavePhasePushInFlight: false,
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
      requests.push({ at: now, method: options.method || 'GET', kind: 'fetch', url, body: options.body });
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

test('alone and idle: metadata polls every 20 s and heartbeats every 30 s', async () => {
  const h = harness();
  h.probe.startCollabPolling();
  await h.advance(5_000); // first ping + startup beat
  const since = h.mark();
  await h.advance(10 * MINUTE);
  const pollsPerMinute = h.polls(since) / 10;
  const beatsPerMinute = h.heartbeats(since) / 10;
  // Was 100 polls/min (0.6 s) and 30 heartbeats/min (2 s).
  assert.ok(pollsPerMinute >= 2.9 && pollsPerMinute <= 3.1, `polls/min ${pollsPerMinute}`);
  assert.ok(beatsPerMinute >= 1.9 && beatsPerMinute <= 2.1, `heartbeats/min ${beatsPerMinute}`);
  assert.equal(h.probe.advertised, 30_000, 'solo beats advertise their 30 s interval');
  const lastBeat = h.requests.filter(r => r.kind === 'fetch' && r.method === 'PATCH').at(-1);
  assert.equal(JSON.parse(lastBeat.body).properties.p_me_r.split('|')[3], (30_000).toString(36), 'runtime field carries the interval in base 36');
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
  assert.ok(h.heartbeats(soloSince) <= 11, `solo heartbeats ${h.heartbeats(soloSince)}`);
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
  await h.advance(last + 30_000 - h.now() + 10); // the due beat is skipped
  h.context.driveSaveInProgress = false;
  const skippedAt = h.now();
  await h.advance(4_100);
  const beats = h.requests.filter(r => r.kind === 'fetch' && r.method === 'PATCH' && r.at > skippedAt);
  assert.equal(beats.length, 1, 'retry beat sent');
  assert.ok(beats[0].at - last <= 34_100, 'gap stays far inside the 61.2 s online window');
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

  // Solo record: 30 s interval, two missed beats (61.2 s).
  const slow = parsePackedPresence({ q: identity, r: packPresenceRuntime(t, 30_000) });
  assert.equal(slow.i, 30_000);
  assert.equal(slow.h, t);
  assert.equal(isPresenceCurrentlyOnline(slow, t + 61_000), true);
  assert.equal(isPresenceCurrentlyOnline(slow, t + 61_300), false);

  // Linked record: 6 s interval keeps the original 13.2 s window.
  const linked = parsePackedPresence({ q: identity, r: packPresenceRuntime(t, 6_000) });
  assert.equal(isPresenceCurrentlyOnline(linked, t + 13_200), true);
  assert.equal(isPresenceCurrentlyOnline(linked, t + 13_300), false);

  // A forged huge interval is capped at two minutes.
  const forged = parsePackedPresence({ q: identity, r: `${t.toString(36)}|||${(10 * 3_600_000).toString(36)}` });
  assert.equal(forged.i, 120_000);
});
