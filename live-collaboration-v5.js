/* Inhouse Notes v5 direct collaboration.
 * Static hosting only: Drive comments are an encrypted, short-lived WebRTC
 * rendezvous. The existing Drive PDF remains the durable source of truth and
 * automatic fallback. OAuth tokens never leave the device. */
const IHN_LIVE_SIGNAL_PREFIX = 'IHN_LIVE_V1:';
const IHN_LIVE_SIGNAL_TTL = 90_000;
const IHN_LIVE_CHUNK = 16_000;
const IHN_LIVE_MAX_CHUNKS = 5000;
const IHN_LIVE_MAX_CHARS = 50_000_000;
const IHN_LIVE_LEADER_TTL = 7000;
const IHN_LIVE_SUPERVISOR_MS = 180;
const IHN_LIVE_SIGNAL_POLL_MS = 220;
const IHN_LIVE_CONNECT_TIMEOUT = 15_000;
const IHN_LIVE_DISCONNECTED_GRACE = 5000;
const IHN_LIVE_PING_INTERVAL = 250;
const IHN_LIVE_ROUTE_STALE_MS = 900;
const IHN_LIVE_HEALTH_TIMEOUT = 20_000;
const IHN_LIVE_ACK_TIMEOUT = 20_000;
const IHN_LIVE_APPLY_ACK_TIMEOUT = 120_000;
const IHN_LIVE_LEGACY_PROBE_TIMEOUT = 90_000;
const IHN_LIVE_PEER_EXPIRY = 30_000;
const IHN_LIVE_HEALTHY_RESET_MS = 15_000;
const IHN_LIVE_FAILED_SIGNAL_RETRY_MS = 5000;
const IHN_LIVE_FAILED_SIGNAL_MAX = 256;
const IHN_LIVE_BROADCAST_RETRY_LIMIT = 4;
const IHN_LIVE_APPLY_COALESCE_MS = 36;
const IHN_LIVE_RESUME_GRACE_MS = 10_000;
const IHN_LIVE_NETWORK_PROBE_MS = 220;
const IHN_LIVE_NETWORK_RECOVERY_GRACE_MS = 650;
const IHN_LIVE_ICE_GATHER_TIMEOUT = 90;
const IHN_LIVE_FAST_ICE_GATHER_TIMEOUT = 35;
const IHN_LIVE_ICE_CANDIDATE_SETTLE_MS = 45;
const IHN_LIVE_ICE_BATCH_MS = 45;
const IHN_LIVE_ICE_BATCH_RETRY_LIMIT = 4;
const IHN_LIVE_SNAPSHOT_CACHE_LIMIT = 3;
const IHN_LIVE_SNAPSHOT_CACHE_MAX_CHARS = 16_000_000;
const IHN_LIVE_STROKE_FRAME_MS = 18;
const IHN_LIVE_STROKE_MAX_POINTS = 700;
const IHN_LIVE_STUN = [
    { urls: 'stun:stun.l.google.com:19302' },
    { urls: 'stun:stun1.l.google.com:19302' }
];
let ihnLiveTabId = '';
let ihnLiveBroadcastChannel = null;
let ihnLiveBroadcastFileId = '';
let ihnLiveLeaderTimer = null;
let ihnLiveSignalTimer = null;
let ihnLiveCapabilityTimer = null;
let ihnLiveSupervisorTimer = null;
let ihnLiveSignalBusy = false;
let ihnLiveSignalQueued = false;
let ihnLiveSignalRunId = 0;
let ihnLiveCryptoKey = null;
let ihnLiveCryptoFileId = '';
let ihnLiveCryptoKeyLoad = null;
let ihnLiveCryptoKeyLoadFileId = '';
let ihnLiveCryptoKeyRefresh = null;
let ihnLiveCryptoKeyRefreshFileId = '';
let ihnLiveCryptoKeyGeneration = 0;
let ihnLiveCryptoKeyFingerprint = '';
let ihnLiveFailedSignalRefreshAt = 0;
let ihnLiveFailedSignalRefreshKey = '';
let ihnLiveBroadcastTimer = null;
let ihnLiveBroadcastBusy = false;
let ihnLiveBroadcastRunId = 0;
let ihnLiveBroadcastQueued = false;
let ihnLiveBroadcastForce = false;
let ihnLiveBroadcastRetryAttempt = 0;
const ihnLiveBroadcastTargets = new Set();
const ihnLiveBroadcastExclusions = new Set();
let ihnLiveApplying = false;
let ihnLiveApplyQueue = Promise.resolve();
let ihnLiveApplyDrainPromise = null;
let ihnLiveActiveApply = null;
let ihnLiveSequence = Date.now();
let ihnLiveLastAppliedHash = '';
let ihnLiveCurrentHash = '';
let ihnLiveLastTabSentHash = '';
let ihnLiveMergeUploadGuard = null;
let ihnLiveMainPeerId = '';
let ihnLiveGeneration = 0;
let ihnLiveNetworkSignature = '';
let ihnLiveNetworkWakeAt = 0;
const ihnLivePeers = new Map();
const ihnLiveKnownPeers = new Map();
const ihnLiveRetryState = new Map();
const ihnLiveSeen = new Map();
const ihnLiveAppliedHashes = new Map();
const ihnLiveChunks = new Map();
const ihnLiveProcessedOffers = new Map();
const ihnLiveOffersInFlight = new Set();
const ihnLiveFailedSignals = new Map();
const ihnLivePendingApplies = new Map();
const ihnLiveFanOutTasks = new Set();
const ihnLiveSnapshotCache = new Map();
const ihnLiveStrokeSends = new Map();
const ihnLiveStrokeSeen = new Map();

function ihnCreatePeerConnection() {
    return new RTCPeerConnection({
        iceServers: IHN_LIVE_STUN,
        iceCandidatePoolSize: 4,
        bundlePolicy: 'max-bundle',
        rtcpMuxPolicy: 'require'
    });
}

function ihnGetLiveTabId() {
    if (ihnLiveTabId) return ihnLiveTabId;
    // sessionStorage can be cloned when a tab is duplicated. A per-JS-realm
    // identity prevents two tabs from sharing one actor/sequence stream and
    // accidentally suppressing each other's simultaneous edits.
    const randomPart = typeof crypto?.randomUUID === 'function'
        ? crypto.randomUUID().replace(/-/g, '')
        : Math.random().toString(36).slice(2);
    ihnLiveTabId = `t${Date.now().toString(36)}${randomPart.slice(0, 14)}`;
    return ihnLiveTabId;
}

function ihnGetLivePeerId(profile = driveUserProfile, clientId = getPresenceClientId()) {
    const accountHash = simpleHash(String(profile?.email || 'unknown').trim().toLowerCase());
    return `peer_${simpleHash(`${accountHash}:${String(clientId || '').trim().toLowerCase()}`)}`;
}

function ihnGetPresencePeerId(record) {
    const accountHash = String(record?.a || simpleHash(String(record?.e || '').trim().toLowerCase()));
    const client = String(record?.c || '').trim().toLowerCase();
    return accountHash && client ? `peer_${simpleHash(`${accountHash}:${client}`)}` : '';
}

function ihnLiveLeaderKey() {
    return state?.driveFileId ? `ihn-live-leader-v1:${state.driveFileId}` : '';
}

function ihnClaimLiveLeader() {
    const key = ihnLiveLeaderKey();
    if (!key) return false;
    const now = Date.now();
    const tabId = ihnGetLiveTabId();
    try {
        const current = JSON.parse(localStorage.getItem(key) || 'null');
        if (current?.tabId && current.tabId !== tabId && Number(current.expiresAt) > now) return false;
        localStorage.setItem(key, JSON.stringify({ tabId, expiresAt: now + IHN_LIVE_LEADER_TTL }));
        return JSON.parse(localStorage.getItem(key) || 'null')?.tabId === tabId;
    } catch (error) {
        return true;
    }
}

function ihnIsLiveLeader() {
    try {
        const current = JSON.parse(localStorage.getItem(ihnLiveLeaderKey()) || 'null');
        return current?.tabId === ihnGetLiveTabId() && Number(current.expiresAt) > Date.now();
    } catch (error) {
        return true;
    }
}

function ihnEnsureTabChannel() {
    if (!state?.driveFileId || typeof BroadcastChannel === 'undefined') return;
    if (ihnLiveBroadcastChannel && ihnLiveBroadcastFileId === state.driveFileId) return;
    try { ihnLiveBroadcastChannel?.close(); } catch (error) {}
    ihnLiveBroadcastFileId = state.driveFileId;
    ihnLiveBroadcastChannel = new BroadcastChannel(`inhousenotes-live-v5:${simpleHash(state.driveFileId)}`);
    ihnLiveBroadcastChannel.onmessage = event => {
        if (event.data?.type === 'live-stroke') {
            ihnHandleRealtimeStrokePacket(event.data, 'tab');
            return;
        }
        ihnHandleLiveEnvelope(event.data, 'tab').catch(error => console.warn('Same-device live update failed:', error));
    };
}

function ihnBytesToBase64Url(bytes) {
    let binary = '';
    for (let i = 0; i < bytes.length; i += 1) binary += String.fromCharCode(bytes[i]);
    return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/g, '');
}

function ihnBase64UrlToBytes(value) {
    const normalized = String(value || '').replace(/-/g, '+').replace(/_/g, '/');
    const binary = atob(normalized + '='.repeat((4 - normalized.length % 4) % 4));
    return Uint8Array.from(binary, char => char.charCodeAt(0));
}

function ihnInvalidateSignalKey(fileId = '') {
    if (fileId && ihnLiveCryptoFileId && ihnLiveCryptoFileId !== fileId) return false;
    ihnLiveCryptoKey = null;
    ihnLiveCryptoFileId = '';
    ihnLiveCryptoKeyFingerprint = '';
    ihnLiveCryptoKeyGeneration += 1;
    return true;
}

async function ihnReadSignalKeyProperty(fileId) {
    const propertyName = 'ihn_live_key_v1';
    const response = await driveFetch(`https://www.googleapis.com/drive/v3/files/${fileId}?fields=properties`, {
        method: 'GET', timeout: DRIVE_META_TIMEOUT, headers: { 'Cache-Control': 'no-cache' }
    });
    return String((await response.json())?.properties?.[propertyName] || '');
}

async function ihnImportSignalKey(encoded) {
    if (!encoded) throw new Error('Live signalling key unavailable');
    return crypto.subtle.importKey(
        'raw',
        ihnBase64UrlToBytes(encoded),
        'AES-GCM',
        false,
        ['encrypt', 'decrypt']
    );
}

function ihnAssertSignalKeyContext(fileId, generation) {
    if (!fileId
        || state?.driveFileId !== fileId
        || generation !== ihnLiveCryptoKeyGeneration) {
        throw new Error('Live signalling document changed');
    }
}

function ihnAssertLiveSessionContext(fileId, generation) {
    if (!fileId
        || state?.driveFileId !== fileId
        || generation !== ihnLiveGeneration) {
        throw new Error('Live collaboration document changed');
    }
}

function ihnCaptureLiveOperationContext(fileId = state?.driveFileId || '') {
    let sessionToken = null;
    try {
        if (typeof getDocumentSessionToken === 'function') {
            sessionToken = getDocumentSessionToken();
        }
    } catch (error) {
        sessionToken = null;
    }
    return {
        fileId: String(fileId || ''),
        generation: ihnLiveGeneration,
        sessionToken
    };
}

function ihnLiveOperationContextIsCurrent(operation) {
    if (!operation
        || !operation.fileId
        || state?.driveFileId !== operation.fileId
        || operation.generation !== ihnLiveGeneration) {
        return false;
    }
    if (operation.sessionToken === null
        || operation.sessionToken === undefined
        || typeof isDocumentSessionTokenValid !== 'function') {
        return true;
    }
    try {
        return !!isDocumentSessionTokenValid(operation.sessionToken);
    } catch (error) {
        return false;
    }
}

function ihnAssertLiveOperationContext(operation) {
    if (ihnLiveOperationContextIsCurrent(operation)) return;
    const error = new Error('Live collaboration document changed');
    error.code = 'IHN_LIVE_STALE_CONTEXT';
    throw error;
}

function ihnCanEditLiveDocument() {
    return !!state && state.driveCanEdit !== false;
}

function ihnCacheSignalKey(fileId, key, generation, encoded = '') {
    if (!key) return key;
    ihnAssertSignalKeyContext(fileId, generation);
    ihnLiveCryptoKey = key;
    ihnLiveCryptoFileId = fileId;
    ihnLiveCryptoKeyFingerprint = encoded ? simpleHash(encoded) : '';
    return key;
}

async function ihnCreateAndVerifySignalKey(fileId, generation) {
    ihnAssertSignalKeyContext(fileId, generation);
    const propertyName = 'ihn_live_key_v1';
    const random = new Uint8Array(32);
    crypto.getRandomValues(random);
    const candidate = ihnBytesToBase64Url(random);
    ihnAssertSignalKeyContext(fileId, generation);
    await driveFetch(`https://www.googleapis.com/drive/v3/files/${fileId}?fields=id,properties`, {
        method: 'PATCH', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ properties: { [propertyName]: candidate } })
    });
    ihnAssertSignalKeyContext(fileId, generation);

    // Another editor may have observed the empty property and written its own
    // candidate at the same time. Never assume our PATCH won: read Drive again
    // and import the value that is authoritative after the write.
    const authoritative = await ihnReadSignalKeyProperty(fileId);
    ihnAssertSignalKeyContext(fileId, generation);
    if (!authoritative) throw new Error('Live signalling key was not confirmed by Drive');
    return ihnCacheSignalKey(fileId, await ihnImportSignalKey(authoritative), generation, authoritative);
}

async function ihnGetAuthoritativeSignalKeyForWrite() {
    if (!state?.driveFileId || !driveAccessToken || !state.driveCanEdit) return null;
    const fileId = state.driveFileId;
    if (ihnLiveCryptoKeyRefresh && ihnLiveCryptoKeyRefreshFileId === fileId) {
        return ihnLiveCryptoKeyRefresh;
    }
    const refresh = (async () => {
        // If this tab is still performing the first read/create, wait for that
        // write to settle and then compare against Drive once more. Joining the
        // creation alone is insufficient: another device may win immediately
        // after our compare-after-write read.
        if (ihnLiveCryptoKeyLoad && ihnLiveCryptoKeyLoadFileId === fileId) {
            try { await ihnLiveCryptoKeyLoad; } catch (error) { /* authoritative read below decides */ }
        }
        if (state?.driveFileId !== fileId) {
            throw new Error('Live signalling document changed');
        }
        const generation = ihnLiveCryptoKeyGeneration;
        const encoded = await ihnReadSignalKeyProperty(fileId);
        ihnAssertSignalKeyContext(fileId, generation);
        if (encoded) {
            return ihnCacheSignalKey(fileId, await ihnImportSignalKey(encoded), generation, encoded);
        }
        return ihnCreateAndVerifySignalKey(fileId, generation);
    })();
    ihnLiveCryptoKeyRefresh = refresh;
    ihnLiveCryptoKeyRefreshFileId = fileId;
    try {
        return await refresh;
    } finally {
        if (ihnLiveCryptoKeyRefresh === refresh) {
            ihnLiveCryptoKeyRefresh = null;
            ihnLiveCryptoKeyRefreshFileId = '';
        }
    }
}

