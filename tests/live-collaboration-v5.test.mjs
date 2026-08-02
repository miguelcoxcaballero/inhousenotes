import assert from 'node:assert/strict';
import fs from 'node:fs';
import { createRequire } from 'node:module';
import test from 'node:test';
import vm from 'node:vm';

const require = createRequire(import.meta.url);
const core = require('../collaboration-core-v5.js');
const liveSource = fs.readFileSync(new URL('../live-collaboration-v5.js', import.meta.url), 'utf8');

function simpleHash(value) {
  let hash = 0;
  const text = String(value || '');
  for (let index = 0; index < text.length; index += 1) {
    hash = ((hash << 5) - hash + text.charCodeAt(index)) | 0;
  }
  return Math.abs(hash).toString(36);
}

function bytesToBase64Url(bytes) {
  return Buffer.from(bytes)
    .toString('base64')
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=+$/g, '');
}

async function encryptTestSignal(keyBytes, payload) {
  const key = await globalThis.crypto.subtle.importKey(
    'raw',
    keyBytes,
    'AES-GCM',
    false,
    ['encrypt']
  );
  const iv = new Uint8Array(12);
  globalThis.crypto.getRandomValues(iv);
  const plain = new TextEncoder().encode(JSON.stringify(payload));
  const cipher = new Uint8Array(await globalThis.crypto.subtle.encrypt(
    { name: 'AES-GCM', iv },
    key,
    plain
  ));
  const packed = new Uint8Array(iv.length + cipher.length);
  packed.set(iv);
  packed.set(cipher, iv.length);
  return `IHN_LIVE_V1:${bytesToBase64Url(packed)}`;
}

async function decryptTestSignal(content, keyBytes) {
  const key = await globalThis.crypto.subtle.importKey(
    'raw',
    keyBytes,
    'AES-GCM',
    false,
    ['decrypt']
  );
  const packed = Buffer.from(
    String(content).replace(/^IHN_LIVE_V1:/, '').replace(/-/g, '+').replace(/_/g, '/'),
    'base64'
  );
  const iv = packed.subarray(0, 12);
  const cipher = packed.subarray(12);
  const plain = await globalThis.crypto.subtle.decrypt({ name: 'AES-GCM', iv }, key, cipher);
  return JSON.parse(new TextDecoder().decode(plain));
}

function createStorage() {
  const values = new Map();
  return {
    getItem: key => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: key => values.delete(key)
  };
}

function createChannel() {
  return {
    readyState: 'open',
    bufferedAmount: 0,
    bufferedAmountLowThreshold: 0,
    sent: [],
    send(value) { this.sent.push(value); },
    close() { this.readyState = 'closed'; },
    addEventListener() {},
    removeEventListener() {}
  };
}

function createHarness(overrides = {}) {
  let timerId = 0;
  let documentSessionId = 1;
  const pendingTimers = new Map();
  const broadcastChannels = [];
  class FakeBroadcastChannel {
    constructor(name) {
      this.name = name;
      this.messages = [];
      broadcastChannels.push(this);
    }
    postMessage(value) { this.messages.push(value); }
    close() {}
  }
  class FakePeerConnection {
    constructor() {
      this.connectionState = 'connecting';
      this.iceConnectionState = 'checking';
      this.iceGatheringState = 'complete';
    }
    close() { this.connectionState = 'closed'; }
    addEventListener() {}
    removeEventListener() {}
    getStats() { return Promise.resolve(new Map()); }
  }

  const state = {
    driveFileId: 'file-1',
    driveCanEdit: true,
    driveAutosave: true,
    isReadOnly: false,
    driveHeadRevisionId: 'rev-1',
    exportName: 'Document',
    calendarPageConfig: null,
    collabStructure: core.ihnNormalizeStructureMeta(null, ['p1']),
    collabFields: core.ihnNormalizeFieldMeta(null),
    pages: [{
      pageId: 'p1',
      strokes: [{ id: 'local-stroke', points: [{ x: 1, y: 1 }] }],
      images: [],
      deletedStrokeIds: [],
      backgroundSource: 'template',
      pageWidth: 210,
      pageHeight: 297
    }]
  };
  const context = vm.createContext({
    ...core,
    console,
    state,
    driveAccessToken: 'token',
    driveUserProfile: { email: 'a@example.com' },
    driveDirty: false,
    driveConfirmedContentHash: 'drive-base',
    driveContentVersion: 1,
    bumpCount: 0,
    DRIVE_META_TIMEOUT: 1000,
    DRIVE_POLL_TIMEOUT: 1000,
    simpleHash,
    getPresenceClientId: () => 'device-a',
    getDocumentSessionToken: () => documentSessionId,
    isDocumentSessionTokenValid: token => token === documentSessionId,
    sessionStorage: createStorage(),
    localStorage: createStorage(),
    document: {
      visibilityState: 'visible',
      addEventListener() {},
      getElementById() { return null; }
    },
    window: { addEventListener() {} },
    navigator: {},
    BroadcastChannel: FakeBroadcastChannel,
    RTCPeerConnection: FakePeerConnection,
    setTimeout(fn, ms = 0) {
      timerId += 1;
      pendingTimers.set(timerId, { fn, ms });
      return timerId;
    },
    clearTimeout(id) { pendingTimers.delete(id); },
    setInterval() { timerId += 1; return timerId; },
    clearInterval() {},
    crypto: globalThis.crypto,
    btoa: value => Buffer.from(value, 'binary').toString('base64'),
    atob: value => Buffer.from(value, 'base64').toString('binary'),
    TextEncoder,
    TextDecoder,
    flushStrokeOpsQueue: async () => true,
    ensureAllPagesLoadedForStructureChange: async () => true,
    getCollabStructureSnapshot: () => JSON.parse(JSON.stringify(state.collabStructure)),
    getCollabFieldSnapshot: () => JSON.parse(JSON.stringify(state.collabFields)),
    sanitizePageForStorage: page => JSON.parse(JSON.stringify(page)),
    cloneTimelineValue: value => JSON.parse(JSON.stringify(value)),
    hasSmoothInteraction: () => false,
    hasUnsyncedLocalDriveChanges: () => false,
    applyRemotePages: async () => ({ changed: false, hasLocalMerges: true, pagesNeedingPdfBackground: [] }),
    buildMetaPayload: () => ({}),
    saveToIndexedDb: async () => true,
    scheduleLocalStorageBackup() {},
    requestImmediateDriveSave() { context.immediateDriveSaves += 1; },
    bumpDriveContentVersion() {
      context.bumpCount += 1;
      context.driveContentVersion += 1;
    },
    immediateDriveSaves: 0,
    driveFetch: async () => ({ json: async () => ({}) }),
    refreshPresenceViews() {},
    showStatus() {},
    clearPublicInlinePreview() {},
    setReadOnlyMode() {},
    updateViewerModeUI() {},
    showEditorView() {},
    updateDocTitle() {},
    setReadOnly: false,
    ...overrides
  });

  vm.runInContext(`${liveSource}
globalThis.__liveTest = {
  addPeer(id, peer) { ihnLivePeers.set(id, peer); },
  getPeer(id) { return ihnLivePeers.get(id); },
  addKnown(id, at = Date.now()) { ihnLiveKnownPeers.set(id, { lastSeenAt: at, record: {} }); },
  configureChannel: ihnConfigureChannel,
  observePeerHash: ihnObservePeerDocumentHash,
  trackMergeCurrentState: ihnTrackMergeCurrentState,
  shouldQueueMergeUpload: ihnShouldQueueMergeUpload,
  queue(options = {}) {
    ihnLiveBroadcastQueued = true;
    if (options.force) ihnLiveBroadcastForce = true;
    if (options.targetPeerId) ihnLiveBroadcastTargets.add(options.targetPeerId);
  },
  buildSnapshot: ihnBuildLiveSnapshot,
  rememberSnapshot: ihnRememberLiveSnapshot,
  buildPeerSnapshot: ihnBuildPeerSnapshot,
  broadcast: ihnBroadcastDocument,
  wire: ihnHandleWireMessage,
  envelope: ihnHandleLiveEnvelope,
  supervise: ihnSuperviseConnections,
  wake: ihnWakeLiveCollaboration,
  claimLeader: ihnClaimLiveLeader,
  pollSignals: ihnPollSignals,
  ownId: ihnGetLivePeerId,
  generation() { return ihnLiveGeneration; },
  cryptoFileId() { return ihnLiveCryptoFileId; },
  processedOfferCount() { return ihnLiveProcessedOffers.size; },
  retry(id) { return ihnLiveRetryState.get(id); },
  retryState: ihnGetRetryState,
  seen(actor) { return ihnLiveSeen.get(actor); },
  queued() { return ihnLiveBroadcastQueued; },
  forced() { return ihnLiveBroadcastForce; },
  lastApplied() { return ihnLiveLastAppliedHash; },
  ensureSignalKey: ihnEnsureSignalKey,
  encodeSignal: ihnEncodeSignal,
  decodeSignal: ihnDecodeSignal,
  invalidateSignalKey: ihnInvalidateSignalKey,
  ensureTabChannel: ihnEnsureTabChannel,
  tabChannel() { return ihnLiveBroadcastChannel; },
  flushFanOut: ihnFlushLiveFanOutTasks,
  stop: stopLiveCollaboration,
  signalBusy() { return ihnLiveSignalBusy; },
  broadcastBusy() { return ihnLiveBroadcastBusy; },
  retryAttempt() { return ihnLiveBroadcastRetryAttempt; },
  targets() { return [...ihnLiveBroadcastTargets]; },
  pendingApplyCount() { return ihnLivePendingApplies.size; },
  activeApplySequence() { return Number(ihnLiveActiveApply?.sequence || 0); },
  canonicalHash() {
    return ihnCanonicalDocumentHash(
      state.pages.map(page => sanitizePageForStorage(page)),
      getCollabStructureSnapshot(),
      state.calendarPageConfig,
      state.exportName,
      getCollabFieldSnapshot()
    );
  }
};`, context, { filename: 'live-collaboration-v5.js' });

  async function runNextTimer() {
    const next = pendingTimers.entries().next().value;
    if (!next) return false;
    const [id, timer] = next;
    pendingTimers.delete(id);
    timer.fn();
    await Promise.resolve();
    return true;
  }

  return {
    context,
    api: context.__liveTest,
    state,
    pendingTimers,
    broadcastChannels,
    runNextTimer,
    advanceDocumentSession() { documentSessionId += 1; }
  };
}

