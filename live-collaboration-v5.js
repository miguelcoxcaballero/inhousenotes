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
let ihnLiveSignalBusy = false;
let ihnLiveCryptoKey = null;
let ihnLiveCryptoFileId = '';
let ihnLiveBroadcastTimer = null;
let ihnLiveBroadcastBusy = false;
let ihnLiveBroadcastQueued = false;
let ihnLiveApplying = false;
let ihnLiveSequence = Date.now();
let ihnLiveLastHash = '';
let ihnLiveMainPeerId = '';
const ihnLivePeers = new Map();
const ihnLiveSeen = new Map();
const ihnLiveChunks = new Map();
const ihnLiveProcessedOffers = new Set();

function ihnGetLiveTabId() {
    if (ihnLiveTabId) return ihnLiveTabId;
    try { ihnLiveTabId = sessionStorage.getItem('ihn-live-tab-v1') || ''; } catch (error) {}
    if (!ihnLiveTabId) {
        ihnLiveTabId = `t${Date.now().toString(36)}${Math.random().toString(36).slice(2, 9)}`;
        try { sessionStorage.setItem('ihn-live-tab-v1', ihnLiveTabId); } catch (error) {}
    }
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

async function ihnEnsureSignalKey() {
    if (!state?.driveFileId || !driveAccessToken || !state.driveCanEdit) return null;
    if (ihnLiveCryptoKey && ihnLiveCryptoFileId === state.driveFileId) return ihnLiveCryptoKey;
    const fileId = state.driveFileId;
    const propertyName = 'ihn_live_key_v1';
    const read = async () => {
        const response = await driveFetch(`https://www.googleapis.com/drive/v3/files/${fileId}?fields=properties`, {
            method: 'GET', timeout: DRIVE_META_TIMEOUT, headers: { 'Cache-Control': 'no-cache' }
        });
        return String((await response.json())?.properties?.[propertyName] || '');
    };
    let encoded = await read().catch(() => '');
    if (!encoded) {
        const random = new Uint8Array(32);
        crypto.getRandomValues(random);
        const candidate = ihnBytesToBase64Url(random);
        await driveFetch(`https://www.googleapis.com/drive/v3/files/${fileId}?fields=id,properties`, {
            method: 'PATCH', headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ properties: { [propertyName]: candidate } })
        });
        encoded = await read().catch(() => candidate);
    }
    ihnLiveCryptoKey = await crypto.subtle.importKey('raw', ihnBase64UrlToBytes(encoded), 'AES-GCM', false, ['encrypt', 'decrypt']);
    ihnLiveCryptoFileId = fileId;
    return ihnLiveCryptoKey;
}

async function ihnEncodeSignal(payload) {
    const key = await ihnEnsureSignalKey();
    if (!key) throw new Error('Live signalling key unavailable');
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const plain = new TextEncoder().encode(JSON.stringify(payload));
    const cipher = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, key, plain));
    const packed = new Uint8Array(iv.length + cipher.length);
    packed.set(iv); packed.set(cipher, iv.length);
    return IHN_LIVE_SIGNAL_PREFIX + ihnBytesToBase64Url(packed);
}

async function ihnDecodeSignal(content) {
    if (!String(content || '').startsWith(IHN_LIVE_SIGNAL_PREFIX)) return null;
    try {
        const key = await ihnEnsureSignalKey();
        const packed = ihnBase64UrlToBytes(String(content).slice(IHN_LIVE_SIGNAL_PREFIX.length));
        const plain = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: packed.slice(0, 12) }, key, packed.slice(12));
        return JSON.parse(new TextDecoder().decode(plain));
    } catch (error) {
        return null;
    }
}

function ihnWaitForIce(pc, timeout = 3800) {
    if (pc.iceGatheringState === 'complete') return Promise.resolve();
    return new Promise(resolve => {
        let done = false;
        const finish = () => {
            if (done) return;
            done = true; clearTimeout(timer);
            pc.removeEventListener('icegatheringstatechange', check); resolve();
        };
        const check = () => { if (pc.iceGatheringState === 'complete') finish(); };
        const timer = setTimeout(finish, timeout);
        pc.addEventListener('icegatheringstatechange', check);
    });
}

function ihnClosePeer(peerId, reason = '') {
    const peer = ihnLivePeers.get(peerId);
    if (!peer) return;
    try { peer.channel?.close(); } catch (error) {}
    try { peer.pc?.close(); } catch (error) {}
    ihnLivePeers.delete(peerId);
    if (reason) console.info(`Live peer ${peerId} closed: ${reason}`);
}