async function ihnEnsureSignalKey() {
    if (!state?.driveFileId || !driveAccessToken || !state.driveCanEdit) return null;
    if (ihnLiveCryptoKeyRefresh && ihnLiveCryptoKeyRefreshFileId === state.driveFileId) {
        return ihnLiveCryptoKeyRefresh;
    }
    if (ihnLiveCryptoKey && ihnLiveCryptoFileId === state.driveFileId) return ihnLiveCryptoKey;
    const fileId = state.driveFileId;
    if (ihnLiveCryptoKeyLoad && ihnLiveCryptoKeyLoadFileId === fileId) {
        return ihnLiveCryptoKeyLoad;
    }
    const generation = ihnLiveCryptoKeyGeneration;
    const load = (async () => {
        // A failed GET is not evidence that the property is absent. Propagating
        // that error prevents a transient Drive failure from replacing a valid
        // shared key with a new candidate.
        const encoded = await ihnReadSignalKeyProperty(fileId);
        ihnAssertSignalKeyContext(fileId, generation);
        if (encoded) {
            return ihnCacheSignalKey(fileId, await ihnImportSignalKey(encoded), generation, encoded);
        }
        return ihnCreateAndVerifySignalKey(fileId, generation);
    })();
    ihnLiveCryptoKeyLoad = load;
    ihnLiveCryptoKeyLoadFileId = fileId;
    try {
        return await load;
    } finally {
        if (ihnLiveCryptoKeyLoad === load) {
            ihnLiveCryptoKeyLoad = null;
            ihnLiveCryptoKeyLoadFileId = '';
        }
    }
}

async function ihnReloadSignalKeyFromDrive(fileId) {
    if (!fileId || state?.driveFileId !== fileId) {
        throw new Error('Live signalling document changed');
    }
    ihnInvalidateSignalKey(fileId);
    const generation = ihnLiveCryptoKeyGeneration;
    const encoded = await ihnReadSignalKeyProperty(fileId);
    ihnAssertSignalKeyContext(fileId, generation);
    if (!encoded) throw new Error('Live signalling key unavailable');
    return ihnCacheSignalKey(fileId, await ihnImportSignalKey(encoded), generation, encoded);
}

async function ihnEncodeSignal(payload) {
    // Offers and answers are retry boundaries. Always re-read the property so
    // an initiator that lost a concurrent first-write race adopts the final key
    // on its next offer even when it never received a decryptable answer.
    const fileId = state?.driveFileId || '';
    if (payload?.fileId && payload.fileId !== fileId) {
        throw new Error('Live signalling document changed');
    }
    const key = await ihnGetAuthoritativeSignalKeyForWrite();
    if (!key) throw new Error('Live signalling key unavailable');
    const generation = ihnLiveCryptoKeyGeneration;
    ihnAssertSignalKeyContext(fileId, generation);
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const plain = new TextEncoder().encode(JSON.stringify(payload));
    const cipher = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, key, plain));
    ihnAssertSignalKeyContext(fileId, generation);
    const packed = new Uint8Array(iv.length + cipher.length);
    packed.set(iv); packed.set(cipher, iv.length);
    return IHN_LIVE_SIGNAL_PREFIX + ihnBytesToBase64Url(packed);
}

function ihnFailedSignalId(content) {
    const value = String(content || '');
    return `${simpleHash(value)}:${value.length}`;
}

function ihnPruneFailedSignals(now = Date.now()) {
    ihnLiveFailedSignals.forEach((record, signalId) => {
        if (!record || Number(record.retryAfter || 0) <= now) {
            ihnLiveFailedSignals.delete(signalId);
        }
    });
    while (ihnLiveFailedSignals.size > IHN_LIVE_FAILED_SIGNAL_MAX) {
        const oldest = ihnLiveFailedSignals.keys().next().value;
        if (oldest === undefined) break;
        ihnLiveFailedSignals.delete(oldest);
    }
}

function ihnRememberFailedSignal(content, keyFingerprint, retryAfter) {
    if (!keyFingerprint) return;
    const ciphertext = String(content || '');
    const signalId = ihnFailedSignalId(content);
    ihnLiveFailedSignals.delete(signalId);
    ihnLiveFailedSignals.set(signalId, {
        ciphertext,
        keyFingerprint,
        retryAfter: Number(retryAfter) || (Date.now() + IHN_LIVE_FAILED_SIGNAL_RETRY_MS)
    });
    ihnPruneFailedSignals();
}

function ihnShouldSkipFailedSignal(content, keyFingerprint, now = Date.now()) {
    if (!keyFingerprint) return false;
    ihnPruneFailedSignals(now);
    const ciphertext = String(content || '');
    const record = ihnLiveFailedSignals.get(ihnFailedSignalId(ciphertext));
    return !!record
        && record.ciphertext === ciphertext
        && record.keyFingerprint === keyFingerprint
        && Number(record.retryAfter || 0) > now;
}

async function ihnDecodeSignal(content) {
    if (!String(content || '').startsWith(IHN_LIVE_SIGNAL_PREFIX)) return null;
    let packed;
    try {
        packed = ihnBase64UrlToBytes(String(content).slice(IHN_LIVE_SIGNAL_PREFIX.length));
        if (packed.length <= 12) return null;
    } catch (error) {
        return null;
    }
    const fileId = state?.driveFileId || '';
    let key;
    try {
        key = await ihnEnsureSignalKey();
        if (!key) return null;
    } catch (error) {
        return null;
    }
    const keyFingerprint = ihnLiveCryptoKeyFingerprint;
    if (ihnShouldSkipFailedSignal(content, keyFingerprint)) return null;
    let plain;
    try {
        plain = await crypto.subtle.decrypt(
            { name: 'AES-GCM', iv: packed.slice(0, 12) },
            key,
            packed.slice(12)
        );
    } catch (firstDecryptError) {
        // A concurrent first-write can leave this client with the losing key.
        // Refresh directly from Drive and retry the decrypt once. This is
        // deliberately non-recursive so corrupt/stale signals cannot cause a
        // refresh loop.
        const failedAt = Date.now();
        if (keyFingerprint
            && ihnLiveFailedSignalRefreshKey === keyFingerprint
            && failedAt - ihnLiveFailedSignalRefreshAt < IHN_LIVE_FAILED_SIGNAL_RETRY_MS) {
            ihnRememberFailedSignal(
                content,
                keyFingerprint,
                ihnLiveFailedSignalRefreshAt + IHN_LIVE_FAILED_SIGNAL_RETRY_MS
            );
            return null;
        }
        let refreshedKey;
        try {
            refreshedKey = await ihnReloadSignalKeyFromDrive(fileId);
        } catch (refreshError) {
            ihnLiveFailedSignalRefreshAt = failedAt;
            ihnLiveFailedSignalRefreshKey = keyFingerprint;
            ihnRememberFailedSignal(content, keyFingerprint, failedAt + IHN_LIVE_FAILED_SIGNAL_RETRY_MS);
            return null;
        }
        const refreshedAt = Date.now();
        const refreshedFingerprint = ihnLiveCryptoKeyFingerprint || keyFingerprint;
        ihnLiveFailedSignalRefreshAt = refreshedAt;
        ihnLiveFailedSignalRefreshKey = refreshedFingerprint;
        try {
            plain = await crypto.subtle.decrypt(
                { name: 'AES-GCM', iv: packed.slice(0, 12) },
                refreshedKey,
                packed.slice(12)
            );
        } catch (secondDecryptError) {
            ihnRememberFailedSignal(
                content,
                refreshedFingerprint,
                refreshedAt + IHN_LIVE_FAILED_SIGNAL_RETRY_MS
            );
            return null;
        }
    }
    try {
        return JSON.parse(new TextDecoder().decode(plain));
    } catch (error) {
        return null;
    }
}

function ihnWaitForIce(pc, timeout = IHN_LIVE_ICE_GATHER_TIMEOUT) {
    if (pc.iceGatheringState === 'complete') return Promise.resolve();
    return new Promise(resolve => {
        let done = false;
        let settleTimer = null;
        const finish = () => {
            if (done) return;
            done = true;
            clearTimeout(timer);
            clearTimeout(settleTimer);
            pc.removeEventListener('icegatheringstatechange', check);
            pc.removeEventListener('icecandidate', onCandidate);
            resolve();
        };
        const check = () => { if (pc.iceGatheringState === 'complete') finish(); };
        const onCandidate = event => {
            if (!event?.candidate || settleTimer) return;
            // Publish the SDP as soon as the first useful candidates settle;
            // later candidates continue through encrypted trickle batches.
            settleTimer = setTimeout(finish, Math.min(IHN_LIVE_ICE_CANDIDATE_SETTLE_MS, timeout));
        };
        const timer = setTimeout(finish, timeout);
        pc.addEventListener('icegatheringstatechange', check);
        pc.addEventListener('icecandidate', onCandidate);
    });
}

function ihnSerializeIceCandidate(candidate) {
    if (!candidate) return null;
    const source = typeof candidate.toJSON === 'function' ? candidate.toJSON() : candidate;
    const serialized = {
        candidate: String(source?.candidate || ''),
        sdpMid: source?.sdpMid === null || source?.sdpMid === undefined
            ? null
            : String(source.sdpMid),
        sdpMLineIndex: Number.isFinite(Number(source?.sdpMLineIndex))
            ? Number(source.sdpMLineIndex)
            : null
    };
    if (source?.usernameFragment) serialized.usernameFragment = String(source.usernameFragment);
    return serialized.candidate ? serialized : null;
}

function ihnIceCandidateKey(candidate) {
    return [
        String(candidate?.candidate || ''),
        String(candidate?.sdpMid ?? ''),
        String(candidate?.sdpMLineIndex ?? ''),
        String(candidate?.usernameFragment || '')
    ].join('|');
}

function ihnTouchPeerSignalling(peer, at = Date.now()) {
    if (!peer) return;
    const now = Number(at) || Date.now();
    peer.lastSignalActivityAt = now;
    const createdAt = Number(peer.createdAt || now);
    const minimumDeadline = createdAt + IHN_LIVE_CONNECT_TIMEOUT;
    const absoluteDeadline = createdAt + 30_000;
    peer.connectionDeadlineAt = Math.min(
        absoluteDeadline,
        Math.max(Number(peer.connectionDeadlineAt || minimumDeadline), now + 6000)
    );
}

function ihnScheduleLocalIceFlush(peerId, peer, delay = IHN_LIVE_ICE_BATCH_MS) {
    if (!peer
        || peer.closing
        || ihnLivePeers.get(peerId) !== peer
        || !peer.commentId
        || peer.channel?.readyState === 'open'
        || peer.localIceFlushTimer
        || peer.localIceFlushBusy
        || !peer.pendingLocalIceCandidates?.length) return;
    peer.localIceFlushTimer = setTimeout(() => {
        peer.localIceFlushTimer = null;
        ihnFlushLocalIceCandidates(peerId, peer).catch(error => {
            console.warn('Live ICE candidate batch deferred:', error);
        });
    }, Math.max(0, delay));
}

async function ihnFlushLocalIceCandidates(peerId, peer) {
    if (!peer
        || peer.closing
        || ihnLivePeers.get(peerId) !== peer
        || !peer.commentId
        || peer.channel?.readyState === 'open'
        || peer.localIceFlushBusy) return false;
    if (typeof navigator !== 'undefined' && navigator.onLine === false) return false;
    const queued = Array.isArray(peer.pendingLocalIceCandidates)
        ? peer.pendingLocalIceCandidates.splice(0)
        : [];
    const signalledSdp = String(peer.signalledLocalSdp || '');
    const candidates = queued.filter(candidate => {
        const candidateLine = String(candidate?.candidate || '');
        return candidateLine && !signalledSdp.includes(candidateLine);
    });
    if (candidates.length === 0) return true;
    const operation = ihnCaptureLiveOperationContext(peer.fileId);
    peer.localIceFlushBusy = true;
    try {
        const content = await ihnEncodeSignal({
            v: 1,
            type: 'candidates',
            fileId: peer.fileId,
            from: ihnGetLivePeerId(),
            to: peerId,
            sessionId: peer.sessionId,
            expiresAt: Date.now() + IHN_LIVE_SIGNAL_TTL,
            candidates
        });
        ihnAssertLiveOperationContext(operation);
        if (ihnLivePeers.get(peerId) !== peer || peer.closing || !peer.commentId) return false;
        await driveFetch(`https://www.googleapis.com/drive/v3/files/${peer.fileId}/comments/${peer.commentId}/replies?fields=id,createdTime`, {
            method: 'POST',
            timeout: DRIVE_META_TIMEOUT,
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ content })
        });
        peer.localIceRetryAttempt = 0;
        peer.localIceRetryDelay = 0;
        peer.localIceLastFailureAt = 0;
        ihnTouchPeerSignalling(peer);
        return true;
    } catch (error) {
        if (ihnLiveOperationContextIsCurrent(operation)
            && ihnLivePeers.get(peerId) === peer
            && !peer.closing) {
            const existingKeys = new Set((peer.pendingLocalIceCandidates || []).map(ihnIceCandidateKey));
            const retryCandidates = candidates.filter(candidate => !existingKeys.has(ihnIceCandidateKey(candidate)));
            peer.pendingLocalIceCandidates = [...retryCandidates, ...(peer.pendingLocalIceCandidates || [])];
            peer.localIceRetryAttempt = Number(peer.localIceRetryAttempt || 0) + 1;
            peer.localIceRetryDelay = Math.min(
                2400,
                250 * (2 ** Math.max(0, peer.localIceRetryAttempt - 1))
            );
            peer.localIceLastFailureAt = Date.now();
        }
        return false;
    } finally {
        peer.localIceFlushBusy = false;
        if (ihnLivePeers.get(peerId) === peer
            && !peer.closing
            && peer.pendingLocalIceCandidates?.length
            && peer.localIceRetryAttempt <= IHN_LIVE_ICE_BATCH_RETRY_LIMIT) {
            ihnScheduleLocalIceFlush(peerId, peer, peer.localIceRetryDelay || IHN_LIVE_ICE_BATCH_MS);
        }
    }
}

function ihnQueueLocalIceCandidate(peerId, peer, candidate) {
    if (!peer || peer.closing || ihnLivePeers.get(peerId) !== peer) return;
    if (!candidate) {
        peer.localIceGatheringComplete = true;
        ihnScheduleLocalIceFlush(peerId, peer, 0);
        return;
    }
    const serialized = ihnSerializeIceCandidate(candidate);
    if (!serialized) return;
    if (!peer.localIceCandidateKeys) peer.localIceCandidateKeys = new Set();
    const key = ihnIceCandidateKey(serialized);
    if (peer.localIceCandidateKeys.has(key)) return;
    peer.localIceCandidateKeys.add(key);
    if (!Array.isArray(peer.pendingLocalIceCandidates)) peer.pendingLocalIceCandidates = [];
    peer.pendingLocalIceCandidates.push(serialized);
    ihnTouchPeerSignalling(peer);
    ihnScheduleLocalIceFlush(peerId, peer);
}