function peerWithChannel(channel, overrides = {}) {
  return {
    pc: { close() {}, getStats: async () => new Map() },
    channel,
    status: 'open',
    initiator: true,
    fileId: 'file-1',
    generation: 0,
    createdAt: Date.now(),
    openedAt: Date.now(),
    lastReceivedAt: Date.now(),
    lastPongAt: Date.now(),
    lastSentHash: '',
    lastAckedHash: '',
    remoteCurrentHash: '',
    pendingAckHash: '',
    pendingAckAt: 0,
    pendingAckReceivedHash: '',
    pendingAckReceivedAt: 0,
    ...overrides
  };
}

function liveEnvelope(state, overrides = {}) {
  return {
    v: 1,
    type: 'document-snapshot',
    fileId: state.driveFileId,
    actorId: 'peer-remote:tab',
    sequence: 1,
    sentAt: Date.now(),
    exportName: state.exportName,
    calendarPageConfig: state.calendarPageConfig,
    structure: JSON.parse(JSON.stringify(state.collabStructure)),
    fields: JSON.parse(JSON.stringify(state.collabFields)),
    pages: JSON.parse(JSON.stringify(state.pages)),
    ...overrides
  };
}

test('live snapshots hold and always release the page-structure lock', async () => {
  const events = [];
  const token = { id: 'snapshot-lock' };
  const success = createHarness({
    acquireRemotePageMerge: async () => {
      events.push('acquire');
      return token;
    },
    releaseRemotePageMerge: released => events.push(released === token ? 'release' : 'wrong-release'),
    flushStrokeOpsQueue: async () => {
      events.push('flush');
      return true;
    },
    ensureAllPagesLoadedForStructureChange: async () => {
      events.push('hydrate');
      return true;
    }
  });
  await success.api.buildSnapshot();
  assert.deepEqual(events, ['acquire', 'flush', 'hydrate', 'release']);

  const failureEvents = [];
  const failure = createHarness({
    acquireRemotePageMerge: async () => token,
    releaseRemotePageMerge: released => failureEvents.push(released === token ? 'release' : 'wrong-release'),
    ensureAllPagesLoadedForStructureChange: async () => {
      throw new Error('hydrate failed');
    }
  });
  await assert.rejects(() => failure.api.buildSnapshot(), /hydrate failed/);
  assert.deepEqual(failureEvents, ['release']);
});

test('delivery bookkeeping is per peer, so a new peer gets current state without a new edit', async () => {
  const { api } = createHarness();
  const firstChannel = createChannel();
  api.addPeer('peer-first', peerWithChannel(firstChannel));
  api.queue();
  await api.broadcast();
  assert.ok(firstChannel.sent.length > 0);

  const firstPeer = api.getPeer('peer-first');
  await api.wire(JSON.stringify({ t: 'snapshot-ack', hash: firstPeer.lastSentHash }), 'peer-first');
  const firstMessageCount = firstChannel.sent.length;

  const secondChannel = createChannel();
  api.addPeer('peer-second', peerWithChannel(secondChannel));
  api.queue();
  await api.broadcast();

  assert.equal(firstChannel.sent.length, firstMessageCount, 'acked peer is not sent the same state again');
  assert.ok(secondChannel.sent.length > 0, 'new peer receives current snapshot immediately');
  assert.equal(api.getPeer('peer-second').pendingAckHash, api.getPeer('peer-first').lastAckedHash);
});

test('an acknowledged peer receives only changed pages and keeps a full snapshot fallback', async () => {
  const { api, state } = createHarness();
  state.pages = ['p1', 'p2', 'p3'].map((pageId, index) => ({
    pageId,
    strokes: Array.from({ length: 24 }, (_, strokeIndex) => ({
      id: `stroke-${index}-${strokeIndex}`,
      points: Array.from({ length: 10 }, (__, pointIndex) => ({
        x: pointIndex + index,
        y: pointIndex + strokeIndex
      }))
    })),
    images: [],
    deletedStrokeIds: [],
    backgroundSource: 'template',
    pageWidth: 210,
    pageHeight: 297
  }));
  state.collabStructure = core.ihnNormalizeStructureMeta(null, state.pages.map(page => page.pageId));

  const base = await api.buildSnapshot();
  api.rememberSnapshot(base);
  const peer = peerWithChannel(createChannel(), { lastAckedHash: base.contentHash });

  state.pages[1].strokes = [
    ...state.pages[1].strokes,
    { id: 'stroke-new', points: [{ x: 20, y: 30 }] }
  ];
  const current = await api.buildSnapshot();
  api.rememberSnapshot(current);
  const delta = api.buildPeerSnapshot(current, peer, false);

  assert.equal(delta.partial, true);
  assert.equal(delta.baseHash, base.contentHash);
  assert.equal(delta.contentHash, current.contentHash);
  assert.equal(delta.pages.map(page => page.pageId).join(','), 'p2');
  assert.equal(api.buildPeerSnapshot(current, peer, true).partial, undefined);
});