function ihnConfigurePeer(peerId, pc, peer) {
    pc.onconnectionstatechange = () => {
        peer.status = pc.connectionState;
        if (pc.connectionState === 'connected') ihnRefreshPeerPath(peer).catch(() => {});
        if (pc.connectionState === 'failed' || pc.connectionState === 'closed') ihnClosePeer(peerId, pc.connectionState);
    };
    pc.ondatachannel = event => ihnConfigureChannel(peerId, event.channel, peer);
}

function ihnConfigureChannel(peerId, channel, peer) {
    peer.channel = channel;
    channel.bufferedAmountLowThreshold = 512_000;
    channel.onopen = () => {
        peer.status = 'open'; peer.openedAt = Date.now();
        ihnRefreshPeerPath(peer).catch(() => {});
        if (peer.commentId && peer.initiator && state.driveFileId && driveAccessToken) {
            driveFetch(`https://www.googleapis.com/drive/v3/files/${state.driveFileId}/comments/${peer.commentId}`, { method: 'DELETE' }).catch(() => {});
            peer.commentId = '';
        }
        scheduleLiveDocumentBroadcast({ immediate: true });
    };
    channel.onclose = () => { peer.status = 'closed'; };
    channel.onerror = error => console.warn('Live data channel error:', error);
    channel.onmessage = event => ihnHandleWireMessage(event.data, peerId).catch(error => console.warn('Live message rejected:', error));
}

async function ihnCreateOffer(targetPeerId) {
    if (!ihnIsLiveLeader() || !state.driveCanEdit || !driveAccessToken || !state.driveFileId) return;
    const ownId = ihnGetLivePeerId();
    if (!targetPeerId || ownId >= targetPeerId) return;
    const current = ihnLivePeers.get(targetPeerId);
    if (current && ['new', 'connecting', 'open'].includes(current.status)) return;
    if (current) ihnClosePeer(targetPeerId, 'reconnect');
    const pc = new RTCPeerConnection({ iceServers: IHN_LIVE_STUN });
    const peer = { pc, channel: null, status: 'connecting', initiator: true,
        sessionId: `s${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`,
        commentId: '', createdAt: Date.now() };
    ihnLivePeers.set(targetPeerId, peer);
    ihnConfigurePeer(targetPeerId, pc, peer);
    ihnConfigureChannel(targetPeerId, pc.createDataChannel('inhousenotes-live-v5', { ordered: true }), peer);
    try {
        await pc.setLocalDescription(await pc.createOffer());
        await ihnWaitForIce(pc);
        const content = await ihnEncodeSignal({ v: 1, type: 'offer', fileId: state.driveFileId,
            from: ownId, to: targetPeerId, sessionId: peer.sessionId,
            expiresAt: Date.now() + IHN_LIVE_SIGNAL_TTL, description: pc.localDescription });
        const response = await driveFetch(`https://www.googleapis.com/drive/v3/files/${state.driveFileId}/comments?fields=id,createdTime`, {
            method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ content })
        });
        peer.commentId = (await response.json()).id || '';
    } catch (error) {
        console.warn('Direct connection unavailable; Drive fallback remains active:', error);
        ihnClosePeer(targetPeerId, 'offer failed');
    }
}

async function ihnAcceptOffer(comment, offer) {
    const ownId = ihnGetLivePeerId();
    if (!offer || offer.to !== ownId || offer.from === ownId || offer.fileId !== state.driveFileId || Number(offer.expiresAt) < Date.now()) return;
    const offerKey = `${comment.id}:${offer.sessionId}`;
    if (ihnLiveProcessedOffers.has(offerKey)) return;
    ihnLiveProcessedOffers.add(offerKey);
    if (ihnLivePeers.get(offer.from)?.status === 'open') return;
    ihnClosePeer(offer.from, 'new offer');
    const pc = new RTCPeerConnection({ iceServers: IHN_LIVE_STUN });
    const peer = { pc, channel: null, status: 'connecting', initiator: false,
        sessionId: offer.sessionId, commentId: comment.id, createdAt: Date.now() };
    ihnLivePeers.set(offer.from, peer);
    ihnConfigurePeer(offer.from, pc, peer);
    try {
        await pc.setRemoteDescription(offer.description);
        await pc.setLocalDescription(await pc.createAnswer());
        await ihnWaitForIce(pc);
        const content = await ihnEncodeSignal({ v: 1, type: 'answer', fileId: state.driveFileId,
            from: ownId, to: offer.from, sessionId: offer.sessionId,
            expiresAt: Date.now() + IHN_LIVE_SIGNAL_TTL, description: pc.localDescription });
        await driveFetch(`https://www.googleapis.com/drive/v3/files/${state.driveFileId}/comments/${comment.id}/replies?fields=id,createdTime`, {
            method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ content })
        });
    } catch (error) {
        console.warn('Direct offer could not be accepted:', error);
        ihnClosePeer(offer.from, 'answer failed');
    }
}