async function ihnDrainRemoteIceCandidates(peerId, peer) {
    if (!peer?.pc?.remoteDescription || typeof peer.pc.addIceCandidate !== 'function') return false;
    const pending = peer.pendingRemoteIceCandidates instanceof Map
        ? [...peer.pendingRemoteIceCandidates.entries()]
        : [];
    if (pending.length === 0) return true;
    for (const [key, candidate] of pending) {
        if (ihnLivePeers.get(peerId) !== peer || peer.closing) return false;
        try {
            await peer.pc.addIceCandidate(candidate);
            peer.remoteIceCandidateKeys.add(key);
            peer.pendingRemoteIceCandidates.delete(key);
            ihnTouchPeerSignalling(peer);
        } catch (error) {
            // One browser-specific candidate must not poison the whole route;
            // the SDP and every other candidate remain usable.
            peer.remoteIceCandidateKeys.add(key);
            peer.pendingRemoteIceCandidates.delete(key);
            console.warn('A remote ICE candidate was not usable:', error);
        }
    }
    return true;
}

async function ihnApplyCandidateSignal(signal) {
    const ownId = ihnGetLivePeerId();
    if (!signal
        || signal.type !== 'candidates'
        || signal.to !== ownId
        || signal.from === ownId
        || signal.fileId !== state?.driveFileId
        || Number(signal.expiresAt) < Date.now()
        || !Array.isArray(signal.candidates)) return false;
    const peer = ihnLivePeers.get(signal.from);
    if (!peer || peer.sessionId !== signal.sessionId || peer.closing) return false;
    if (!(peer.pendingRemoteIceCandidates instanceof Map)) peer.pendingRemoteIceCandidates = new Map();
    if (!(peer.remoteIceCandidateKeys instanceof Set)) peer.remoteIceCandidateKeys = new Set();
    for (const rawCandidate of signal.candidates) {
        const candidate = ihnSerializeIceCandidate(rawCandidate);
        if (!candidate) continue;
        const key = ihnIceCandidateKey(candidate);
        if (peer.remoteIceCandidateKeys.has(key) || peer.pendingRemoteIceCandidates.has(key)) continue;
        peer.pendingRemoteIceCandidates.set(key, candidate);
    }
    ihnTouchPeerSignalling(peer);
    if (peer.pc.remoteDescription) await ihnDrainRemoteIceCandidates(signal.from, peer);
    return true;
}

function ihnGetRetryState(peerId) {
    if (!ihnLiveRetryState.has(peerId)) {
        ihnLiveRetryState.set(peerId, {
            failures: 0,
            nextAttemptAt: 0,
            lastReason: '',
            allowReverse: false
        });
    }
    return ihnLiveRetryState.get(peerId);
}

function ihnKnownPeerLastSeenAt(known) {
    return Math.max(
        Number(known?.lastSeenAt || 0),
        Number(known?.lastPeerSeenAt || 0)
    );
}

function ihnTouchKnownPeerFromChannel(peerId, at = Date.now()) {
    if (!peerId) return;
    const known = ihnLiveKnownPeers.get(peerId);
    if (known) {
        known.lastPeerSeenAt = Math.max(Number(known.lastPeerSeenAt || 0), Number(at) || 0);
        return;
    }
    // A decrypted signalling exchange and an open data channel are direct
    // evidence of the peer even if Drive presence is temporarily delayed.
    ihnLiveKnownPeers.set(peerId, {
        lastSeenAt: 0,
        lastPeerSeenAt: Number(at) || Date.now(),
        record: null
    });
}

function ihnPeerIsExpected(peerId) {
    const known = ihnLiveKnownPeers.get(peerId);
    return !!known && Date.now() - ihnKnownPeerLastSeenAt(known) <= IHN_LIVE_PEER_EXPIRY;
}

function ihnCanInitiatePeer(peerId) {
    return !!(
        peerId
        && ihnPeerIsExpected(peerId)
        && ihnIsLiveLeader()
        && state?.driveFileId
        && state.driveCanEdit
        && driveAccessToken
        && ihnGetLivePeerId() < peerId
    );
}

function ihnCanRecoverPeer(peerId) {
    return !!(
        peerId
        && ihnPeerIsExpected(peerId)
        && ihnIsLiveLeader()
        && state?.driveFileId
        && state.driveCanEdit
        && driveAccessToken
        && ihnGetLivePeerId() !== peerId
    );
}

function ihnSchedulePeerRetry(peerId, reason = '', options = {}) {
    const allowReverse = !!options.allowReverse;
    if (!ihnCanInitiatePeer(peerId) && !(allowReverse && ihnCanRecoverPeer(peerId))) return;
    const retry = ihnGetRetryState(peerId);
    if (options.reset) retry.failures = 0;
    else if (!options.preserveFailures) retry.failures = Math.min(7, retry.failures + 1);
    retry.lastReason = reason;
    retry.allowReverse = allowReverse;
    retry.nextAttemptAt = options.immediate
        ? Date.now()
        : Date.now() + ihnComputeReconnectDelay(Math.max(0, retry.failures - 1), peerId);
}

function ihnResetPeerRetry(peerId) {
    const retry = ihnGetRetryState(peerId);
    retry.failures = 0;
    retry.nextAttemptAt = 0;
    retry.lastReason = '';
    retry.allowReverse = false;
}

function ihnPeerChannelIsHealthy(peer, now = Date.now()) {
    if (peer?.channel?.readyState !== 'open') return false;
    const lastReceivedAt = Number(peer.lastReceivedAt || peer.openedAt || 0);
    const timeout = peer.protocolV2 ? IHN_LIVE_HEALTH_TIMEOUT : IHN_LIVE_LEGACY_PROBE_TIMEOUT;
    return lastReceivedAt > 0 && now - lastReceivedAt <= timeout;
}

function ihnMarkPeerHealthy(peerId, peer, reason = '') {
    if (!peer || ihnLivePeers.get(peerId) !== peer || peer.channel?.readyState !== 'open') return false;
    if (reason !== 'ack'
        && Date.now() - Number(peer.openedAt || 0) < IHN_LIVE_HEALTHY_RESET_MS) {
        return false;
    }
    ihnResetPeerRetry(peerId);
    peer.healthyAt = Date.now();
    peer.resumeGraceUntil = 0;
    return true;
}

function ihnDeleteSignalComment(peer) {
    const commentId = String(peer?.commentId || '');
    const fileId = String(peer?.fileId || state?.driveFileId || '');
    if (!commentId || !fileId || !driveAccessToken) return;
    peer.commentId = '';
    driveFetch(`https://www.googleapis.com/drive/v3/files/${fileId}/comments/${commentId}`, {
        method: 'DELETE',
        timeout: DRIVE_META_TIMEOUT
    }).catch(() => {});
}

function ihnClosePeer(peerId, reason = '', options = {}) {
    const peer = ihnLivePeers.get(peerId);
    if (!peer) {
        if (options.retry) ihnSchedulePeerRetry(peerId, reason, { ...options, allowReverse: true });
        return;
    }
    if (peer.closing) return;
    peer.closing = true;
    ihnLivePeers.delete(peerId);
    clearTimeout(peer.localIceFlushTimer);
    peer.localIceFlushTimer = null;
    if (Array.isArray(peer.pendingLocalIceCandidates)) peer.pendingLocalIceCandidates.length = 0;
    peer.pendingRemoteIceCandidates?.clear?.();
    peer.pendingHealthPings?.clear?.();
    ihnDeleteSignalComment(peer);
    try { peer.channel?.close(); } catch (error) {}
    try { peer.pc?.close(); } catch (error) {}
    if (options.retry !== false) {
        // After first contact either endpoint may recover a broken route. The
        // lower-id peer still owns first contact, while glare handling resolves
        // the rare case where both recovered endpoints offer simultaneously.
        ihnSchedulePeerRetry(peerId, reason, { ...options, allowReverse: true });
    }
    if (reason) console.info(`Live peer ${peerId} closed: ${reason}`);
    if (typeof refreshPresenceViews === 'function') refreshPresenceViews();
}

function ihnSendControl(peer, payload) {
    if (peer?.channel?.readyState !== 'open') return false;
    try {
        peer.channel.send(JSON.stringify(payload));
        peer.lastSentAt = Date.now();
        return true;
    } catch (error) {
        return false;
    }
}

function ihnSendHealthPing(peer, payload = {}) {
    if (!peer || peer.channel?.readyState !== 'open') return 0;
    const at = Date.now();
    if (!(peer.pendingHealthPings instanceof Set)) peer.pendingHealthPings = new Set();
    // Bound the set even if a browser silently drops an old route without
    // emitting an ICE state transition.
    for (const sentAt of peer.pendingHealthPings) {
        if (at - Number(sentAt || 0) > IHN_LIVE_HEALTH_TIMEOUT) {
            peer.pendingHealthPings.delete(sentAt);
        }
    }
    if (!ihnSendControl(peer, { t: 'ping', ...payload, at })) return 0;
    peer.pendingHealthPings.add(at);
    peer.lastPingAt = at;
    return at;
}

function ihnSanitizeLiveStrokePoints(points) {
    if (!Array.isArray(points)) return [];
    return points.map(point => ({
        x: Number(point?.x) || 0,
        y: Number(point?.y) || 0,
        p: Number.isFinite(Number(point?.p)) ? Number(point.p) : 0.5
    }));
}

function ihnSendRealtimeStrokePacket(packet, excludePeerId = '') {
    if (!packet || packet.fileId !== state?.driveFileId) return false;
    ihnEnsureTabChannel();
    try { ihnLiveBroadcastChannel?.postMessage(packet); } catch (error) {}
    let sent = false;
    ihnLivePeers.forEach((peer, peerId) => {
        if (peerId === excludePeerId) return;
        sent = ihnSendControl(peer, packet) || sent;
    });
    return sent;
}

function ihnFlushLiveStrokePreview(strokeId, options = {}) {
    const record = ihnLiveStrokeSends.get(String(strokeId || ''));
    if (!record || !record.latest) return false;
    clearTimeout(record.timer);
    record.timer = null;
    const latest = record.latest;
    const allPoints = ihnSanitizeLiveStrokePoints(latest.points);
    // Repeat the previous endpoint so quadratic rendering joins batches without
    // a visible seam. A final packet always carries the complete stroke and is
    // therefore independently recoverable after a dropped preview frame.
    const offset = options.final
        ? 0
        : Math.max(0, Math.min(allPoints.length, record.sentPoints) - 1);
    const basePacket = {
        v: 1, type: 'live-stroke', fileId: state?.driveFileId || '',
        actorId: `${ihnGetLivePeerId()}:${ihnGetLiveTabId()}`,
        strokeId: record.strokeId, pageId: record.pageId,
        tool: String(latest.tool || 'pen'), color: String(latest.color || '#111111'),
        width: Math.max(0.1, Number(latest.width) || 1), sentAt: Date.now()
    };
    if (!basePacket.fileId || !basePacket.pageId || !basePacket.strokeId) return false;
    if (options.cancel) {
        ihnSendRealtimeStrokePacket({
            ...basePacket, sequence: ++record.sequence, offset: 0,
            points: [], final: false, cancel: true
        });
    } else {
        // Keep every RTCDataChannel message comfortably below mobile browser
        // limits. Long strokes are split into ordered point ranges; the final
        // range carries the completion flag.
        let chunkOffset = offset;
        do {
            const chunkEnd = Math.min(allPoints.length, chunkOffset + IHN_LIVE_STROKE_MAX_POINTS);
            ihnSendRealtimeStrokePacket({
                ...basePacket,
                sequence: ++record.sequence,
                offset: chunkOffset,
                points: allPoints.slice(chunkOffset, chunkEnd),
                final: !!options.final && chunkEnd >= allPoints.length,
                cancel: false
            });
            chunkOffset = chunkEnd;
        } while (chunkOffset < allPoints.length);
    }
    record.sentPoints = allPoints.length;
    if (options.final || options.cancel) ihnLiveStrokeSends.delete(record.strokeId);
    return true;
}

function publishLiveStrokePreview(pageId, stroke, options = {}) {
    if (!state?.driveFileId || !ihnCanEditLiveDocument() || !stroke?.id || !pageId) return false;
    startLiveCollaboration();
    const strokeId = String(stroke.id);
    let record = ihnLiveStrokeSends.get(strokeId);
    if (!record) {
        record = { strokeId, pageId: String(pageId), sequence: 0, sentPoints: 0, latest: null, timer: null };
        ihnLiveStrokeSends.set(strokeId, record);
    }
    record.pageId = String(pageId);
    record.latest = {
        tool: stroke.tool,
        color: stroke.color,
        width: stroke.width,
        points: Array.isArray(stroke.points) ? stroke.points : []
    };
    if (options.final || options.cancel) {
        return ihnFlushLiveStrokePreview(strokeId, options);
    }
    if (!record.timer) {
        record.timer = setTimeout(() => ihnFlushLiveStrokePreview(strokeId), IHN_LIVE_STROKE_FRAME_MS);
    }
    return true;
}

function ihnHandleRealtimeStrokePacket(packet, transport = '', sourcePeerId = '') {
    if (!packet
        || packet.v !== 1
        || packet.type !== 'live-stroke'
        || packet.fileId !== state?.driveFileId
        || !packet.actorId
        || !packet.strokeId
        || !packet.pageId
        || !Array.isArray(packet.points)) return false;
    const ownActorId = `${ihnGetLivePeerId()}:${ihnGetLiveTabId()}`;
    if (packet.actorId === ownActorId) return true;
    const sequence = Number(packet.sequence) || 0;
    const key = `${packet.actorId}:${packet.strokeId}`;
    if (sequence <= Number(ihnLiveStrokeSeen.get(key) || 0)) return true;
    ihnLiveStrokeSeen.set(key, sequence);
    if (packet.final || packet.cancel) {
        setTimeout(() => {
            if (Number(ihnLiveStrokeSeen.get(key) || 0) === sequence) ihnLiveStrokeSeen.delete(key);
        }, 30_000);
    }
    if (typeof applyRemoteLiveStrokePreview === 'function') {
        applyRemoteLiveStrokePreview(packet);
    }
    // The peer graph can be temporarily non-meshed during recovery. Relay the
    // tiny preview packet so every connected collaborator still sees the pen.
    if (transport === 'webrtc') {
        ihnEnsureTabChannel();
        try { ihnLiveBroadcastChannel?.postMessage(packet); } catch (error) {}
    }
    ihnLivePeers.forEach((peer, peerId) => {
        if (transport === 'webrtc' && peerId === sourcePeerId) return;
        const originPeerId = String(packet.actorId).split(':')[0];
        if (peerId === originPeerId) return;
        ihnSendControl(peer, packet);
    });
    return true;
}

function ihnObservePeerDocumentHash(peerId, contentHash) {
    const peer = ihnLivePeers.get(peerId);
    const observedHash = String(contentHash || '');
    if (!peer || !observedHash) return false;
    peer.remoteCurrentHash = observedHash;
    // An ACK is evidence of a past delivery, not a permanent assertion of the
    // peer's current state. Once that peer advertises another document hash,
    // a later local return to the old hash must be delivered again.
    if (peer.lastAckedHash && peer.lastAckedHash !== observedHash) {
        peer.lastAckedHash = '';
    }
    if (peer.pendingAckHash && peer.pendingAckHash !== observedHash) {
        peer.pendingAckHash = '';
        peer.pendingAckAt = 0;
        peer.pendingAckReceivedHash = '';
        peer.pendingAckReceivedAt = 0;
    }
    return true;
}