test('a page delta with the wrong base is rejected and requests a full snapshot', async () => {
  let applyCalls = 0;
  const harness = createHarness({
    applyRemotePages: async () => {
      applyCalls += 1;
      return { changed: true, hasLocalMerges: false, pagesNeedingPdfBackground: [] };
    }
  });
  const { api, state } = harness;
  const channel = createChannel();
  api.addPeer('peer-remote', peerWithChannel(channel, {
    pendingAckHash: 'target-hash',
    pendingAckAt: Date.now()
  }));
  const delta = liveEnvelope(state, {
    partial: true,
    baseHash: 'different-base',
    contentHash: 'target-hash',
    pages: []
  });

  assert.equal(await api.envelope(delta, 'webrtc', 'peer-remote'), false);
  assert.equal(applyCalls, 0);
  assert.equal(JSON.parse(channel.sent.at(-1)).t, 'snapshot-nack');

  await api.wire(JSON.stringify({
    t: 'snapshot-nack',
    hash: 'target-hash',
    currentHash: 'receiver-current'
  }), 'peer-remote');
  assert.equal(api.queued(), true);
  assert.equal(api.forced(), true);
  assert.equal(api.targets().join(','), 'peer-remote');
});

test('a page delta with the confirmed base applies and advances to the advertised full hash', async () => {
  let targetState = null;
  let targetPages = null;
  let applyCalls = 0;
  const harness = createHarness({
    applyRemotePages: async pages => {
      applyCalls += 1;
      assert.equal(pages.length, 1);
      targetState.pages = JSON.parse(JSON.stringify(targetPages));
      return { changed: true, hasLocalMerges: false, pagesNeedingPdfBackground: [] };
    }
  });
  const { api, state } = harness;
  targetState = state;
  const baseHash = api.canonicalHash();
  targetPages = JSON.parse(JSON.stringify(state.pages));
  targetPages[0].strokes.push({ id: 'remote-stroke', points: [{ x: 4, y: 8 }] });
  const targetHash = core.ihnCanonicalDocumentHash(
    targetPages,
    state.collabStructure,
    state.calendarPageConfig,
    state.exportName,
    state.collabFields
  );
  const delta = liveEnvelope(state, {
    partial: true,
    baseHash,
    contentHash: targetHash,
    pages: JSON.parse(JSON.stringify(targetPages))
  });

  assert.equal(await api.envelope(delta, 'webrtc', 'peer-remote'), true);
  assert.equal(applyCalls, 1);
  assert.equal(api.lastApplied(), targetHash);
  assert.equal(api.canonicalHash(), targetHash);
});

test('resuming gives an open peer a ping grace period before stale eviction', () => {
  const { api } = createHarness();
  const channel = createChannel();
  const peer = peerWithChannel(channel, {
    protocolV2: true,
    lastReceivedAt: Date.now() - 60_000,
    lastPongAt: Date.now() - 60_000
  });
  api.addPeer('peer-remote', peer);

  api.wake('app visible');

  assert.equal(api.getPeer('peer-remote'), peer);
  assert.ok(peer.resumeGraceUntil > Date.now());
  assert.equal(JSON.parse(channel.sent.at(-1)).t, 'ping');
});

test('C to D to C sends C again instead of treating an old ACK as current state', async () => {
  const { api } = createHarness();
  const channel = createChannel();
  const peer = peerWithChannel(channel);
  api.addPeer('peer-remote', peer);

  api.queue();
  await api.broadcast();
  const hashC = peer.pendingAckHash;
  assert.ok(hashC);
  await api.wire(JSON.stringify({ t: 'snapshot-ack', hash: hashC }), 'peer-remote');
  assert.equal(peer.lastAckedHash, hashC);
  const sentAfterFirstC = channel.sent.length;

  api.observePeerHash('peer-remote', 'hash-D');
  assert.equal(peer.lastAckedHash, '', 'incoming D invalidates the historical ACK for C');

  api.queue();
  await api.broadcast();
  assert.ok(channel.sent.length > sentAfterFirstC, 'the local return to C is delivered again');
  assert.equal(peer.pendingAckHash, hashC);

  const sentWithPendingC = channel.sent.length;
  api.observePeerHash('peer-remote', 'hash-E');
  assert.equal(peer.pendingAckHash, '', 'incoming E invalidates an unconfirmed delivery of C');
  assert.equal(peer.pendingAckAt, 0);

  api.queue();
  await api.broadcast();
  assert.ok(channel.sent.length > sentWithPendingC, 'C is retried after the peer moves away while its ACK is pending');
  assert.equal(peer.pendingAckHash, hashC);
});

test('a local merge is queued and sent on the first pass, not the next edit', async () => {
  const { api, context } = createHarness();
  const channel = createChannel();
  api.addKnown('peer-remote');
  api.addPeer('peer-remote', peerWithChannel(channel, { lastAckedHash: 'remote-hash' }));
  context.driveAccessToken = null;

  const envelope = {
    v: 1,
    type: 'document-snapshot',
    fileId: 'file-1',
    actorId: 'peer-remote:tab',
    sequence: 1,
    sentAt: Date.now(),
    contentHash: 'remote-hash',
    structure: core.ihnNormalizeStructureMeta(null, ['p1']),
    pages: [{
      pageId: 'p1',
      strokes: [],
      images: [],
      deletedStrokeIds: [],
      backgroundSource: 'template',
      pageWidth: 210,
      pageHeight: 297
    }]
  };
  await api.envelope(envelope, 'webrtc', 'peer-remote');
  assert.equal(api.seen(envelope.actorId), 1);
  assert.equal(api.queued(), true);
  assert.equal(api.forced(), true);
  assert.equal(context.immediateDriveSaves, 1);
  assert.equal(context.bumpCount, 1, 'the converged Drive PDF generation is invalidated before saving');

  await api.broadcast();
  assert.ok(channel.sent.length > 0, 'merged state is sent without requiring another user edit');
  assert.notEqual(api.lastApplied(), 'remote-hash');
});

test('UI Read mode preserves editable local changes while a true viewer remains authoritative', async () => {
  let editableApplyOptions;
  const editable = createHarness({
    hasUnsyncedLocalDriveChanges: () => true,
    applyRemotePages: async (_pages, options) => {
      editableApplyOptions = options;
      return { changed: false, hasLocalMerges: true, pagesNeedingPdfBackground: [] };
    }
  });
  editable.context.driveAccessToken = null;
  editable.state.isReadOnly = true;
  const editableEnvelope = liveEnvelope(editable.state, {
    actorId: 'read-mode-peer',
    sequence: 1,
    contentHash: 'read-mode-remote'
  });

  assert.equal(await editable.api.envelope(editableEnvelope, 'webrtc', 'peer-read-mode'), true);
  assert.equal(editableApplyOptions.preserveLocalUnsynced, true);
  assert.equal(editableApplyOptions.additiveById, true);
  assert.equal(editable.context.bumpCount, 1);
  assert.equal(editable.context.immediateDriveSaves, 1);
  assert.equal(editable.api.queued(), true);
  assert.equal(editable.api.forced(), true);

  let viewerApplyOptions;
  const viewer = createHarness({
    hasUnsyncedLocalDriveChanges: () => true,
    applyRemotePages: async (_pages, options) => {
      viewerApplyOptions = options;
      return { changed: false, hasLocalMerges: true, pagesNeedingPdfBackground: [] };
    }
  });
  viewer.state.isReadOnly = true;
  viewer.state.driveCanEdit = false;
  viewer.state.driveAutosave = false;
  const viewerEnvelope = liveEnvelope(viewer.state, {
    actorId: 'viewer-peer',
    sequence: 1,
    contentHash: 'viewer-remote'
  });

  assert.equal(await viewer.api.envelope(viewerEnvelope, 'webrtc', 'peer-viewer'), true);
  assert.equal(viewerApplyOptions.preserveLocalUnsynced, false);
  assert.equal(viewerApplyOptions.additiveById, false);
  assert.equal(viewer.context.bumpCount, 0);
  assert.equal(viewer.context.immediateDriveSaves, 0);
  assert.equal(viewer.api.queued(), false);
});