async function ihnApplyAnswer(answer) {
    const ownId = ihnGetLivePeerId();
    if (!answer || answer.type !== 'answer' || answer.to !== ownId || answer.fileId !== state.driveFileId || Number(answer.expiresAt) < Date.now()) return;
    const peer = ihnLivePeers.get(answer.from);
    if (!peer?.initiator || peer.sessionId !== answer.sessionId || peer.pc.remoteDescription) return;
    try { await peer.pc.setRemoteDescription(answer.description); }
    catch (error) { ihnClosePeer(answer.from, 'invalid answer'); }
}

async function ihnPollSignals() {
    if (ihnLiveSignalBusy || !ihnIsLiveLeader() || !state.driveCanEdit || !driveAccessToken || !state.driveFileId || document.visibilityState === 'hidden') return;
    ihnLiveSignalBusy = true;
    try {
        await ihnEnsureSignalKey();
        const fields = encodeURIComponent('comments(id,content,createdTime,resolved,replies(id,content,createdTime))');
        const response = await driveFetch(`https://www.googleapis.com/drive/v3/files/${state.driveFileId}/comments?pageSize=100&includeDeleted=false&fields=${fields}`, {
            method: 'GET', timeout: DRIVE_POLL_TIMEOUT, headers: { 'Cache-Control': 'no-cache' }
        });
        const ownId = ihnGetLivePeerId();
        for (const comment of (await response.json()).comments || []) {
            if (!String(comment?.content || '').startsWith(IHN_LIVE_SIGNAL_PREFIX)) continue;
            const offer = await ihnDecodeSignal(comment.content);
            if (!offer || offer.v !== 1 || offer.type !== 'offer') continue;
            if (offer.to === ownId) await ihnAcceptOffer(comment, offer);
            if (offer.from !== ownId) continue;
            for (const reply of comment.replies || []) {
                const answer = await ihnDecodeSignal(reply.content);
                if (answer) await ihnApplyAnswer(answer);
            }
        }
    } catch (error) {
        console.warn('Live signalling unavailable; continuing with Drive sync:', error);
    } finally {
        ihnLiveSignalBusy = false;
    }
}

function liveCollabUpdatePeers(users) {
    if (!state?.driveFileId || !driveAccessToken || !state.driveCanEdit) return;
    startLiveCollaboration();
    const ownId = ihnGetLivePeerId();
    const active = new Set();
    for (const user of users || []) {
        if (!user?.isOnline) continue;
        const peerId = ihnGetPresencePeerId(user);
        if (!peerId || peerId === ownId) continue;
        active.add(peerId);
    }
    ihnLiveMainPeerId = [...active, ownId].sort()[0] || ownId;
    if (!ihnIsLiveLeader()) return;
    active.forEach(peerId => {
        if (ownId < peerId) ihnCreateOffer(peerId);
    });
    ihnLivePeers.forEach((peer, peerId) => {
        if (!active.has(peerId) && Date.now() - Number(peer.openedAt || peer.createdAt || 0) > 30_000) ihnClosePeer(peerId, 'presence expired');
    });
    ihnPollSignals();
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
    }
    const mainPeerId = ihnLiveMainPeerId || ownId;
    return {
        peerId,
        transport,
        transportCode,
        isMain: peerId === mainPeerId,
        connected: own || sameDevice || peer?.channel?.readyState === 'open'
    };
}

function getLiveCollaborationOverview() {
    return {
        ownPeerId: ihnGetLivePeerId(),
        mainPeerId: ihnLiveMainPeerId || ihnGetLivePeerId(),
        isMain: (ihnLiveMainPeerId || ihnGetLivePeerId()) === ihnGetLivePeerId(),
        openPeerCount: [...ihnLivePeers.values()].filter(peer => peer.channel?.readyState === 'open').length
    };
}