function ihnConfigurePeer(peerId, pc, peer) {
    const updateConnectionState = () => {
        if (ihnLivePeers.get(peerId) !== peer || peer.closing) return;
        const connectionState = pc.connectionState || pc.iceConnectionState || 'connecting';
        peer.status = connectionState;
        if (connectionState === 'connected' || connectionState === 'completed') {
            peer.disconnectedAt = 0;
            peer.networkRecoveryProbeAt = 0;
            ihnRefreshPeerPath(peer).catch(() => {});
        } else if (connectionState === 'disconnected') {
            if (!peer.disconnectedAt) peer.disconnectedAt = Date.now();
            ihnProbePeerForFastRecovery(peerId, peer, 'ICE path disconnected');
        } else if (connectionState === 'failed' || connectionState === 'closed') {
            ihnClosePeer(peerId, connectionState, { retry: true });
        }
    };
    pc.onconnectionstatechange = updateConnectionState;
    pc.oniceconnectionstatechange = updateConnectionState;
    pc.onicecandidate = event => ihnQueueLocalIceCandidate(peerId, peer, event?.candidate || null);
    pc.ondatachannel = event => ihnConfigureChannel(peerId, event.channel, peer);
}

function ihnConfigureChannel(peerId, channel, peer) {
    peer.channel = channel;
    channel.bufferedAmountLowThreshold = 512_000;
    channel.onopen = () => {
        if (ihnLivePeers.get(peerId) !== peer || peer.closing) {
            try { channel.close(); } catch (error) {}
            return;
        }
        const now = Date.now();
        peer.status = 'open';
        peer.openedAt = now;
        peer.lastReceivedAt = now;
        peer.lastPongAt = now;
        peer.lastPongEchoAt = 0;
        peer.lastPingAt = 0;
        peer.pendingHealthPings = new Set();
        peer.disconnectedAt = 0;
        peer.lastSentHash = '';
        peer.pendingAckHash = '';
        peer.pendingAckAt = 0;
        peer.pendingAckReceivedHash = '';
        peer.pendingAckReceivedAt = 0;
        peer.resumeGraceUntil = 0;
        peer.sendQueue = Promise.resolve();
        ihnTouchKnownPeerFromChannel(peerId, now);
        ihnRefreshPeerPath(peer).catch(() => {});
        ihnDeleteSignalComment(peer);
        ihnSendControl(peer, { t: 'hello', at: now });
        ihnSendControl(peer, { t: 'state-request', at: now });
        // Delivery state is per peer. A newly recovered route always receives
        // the current document even when no user edit occurred during fallback.
        scheduleLiveDocumentBroadcast({ immediate: true, targetPeerId: peerId, force: true });
        if (typeof refreshPresenceViews === 'function') refreshPresenceViews();
    };
    channel.onclose = () => {
        if (!peer.closing && ihnLivePeers.get(peerId) === peer) {
            ihnClosePeer(peerId, 'data channel closed', { retry: true });
        }
    };
    channel.onerror = error => {
        console.warn('Live data channel error:', error);
        if (!peer.channelErrorAt) peer.channelErrorAt = Date.now();
    };
    channel.onmessage = event => {
        if (ihnLivePeers.get(peerId) !== peer
            || peer.channel !== channel
            || peer.closing
            || peer.fileId !== state?.driveFileId
            || peer.generation !== ihnLiveGeneration) {
            return;
        }
        ihnHandleWireMessage(
            event.data,
            peerId,
            peer,
            channel,
            ihnCaptureLiveOperationContext(peer.fileId)
        ).catch(error => console.warn('Live message rejected:', error));
    };
}

async function ihnCreateOffer(targetPeerId, options = {}) {
    if (!ihnIsLiveLeader() || !state.driveCanEdit || !driveAccessToken || !state.driveFileId) return;
    const ownId = ihnGetLivePeerId();
    if (!targetPeerId || ownId === targetPeerId || (!options.allowReverse && ownId >= targetPeerId)) return;
    const current = ihnLivePeers.get(targetPeerId);
    if (current && ['new', 'connecting', 'open'].includes(current.status)) return;
    if (current) ihnClosePeer(targetPeerId, 'reconnect', { retry: false });
    const fileId = state.driveFileId;
    const generation = ihnLiveGeneration;
    const pc = ihnCreatePeerConnection();
    const peer = { pc, channel: null, status: 'connecting', initiator: true,
        sessionId: `s${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`,
        commentId: '', createdAt: Date.now(), fileId, generation, closing: false,
        lastReceivedAt: 0, lastPongAt: 0, lastSentHash: '', lastAckedHash: '',
        remoteCurrentHash: '', pendingAckHash: '', pendingAckAt: 0,
        pendingAckReceivedHash: '', pendingAckReceivedAt: 0, disconnectedAt: 0,
        pendingLocalIceCandidates: [], localIceCandidateKeys: new Set(),
        pendingRemoteIceCandidates: new Map(), remoteIceCandidateKeys: new Set(),
        localIceFlushTimer: null, localIceFlushBusy: false, localIceRetryAttempt: 0,
        localIceRetryDelay: 0, localIceLastFailureAt: 0,
        signalledLocalSdp: '', lastSignalActivityAt: Date.now() };
    ihnTouchPeerSignalling(peer);
    ihnLivePeers.set(targetPeerId, peer);
    ihnConfigurePeer(targetPeerId, pc, peer);
    try {
        ihnConfigureChannel(targetPeerId, pc.createDataChannel('inhousenotes-live-v5', { ordered: true }), peer);
        await pc.setLocalDescription(await pc.createOffer());
        await ihnWaitForIce(pc, options.fastRecovery
            ? IHN_LIVE_FAST_ICE_GATHER_TIMEOUT
            : IHN_LIVE_ICE_GATHER_TIMEOUT);
        peer.signalledLocalSdp = String(pc.localDescription?.sdp || '');
        ihnAssertLiveSessionContext(fileId, generation);
        if (ihnLivePeers.get(targetPeerId) !== peer) {
            throw new Error('Live peer connection was replaced');
        }
        const content = await ihnEncodeSignal({ v: 1, type: 'offer', fileId,
            from: ownId, to: targetPeerId, sessionId: peer.sessionId,
            expiresAt: Date.now() + IHN_LIVE_SIGNAL_TTL,
            fastRecovery: !!options.fastRecovery,
            description: pc.localDescription });
        ihnAssertLiveSessionContext(fileId, generation);
        if (ihnLivePeers.get(targetPeerId) !== peer) {
            throw new Error('Live peer connection was replaced');
        }
        const response = await driveFetch(`https://www.googleapis.com/drive/v3/files/${fileId}/comments?fields=id,createdTime`, {
            method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ content })
        });
        const responseData = await response.json();
        if (ihnLivePeers.get(targetPeerId) !== peer
            || generation !== ihnLiveGeneration
            || state.driveFileId !== fileId) {
            const staleCommentId = responseData.id || '';
            if (staleCommentId && driveAccessToken) {
                driveFetch(`https://www.googleapis.com/drive/v3/files/${fileId}/comments/${staleCommentId}`, {
                    method: 'DELETE'
                }).catch(() => {});
            }
            return;
        }
        peer.commentId = responseData.id || '';
        ihnTouchPeerSignalling(peer);
        ihnScheduleLocalIceFlush(targetPeerId, peer, 0);
        ihnScheduleRapidSignalPolls('offer posted');
    } catch (error) {
        console.warn('Direct connection unavailable; Drive fallback remains active:', error);
        if (ihnLivePeers.get(targetPeerId) === peer) {
            ihnClosePeer(targetPeerId, 'offer failed', { retry: true });
        }
    }
}

async function ihnAcceptOffer(comment, offer) {
    const ownId = ihnGetLivePeerId();
    if (!offer || offer.to !== ownId || offer.from === ownId || offer.fileId !== state.driveFileId || Number(offer.expiresAt) < Date.now()) return;
    const offerKey = `${comment.id}:${offer.sessionId}`;
    if (ihnLiveProcessedOffers.has(offerKey) || ihnLiveOffersInFlight.has(offerKey)) return;
    const existingPeer = ihnLivePeers.get(offer.from);
    if (existingPeer?.status === 'open' && !offer.fastRecovery) {
        ihnLiveProcessedOffers.set(offerKey, Number(offer.expiresAt) || (Date.now() + IHN_LIVE_SIGNAL_TTL));
        return;
    }
    if (existingPeer
        && Date.now() - Number(existingPeer.createdAt || 0) < IHN_LIVE_CONNECT_TIMEOUT) {
        // Perfect negotiation without a signalling server: if simultaneous
        // offers cross (including fast network recovery), the lower peer's
        // offer wins. The other endpoint drops its local offer and answers the
        // deterministic winner instead of both endpoints discarding each other.
        const localOfferWins = !!existingPeer.initiator && ownId < offer.from;
        if (!existingPeer.initiator || localOfferWins) {
            ihnLiveProcessedOffers.set(offerKey, Number(offer.expiresAt) || (Date.now() + IHN_LIVE_SIGNAL_TTL));
            return;
        }
    }
    ihnLiveOffersInFlight.add(offerKey);
    ihnClosePeer(offer.from, 'new offer', { retry: false });
    const fileId = state.driveFileId;
    const generation = ihnLiveGeneration;
    const pc = ihnCreatePeerConnection();
    const peer = { pc, channel: null, status: 'connecting', initiator: false,
        sessionId: offer.sessionId, commentId: comment.id, createdAt: Date.now(),
        fileId, generation, closing: false,
        lastReceivedAt: 0, lastPongAt: 0, lastSentHash: '', lastAckedHash: '',
        remoteCurrentHash: '', pendingAckHash: '', pendingAckAt: 0,
        pendingAckReceivedHash: '', pendingAckReceivedAt: 0, disconnectedAt: 0,
        pendingLocalIceCandidates: [], localIceCandidateKeys: new Set(),
        pendingRemoteIceCandidates: new Map(), remoteIceCandidateKeys: new Set(),
        localIceFlushTimer: null, localIceFlushBusy: false, localIceRetryAttempt: 0,
        localIceRetryDelay: 0, localIceLastFailureAt: 0,
        signalledLocalSdp: '', lastSignalActivityAt: Date.now() };
    ihnTouchPeerSignalling(peer);
    ihnLivePeers.set(offer.from, peer);
    ihnConfigurePeer(offer.from, pc, peer);
    try {
        await pc.setRemoteDescription(offer.description);
        await ihnDrainRemoteIceCandidates(offer.from, peer);
        await pc.setLocalDescription(await pc.createAnswer());
        await ihnWaitForIce(pc, offer.fastRecovery
            ? IHN_LIVE_FAST_ICE_GATHER_TIMEOUT
            : IHN_LIVE_ICE_GATHER_TIMEOUT);
        peer.signalledLocalSdp = String(pc.localDescription?.sdp || '');
        ihnAssertLiveSessionContext(fileId, generation);
        if (ihnLivePeers.get(offer.from) !== peer) {
            throw new Error('Live peer connection was replaced');
        }
        const content = await ihnEncodeSignal({ v: 1, type: 'answer', fileId,
            from: ownId, to: offer.from, sessionId: offer.sessionId,
            expiresAt: Date.now() + IHN_LIVE_SIGNAL_TTL, description: pc.localDescription });
        ihnAssertLiveSessionContext(fileId, generation);
        if (ihnLivePeers.get(offer.from) !== peer) {
            throw new Error('Live peer connection was replaced');
        }
        await driveFetch(`https://www.googleapis.com/drive/v3/files/${fileId}/comments/${comment.id}/replies?fields=id,createdTime`, {
            method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ content })
        });
        ihnTouchPeerSignalling(peer);
        ihnScheduleLocalIceFlush(offer.from, peer, 0);
        ihnLiveProcessedOffers.set(offerKey, Number(offer.expiresAt) || (Date.now() + IHN_LIVE_SIGNAL_TTL));
    } catch (error) {
        console.warn('Direct offer could not be accepted:', error);
        if (ihnLivePeers.get(offer.from) === peer) {
            ihnClosePeer(offer.from, 'answer failed', { retry: false });
        }
    } finally {
        ihnLiveOffersInFlight.delete(offerKey);
    }
}

async function ihnApplyAnswer(answer) {
    const ownId = ihnGetLivePeerId();
    if (!answer || answer.type !== 'answer' || answer.to !== ownId || answer.fileId !== state.driveFileId || Number(answer.expiresAt) < Date.now()) return;
    const peer = ihnLivePeers.get(answer.from);
    if (!peer?.initiator || peer.sessionId !== answer.sessionId || peer.pc.remoteDescription) return;
    const operation = ihnCaptureLiveOperationContext(peer.fileId);
    try {
        await peer.pc.setRemoteDescription(answer.description);
        await ihnDrainRemoteIceCandidates(answer.from, peer);
        ihnAssertLiveOperationContext(operation);
        if (ihnLivePeers.get(answer.from) !== peer) return;
    } catch (error) {
        if (ihnLiveOperationContextIsCurrent(operation)
            && ihnLivePeers.get(answer.from) === peer) {
            ihnClosePeer(answer.from, 'invalid answer', { retry: true });
        }
    }
}

async function ihnPollSignals() {
    if (ihnLiveSignalBusy) {
        // A recovery burst often lands while Drive is still returning the
        // previous comments page. Keep one trailing poll so a fresh answer is
        // consumed immediately instead of waiting for the periodic interval.
        ihnLiveSignalQueued = true;
        return;
    }
    if (!ihnIsLiveLeader()
        || !state.driveCanEdit
        || !driveAccessToken
        || !state.driveFileId
        || document.visibilityState === 'hidden'
        || (typeof navigator !== 'undefined' && navigator.onLine === false)) return;
    const runId = ++ihnLiveSignalRunId;
    ihnLiveSignalBusy = true;
    ihnLiveSignalQueued = false;
    const fileId = state.driveFileId;
    const generation = ihnLiveGeneration;
    const operation = ihnCaptureLiveOperationContext(fileId);
    try {
        await ihnEnsureSignalKey();
        ihnAssertLiveSessionContext(fileId, generation);
        ihnAssertLiveOperationContext(operation);
        const now = Date.now();
        ihnLiveProcessedOffers.forEach((expiresAt, key) => {
            if (Number(expiresAt) < now) ihnLiveProcessedOffers.delete(key);
        });
        const ownId = ihnGetLivePeerId();
        const fields = encodeURIComponent('nextPageToken,comments(id,content,createdTime,resolved,replies(id,content,createdTime))');
        // Drive can contain years of ordinary comments. Ask only for the
        // recent signalling window, then follow every returned page so fresh
        // offers cannot be hidden by unrelated or stale discussion threads.
        const startModifiedTime = encodeURIComponent(
            new Date(now - (IHN_LIVE_SIGNAL_TTL * 4)).toISOString()
        );
        let pageToken = '';
        const requestedPageTokens = new Set();
        while (true) {
            if (requestedPageTokens.has(pageToken)) {
                throw new Error('Drive comments returned a repeated page token');
            }
            requestedPageTokens.add(pageToken);
            const tokenParam = pageToken ? `&pageToken=${encodeURIComponent(pageToken)}` : '';
            const response = await driveFetch(`https://www.googleapis.com/drive/v3/files/${fileId}/comments?pageSize=100&includeDeleted=false&startModifiedTime=${startModifiedTime}&fields=${fields}${tokenParam}`, {
                method: 'GET', timeout: DRIVE_POLL_TIMEOUT, headers: { 'Cache-Control': 'no-cache' }
            });
            const data = await response.json();
            ihnAssertLiveSessionContext(fileId, generation);
            ihnAssertLiveOperationContext(operation);
            for (const comment of data.comments || []) {
                if (!String(comment?.content || '').startsWith(IHN_LIVE_SIGNAL_PREFIX)) continue;
                const offer = await ihnDecodeSignal(comment.content);
                ihnAssertLiveSessionContext(fileId, generation);
                ihnAssertLiveOperationContext(operation);
                if (!offer || offer.v !== 1 || offer.type !== 'offer') continue;
                if (Number(offer.expiresAt) < now) {
                    if (offer.from === ownId || offer.to === ownId) {
                        driveFetch(`https://www.googleapis.com/drive/v3/files/${fileId}/comments/${comment.id}`, {
                            method: 'DELETE'
                        }).catch(() => {});
                    }
                    continue;
                }
                if (offer.to === ownId) await ihnAcceptOffer(comment, offer);
                for (const reply of comment.replies || []) {
                    const signal = await ihnDecodeSignal(reply.content);
                    if (!signal) continue;
                    if (signal.type === 'answer') await ihnApplyAnswer(signal);
                    else if (signal.type === 'candidates') await ihnApplyCandidateSignal(signal);
                }
            }
            pageToken = String(data.nextPageToken || '');
            if (!pageToken) break;
        }
    } catch (error) {
        console.warn('Live signalling unavailable; continuing with Drive sync:', error);
    } finally {
        if (runId === ihnLiveSignalRunId) {
            ihnLiveSignalBusy = false;
            if (ihnLiveSignalQueued) {
                ihnLiveSignalQueued = false;
                setTimeout(() => ihnPollSignals(), 0);
            }
        }
    }
}