test('a failed apply does not consume the remote sequence', async () => {
  const harness = createHarness({
    applyRemotePages: async () => { throw new Error('transient IDB failure'); }
  });
  const envelope = {
    v: 1,
    type: 'document-snapshot',
    fileId: 'file-1',
    actorId: 'peer-b:tab',
    sequence: 7,
    contentHash: 'hash-b',
    pages: [{ pageId: 'p1', strokes: [], images: [] }]
  };
  await assert.rejects(harness.api.envelope(envelope, 'webrtc', 'peer-b'));
  assert.equal(harness.api.seen(envelope.actorId), undefined);
});

test('exact snapshots fan out between local tabs and WebRTC peers without echoing to their source', async () => {
  const exactApply = async () => ({
    changed: false,
    hasLocalMerges: false,
    pagesNeedingPdfBackground: []
  });
  const tabHarness = createHarness({ applyRemotePages: exactApply });
  tabHarness.context.driveAccessToken = null;
  const tabPeerChannel = createChannel();
  tabHarness.api.addPeer('peer-tab-target', peerWithChannel(tabPeerChannel));
  tabHarness.api.ensureTabChannel();
  const tabBroadcast = tabHarness.api.tabChannel();
  const tabEnvelope = liveEnvelope(tabHarness.state, {
    actorId: 'tab-actor',
    sequence: 1,
    contentHash: tabHarness.api.canonicalHash()
  });

  assert.equal(await tabHarness.api.envelope(tabEnvelope, 'tab'), true);
  await tabHarness.api.flushFanOut();
  assert.ok(tabPeerChannel.sent.length > 0, 'a non-leader tab reaches the network peer immediately');
  assert.equal(tabBroadcast.messages.length, 0, 'the incoming tab message is not echoed into BroadcastChannel');
  const tabSentCount = tabPeerChannel.sent.length;
  assert.equal(await tabHarness.api.envelope(tabEnvelope, 'tab'), true);
  await tabHarness.api.flushFanOut();
  assert.equal(tabPeerChannel.sent.length, tabSentCount, 'the same actor/sequence is never relayed twice');

  const peerHarness = createHarness({ applyRemotePages: exactApply });
  peerHarness.context.driveAccessToken = null;
  const sourceChannel = createChannel();
  const otherChannel = createChannel();
  peerHarness.api.addPeer('peer-source', peerWithChannel(sourceChannel));
  peerHarness.api.addPeer('peer-other', peerWithChannel(otherChannel));
  peerHarness.api.ensureTabChannel();
  const peerBroadcast = peerHarness.api.tabChannel();
  const peerEnvelope = liveEnvelope(peerHarness.state, {
    actorId: 'peer-source:tab',
    sequence: 4,
    contentHash: peerHarness.api.canonicalHash()
  });

  assert.equal(await peerHarness.api.envelope(peerEnvelope, 'webrtc', 'peer-source'), true);
  await peerHarness.api.flushFanOut();
  assert.equal(sourceChannel.sent.length, 0, 'the snapshot is never sent back to its source peer');
  assert.ok(otherChannel.sent.length > 0, 'another connected peer receives the exact envelope');
  assert.equal(peerBroadcast.messages.length, 1, 'same-device tabs receive the peer snapshot');
  assert.equal(peerBroadcast.messages[0].actorId, peerEnvelope.actorId);
  assert.equal(peerBroadcast.messages[0].sequence, peerEnvelope.sequence);
});

test('an already-applied hash can still bridge a new actor snapshot to a newly connected peer', async () => {
  const harness = createHarness({
    applyRemotePages: async () => ({
      changed: false,
      hasLocalMerges: false,
      pagesNeedingPdfBackground: []
    })
  });
  harness.context.driveAccessToken = null;
  const hash = harness.api.canonicalHash();
  assert.equal(await harness.api.envelope(liveEnvelope(harness.state, {
    actorId: 'actor-a',
    sequence: 1,
    contentHash: hash
  }), 'webrtc', 'peer-a'), true);
  await harness.api.flushFanOut();

  const lateChannel = createChannel();
  harness.api.addPeer('peer-late', peerWithChannel(lateChannel));
  assert.equal(await harness.api.envelope(liveEnvelope(harness.state, {
    actorId: 'actor-b',
    sequence: 1,
    contentHash: hash
  }), 'tab'), true);
  await harness.api.flushFanOut();

  assert.ok(lateChannel.sent.length > 0, 'hash dedupe does not swallow a required transport bridge');
});

test('a historical applied hash is merged again after the document moves C to D to C', async () => {
  const appliedMarkers = [];
  let harness;
  harness = createHarness({
    applyRemotePages: async pages => {
      appliedMarkers.push(pages[0].marker);
      harness.state.pages = JSON.parse(JSON.stringify(pages));
      return { changed: true, hasLocalMerges: false, pagesNeedingPdfBackground: [] };
    }
  });
  harness.context.driveAccessToken = null;
  const pageFor = marker => [{
    pageId: 'p1',
    marker,
    strokes: [{ id: `stroke-${marker}`, points: [{ x: marker.charCodeAt(0), y: 1 }] }],
    images: [],
    deletedStrokeIds: [],
    backgroundSource: 'template',
    pageWidth: 210,
    pageHeight: 297
  }];
  const hashFor = pages => core.ihnCanonicalDocumentHash(
    pages,
    harness.state.collabStructure,
    harness.state.calendarPageConfig,
    harness.state.exportName,
    harness.state.collabFields
  );
  const pagesC = pageFor('C');
  const pagesD = pageFor('D');

  assert.equal(await harness.api.envelope(liveEnvelope(harness.state, {
    actorId: 'actor-c-first',
    sequence: 1,
    pages: pagesC,
    contentHash: hashFor(pagesC)
  })), true);
  assert.equal(await harness.api.envelope(liveEnvelope(harness.state, {
    actorId: 'actor-d',
    sequence: 1,
    pages: pagesD,
    contentHash: hashFor(pagesD)
  })), true);
  assert.equal(await harness.api.envelope(liveEnvelope(harness.state, {
    actorId: 'actor-c-return',
    sequence: 1,
    pages: pagesC,
    contentHash: hashFor(pagesC)
  })), true);

  assert.deepEqual(appliedMarkers, ['C', 'D', 'C']);
});

test('the apply coalescer discards superseded snapshots while preserving other actors', async () => {
  let releaseFirstApply;
  const firstApplyGate = new Promise(resolve => { releaseFirstApply = resolve; });
  let announceFirstApply;
  const firstApplyStarted = new Promise(resolve => { announceFirstApply = resolve; });
  const applied = [];
  const harness = createHarness({
    applyRemotePages: async pages => {
      const marker = pages[0].marker;
      applied.push(marker);
      if (marker === 'A1') {
        announceFirstApply();
        await firstApplyGate;
      }
      return { changed: false, hasLocalMerges: false, pagesNeedingPdfBackground: [] };
    }
  });
  harness.context.driveAccessToken = null;
  const make = (actorId, sequence, marker) => liveEnvelope(harness.state, {
    actorId,
    sequence,
    contentHash: '',
    pages: [{ pageId: 'p1', marker, strokes: [], images: [] }]
  });

  const first = harness.api.envelope(make('actor-a', 1, 'A1'));
  await firstApplyStarted;
  const superseded = harness.api.envelope(make('actor-a', 2, 'A2'));
  const latest = harness.api.envelope(make('actor-a', 3, 'A3'));
  const otherActor = harness.api.envelope(make('actor-b', 1, 'B1'));

  assert.equal(await superseded, false, 'a pending older snapshot is explicitly superseded');
  releaseFirstApply();
  assert.equal(await first, true);
  assert.equal(await harness.runNextTimer(), true);
  assert.equal(await latest, true);
  assert.equal(await harness.runNextTimer(), true);
  assert.equal(await otherActor, true);
  assert.deepEqual(applied, ['A1', 'A3', 'B1']);
});