async function ihnVerifyLiveCapability() {
    if (!state?.driveFileId || !driveAccessToken || !state.driveCanEdit) return;
    try {
        const response = await driveFetch(`https://www.googleapis.com/drive/v3/files/${state.driveFileId}?fields=capabilities(canEdit,canModifyContent),trashed`, {
            method: 'GET', timeout: DRIVE_META_TIMEOUT, headers: { 'Cache-Control': 'no-cache' }
        });
        const data = await response.json();
        if (data.trashed || data.capabilities?.canEdit === false || data.capabilities?.canModifyContent === false) {
            state.driveCanEdit = false; stopLiveCollaboration(); setReadOnlyMode(true, { force: true });
            showStatus('Editing access was removed. Switched to view only.', { error: true });
        }
    } catch (error) {
        if (error?.status === 403 || error?.status === 404) stopLiveCollaboration();
    }
}

function startLiveCollaboration() {
    if (!state?.driveFileId) return;
    ihnEnsureTabChannel();
    if (!driveAccessToken || !state.driveCanEdit || typeof RTCPeerConnection === 'undefined') return;
    if (!ihnLiveLeaderTimer) {
        ihnClaimLiveLeader();
        ihnLiveLeaderTimer = setInterval(() => { if (ihnClaimLiveLeader()) ihnPollSignals(); }, 2000);
    }
    if (!ihnLiveSignalTimer) ihnLiveSignalTimer = setInterval(ihnPollSignals, 2000);
    if (!ihnLiveCapabilityTimer) ihnLiveCapabilityTimer = setInterval(ihnVerifyLiveCapability, 30_000);
    ihnPollSignals();
}

function stopLiveCollaboration() {
    clearTimeout(ihnLiveBroadcastTimer); ihnLiveBroadcastTimer = null;
    if (ihnLiveLeaderTimer) clearInterval(ihnLiveLeaderTimer);
    if (ihnLiveSignalTimer) clearInterval(ihnLiveSignalTimer);
    if (ihnLiveCapabilityTimer) clearInterval(ihnLiveCapabilityTimer);
    ihnLiveLeaderTimer = ihnLiveSignalTimer = ihnLiveCapabilityTimer = null;
    ihnLiveSignalBusy = ihnLiveBroadcastBusy = ihnLiveBroadcastQueued = ihnLiveApplying = false;
    [...ihnLivePeers.keys()].forEach(peerId => ihnClosePeer(peerId, 'document closed'));
    ihnLiveChunks.clear(); ihnLiveSeen.clear(); ihnLiveProcessedOffers.clear();
    try { ihnLiveBroadcastChannel?.close(); } catch (error) {}
    ihnLiveBroadcastChannel = null; ihnLiveBroadcastFileId = '';
    ihnLiveCryptoKey = null; ihnLiveCryptoFileId = ''; ihnLiveLastHash = ''; ihnLiveMainPeerId = '';
}

async function ihnBuildLiveSnapshot() {
    await flushStrokeOpsQueue();
    await ensureAllPagesLoadedForStructureChange();
    const snapshot = { v: 1, type: 'document-snapshot', fileId: state.driveFileId,
        actorId: `${ihnGetLivePeerId()}:${ihnGetLiveTabId()}`, deviceId: getPresenceClientId(), sequence: ++ihnLiveSequence,
        sentAt: Date.now(), baseRevision: state.driveHeadRevisionId || null,
        exportName: state.exportName || '', calendarPageConfig: cloneTimelineValue(state.calendarPageConfig || null, null),
        pages: state.pages.map(page => sanitizePageForStorage(page)) };
    snapshot.contentHash = timelineSnapshotHash(snapshot.pages, snapshot.calendarPageConfig, snapshot.exportName);
    return snapshot;
}

function scheduleLiveDocumentBroadcast(options = {}) {
    if (!state?.driveFileId || state.isReadOnly) return;
    startLiveCollaboration(); ihnLiveBroadcastQueued = true; clearTimeout(ihnLiveBroadcastTimer);
    ihnLiveBroadcastTimer = setTimeout(() => {
        ihnLiveBroadcastTimer = null;
        ihnBroadcastDocument().catch(error => console.warn('Live broadcast failed:', error));
    }, options.immediate ? 0 : (ihnLiveApplying || hasSmoothInteraction() ? 60 : 40));
}