function liveCollabUpdatePeers(users) {
    if (!state?.driveFileId || !driveAccessToken || !state.driveCanEdit) return;
    startLiveCollaboration();
    const ownId = ihnGetLivePeerId();
    const active = new Set();
    const now = Date.now();
    for (const user of users || []) {
        if (!user?.isOnline) continue;
        const peerId = ihnGetPresencePeerId(user);
        if (!peerId || peerId === ownId) continue;
        active.add(peerId);
        const wasExpected = ihnPeerIsExpected(peerId);
        ihnLiveKnownPeers.set(peerId, {
            lastSeenAt: now,
            record: user
        });
        if (!wasExpected) {
            ihnSchedulePeerRetry(peerId, 'peer online', {
                immediate: true,
                preserveFailures: true,
                // The device that first discovers the other device must be
                // allowed to offer regardless of lexical peer order. Glare is
                // resolved deterministically in ihnAcceptOffer.
                allowReverse: true
            });
        }
    }
    ihnLiveMainPeerId = [...active, ownId].sort()[0] || ownId;
    ihnSuperviseConnections({ immediate: true });
}

function ihnSuperviseConnections(options = {}) {
    if (!state?.driveFileId || !driveAccessToken || !state.driveCanEdit || typeof RTCPeerConnection === 'undefined') return;
    ihnCheckNetworkRouteSignature();
    if (typeof navigator !== 'undefined' && navigator.onLine === false) return;
    const now = Date.now();
    if (!ihnIsLiveLeader() && !ihnClaimLiveLeader()) {
        // Only the elected tab owns network peers. Other tabs remain fully
        // current through BroadcastChannel and can take over after the lease.
        [...ihnLivePeers.keys()].forEach(peerId => {
            ihnClosePeer(peerId, 'coordination moved to another tab', { retry: false });
        });
        return;
    }

    ihnLiveKnownPeers.forEach((known, peerId) => {
        if (now - ihnKnownPeerLastSeenAt(known) <= IHN_LIVE_PEER_EXPIRY) return;
        const peer = ihnLivePeers.get(peerId);
        if (ihnPeerChannelIsHealthy(peer, now)) {
            if (!known.presenceExpiredAt) known.presenceExpiredAt = now;
            return;
        }
        ihnLiveKnownPeers.delete(peerId);
        ihnLiveRetryState.delete(peerId);
        ihnClosePeer(peerId, 'presence expired', { retry: false });
    });

    ihnLivePeers.forEach((peer, peerId) => {
        if (!ihnPeerIsExpected(peerId) && !ihnPeerChannelIsHealthy(peer, now)) {
            ihnClosePeer(peerId, 'presence expired', { retry: false });
            return;
        }
        if (peer.generation !== ihnLiveGeneration || peer.fileId !== state.driveFileId) {
            ihnClosePeer(peerId, 'stale document connection', { retry: false });
            return;
        }
        const channelOpen = peer.channel?.readyState === 'open';
        const connectionDeadlineAt = Number(peer.connectionDeadlineAt)
            || (Number(peer.createdAt || 0) + IHN_LIVE_CONNECT_TIMEOUT);
        if (!channelOpen && now > connectionDeadlineAt) {
            ihnClosePeer(peerId, 'connection timeout', { retry: true });
            return;
        }
        if (!channelOpen
            && peer.pendingLocalIceCandidates?.length
            && !peer.localIceFlushBusy
            && !peer.localIceFlushTimer
            && now - Number(peer.localIceLastFailureAt || 0) > 5000) {
            peer.localIceRetryAttempt = 0;
            ihnScheduleLocalIceFlush(peerId, peer, 0);
        }
        const resumeGraceActive = now < Number(peer.resumeGraceUntil || 0);
        if (!resumeGraceActive
            && peer.disconnectedAt
            && now - peer.disconnectedAt > IHN_LIVE_DISCONNECTED_GRACE) {
            ihnClosePeer(peerId, 'connection interrupted', { retry: true });
            return;
        }
        if (!channelOpen) return;
        const pendingAckAge = peer.pendingAckHash
            ? now - Number(peer.pendingAckAt || 0)
            : 0;
        const applyReceiptMatches = !!(
            peer.pendingAckHash
            && peer.pendingAckReceivedHash === peer.pendingAckHash
            && Number(peer.pendingAckReceivedAt || 0) >= Number(peer.pendingAckAt || 0)
        );
        const hasRecentVerifiedPong = Number(peer.lastPongAt || 0) > Number(peer.openedAt || 0)
            && now - Number(peer.lastPongAt || 0) <= IHN_LIVE_HEALTH_TIMEOUT;
        const remoteApplyInProgress = pendingAckAge > IHN_LIVE_ACK_TIMEOUT
            && pendingAckAge <= IHN_LIVE_APPLY_ACK_TIMEOUT
            && applyReceiptMatches;
        if (peer.protocolV2
            && !resumeGraceActive
            && now - Number(peer.lastReceivedAt || peer.openedAt || 0) > IHN_LIVE_HEALTH_TIMEOUT
            && !remoteApplyInProgress) {
            ihnClosePeer(peerId, 'health check timeout', { retry: true });
            return;
        }
        if (peer.protocolV2
            && peer.pendingAckHash
            && pendingAckAge > IHN_LIVE_ACK_TIMEOUT) {
            if (pendingAckAge > IHN_LIVE_APPLY_ACK_TIMEOUT && hasRecentVerifiedPong) {
                // The path itself is healthy, so replacing it would create a
                // reconnect loop. Drop only the stale delivery attempt and
                // resend the newest full state through the same channel.
                peer.pendingAckHash = '';
                peer.pendingAckAt = 0;
                peer.pendingAckReceivedHash = '';
                peer.pendingAckReceivedAt = 0;
                scheduleLiveDocumentBroadcast({
                    immediate: true,
                    targetPeerId: peerId,
                    force: true
                });
                return;
            }
            if (pendingAckAge > IHN_LIVE_APPLY_ACK_TIMEOUT
                || (!applyReceiptMatches && !hasRecentVerifiedPong)) {
                ihnClosePeer(peerId, 'snapshot acknowledgement timeout', { retry: true });
                return;
            }
        }
        if (!peer.protocolV2
            && now - Number(peer.lastReceivedAt || peer.openedAt || 0) > IHN_LIVE_LEGACY_PROBE_TIMEOUT) {
            ihnClosePeer(peerId, 'legacy peer health probe', { retry: true });
            return;
        }
        const routeLastSeenAt = Math.max(
            Number(peer.lastPongAt || 0),
            Number(peer.lastReceivedAt || 0),
            Number(peer.openedAt || 0)
        );
        if (peer.protocolV2
            && document.visibilityState !== 'hidden'
            && !resumeGraceActive
            && now - routeLastSeenAt > IHN_LIVE_ROUTE_STALE_MS) {
            // Some mobile browsers never emit navigator.connection.change when
            // moving between Wi-Fi and cellular. The fast heartbeat is the
            // portable route-change detector and replaces the dead path before
            // the browser's multi-second ICE timeout.
            ihnClosePeer(peerId, 'live route watchdog', {
                retry: true,
                immediate: true,
                preserveFailures: true,
                allowReverse: true
            });
            return;
        }
        if (now - Number(peer.lastPingAt || 0) >= IHN_LIVE_PING_INTERVAL) {
            ihnSendHealthPing(peer);
        }
        if (now - Number(peer.lastPathRefreshAt || 0) > 15_000) {
            peer.lastPathRefreshAt = now;
            ihnRefreshPeerPath(peer).catch(() => {});
        }
    });

    ihnLiveKnownPeers.forEach((known, peerId) => {
        if (ihnLivePeers.has(peerId)) return;
        const retry = ihnGetRetryState(peerId);
        const deterministicInitiator = ihnCanInitiatePeer(peerId);
        const reverseRecovery = !!retry.allowReverse && ihnCanRecoverPeer(peerId);
        if (!deterministicInitiator && !reverseRecovery) return;
        if (Number(retry.nextAttemptAt || 0) > now) return;
        // Reserve a future slot immediately; the peer map itself prevents a
        // second in-flight offer once ihnCreateOffer starts.
        retry.nextAttemptAt = now + IHN_LIVE_CONNECT_TIMEOUT;
        const fastRecovery = /network|route|ICE path|browser/i.test(String(retry.lastReason || ''));
        ihnCreateOffer(peerId, { fastRecovery, allowReverse: reverseRecovery }).catch(error => {
            console.warn('Live reconnect attempt failed:', error);
            ihnSchedulePeerRetry(peerId, 'reconnect failed');
        });
    });
    if (options.immediate) ihnPollSignals();
}

function ihnWakeLiveCollaboration(reason = 'network available') {
    if (!state?.driveFileId) return;
    const now = Date.now();
    const browserOffline = reason === 'browser offline';
    const networkRouteChanged = /network|browser online/i.test(String(reason));
    const replaceRouteImmediately = reason === 'network transport changed'
        || reason === 'browser online';
    if (browserOffline) {
        ihnLivePeers.forEach((_peer, peerId) => {
            ihnClosePeer(peerId, reason, {
                retry: true,
                immediate: true,
                preserveFailures: true,
                allowReverse: true
            });
        });
        return;
    }
    ihnLivePeers.forEach((peer, peerId) => {
        if (networkRouteChanged) {
            peer.localIceRetryAttempt = 0;
            ihnScheduleLocalIceFlush(peerId, peer, 0);
            if (peer.channel?.readyState === 'open' && !replaceRouteImmediately) {
                ihnProbePeerForFastRecovery(peerId, peer, reason);
            } else {
                ihnClosePeer(peerId, reason, {
                    retry: true,
                    immediate: true,
                    preserveFailures: true
                });
            }
            return;
        }
        if (peer.channel?.readyState !== 'open') return;
        // Background tabs and mobile WebViews can suspend timers and sockets
        // without emitting a useful WebRTC state transition. Give the resumed
        // channel one fresh ping round-trip before classifying it as stale.
        peer.resumeGraceUntil = now + IHN_LIVE_RESUME_GRACE_MS;
        peer.disconnectedAt = 0;
        ihnTouchKnownPeerFromChannel(peerId, now);
        ihnSendHealthPing(peer, { reason });
    });
    ihnLiveRetryState.forEach((retry, peerId) => {
        if (!ihnCanInitiatePeer(peerId)
            && !(retry.allowReverse && ihnCanRecoverPeer(peerId))) return;
        retry.nextAttemptAt = now;
        retry.lastReason = reason;
    });
    ihnSuperviseConnections({ immediate: true });
    if (networkRouteChanged) {
        ihnScheduleRapidSignalPolls(reason);
    } else {
        setTimeout(() => ihnSuperviseConnections({ immediate: true }), 1200);
    }
}

function ihnScheduleRapidSignalPolls(reason = 'network recovery') {
    const fileId = state?.driveFileId || '';
    const generation = ihnLiveGeneration;
    [40, 120, 260, 480, 800].forEach(delay => {
        setTimeout(() => {
            if (generation !== ihnLiveGeneration || state?.driveFileId !== fileId) return;
            ihnLiveRetryState.forEach((retry, peerId) => {
                const mayInitiate = ihnCanInitiatePeer(peerId)
                    || (retry.allowReverse && ihnCanRecoverPeer(peerId));
                if (!mayInitiate || ihnLivePeers.has(peerId)) return;
                retry.nextAttemptAt = Date.now();
                retry.lastReason = reason;
            });
            ihnSuperviseConnections({ immediate: true });
        }, delay);
    });
}

function ihnProbePeerForFastRecovery(peerId, peer, reason = 'network path changed') {
    if (!peer
        || ihnLivePeers.get(peerId) !== peer
        || peer.closing
        || peer.channel?.readyState !== 'open') return false;
    const now = Date.now();
    if (now - Number(peer.networkRecoveryProbeAt || 0) < IHN_LIVE_NETWORK_PROBE_MS) return true;
    const knownRtt = Number(peer.rttMs);
    const probeDelay = Number.isFinite(knownRtt) && knownRtt > 0
        ? Math.min(700, Math.max(IHN_LIVE_NETWORK_PROBE_MS, Math.round((knownRtt * 2.5) + 80)))
        : 320;
    peer.networkRecoveryProbeAt = now;
    peer.resumeGraceUntil = now + Math.max(IHN_LIVE_NETWORK_RECOVERY_GRACE_MS, probeDelay + 180);
    ihnTouchKnownPeerFromChannel(peerId, now);
    const probeAt = ihnSendHealthPing(peer, { reason, networkProbe: true });
    if (!probeAt) {
        ihnClosePeer(peerId, reason, {
            retry: true,
            immediate: true,
            preserveFailures: true,
            allowReverse: true
        });
        ihnSuperviseConnections({ immediate: true });
        return true;
    }
    setTimeout(() => {
        if (ihnLivePeers.get(peerId) !== peer || peer.closing) return;
        if (Number(peer.lastPongEchoAt || 0) >= probeAt) {
            peer.networkRecoveryProbeAt = 0;
            peer.resumeGraceUntil = 0;
            scheduleLiveDocumentBroadcast({ immediate: true, targetPeerId: peerId, force: true });
            return;
        }
        // The old candidate pair did not survive the route change. Replace it
        // immediately instead of waiting for the normal 5–20 second watchdog.
        ihnClosePeer(peerId, reason, {
            retry: true,
            immediate: true,
            preserveFailures: true,
            allowReverse: true
        });
        ihnSuperviseConnections({ immediate: true });
    }, probeDelay);
    return true;
}