test('stopping a document invalidates an in-flight apply before any post-merge side effect', async () => {
  let releaseApply;
  const applyGate = new Promise(resolve => { releaseApply = resolve; });
  let announceApply;
  const applyStarted = new Promise(resolve => { announceApply = resolve; });
  let statusCount = 0;
  const harness = createHarness({
    applyRemotePages: async () => {
      announceApply();
      await applyGate;
      return { changed: true, hasLocalMerges: true, pagesNeedingPdfBackground: [] };
    },
    showStatus() { statusCount += 1; }
  });
  const envelope = liveEnvelope(harness.state, {
    actorId: 'old-document-actor',
    sequence: 9,
    contentHash: 'old-document-hash'
  });
  const applying = harness.api.envelope(envelope, 'webrtc', 'peer-old');
  await applyStarted;

  harness.api.stop();
  harness.state.driveFileId = 'file-2';
  harness.advanceDocumentSession();
  releaseApply();

  assert.equal(await applying, false);
  assert.equal(harness.api.seen(envelope.actorId), undefined);
  assert.equal(harness.context.immediateDriveSaves, 0);
  assert.equal(harness.context.bumpCount, 0);
  assert.equal(statusCount, 0);
});

test('an in-flight snapshot build cannot publish into a later document session', async () => {
  let releaseAcquire;
  const acquireGate = new Promise(resolve => { releaseAcquire = resolve; });
  let announceAcquire;
  const acquireStarted = new Promise(resolve => { announceAcquire = resolve; });
  let releaseCount = 0;
  const harness = createHarness({
    acquireRemotePageMerge: async () => {
      announceAcquire();
      await acquireGate;
      return { id: 'old-lock' };
    },
    releaseRemotePageMerge() { releaseCount += 1; }
  });
  harness.context.driveAccessToken = null;
  harness.api.queue({ force: true, targetPeerId: 'peer-next' });
  const staleBroadcast = harness.api.broadcast();
  await acquireStarted;

  harness.api.stop();
  harness.state.driveFileId = 'file-2';
  harness.advanceDocumentSession();
  const nextChannel = createChannel();
  harness.api.addPeer('peer-next', peerWithChannel(nextChannel, {
    fileId: 'file-2',
    generation: harness.api.generation()
  }));
  releaseAcquire();

  await assert.rejects(staleBroadcast, /document changed/);
  assert.equal(nextChannel.sent.length, 0);
  assert.equal(releaseCount, 1, 'the stale build still releases the structure lock');
  assert.equal(harness.api.queued(), false);
});

test('an old broadcast finally cannot clear the busy owner of a new document build', async () => {
  let releaseOld;
  const oldGate = new Promise(resolve => { releaseOld = resolve; });
  let announceOld;
  const oldStarted = new Promise(resolve => { announceOld = resolve; });
  let releaseCurrent;
  const currentGate = new Promise(resolve => { releaseCurrent = resolve; });
  let announceCurrent;
  const currentStarted = new Promise(resolve => { announceCurrent = resolve; });
  let acquireCount = 0;
  const harness = createHarness({
    acquireRemotePageMerge: async () => {
      acquireCount += 1;
      if (acquireCount === 1) {
        announceOld();
        await oldGate;
        return { id: 'old-lock' };
      }
      announceCurrent();
      await currentGate;
      return { id: 'current-lock' };
    },
    releaseRemotePageMerge() {}
  });
  harness.context.driveAccessToken = null;
  harness.api.queue();
  const oldBroadcast = harness.api.broadcast();
  const oldRejected = assert.rejects(oldBroadcast, /document changed/);
  await oldStarted;

  harness.api.stop();
  harness.state.driveFileId = 'file-2';
  harness.advanceDocumentSession();
  harness.api.queue();
  const currentBroadcast = harness.api.broadcast();
  await currentStarted;
  assert.equal(harness.api.broadcastBusy(), true);

  releaseOld();
  await oldRejected;
  assert.equal(harness.api.broadcastBusy(), true, 'the stale finally does not release the new run');

  releaseCurrent();
  await currentBroadcast;
  assert.equal(harness.api.broadcastBusy(), false);
});

test('a transient snapshot-lock failure preserves force and targets for bounded retry', async () => {
  let acquireCount = 0;
  let releaseCount = 0;
  const harness = createHarness({
    acquireRemotePageMerge: async () => {
      acquireCount += 1;
      return acquireCount === 1 ? null : { id: 'retry-lock' };
    },
    releaseRemotePageMerge() { releaseCount += 1; }
  });
  harness.context.driveAccessToken = null;
  const channel = createChannel();
  harness.api.addPeer('peer-retry', peerWithChannel(channel));
  harness.api.queue({ force: true, targetPeerId: 'peer-retry' });

  await assert.rejects(harness.api.broadcast(), /structure remained busy/);
  assert.equal(harness.api.queued(), true);
  assert.equal(harness.api.forced(), true);
  assert.equal(harness.api.targets().join(','), 'peer-retry');
  assert.equal(harness.api.retryAttempt(), 1);
  assert.equal(harness.pendingTimers.size, 1, 'only one bounded retry timer is armed');

  await harness.api.broadcast();
  assert.ok(channel.sent.length > 0, 'the preserved request succeeds without another edit');
  assert.ok(harness.api.getPeer('peer-retry').pendingAckHash);
  assert.equal(harness.api.retryAttempt(), 0);
  assert.equal(releaseCount, 1);
  assert.equal(harness.pendingTimers.size, 0);
});

test('supervisor evicts a stuck connecting peer and schedules autonomous retry', () => {
  const { api } = createHarness();
  const ownId = api.ownId();
  const remoteId = `${ownId}z`;
  api.addKnown(remoteId);
  let closed = false;
  api.addPeer(remoteId, {
    pc: { close() { closed = true; } },
    channel: { readyState: 'connecting', close() { closed = true; } },
    status: 'connecting',
    initiator: true,
    fileId: 'file-1',
    generation: api.generation(),
    createdAt: Date.now() - 13_000,
    commentId: ''
  });

  api.supervise();
  assert.equal(api.getPeer(remoteId), undefined);
  assert.equal(closed, true);
  const retry = api.retry(remoteId);
  assert.ok(retry.failures >= 1);
  assert.ok(retry.nextAttemptAt > Date.now());
});

test('supervisor replaces an apparently open but silent zombie channel', () => {
  const { api } = createHarness();
  const ownId = api.ownId();
  const remoteId = `${ownId}z`;
  api.addKnown(remoteId);
  let closed = false;
  api.addPeer(remoteId, peerWithChannel({
    ...createChannel(),
    close() { closed = true; this.readyState = 'closed'; }
  }, {
    generation: api.generation(),
    protocolV2: true,
    lastReceivedAt: Date.now() - 21_000
  }));

  api.supervise();
  assert.equal(api.getPeer(remoteId), undefined);
  assert.equal(closed, true);
  assert.ok(api.retry(remoteId).nextAttemptAt > Date.now());
});