async function ihnBroadcastDocument() {
    if (!ihnLiveBroadcastQueued || state.isReadOnly) return;
    if (ihnLiveBroadcastBusy || ihnLiveApplying || hasSmoothInteraction()) {
        scheduleLiveDocumentBroadcast();
        return;
    }
    ihnLiveBroadcastBusy = true; ihnLiveBroadcastQueued = false;
    try {
        const snapshot = await ihnBuildLiveSnapshot();
        const hash = snapshot.contentHash;
        if (hash === ihnLiveLastHash) return;
        ihnLiveLastHash = hash; ihnEnsureTabChannel();
        try { ihnLiveBroadcastChannel?.postMessage(snapshot); } catch (error) {}
        await Promise.all([...ihnLivePeers.values()].filter(peer => peer.channel?.readyState === 'open').map(peer => ihnSendPayload(peer.channel, snapshot)));
    } finally {
        ihnLiveBroadcastBusy = false;
        if (ihnLiveBroadcastQueued) scheduleLiveDocumentBroadcast();
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

async function ihnSendPayload(channel, payload) {
    if (channel?.readyState !== 'open') return;
    const raw = JSON.stringify(payload);
    if (raw.length > IHN_LIVE_MAX_CHARS) return;
    const id = `x${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
    const total = Math.ceil(raw.length / IHN_LIVE_CHUNK);
    if (!total || total > IHN_LIVE_MAX_CHUNKS) return;
    channel.send(JSON.stringify({ t: 'start', id, total, chars: raw.length }));
    for (let index = 0; index < total; index += 1) {
        await ihnWaitForBackpressure(channel);
        if (channel.readyState !== 'open') return;
        channel.send(JSON.stringify({ t: 'chunk', id, index, data: raw.slice(index * IHN_LIVE_CHUNK, (index + 1) * IHN_LIVE_CHUNK) }));
    }
}

async function ihnHandleWireMessage(raw, peerId) {
    if (typeof raw !== 'string') return;
    const staleBefore = Date.now() - 60_000;
    ihnLiveChunks.forEach((assembly, chunkKey) => {
        if (Number(assembly.createdAt || 0) < staleBefore) ihnLiveChunks.delete(chunkKey);
    });
    let message; try { message = JSON.parse(raw); } catch (error) { return; }
    const key = `${peerId}:${message?.id || ''}`;
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
    if (combined.length === assembly.chars) await ihnHandleLiveEnvelope(JSON.parse(combined), 'webrtc');
}

async function ihnHandleLiveEnvelope(envelope, transport = '') {
    if (!envelope || envelope.v !== 1 || envelope.type !== 'document-snapshot' || envelope.fileId !== state?.driveFileId) return;
    const ownActorId = `${ihnGetLivePeerId()}:${ihnGetLiveTabId()}`;
    if (!Array.isArray(envelope.pages) || !envelope.pages.length || envelope.actorId === ownActorId) return;
    const sequence = Number(envelope.sequence) || 0, previous = ihnLiveSeen.get(envelope.actorId) || 0;
    if (sequence <= previous) return;
    ihnLiveSeen.set(envelope.actorId, sequence);
    if (envelope.contentHash && envelope.contentHash === ihnLiveLastHash) return;
    ihnLiveApplying = true;
    try {
        const preserveLocal = !state.isReadOnly && hasUnsyncedLocalDriveChanges();
        const previousCalendar = JSON.stringify(state.calendarPageConfig || null);
        const result = await applyRemotePages(envelope.pages, { preserveLocalUnsynced: preserveLocal });
        if (!preserveLocal) {
            state.calendarPageConfig = cloneTimelineValue(envelope.calendarPageConfig || null, null);
            if (envelope.exportName) { state.exportName = envelope.exportName; updateDocTitle(); }
        }
        const calendarChanged = previousCalendar !== JSON.stringify(state.calendarPageConfig || null);
        if (calendarChanged && !result?.changed) {
            const savedAt = Date.now();
            const payload = buildMetaPayload(savedAt);
            await saveToIndexedDb(payload, savedAt);
            scheduleLocalStorageBackup(payload);
        }
        if (result?.changed || calendarChanged) showStatus(transport === 'tab' ? 'Live changes from another tab' : 'Live changes from collaborator', { savedAt: Number(envelope.sentAt) || Date.now() });
        const mergedHash = timelineSnapshotHash(
            state.pages.map(page => sanitizePageForStorage(page)),
            state.calendarPageConfig,
            state.exportName
        );
        ihnLiveLastHash = mergedHash;
        if (result?.hasLocalMerges && !state.isReadOnly) {
            driveDirty = true;
            requestImmediateDriveSave();
            ihnLiveBroadcastQueued = true;
        }
    } finally {
        ihnLiveApplying = false;
        if (ihnLiveBroadcastQueued) scheduleLiveDocumentBroadcast({ immediate: true });
    }
}