let ihnLiveLifecycleListenersInstalled = false;
function ihnReadNetworkSignature() {
    try {
        const connection = navigator.connection || navigator.mozConnection || navigator.webkitConnection;
        if (!connection) return '';
        return [
            String(connection.type || ''),
            String(connection.effectiveType || ''),
            connection.saveData ? 'save' : 'normal'
        ].join('|');
    } catch (error) {
        return '';
    }
}

function ihnHandleNetworkRouteChange(reason = 'network changed') {
    const now = Date.now();
    const previousSignature = ihnLiveNetworkSignature;
    const nextSignature = ihnReadNetworkSignature();
    ihnLiveNetworkSignature = nextSignature;
    if (reason === 'network changed'
        && previousSignature
        && nextSignature
        && previousSignature === nextSignature) return false;
    if (reason !== 'browser online' && now - ihnLiveNetworkWakeAt < 120) return false;
    ihnLiveNetworkWakeAt = now;
    const previousTransport = String(previousSignature || '').split('|')[0];
    const nextTransport = String(nextSignature || '').split('|')[0];
    const confirmedTransportChange = previousTransport
        && nextTransport
        && previousTransport !== nextTransport;
    ihnWakeLiveCollaboration(confirmedTransportChange ? 'network transport changed' : reason);
    return true;
}

function ihnCheckNetworkRouteSignature() {
    const next = ihnReadNetworkSignature();
    if (!next) return false;
    if (!ihnLiveNetworkSignature) {
        ihnLiveNetworkSignature = next;
        return false;
    }
    if (next === ihnLiveNetworkSignature) return false;
    return ihnHandleNetworkRouteChange('network signature changed');
}

function ihnInstallLiveLifecycleListeners() {
    if (ihnLiveLifecycleListenersInstalled || typeof window === 'undefined') return;
    ihnLiveLifecycleListenersInstalled = true;
    ihnLiveNetworkSignature = ihnReadNetworkSignature();
    window.addEventListener('online', () => ihnHandleNetworkRouteChange('browser online'));
    window.addEventListener('offline', () => ihnHandleNetworkRouteChange('browser offline'));
    window.addEventListener('pageshow', () => ihnWakeLiveCollaboration('page resumed'));
    document.addEventListener('visibilitychange', () => {
        if (document.visibilityState === 'visible') ihnWakeLiveCollaboration('app visible');
    });
    try {
        const connection = navigator.connection || navigator.mozConnection || navigator.webkitConnection;
        connection?.addEventListener?.('change', () => ihnHandleNetworkRouteChange('network changed'));
    } catch (error) { /* optional Network Information API */ }
}

async function ihnRefreshPeerPath(peer) {
    if (!peer?.pc || typeof peer.pc.getStats !== 'function') return;
    try {
        const stats = await peer.pc.getStats();
        let selectedPair = null;
        stats.forEach(report => {
            if (report.type === 'transport' && report.selectedCandidatePairId) {
                selectedPair = stats.get(report.selectedCandidatePairId) || selectedPair;
            } else if (report.type === 'candidate-pair'
                && report.state === 'succeeded'
                && (report.nominated || report.selected)) {
                selectedPair = report;
            }
        });
        if (!selectedPair) return;
        const local = stats.get(selectedPair.localCandidateId);
        const remote = stats.get(selectedPair.remoteCandidateId);
        peer.networkPath = local?.candidateType === 'host' && remote?.candidateType === 'host'
            ? 'local-network'
            : 'peer';
        if (typeof refreshPresenceViews === 'function') refreshPresenceViews();
    } catch (error) {
        peer.networkPath = peer.networkPath || 'peer';
    }
}

function getLiveCollaborationConnectionInfo(record = null) {
    const ownId = ihnGetLivePeerId();
    const own = !record || String(record?.c || '') === String(getPresenceClientId());
    const peerId = own ? ownId : ihnGetPresencePeerId(record);
    const peer = peerId ? ihnLivePeers.get(peerId) : null;
    const sameDevice = !own
        && String(record?.c || '')
        && String(record.c) === String(getPresenceClientId());
    let transport = 'Drive fallback';
    let transportCode = 'drive';
    if (own) {
        transport = ihnIsLiveLeader() ? 'This device · coordination leader' : 'This device · local tab';
        transportCode = 'device';
    } else if (sameDevice) {
        transport = 'Same device';
        transportCode = 'local-tab';
    } else if (peer?.channel?.readyState === 'open') {
        transportCode = peer.networkPath === 'local-network' ? 'local-network' : 'peer';
        transport = transportCode === 'local-network' ? 'Local network · peer-to-peer' : 'Peer-to-peer';
    } else if (!own && ihnPeerIsExpected(peerId)) {
        transport = 'Reconnecting peer-to-peer · Drive active';
    }
    const mainPeerId = ihnLiveMainPeerId || ownId;
    return {
        peerId,
        transport,
        transportCode,
        isMain: peerId === mainPeerId,
        connected: own || sameDevice || peer?.channel?.readyState === 'open',
        recovering: !own && !sameDevice && ihnPeerIsExpected(peerId) && peer?.channel?.readyState !== 'open',
        rttMs: Number.isFinite(peer?.rttMs) ? peer.rttMs : null
    };
}

function getLiveCollaborationOverview() {
    const peers = [...ihnLivePeers.values()];
    const openPeers = peers.filter(peer => peer.channel?.readyState === 'open');
    const expectedPeerCount = [...ihnLiveKnownPeers.keys()].filter(ihnPeerIsExpected).length;
    const connectingPeerCount = peers.filter(peer => peer.channel?.readyState !== 'open').length;
    const bestRttMs = openPeers.reduce((best, peer) => {
        const rtt = Number(peer?.rttMs);
        return Number.isFinite(rtt) ? Math.min(best, rtt) : best;
    }, Infinity);
    const connectionState = openPeers.length > 0
        ? 'live'
        : ((expectedPeerCount > 0 || connectingPeerCount > 0 || ihnLiveSignalBusy || ihnLiveSignalQueued)
            ? 'reconnecting'
            : 'solo');
    return {
        ownPeerId: ihnGetLivePeerId(),
        mainPeerId: ihnLiveMainPeerId || ihnGetLivePeerId(),
        isMain: (ihnLiveMainPeerId || ihnGetLivePeerId()) === ihnGetLivePeerId(),
        openPeerCount: openPeers.length,
        expectedPeerCount,
        connectingPeerCount,
        connectionState,
        signallingBusy: ihnLiveSignalBusy || ihnLiveSignalQueued,
        bestRttMs: Number.isFinite(bestRttMs) ? bestRttMs : null
    };
}

async function ihnVerifyLiveCapability() {
    if (!state?.driveFileId || !driveAccessToken || !state.driveCanEdit) return;
    const operation = ihnCaptureLiveOperationContext();
    try {
        const response = await driveFetch(`https://www.googleapis.com/drive/v3/files/${operation.fileId}?fields=capabilities(canEdit,canModifyContent),trashed`, {
            method: 'GET', timeout: DRIVE_META_TIMEOUT, headers: { 'Cache-Control': 'no-cache' }
        });
        ihnAssertLiveOperationContext(operation);
        const data = await response.json();
        ihnAssertLiveOperationContext(operation);
        if (data.trashed || data.capabilities?.canEdit === false || data.capabilities?.canModifyContent === false) {
            state.driveCanEdit = false; stopLiveCollaboration(); setReadOnlyMode(true, { force: true });
            showStatus('Editing access was removed. Switched to view only.', { error: true });
        }
    } catch (error) {
        if (error?.code === 'IHN_LIVE_STALE_CONTEXT') return;
        // A failed capability probe is not proof that access was revoked:
        // transient Drive/rate-limit responses can also be 403/404. Keep the
        // supervisor alive and retry; only an explicit successful capabilities
        // response above may downgrade the document.
        console.warn('Live capability check deferred; keeping P2P recovery active:', error);
    }
}

function startLiveCollaboration() {
    if (!state?.driveFileId) return;
    ihnEnsureTabChannel();
    if (!driveAccessToken || !state.driveCanEdit || typeof RTCPeerConnection === 'undefined') return;
    ihnInstallLiveLifecycleListeners();
    if (!ihnLiveLeaderTimer) {
        ihnClaimLiveLeader();
        ihnLiveLeaderTimer = setInterval(() => ihnClaimLiveLeader(), 1000);
    }
    if (!ihnLiveSignalTimer) {
        ihnLiveSignalTimer = setInterval(ihnPollSignals, IHN_LIVE_SIGNAL_POLL_MS);
    }
    if (!ihnLiveCapabilityTimer) ihnLiveCapabilityTimer = setInterval(ihnVerifyLiveCapability, 30_000);
    if (!ihnLiveSupervisorTimer) {
        ihnLiveSupervisorTimer = setInterval(ihnSuperviseConnections, IHN_LIVE_SUPERVISOR_MS);
    }
    ihnSuperviseConnections({ immediate: true });
    ihnPollSignals();
}

async function flushLiveCollaborationBeforeExit(timeoutMs = 900) {
    if (!state?.driveFileId || !ihnCanEditLiveDocument()) return true;
    const deadline = Date.now() + Math.max(120, Number(timeoutMs) || 900);
    scheduleLiveDocumentBroadcast({ immediate: true, force: true });
    while (Date.now() < deadline) {
        if (!ihnLiveBroadcastBusy
            && !ihnLiveApplying
            && ihnLiveBroadcastQueued
            && !hasSmoothInteraction()) {
            try { await ihnBroadcastDocument(); } catch (error) { /* Drive fallback remains authoritative */ }
        }
        const deliveries = [...ihnLivePeers.values()]
            .map(peer => peer.sendQueue)
            .filter(Boolean);
        if (!ihnLiveBroadcastBusy && !ihnLiveBroadcastQueued) {
            await Promise.allSettled([...deliveries, ...ihnLiveFanOutTasks]);
            return true;
        }
        await new Promise(resolve => setTimeout(resolve, 24));
    }
    return false;
}

function stopLiveCollaboration() {
    ihnLiveGeneration += 1;
    ihnLiveSignalRunId += 1;
    ihnLiveBroadcastRunId += 1;
    clearTimeout(ihnLiveBroadcastTimer); ihnLiveBroadcastTimer = null;
    if (ihnLiveLeaderTimer) clearInterval(ihnLiveLeaderTimer);
    if (ihnLiveSignalTimer) clearInterval(ihnLiveSignalTimer);
    if (ihnLiveCapabilityTimer) clearInterval(ihnLiveCapabilityTimer);
    if (ihnLiveSupervisorTimer) clearInterval(ihnLiveSupervisorTimer);
    ihnLiveLeaderTimer = ihnLiveSignalTimer = ihnLiveCapabilityTimer = ihnLiveSupervisorTimer = null;
    ihnLiveSignalBusy = false;
    ihnLiveSignalQueued = false;
    ihnLiveBroadcastBusy = false;
    ihnLiveBroadcastQueued = false;
    if (!ihnLiveActiveApply) ihnLiveApplying = false;
    ihnCancelPendingLiveApplies();
    ihnLiveBroadcastForce = false; ihnLiveBroadcastTargets.clear(); ihnLiveBroadcastExclusions.clear();
    ihnLiveBroadcastRetryAttempt = 0;
    [...ihnLivePeers.keys()].forEach(peerId => ihnClosePeer(peerId, 'document closed', { retry: false }));
    ihnLiveChunks.clear(); ihnLiveSeen.clear(); ihnLiveAppliedHashes.clear(); ihnLiveProcessedOffers.clear();
    ihnLiveOffersInFlight.clear();
    ihnLiveFailedSignals.clear();
    ihnLiveStrokeSends.forEach(record => clearTimeout(record.timer));
    ihnLiveStrokeSends.clear(); ihnLiveStrokeSeen.clear();
    if (typeof clearRemoteLiveStrokePreviews === 'function') clearRemoteLiveStrokePreviews();
    ihnLiveFailedSignalRefreshAt = 0; ihnLiveFailedSignalRefreshKey = '';
    ihnLiveKnownPeers.clear(); ihnLiveRetryState.clear();
    ihnLiveSnapshotCache.clear();
    try { ihnLiveBroadcastChannel?.close(); } catch (error) {}
    ihnLiveBroadcastChannel = null; ihnLiveBroadcastFileId = '';
    ihnInvalidateSignalKey();
    ihnLiveCryptoKeyLoad = null; ihnLiveCryptoKeyLoadFileId = '';
    ihnLiveCryptoKeyRefresh = null; ihnLiveCryptoKeyRefreshFileId = '';
    ihnLiveLastAppliedHash = ''; ihnLiveCurrentHash = ''; ihnLiveLastTabSentHash = '';
    ihnLiveMergeUploadGuard = null; ihnLiveMainPeerId = '';
    ihnLiveNetworkSignature = ''; ihnLiveNetworkWakeAt = 0;
}

function ihnSerializeSnapshotPage(page) {
    try {
        return typeof ihnStableStringify === 'function'
            ? ihnStableStringify(page)
            : JSON.stringify(page);
    } catch (error) {
        return JSON.stringify(page);
    }
}

function ihnRememberLiveSnapshot(snapshot) {
    const hash = String(snapshot?.contentHash || '');
    if (!hash || !Array.isArray(snapshot.pages)) return null;
    const pagePayloads = new Map();
    let pageChars = 0;
    snapshot.pages.forEach((page, index) => {
        const pageId = String(page?.pageId || `legacy-page-${index + 1}`);
        const serialized = ihnSerializeSnapshotPage(page);
        pagePayloads.set(pageId, serialized);
        pageChars += serialized.length;
    });
    const record = { pagePayloads, pageChars };
    ihnLiveSnapshotCache.delete(hash);
    ihnLiveSnapshotCache.set(hash, record);
    const cachedChars = () => [...ihnLiveSnapshotCache.values()]
        .reduce((total, entry) => total + Number(entry?.pageChars || 0), 0);
    while (ihnLiveSnapshotCache.size > IHN_LIVE_SNAPSHOT_CACHE_LIMIT
        || (ihnLiveSnapshotCache.size > 1
            && cachedChars() > IHN_LIVE_SNAPSHOT_CACHE_MAX_CHARS)) {
        const oldestHash = ihnLiveSnapshotCache.keys().next().value;
        if (oldestHash === undefined) break;
        ihnLiveSnapshotCache.delete(oldestHash);
    }
    return record;
}

function ihnBuildPeerSnapshot(snapshot, peer, forceFull = false) {
    const current = ihnLiveSnapshotCache.get(String(snapshot?.contentHash || ''))
        || ihnRememberLiveSnapshot(snapshot);
    if (!current || forceFull) return snapshot;
    const baseHash = [peer?.lastAckedHash, peer?.remoteCurrentHash]
        .map(value => String(value || ''))
        .find(hash => hash && hash !== snapshot.contentHash && ihnLiveSnapshotCache.has(hash));
    if (!baseHash) return snapshot;
    const base = ihnLiveSnapshotCache.get(baseHash);
    const changedPages = [];
    let changedChars = 0;
    snapshot.pages.forEach((page, index) => {
        const pageId = String(page?.pageId || `legacy-page-${index + 1}`);
        const serialized = current.pagePayloads.get(pageId) || ihnSerializeSnapshotPage(page);
        if (base.pagePayloads.get(pageId) === serialized) return;
        changedPages.push(page);
        changedChars += serialized.length;
    });
    // Full snapshots remain the recovery format. Use a delta only when it is
    // materially smaller and the peer has explicitly acknowledged its base.
    if (changedPages.length === snapshot.pages.length
        || changedChars + 2048 >= Math.max(1, current.pageChars) * 0.8) {
        return snapshot;
    }
    return {
        ...snapshot,
        partial: true,
        baseHash,
        pages: changedPages
    };
}