test('healthy pongs extend a slow snapshot apply and hard expiry resends without reconnecting', async () => {
  const { api, pendingTimers } = createHarness();
  const remoteId = 'peer-slow-apply';
  api.addKnown(remoteId);
  const channel = createChannel();
  const now = Date.now();
  const peer = peerWithChannel(channel, {
    generation: api.generation(),
    protocolV2: true,
    openedAt: now - 60_000,
    lastReceivedAt: now - 100,
    lastPongAt: now - 100,
    pendingAckHash: 'slow-hash',
    pendingAckAt: now - 21_000
  });
  api.addPeer(remoteId, peer);

  await api.wire(JSON.stringify({ t: 'snapshot-received', hash: 'slow-hash' }), remoteId);
  assert.equal(peer.pendingAckReceivedHash, 'slow-hash');
  api.supervise();
  assert.equal(api.getPeer(remoteId), peer, 'a healthy route survives the normal 20-second ACK window');
  assert.equal(peer.pendingAckHash, 'slow-hash');

  peer.pendingAckAt = Date.now() - 121_000;
  peer.lastReceivedAt = Date.now();
  peer.lastPongAt = Date.now();
  api.supervise();

  assert.equal(api.getPeer(remoteId), peer, 'a responsive channel is not replaced in a reconnect loop');
  assert.equal(peer.pendingAckHash, '', 'only the stale delivery attempt is retired');
  assert.equal(api.queued(), true);
  assert.equal(api.forced(), true);
  assert.equal(api.targets().join(','), remoteId);
  assert.ok(pendingTimers.size >= 1, 'the newest state is scheduled through the same peer');
});

test('a healthy peer survives stale Drive presence and stays eligible for autonomous retry', () => {
  const signalKey = bytesToBase64Url(new Uint8Array(32).fill(0x70));
  const { api } = createHarness({
    driveFetch: async () => ({
      json: async () => ({ properties: { ihn_live_key_v1: signalKey } })
    })
  });
  const ownId = api.ownId();
  const remoteId = `${ownId}z`;
  api.addKnown(remoteId, Date.now() - 31_000);
  const channel = createChannel();
  const peer = peerWithChannel(channel, {
    generation: api.generation(),
    protocolV2: true,
    lastReceivedAt: Date.now()
  });
  api.addPeer(remoteId, peer);
  api.configureChannel(remoteId, channel, peer);
  assert.equal(api.claimLeader(), true);
  channel.onopen();

  api.supervise();

  assert.equal(api.getPeer(remoteId), peer);
  assert.equal(channel.readyState, 'open');

  channel.readyState = 'closed';
  channel.onclose();
  assert.equal(api.getPeer(remoteId), undefined);
  assert.ok(api.retry(remoteId).nextAttemptAt > Date.now(), 'recent P2P activity keeps retry eligibility');
});

test('backoff resets only after a valid ACK, not on open or mismatched ACK', async () => {
  const { api, context } = createHarness();
  context.driveAccessToken = null;
  const channel = createChannel();
  const peer = peerWithChannel(channel);
  const retry = api.retryState('peer-flapping');
  retry.failures = 4;
  retry.nextAttemptAt = Date.now() + 30_000;
  api.addPeer('peer-flapping', peer);
  api.configureChannel('peer-flapping', channel, peer);

  channel.onopen();
  assert.equal(retry.failures, 4, 'opening alone is not proof of a healthy route');

  peer.openedAt = Date.now() - 20_000;
  peer.lastPingAt = Date.now() - 10;
  await api.wire(JSON.stringify({ t: 'pong', at: peer.lastPingAt - 1 }), 'peer-flapping');
  assert.equal(retry.failures, 4, 'an unrelated or stale pong is not valid health evidence');

  await api.wire(JSON.stringify({ t: 'pong', at: peer.lastPingAt }), 'peer-flapping');
  assert.equal(retry.failures, 0, 'the response to the current health probe resets backoff');

  retry.failures = 4;
  retry.nextAttemptAt = Date.now() + 30_000;
  peer.pendingAckHash = 'expected-hash';
  peer.pendingAckAt = Date.now();
  await api.wire(JSON.stringify({ t: 'snapshot-ack', hash: 'stale-hash' }), 'peer-flapping');
  assert.equal(retry.failures, 4);
  assert.equal(peer.pendingAckHash, 'expected-hash');

  await api.wire(JSON.stringify({ t: 'snapshot-ack', hash: 'expected-hash' }), 'peer-flapping');
  assert.equal(retry.failures, 0);
  assert.equal(peer.pendingAckHash, '');
  assert.equal(peer.lastAckedHash, 'expected-hash');
});

test('signal-key creation imports Drive authoritative value after a concurrent write', async () => {
  const winnerBytes = new Uint8Array(32).fill(0x57);
  const winnerEncoded = bytesToBase64Url(winnerBytes);
  let getCount = 0;
  let patchCount = 0;
  let candidate = '';
  const { api } = createHarness({
    driveFetch: async (_url, options = {}) => {
      if ((options.method || 'GET') === 'PATCH') {
        patchCount += 1;
        candidate = JSON.parse(options.body).properties.ihn_live_key_v1;
        return { json: async () => ({ properties: { ihn_live_key_v1: candidate } }) };
      }
      getCount += 1;
      return {
        json: async () => getCount === 1
          ? { properties: {} }
          : { properties: { ihn_live_key_v1: winnerEncoded } }
      };
    }
  });

  const payload = { v: 1, type: 'offer', sessionId: 'race-winner' };
  const encodedSignal = await api.encodeSignal(payload);

  assert.equal(patchCount, 1);
  assert.equal(getCount, 2, 'Drive is read again after the candidate write');
  assert.notEqual(candidate, winnerEncoded, 'the test simulates another writer winning the race');
  assert.deepEqual(await decryptTestSignal(encodedSignal, winnerBytes), payload);
});

test('a losing initiator adopts the final Drive key on its second offer', async () => {
  const finalBytes = new Uint8Array(32).fill(0x68);
  const finalEncoded = bytesToBase64Url(finalBytes);
  let stored = '';
  let firstCandidate = '';
  let getCount = 0;
  let patchCount = 0;
  const { api } = createHarness({
    driveFetch: async (_url, options = {}) => {
      if ((options.method || 'GET') === 'PATCH') {
        patchCount += 1;
        stored = JSON.parse(options.body).properties.ihn_live_key_v1;
        firstCandidate = stored;
        return { json: async () => ({ properties: { ihn_live_key_v1: stored } }) };
      }
      getCount += 1;
      return { json: async () => ({ properties: stored ? { ihn_live_key_v1: stored } : {} }) };
    }
  });

  const firstOffer = { v: 1, type: 'offer', sessionId: 'attempt-1' };
  const firstSignal = await api.encodeSignal(firstOffer);
  assert.equal(patchCount, 1);
  assert.equal(getCount, 2, 'first offer performs initial read plus post-write comparison');
  assert.deepEqual(
    await decryptTestSignal(
      firstSignal,
      new Uint8Array(Buffer.from(firstCandidate.replace(/-/g, '+').replace(/_/g, '/'), 'base64'))
    ),
    firstOffer
  );

  // Exact losing-initiator interleaving: A has already PATCHed/read/cached A,
  // then B's later PATCH leaves B as the final Drive property. A receives no
  // answer because B could not decrypt attempt 1.
  stored = finalEncoded;

  const secondOffer = { v: 1, type: 'offer', sessionId: 'attempt-2' };
  const secondSignal = await api.encodeSignal(secondOffer);

  assert.equal(patchCount, 1, 'the retry adopts the winner instead of creating another key');
  assert.equal(getCount, 3, 'the new offer uses one coalesced authoritative read');
  assert.deepEqual(await decryptTestSignal(secondSignal, finalBytes), secondOffer);
  await assert.rejects(
    decryptTestSignal(
      secondSignal,
      new Uint8Array(Buffer.from(firstCandidate.replace(/-/g, '+').replace(/_/g, '/'), 'base64'))
    ),
    'the retry is no longer encrypted with the losing cached key'
  );
});