async function ihnBuildLiveSnapshot(operation = ihnCaptureLiveOperationContext()) {
    let structureToken = null;
    try {
        ihnAssertLiveOperationContext(operation);
        structureToken = typeof acquireRemotePageMerge === 'function'
            ? await acquireRemotePageMerge()
            : {};
        ihnAssertLiveOperationContext(operation);
        if (!structureToken) {
            throw new Error('Page structure remained busy while building a live snapshot');
        }
        await flushStrokeOpsQueue();
        ihnAssertLiveOperationContext(operation);
        await ensureAllPagesLoadedForStructureChange();
        ihnAssertLiveOperationContext(operation);
        const snapshot = { v: 1, type: 'document-snapshot', fileId: operation.fileId,
            actorId: `${ihnGetLivePeerId()}:${ihnGetLiveTabId()}`, deviceId: getPresenceClientId(), sequence: ++ihnLiveSequence,
            sentAt: Date.now(), baseRevision: state.driveHeadRevisionId || null,
            exportName: state.exportName || '', calendarPageConfig: cloneTimelineValue(state.calendarPageConfig || null, null),
            structure: getCollabStructureSnapshot(),
            fields: getCollabFieldSnapshot(),
            pages: state.pages.map(page => {
                if (typeof cloneSanitizedPageForStorage === 'function') {
                    return cloneSanitizedPageForStorage(page);
                }
                return JSON.parse(JSON.stringify(sanitizePageForStorage(page)));
            }) };
        snapshot.snapshotId = `${snapshot.actorId}:${snapshot.sequence}`;
        snapshot.contentHash = ihnCanonicalDocumentHash(
            snapshot.pages,
            snapshot.structure,
            snapshot.calendarPageConfig,
            snapshot.exportName,
            snapshot.fields
        );
        ihnAssertLiveOperationContext(operation);
        ihnLiveCurrentHash = snapshot.contentHash;
        return snapshot;
    } finally {
        if (structureToken && typeof releaseRemotePageMerge === 'function') {
            releaseRemotePageMerge(structureToken);
        }
    }
}

function scheduleLiveDocumentBroadcast(options = {}) {
    if (!state?.driveFileId || !ihnCanEditLiveDocument()) return;
    if (!options.internal && !options.retry && !options.targetPeerId) {
        ihnLiveCurrentHash = '';
    }
    startLiveCollaboration();
    if (!options.retry && !options.internal) {
        ihnLiveBroadcastRetryAttempt = 0;
    }
    ihnLiveBroadcastQueued = true;
    if (options.force) ihnLiveBroadcastForce = true;
    if (options.targetPeerId) ihnLiveBroadcastTargets.add(options.targetPeerId);
    if (options.excludePeerId) ihnLiveBroadcastExclusions.add(options.excludePeerId);
    clearTimeout(ihnLiveBroadcastTimer);
    ihnLiveBroadcastTimer = setTimeout(() => {
        ihnLiveBroadcastTimer = null;
        ihnBroadcastDocument().catch(error => console.warn('Live broadcast failed:', error));
    }, Number.isFinite(options.delay)
        ? Math.max(0, Number(options.delay))
        : (options.immediate ? 0 : (ihnLiveApplying || hasSmoothInteraction() ? 60 : 40)));
}

function ihnLiveBroadcastRetryDelay(attempt) {
    return Math.min(2000, 120 * (2 ** Math.max(0, Number(attempt || 1) - 1)));
}

async function ihnBroadcastDocument() {
    if (!ihnLiveBroadcastQueued || !ihnCanEditLiveDocument()) return;
    if (ihnLiveBroadcastBusy || ihnLiveApplying || hasSmoothInteraction()) {
        scheduleLiveDocumentBroadcast({ internal: true });
        return;
    }
    clearTimeout(ihnLiveBroadcastTimer);
    ihnLiveBroadcastTimer = null;
    const runId = ++ihnLiveBroadcastRunId;
    const operation = ihnCaptureLiveOperationContext();
    ihnLiveBroadcastBusy = true; ihnLiveBroadcastQueued = false;
    const force = ihnLiveBroadcastForce;
    const targetPeerIds = new Set(ihnLiveBroadcastTargets);
    const excludedPeerIds = new Set(ihnLiveBroadcastExclusions);
    ihnLiveBroadcastForce = false;
    ihnLiveBroadcastTargets.clear();
    ihnLiveBroadcastExclusions.clear();
    let retryScheduled = false;
    let suppressAutomaticSchedule = false;
    try {
        const snapshot = await ihnBuildLiveSnapshot(operation);
        ihnAssertLiveOperationContext(operation);
        const hash = snapshot.contentHash;
        ihnRememberLiveSnapshot(snapshot);
        ihnEnsureTabChannel();
        if (targetPeerIds.size === 0 && (force || hash !== ihnLiveLastTabSentHash)) {
            ihnAssertLiveOperationContext(operation);
            try {
                ihnLiveBroadcastChannel?.postMessage(snapshot);
                ihnLiveLastTabSentHash = hash;
            } catch (error) {}
        }
        const deliveries = [];
        ihnLivePeers.forEach((peer, peerId) => {
            if (peer.channel?.readyState !== 'open') return;
            if (excludedPeerIds.has(peerId)) return;
            if (targetPeerIds.size > 0 && !targetPeerIds.has(peerId)) return;
            if (!force && peer.lastAckedHash === hash) return;
            if (!force && peer.pendingAckHash === hash) return;
            const peerSnapshot = ihnBuildPeerSnapshot(snapshot, peer, force);
            deliveries.push(ihnQueuePeerPayload(peerId, peer, peerSnapshot, operation, {
                trackAcknowledgement: true
            }));
        });
        await Promise.all(deliveries);
        ihnAssertLiveOperationContext(operation);
        ihnLiveBroadcastRetryAttempt = 0;
    } catch (error) {
        if (ihnLiveOperationContextIsCurrent(operation) && ihnCanEditLiveDocument()) {
            ihnLiveBroadcastQueued = true;
            if (force) ihnLiveBroadcastForce = true;
            targetPeerIds.forEach(peerId => ihnLiveBroadcastTargets.add(peerId));
            excludedPeerIds.forEach(peerId => ihnLiveBroadcastExclusions.add(peerId));
            if (ihnLiveBroadcastRetryAttempt < IHN_LIVE_BROADCAST_RETRY_LIMIT) {
                ihnLiveBroadcastRetryAttempt += 1;
                retryScheduled = true;
                scheduleLiveDocumentBroadcast({
                    retry: true,
                    delay: ihnLiveBroadcastRetryDelay(ihnLiveBroadcastRetryAttempt)
                });
            } else {
                suppressAutomaticSchedule = true;
                console.warn('Live broadcast paused after repeated transient failures; the next edit will retry.', error);
            }
        }
        throw error;
    } finally {
        if (runId === ihnLiveBroadcastRunId) {
            ihnLiveBroadcastBusy = false;
            if (ihnLiveBroadcastQueued && !retryScheduled && !suppressAutomaticSchedule) {
                scheduleLiveDocumentBroadcast({ immediate: true, internal: true });
            }
        }
    }
}

function ihnWaitForBackpressure(channel) {
    if (channel.bufferedAmount < 1_000_000) return Promise.resolve();
    return new Promise(resolve => {
        let done = false;
        const finish = () => { if (done) return; done = true; channel.removeEventListener('bufferedamountlow', finish); resolve(); };
        channel.addEventListener('bufferedamountlow', finish, { once: true }); setTimeout(finish, 3000);
    });
}

async function ihnSendPayload(channel, payload, operation = null) {
    if (operation) ihnAssertLiveOperationContext(operation);
    if (channel?.readyState !== 'open') return false;
    const raw = JSON.stringify(payload);
    if (raw.length > IHN_LIVE_MAX_CHARS) return false;
    const id = `x${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
    const total = Math.ceil(raw.length / IHN_LIVE_CHUNK);
    if (!total || total > IHN_LIVE_MAX_CHUNKS) return false;
    if (operation) ihnAssertLiveOperationContext(operation);
    channel.send(JSON.stringify({ t: 'start', id, total, chars: raw.length }));
    for (let index = 0; index < total; index += 1) {
        await ihnWaitForBackpressure(channel);
        if (operation) ihnAssertLiveOperationContext(operation);
        if (channel.readyState !== 'open') return false;
        channel.send(JSON.stringify({ t: 'chunk', id, index, data: raw.slice(index * IHN_LIVE_CHUNK, (index + 1) * IHN_LIVE_CHUNK) }));
    }
    if (operation) ihnAssertLiveOperationContext(operation);
    return true;
}

function ihnQueuePeerPayload(peerId, peer, payload, operation, options = {}) {
    const previous = peer?.sendQueue || Promise.resolve();
    const delivery = previous.catch(() => false).then(async () => {
        ihnAssertLiveOperationContext(operation);
        if (ihnLivePeers.get(peerId) !== peer || peer.channel?.readyState !== 'open') return false;
        const sent = await ihnSendPayload(peer.channel, payload, operation);
        ihnAssertLiveOperationContext(operation);
        if (!sent || ihnLivePeers.get(peerId) !== peer) return false;
        if (options.trackAcknowledgement && payload?.contentHash) {
            peer.lastSentHash = payload.contentHash;
            peer.pendingAckHash = payload.contentHash;
            peer.pendingAckAt = Date.now();
            peer.pendingAckReceivedHash = '';
            peer.pendingAckReceivedAt = 0;
        }
        return true;
    });
    peer.sendQueue = delivery.catch(() => false);
    return delivery;
}

async function ihnHandleWireMessage(
    raw,
    peerId,
    expectedPeer = null,
    expectedChannel = null,
    operation = null
) {
    if (typeof raw !== 'string') return;
    const peer = expectedPeer || ihnLivePeers.get(peerId);
    const messageOperation = operation || ihnCaptureLiveOperationContext(peer?.fileId);
    const peerContextIsCurrent = () => !!peer
        && ihnLivePeers.get(peerId) === peer
        && (!expectedChannel || peer.channel === expectedChannel)
        && !peer.closing
        && ihnLiveOperationContextIsCurrent(messageOperation);
    if (!peerContextIsCurrent()) return;
    peer.lastReceivedAt = Date.now();
    const staleBefore = Date.now() - 60_000;
    ihnLiveChunks.forEach((assembly, chunkKey) => {
        if (Number(assembly.createdAt || 0) < staleBefore) ihnLiveChunks.delete(chunkKey);
    });
    let message; try { message = JSON.parse(raw); } catch (error) { return; }
    if (!peerContextIsCurrent()) return;
    ihnTouchKnownPeerFromChannel(peerId);
    if (message?.t === 'ping') {
        if (peer) peer.protocolV2 = true;
        ihnSendControl(peer, { t: 'pong', at: Number(message.at) || Date.now(), receivedAt: Date.now() });
        return;
    }
    if (message?.t === 'pong') {
        if (peer) {
            peer.protocolV2 = true;
            const receivedAt = Date.now();
            const sentAt = Number(message.at) || 0;
            const expectedAt = Number(peer.lastPingAt) || 0;
            const pendingPings = peer.pendingHealthPings instanceof Set
                ? peer.pendingHealthPings
                : null;
            const validPong = sentAt > 0
                && (pendingPings?.has(sentAt)
                    || (!pendingPings?.size && sentAt === expectedAt))
                && sentAt >= Number(peer.openedAt || 0)
                && sentAt <= receivedAt;
            if (validPong) {
                pendingPings?.delete(sentAt);
                peer.lastPongAt = receivedAt;
                peer.lastPongEchoAt = sentAt;
                const rtt = Math.max(0, receivedAt - sentAt);
                peer.rttMs = Number.isFinite(peer.rttMs)
                    ? Math.round(peer.rttMs * 0.7 + rtt * 0.3)
                    : rtt;
                ihnMarkPeerHealthy(peerId, peer, 'pong');
            }
        }
        return;
    }
    if (message?.t === 'hello' || message?.t === 'state-request') {
        if (peer) peer.protocolV2 = true;
        scheduleLiveDocumentBroadcast({ immediate: true, targetPeerId: peerId, force: true });
        return;
    }
    if (message?.t === 'snapshot-received') {
        if (!peer) return;
        peer.protocolV2 = true;
        const receivedHash = String(message.hash || '');
        if (!receivedHash || !peer.pendingAckHash || peer.pendingAckHash !== receivedHash) return;
        peer.pendingAckReceivedHash = receivedHash;
        peer.pendingAckReceivedAt = Date.now();
        return;
    }
    if (message?.t === 'snapshot-ack') {
        if (!peer) return;
        peer.protocolV2 = true;
        const acknowledgedHash = String(message.hash || '');
        if (!acknowledgedHash || !peer.pendingAckHash || peer.pendingAckHash !== acknowledgedHash) return;
        peer.lastAckedHash = acknowledgedHash;
        peer.remoteCurrentHash = acknowledgedHash;
        peer.pendingAckHash = '';
        peer.pendingAckAt = 0;
        peer.pendingAckReceivedHash = '';
        peer.pendingAckReceivedAt = 0;
        peer.healthyAckCount = Number(peer.healthyAckCount || 0) + 1;
        ihnMarkPeerHealthy(peerId, peer, 'ack');
        return;
    }
    if (message?.t === 'snapshot-nack') {
        if (!peer) return;
        peer.protocolV2 = true;
        const rejectedHash = String(message.hash || '');
        if (rejectedHash && peer.pendingAckHash === rejectedHash) {
            peer.pendingAckHash = '';
            peer.pendingAckAt = 0;
            peer.pendingAckReceivedHash = '';
            peer.pendingAckReceivedAt = 0;
        }
        const currentHash = String(message.currentHash || '');
        if (currentHash) ihnObservePeerDocumentHash(peerId, currentHash);
        scheduleLiveDocumentBroadcast({
            immediate: true,
            targetPeerId: peerId,
            force: true
        });
        return;
    }
    if (message?.type === 'live-stroke') {
        ihnHandleRealtimeStrokePacket(message, 'webrtc', peerId);
        return;
    }
    const key = `${peerId}:${peer.sessionId || ''}:${message?.id || ''}`;
    if (message?.t === 'start') {
        const total = Number(message.total), chars = Number(message.chars);
        if (!message.id || total <= 0 || total > IHN_LIVE_MAX_CHUNKS || chars <= 0 || chars > IHN_LIVE_MAX_CHARS) return;
        ihnLiveChunks.set(key, { total, chars, chunks: new Array(total), received: 0, createdAt: Date.now() }); return;
    }
    if (message?.t !== 'chunk') return;
    const assembly = ihnLiveChunks.get(key), index = Number(message.index);
    if (!assembly || !Number.isInteger(index) || index < 0 || index >= assembly.total || typeof message.data !== 'string') return;
    if (assembly.chunks[index] === undefined) { assembly.chunks[index] = message.data; assembly.received += 1; }
    if (assembly.received !== assembly.total) return;
    ihnLiveChunks.delete(key);
    const combined = assembly.chunks.join('');
    if (combined.length === assembly.chars) {
        let envelope;
        try { envelope = JSON.parse(combined); } catch (error) { return; }
        if (!peerContextIsCurrent()) return;
        if (envelope?.v === 1
            && envelope?.type === 'document-snapshot'
            && envelope?.fileId === state?.driveFileId
            && envelope?.contentHash) {
            ihnSendControl(peer, {
                t: 'snapshot-received',
                hash: envelope.contentHash,
                snapshotId: envelope.snapshotId || '',
                at: Date.now()
            });
        }
        const handled = await ihnHandleLiveEnvelope(envelope, 'webrtc', peerId);
        if (!peerContextIsCurrent()) return;
        if (handled !== false && envelope?.contentHash) {
            ihnObservePeerDocumentHash(peerId, envelope.contentHash);
            ihnSendControl(peer, {
                t: 'snapshot-ack',
                hash: envelope.contentHash,
                snapshotId: envelope.snapshotId || '',
                at: Date.now()
            });
        }
    }
}

function ihnLiveApplyKey(envelope, operation) {
    return `${operation.generation}\u0000${operation.fileId}\u0000${String(envelope?.actorId || '')}`;
}

function ihnCancelPendingLiveApplies() {
    ihnLivePendingApplies.forEach(record => record.resolve(false));
    ihnLivePendingApplies.clear();
}

function ihnStartLiveApplyDrain() {
    if (ihnLiveApplyDrainPromise) return ihnLiveApplyDrainPromise;
    const drain = (async () => {
        while (ihnLivePendingApplies.size) {
            const first = ihnLivePendingApplies.entries().next().value;
            if (!first) break;
            const [key, record] = first;
            ihnLivePendingApplies.delete(key);
            if (!ihnLiveOperationContextIsCurrent(record.operation)) {
                record.resolve(false);
                continue;
            }
            ihnLiveActiveApply = record;
            try {
                record.resolve(await ihnApplyLiveEnvelope(
                    record.envelope,
                    record.transport,
                    record.peerId,
                    record.operation,
                    record
                ));
            } catch (error) {
                if (error?.code === 'IHN_LIVE_STALE_CONTEXT') record.resolve(false);
                else record.reject(error);
            } finally {
                if (ihnLiveActiveApply === record) ihnLiveActiveApply = null;
            }
            if (ihnLivePendingApplies.size) {
                // Give local page operations a lock-acquisition turn and allow
                // newer anti-entropy snapshots to replace obsolete pending ones.
                await new Promise(resolve => setTimeout(resolve, IHN_LIVE_APPLY_COALESCE_MS));
            }
        }
    })();
    ihnLiveApplyDrainPromise = drain;
    ihnLiveApplyQueue = drain;
    drain.then(() => {
        if (ihnLiveApplyDrainPromise !== drain) return;
        ihnLiveApplyDrainPromise = null;
        ihnLiveApplyQueue = Promise.resolve();
        if (ihnLivePendingApplies.size) ihnStartLiveApplyDrain();
    });
    return drain;
}

function ihnHandleLiveEnvelope(envelope, transport = '', peerId = '') {
    if (!envelope
        || envelope.v !== 1
        || envelope.type !== 'document-snapshot'
        || envelope.fileId !== state?.driveFileId
        || !envelope.actorId
        || !Array.isArray(envelope.pages)
        || (!envelope.pages.length && !envelope.partial)) {
        return Promise.resolve(false);
    }
    const operation = ihnCaptureLiveOperationContext(envelope.fileId);
    if (!ihnLiveOperationContextIsCurrent(operation)) return Promise.resolve(false);
    const key = ihnLiveApplyKey(envelope, operation);
    const sequence = Number(envelope.sequence) || 0;
    const hash = String(envelope.contentHash || '');
    if (ihnLiveActiveApply?.key === key) {
        const activeSequence = Number(ihnLiveActiveApply.sequence) || 0;
        if (sequence === activeSequence && hash === ihnLiveActiveApply.hash) {
            return ihnLiveActiveApply.promise;
        }
        if (sequence <= activeSequence) return Promise.resolve(false);
    }
    const pending = ihnLivePendingApplies.get(key);
    if (pending) {
        if (sequence === pending.sequence && hash === pending.hash) {
            return pending.promise;
        }
        if (sequence <= pending.sequence) return Promise.resolve(false);
        pending.resolve(false);
    }
    let resolveRecord;
    let rejectRecord;
    const promise = new Promise((resolve, reject) => {
        resolveRecord = resolve;
        rejectRecord = reject;
    });
    const record = {
        key,
        envelope,
        transport,
        peerId,
        operation,
        sequence,
        hash,
        promise,
        resolve: resolveRecord,
        reject: rejectRecord
    };
    ihnLivePendingApplies.set(key, record);
    ihnStartLiveApplyDrain();
    return promise;
}

function ihnTrackMergeCurrentState(currentHash) {
    const hash = String(currentHash || '');
    if (!hash) return null;
    const confirmedHash = typeof driveConfirmedContentHash === 'string'
        ? driveConfirmedContentHash
        : '';
    const contentVersion = typeof driveContentVersion === 'number'
        ? driveContentVersion
        : 0;
    if (!ihnLiveMergeUploadGuard
        || ihnLiveMergeUploadGuard.currentHash !== hash
        || ihnLiveMergeUploadGuard.confirmedHash !== confirmedHash
        || ihnLiveMergeUploadGuard.contentVersion !== contentVersion) {
        ihnLiveMergeUploadGuard = {
            currentHash: hash,
            requestedHash: '',
            confirmedHash,
            contentVersion
        };
    }
    return ihnLiveMergeUploadGuard;
}

function ihnShouldQueueMergeUpload(mergedHash) {
    const guard = ihnTrackMergeCurrentState(mergedHash);
    if (!guard) return false;
    if (guard.confirmedHash && guard.confirmedHash === guard.currentHash) {
        guard.requestedHash = '';
        return false;
    }
    if (guard.requestedHash === guard.currentHash) return false;
    // Suppress only a repeated request for the current state. Tracking every
    // successfully applied state below means an intermediate D clears a
    // request for C even when D itself already matches the incoming peer and
    // therefore does not need a convergence upload.
    guard.requestedHash = guard.currentHash;
    return true;
}

function ihnPeerAlreadyHasLiveHash(peer, hash) {
    if (!peer || !hash) return false;
    return peer.remoteCurrentHash === hash
        || peer.lastAckedHash === hash
        || peer.pendingAckHash === hash;
}

async function ihnFanOutAppliedEnvelope(envelope, transport, sourcePeerId, operation) {
    ihnAssertLiveOperationContext(operation);
    if (envelope?.partial) {
        // A page delta is valid only for the peer that acknowledged its base.
        // Rebuild current state before relaying to tabs or a third peer.
        scheduleLiveDocumentBroadcast({
            immediate: true,
            force: true,
            excludePeerId: transport === 'webrtc' ? sourcePeerId : ''
        });
        return;
    }
    if (transport === 'webrtc') {
        ihnEnsureTabChannel();
        ihnAssertLiveOperationContext(operation);
        try {
            ihnLiveBroadcastChannel?.postMessage(envelope);
            if (envelope.contentHash) ihnLiveLastTabSentHash = envelope.contentHash;
        } catch (error) {
            console.warn('Same-device live relay failed:', error);
        }
    }
    if (transport !== 'webrtc' && transport !== 'tab') return;
    const actorId = String(envelope?.actorId || '');
    const originPeerId = actorId.includes(':') ? actorId.slice(0, actorId.indexOf(':')) : '';
    const deliveries = [];
    ihnLivePeers.forEach((peer, relayPeerId) => {
        if (transport === 'webrtc' && relayPeerId === sourcePeerId) return;
        if (originPeerId && relayPeerId === originPeerId) return;
        if (peer.channel?.readyState !== 'open') return;
        if (ihnPeerAlreadyHasLiveHash(peer, envelope.contentHash)) return;
        deliveries.push(ihnQueuePeerPayload(
            relayPeerId,
            peer,
            envelope,
            operation,
            { trackAcknowledgement: true }
        ));
    });
    await Promise.all(deliveries);
}

function ihnScheduleLiveFanOut(envelope, transport, sourcePeerId, operation) {
    const task = ihnFanOutAppliedEnvelope(envelope, transport, sourcePeerId, operation)
        .catch(error => {
            if (ihnLiveOperationContextIsCurrent(operation)) {
                console.warn('Live snapshot relay failed:', error);
            }
            return false;
        });
    ihnLiveFanOutTasks.add(task);
    task.finally(() => ihnLiveFanOutTasks.delete(task));
}

function ihnFlushLiveFanOutTasks() {
    return Promise.all([...ihnLiveFanOutTasks]);
}

async function ihnApplyLiveEnvelope(
    envelope,
    transport = '',
    peerId = '',
    operation = ihnCaptureLiveOperationContext(envelope?.fileId || ''),
    applyRecord = null
) {
    if (!envelope || envelope.v !== 1 || envelope.type !== 'document-snapshot') return false;
    ihnAssertLiveOperationContext(operation);
    if (envelope.fileId !== operation.fileId) return false;
    const ownActorId = `${ihnGetLivePeerId()}:${ihnGetLiveTabId()}`;
    if (!Array.isArray(envelope.pages)
        || (!envelope.pages.length && !envelope.partial)
        || envelope.actorId === ownActorId) return false;
    if (envelope.partial) {
        const currentHash = ihnLiveCurrentHash || ihnCanonicalDocumentHash(
                state.pages.map(page => sanitizePageForStorage(page)),
                getCollabStructureSnapshot(),
                state.calendarPageConfig,
                state.exportName,
                getCollabFieldSnapshot()
            );
        if (!envelope.baseHash || currentHash !== envelope.baseHash) {
            const peer = transport === 'webrtc' ? ihnLivePeers.get(peerId) : null;
            ihnSendControl(peer, {
                t: 'snapshot-nack',
                hash: String(envelope.contentHash || ''),
                baseHash: String(envelope.baseHash || ''),
                currentHash,
                at: Date.now()
            });
            return false;
        }
    }
    const sequence = Number(envelope.sequence) || 0, previous = ihnLiveSeen.get(envelope.actorId) || 0;
    if (sequence <= previous) return true;
    const now = Date.now();
    ihnLiveAppliedHashes.forEach((appliedAt, hash) => {
        if (now - Number(appliedAt || 0) > 120_000) ihnLiveAppliedHashes.delete(hash);
    });
    if (envelope.contentHash && ihnLiveAppliedHashes.has(envelope.contentHash)) {
        const currentHash = ihnCanonicalDocumentHash(
            state.pages.map(page => sanitizePageForStorage(page)),
            getCollabStructureSnapshot(),
            state.calendarPageConfig,
            state.exportName,
            getCollabFieldSnapshot()
        );
        if (currentHash === envelope.contentHash) {
            ihnLiveSeen.set(envelope.actorId, sequence);
            ihnScheduleLiveFanOut(envelope, transport, peerId, operation);
            return true;
        }
        // A historical hash is not a duplicate after the document has moved
        // through another valid state (C -> D -> C).
        ihnLiveAppliedHashes.delete(envelope.contentHash);
    }
    while (ihnCanEditLiveDocument() && hasSmoothInteraction()) {
        // Never reorder page/content arrays underneath an active pen,
        // selection transform or inertial gesture. The queued snapshot is
        // applied immediately after that atomic local interaction finishes.
        await new Promise(resolve => setTimeout(resolve, 32));
        if (!ihnLiveOperationContextIsCurrent(operation)) return false;
    }
    ihnLiveApplying = true;
    try {
        ihnAssertLiveOperationContext(operation);
        const canEditDocument = ihnCanEditLiveDocument();
        const preserveLocal = canEditDocument && hasUnsyncedLocalDriveChanges();
        const result = await applyRemotePages(envelope.pages, {
            preserveLocalUnsynced: preserveLocal,
            // Live snapshots are anti-entropy state, not authoritative
            // replacements. Absence never deletes content; explicit
            // tombstones and deterministic same-ID resolution do.
            additiveById: canEditDocument,
            remoteStructure: envelope.structure || null,
            remoteFields: envelope.fields || null,
            remoteCalendarPageConfig: cloneTimelineValue(envelope.calendarPageConfig || null, null),
            remoteExportName: envelope.exportName || ''
        });
        ihnAssertLiveOperationContext(operation);
        if (result?.changed
            && typeof clearPublicInlinePreview === 'function'
            && document.getElementById('public-inline-preview')) {
            clearPublicInlinePreview();
            setReadOnlyMode(true, { force: true });
            updateViewerModeUI();
            showEditorView();
        }
        if (result?.changed) showStatus(transport === 'tab' ? 'Live changes from another tab' : 'Live changes from collaborator', { savedAt: Number(envelope.sentAt) || Date.now() });
        if (typeof clearRemoteLiveStrokePreviews === 'function') {
            clearRemoteLiveStrokePreviews(envelope.actorId, Number(envelope.sentAt) || Date.now());
        }
        const mergedHash = ihnCanonicalDocumentHash(
            state.pages.map(page => sanitizePageForStorage(page)),
            getCollabStructureSnapshot(),
            state.calendarPageConfig,
            state.exportName,
            getCollabFieldSnapshot()
        );
        // Advance causal/dedupe state only after the merge completed. A failed
        // IDB/hydration pass can therefore be retried with the same sequence.
        ihnLiveSeen.set(envelope.actorId, sequence);
        if (envelope.contentHash) ihnLiveAppliedHashes.set(envelope.contentHash, Date.now());
        const resolvedCurrentHash = result?.hasLocalMerges
            ? mergedHash
            : String(envelope.contentHash || mergedHash);
        ihnLiveLastAppliedHash = resolvedCurrentHash;
        ihnLiveCurrentHash = resolvedCurrentHash;
        ihnTrackMergeCurrentState(resolvedCurrentHash);
        const needsConvergenceBroadcast = canEditDocument && (
            result?.hasLocalMerges
            || (envelope.contentHash && mergedHash !== envelope.contentHash)
        );
        if (needsConvergenceBroadcast) {
            if (ihnShouldQueueMergeUpload(mergedHash)) {
                if (typeof bumpDriveContentVersion === 'function') {
                    bumpDriveContentVersion();
                    const refreshedGuard = ihnTrackMergeCurrentState(mergedHash);
                    if (refreshedGuard) refreshedGuard.requestedHash = refreshedGuard.currentHash;
                }
                ihnAssertLiveOperationContext(operation);
                driveDirty = true;
                requestImmediateDriveSave();
            }
            scheduleLiveDocumentBroadcast({ immediate: true, force: true });
        } else {
            ihnScheduleLiveFanOut(envelope, transport, peerId, operation);
        }
        return true;
    } finally {
        if (!applyRecord || ihnLiveActiveApply === applyRecord) {
            ihnLiveApplying = false;
            if (ihnLiveBroadcastQueued && ihnLiveOperationContextIsCurrent(operation)) {
                scheduleLiveDocumentBroadcast({ immediate: true, internal: true });
            }
        }
    }
}