test('parallel local key requests share one compare-after-write creation', async () => {
  let releaseFirstRead;
  const firstReadGate = new Promise(resolve => { releaseFirstRead = resolve; });
  let getCount = 0;
  let patchCount = 0;
  let stored = '';
  const { api } = createHarness({
    driveFetch: async (_url, options = {}) => {
      if ((options.method || 'GET') === 'PATCH') {
        patchCount += 1;
        stored = JSON.parse(options.body).properties.ihn_live_key_v1;
        return { json: async () => ({ properties: { ihn_live_key_v1: stored } }) };
      }
      getCount += 1;
      if (getCount === 1) {
        await firstReadGate;
        return { json: async () => ({ properties: {} }) };
      }
      return { json: async () => ({ properties: { ihn_live_key_v1: stored } }) };
    }
  });

  const first = api.ensureSignalKey();
  const second = api.ensureSignalKey();
  releaseFirstRead();
  const [firstKey, secondKey] = await Promise.all([first, second]);

  assert.equal(firstKey, secondKey);
  assert.equal(patchCount, 1);
  assert.equal(getCount, 2);
});

test('a failed initial key read never creates a replacement key', async () => {
  let patchCount = 0;
  const { api } = createHarness({
    driveFetch: async (_url, options = {}) => {
      if ((options.method || 'GET') === 'PATCH') {
        patchCount += 1;
        return { json: async () => ({}) };
      }
      throw new Error('temporary Drive read failure');
    }
  });

  await assert.rejects(api.ensureSignalKey(), /temporary Drive read failure/);
  assert.equal(patchCount, 0);
});

test('a key GET completed after a document switch cannot create or cache a key for the stale document', async () => {
  const nextKey = bytesToBase64Url(new Uint8Array(32).fill(0x71));
  let releaseOldGet;
  const oldGetGate = new Promise(resolve => { releaseOldGet = resolve; });
  let oldPatchCount = 0;
  const { api, state } = createHarness({
    driveFetch: async (url, options = {}) => {
      const method = options.method || 'GET';
      if (method === 'PATCH') {
        if (String(url).includes('/files/file-1?')) oldPatchCount += 1;
        return { json: async () => ({}) };
      }
      if (String(url).includes('/files/file-1?')) {
        await oldGetGate;
        return { json: async () => ({ properties: {} }) };
      }
      if (String(url).includes('/files/file-2?')) {
        return { json: async () => ({ properties: { ihn_live_key_v1: nextKey } }) };
      }
      throw new Error(`Unexpected Drive request: ${url}`);
    }
  });

  const staleLoad = api.ensureSignalKey();
  state.driveFileId = 'file-2';
  api.invalidateSignalKey();
  releaseOldGet();

  await assert.rejects(staleLoad, /document changed/);
  assert.equal(oldPatchCount, 0, 'an empty result from the old GET cannot start a stale PATCH');
  await api.ensureSignalKey();
  assert.equal(api.cryptoFileId(), 'file-2');
});

test('a key PATCH completed after a document switch cannot cache or continue publishing in the new document', async () => {
  const nextKey = bytesToBase64Url(new Uint8Array(32).fill(0x72));
  let announcePatchStarted;
  const patchStarted = new Promise(resolve => { announcePatchStarted = resolve; });
  let releaseOldPatch;
  const oldPatchGate = new Promise(resolve => { releaseOldPatch = resolve; });
  const patchUrls = [];
  let oldGetCount = 0;
  const { api, state } = createHarness({
    driveFetch: async (url, options = {}) => {
      const method = options.method || 'GET';
      if (method === 'PATCH') {
        patchUrls.push(String(url));
        announcePatchStarted();
        await oldPatchGate;
        return { json: async () => ({}) };
      }
      if (String(url).includes('/files/file-1?')) {
        oldGetCount += 1;
        return { json: async () => ({ properties: {} }) };
      }
      if (String(url).includes('/files/file-2?')) {
        return { json: async () => ({ properties: { ihn_live_key_v1: nextKey } }) };
      }
      throw new Error(`Unexpected Drive request: ${url}`);
    }
  });

  const staleLoad = api.ensureSignalKey();
  await patchStarted;
  state.driveFileId = 'file-2';
  api.invalidateSignalKey();
  releaseOldPatch();

  await assert.rejects(staleLoad, /document changed/);
  assert.equal(oldGetCount, 1, 'the stale operation does not perform its post-PATCH verification GET');
  assert.equal(patchUrls.length, 1);
  assert.ok(patchUrls[0].includes('/files/file-1?'));
  assert.ok(!patchUrls.some(url => url.includes('/files/file-2?')));

  await api.ensureSignalKey();
  assert.equal(api.cryptoFileId(), 'file-2');
});

test('signal polling follows every Drive page and accepts an offer after more than 300 recent comments', async () => {
  const keyBytes = new Uint8Array(32).fill(0x73);
  const keyEncoded = bytesToBase64Url(keyBytes);
  const listPageTokens = [];
  let offerContent = '';
  let answerPosts = 0;

  class AcceptingPeerConnection {
    constructor() {
      this.connectionState = 'connecting';
      this.iceConnectionState = 'checking';
      this.iceGatheringState = 'complete';
      this.localDescription = null;
      this.remoteDescription = null;
    }
    async setRemoteDescription(description) { this.remoteDescription = description; }
    async setLocalDescription(description) { this.localDescription = description; }
    async createAnswer() { return { type: 'answer', sdp: 'answer' }; }
    close() { this.connectionState = 'closed'; }
    addEventListener() {}
    removeEventListener() {}
    getStats() { return Promise.resolve(new Map()); }
  }

  const { api } = createHarness({
    RTCPeerConnection: AcceptingPeerConnection,
    driveFetch: async (url, options = {}) => {
      const requestUrl = String(url);
      const method = options.method || 'GET';
      if (requestUrl.includes('fields=properties')) {
        return { json: async () => ({ properties: { ihn_live_key_v1: keyEncoded } }) };
      }
      if (method === 'POST' && requestUrl.includes('/replies?')) {
        answerPosts += 1;
        return { json: async () => ({ id: 'answer-1' }) };
      }
      if (method === 'GET' && requestUrl.includes('/comments?')) {
        const token = new URL(requestUrl).searchParams.get('pageToken') || '';
        listPageTokens.push(token);
        if (token === 'page-4') {
          return {
            json: async () => ({
              comments: [{ id: 'offer-comment', content: offerContent, replies: [] }]
            })
          };
        }
        const nextToken = token === ''
          ? 'page-2'
          : (token === 'page-2' ? 'page-3' : 'page-4');
        return {
          json: async () => ({
            comments: Array.from({ length: 100 }, (_, index) => ({
              id: `${token || 'page-1'}-${index}`,
              content: 'ordinary discussion'
            })),
            nextPageToken: nextToken
          })
        };
      }
      throw new Error(`Unexpected Drive request: ${requestUrl}`);
    }
  });

  offerContent = await encryptTestSignal(keyBytes, {
    v: 1,
    type: 'offer',
    fileId: 'file-1',
    from: 'peer-remote',
    to: api.ownId(),
    sessionId: 'offer-on-fourth-page',
    expiresAt: Date.now() + 60_000,
    description: { type: 'offer', sdp: 'offer' }
  });

  assert.equal(api.claimLeader(), true);
  await api.pollSignals();

  assert.deepEqual(listPageTokens, ['', 'page-2', 'page-3', 'page-4']);
  assert.equal(api.processedOfferCount(), 1);
  assert.equal(answerPosts, 1, 'the later-page offer is accepted and answered');
});

test('a stale signal poll cannot clear the busy owner of the next document', async () => {
  const keyEncoded = bytesToBase64Url(new Uint8Array(32).fill(0x74));
  let releaseFileOne;
  const fileOneGate = new Promise(resolve => { releaseFileOne = resolve; });
  let announceFileOne;
  const fileOneStarted = new Promise(resolve => { announceFileOne = resolve; });
  let releaseFileTwo;
  const fileTwoGate = new Promise(resolve => { releaseFileTwo = resolve; });
  let announceFileTwo;
  const fileTwoStarted = new Promise(resolve => { announceFileTwo = resolve; });
  let fileTwoPropertyReads = 0;
  const harness = createHarness({
    driveFetch: async url => {
      const requestUrl = String(url);
      if (requestUrl.includes('fields=properties') && requestUrl.includes('/files/file-1?')) {
        announceFileOne();
        await fileOneGate;
        return { json: async () => ({ properties: { ihn_live_key_v1: keyEncoded } }) };
      }
      if (requestUrl.includes('fields=properties') && requestUrl.includes('/files/file-2?')) {
        fileTwoPropertyReads += 1;
        announceFileTwo();
        await fileTwoGate;
        return { json: async () => ({ properties: { ihn_live_key_v1: keyEncoded } }) };
      }
      if (requestUrl.includes('/comments?')) {
        return { json: async () => ({ comments: [] }) };
      }
      throw new Error(`Unexpected Drive request: ${requestUrl}`);
    }
  });

  assert.equal(harness.api.claimLeader(), true);
  const stalePoll = harness.api.pollSignals();
  await fileOneStarted;
  assert.equal(harness.api.signalBusy(), true);

  harness.api.stop();
  harness.state.driveFileId = 'file-2';
  harness.advanceDocumentSession();
  assert.equal(harness.api.claimLeader(), true);
  const currentPoll = harness.api.pollSignals();
  await fileTwoStarted;
  assert.equal(harness.api.signalBusy(), true);

  releaseFileOne();
  await stalePoll;
  assert.equal(harness.api.signalBusy(), true, 'the old finally cannot release the new poll owner');
  await harness.api.pollSignals();
  assert.equal(fileTwoPropertyReads, 1, 'another poll cannot start while the current owner is active');

  releaseFileTwo();
  await currentPoll;
  assert.equal(harness.api.signalBusy(), false);
});

test('decrypt mismatch invalidates the cached key, reloads Drive, and succeeds on one retry', async () => {
  const staleBytes = new Uint8Array(32).fill(0x11);
  const winnerBytes = new Uint8Array(32).fill(0x22);
  const staleEncoded = bytesToBase64Url(staleBytes);
  const winnerEncoded = bytesToBase64Url(winnerBytes);
  let getCount = 0;
  let decryptCount = 0;
  const countingCrypto = {
    getRandomValues: array => globalThis.crypto.getRandomValues(array),
    subtle: {
      importKey: (...args) => globalThis.crypto.subtle.importKey(...args),
      encrypt: (...args) => globalThis.crypto.subtle.encrypt(...args),
      decrypt: (...args) => {
        decryptCount += 1;
        return globalThis.crypto.subtle.decrypt(...args);
      }
    }
  };
  const { api } = createHarness({
    crypto: countingCrypto,
    driveFetch: async () => {
      getCount += 1;
      return {
        json: async () => ({
          properties: {
            ihn_live_key_v1: getCount === 1 ? staleEncoded : winnerEncoded
          }
        })
      };
    }
  });
  await api.ensureSignalKey();
  const payload = { v: 1, type: 'answer', sessionId: 'recovered' };
  const content = await encryptTestSignal(winnerBytes, payload);

  assert.equal(JSON.stringify(await api.decodeSignal(content)), JSON.stringify(payload));
  assert.equal(getCount, 2, 'the property is re-read once after the mismatch');
  assert.equal(decryptCount, 2, 'decrypt is attempted initially and exactly once after refresh');
});

test('an undecryptable signal stops after the single refresh retry', async () => {
  const cachedBytes = new Uint8Array(32).fill(0x33);
  const otherBytes = new Uint8Array(32).fill(0x44);
  const cachedEncoded = bytesToBase64Url(cachedBytes);
  let getCount = 0;
  let decryptCount = 0;
  const countingCrypto = {
    getRandomValues: array => globalThis.crypto.getRandomValues(array),
    subtle: {
      importKey: (...args) => globalThis.crypto.subtle.importKey(...args),
      encrypt: (...args) => globalThis.crypto.subtle.encrypt(...args),
      decrypt: (...args) => {
        decryptCount += 1;
        return globalThis.crypto.subtle.decrypt(...args);
      }
    }
  };
  const { api } = createHarness({
    crypto: countingCrypto,
    driveFetch: async () => {
      getCount += 1;
      return {
        json: async () => ({ properties: { ihn_live_key_v1: cachedEncoded } })
      };
    }
  });
  await api.ensureSignalKey();
  const content = await encryptTestSignal(otherBytes, { type: 'offer' });

  assert.equal(await api.decodeSignal(content), null);
  assert.equal(getCount, 2);
  assert.equal(decryptCount, 2, 'there is no recursive refresh/decrypt loop');

  assert.equal(await api.decodeSignal(content), null);
  assert.equal(getCount, 2, 'the same ciphertext/key pair does not re-read Drive every poll');
  assert.equal(decryptCount, 2, 'the same ciphertext/key pair is skipped during the retry window');
});

test('merge-upload dedupe is scoped to the confirmed Drive baseline and content generation', () => {
  const { api, context } = createHarness();

  assert.equal(api.shouldQueueMergeUpload('hash-C'), true);
  assert.equal(api.shouldQueueMergeUpload('hash-C'), false, 'duplicate C on the same baseline is suppressed');

  api.trackMergeCurrentState('hash-D');
  assert.equal(
    api.shouldQueueMergeUpload('hash-C'),
    true,
    'an intermediate current D makes a later C a new transition even when D needed no upload'
  );
  assert.equal(api.shouldQueueMergeUpload('hash-C'), false);

  context.driveConfirmedContentHash = 'hash-D';
  context.driveContentVersion += 1;
  assert.equal(
    api.shouldQueueMergeUpload('hash-C'),
    true,
    'C after a confirmed D is a new transition, not a historical duplicate'
  );

  context.driveConfirmedContentHash = 'hash-C';
  assert.equal(api.shouldQueueMergeUpload('hash-C'), false, 'already-confirmed content is not uploaded again');
});

test('equivalent live convergence bumps the Drive PDF generation exactly once before saving', async () => {
  const harness = createHarness({
    applyRemotePages: async () => ({
      changed: true,
      hasLocalMerges: true,
      pagesNeedingPdfBackground: []
    })
  });
  harness.context.driveAccessToken = null;
  const first = liveEnvelope(harness.state, {
    actorId: 'merge-actor-a',
    sequence: 1,
    contentHash: 'remote-state-a'
  });
  const equivalent = liveEnvelope(harness.state, {
    actorId: 'merge-actor-b',
    sequence: 1,
    contentHash: 'remote-state-b'
  });

  assert.equal(await harness.api.envelope(first, 'webrtc', 'peer-a'), true);
  assert.equal(await harness.api.envelope(equivalent, 'webrtc', 'peer-b'), true);
  assert.equal(harness.context.bumpCount, 1);
  assert.equal(harness.context.immediateDriveSaves, 1);
  assert.equal(harness.context.driveContentVersion, 2);
});
