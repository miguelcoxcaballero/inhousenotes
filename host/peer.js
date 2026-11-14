// WebRTC + BroadcastChannel executor, effect family 3.5 (SYNC §10.3–10.4, §11, §18.3). It makes no decisions.
// Ops and event layouts: docs/v6/NOTES-W2.8.md. Peer = (u32 lo, u32 hi), echoed as 8 LE bytes in events.
const FX = 3 << 8, CHAN = 6, HIGH = 256 << 10, STATES = ['new', 'connecting', 'connected', 'disconnected', 'failed', 'closed'];
const enc = new TextEncoder();

// Gathering complete, or 70 ms after the first srflx candidate, at most 650 ms (5.x values).
const gathered = pc => new Promise(resolve => {
  let settle = 0;
  const done = () => { clearTimeout(max); clearTimeout(settle); pc.onicecandidate = pc.onicegatheringstatechange = null; resolve(); };
  const max = setTimeout(done, 650);
  pc.onicecandidate = e => (!e.candidate ? done() : !settle && / typ srflx /.test(e.candidate.candidate) && (settle = setTimeout(done, 70)));
  pc.onicegatheringstatechange = () => pc.iceGatheringState === 'complete' && done();
});

export default function install(host) {
  const conns = new Map(), tabs = new Map();
  const peerOf = r => { const lo = r.u32(), hi = r.u32(); return { key: `${hi}:${lo}`, lo, hi }; };
  const cat = (...parts) => {
    const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
    parts.reduce((at, p) => (out.set(p, at), at + p.length), 0);
    return out;
  };
  const emit = (ev, peer, head = [], data = []) => {
    const id = new Uint8Array(8), v = new DataView(id.buffer);
    v.setUint32(0, peer.lo, true);
    v.setUint32(4, peer.hi, true);
    host.message(CHAN, cat([ev], id, head, data));
  };
  const drop = key => {
    const e = conns.get(key);
    conns.delete(key);
    try { e?.pc.close(); } catch (_) { /* closed */ }
  };
  // One connection's steps run in order; a failure reports `failed`. Nothing is reported once it is dropped.
  const step = (e, fn) => {
    e.chain = e.chain.then(() => conns.get(e.peer.key) === e && fn()).catch(() => conns.get(e.peer.key) === e && emit(6, e.peer, [4]));
  };
  const local = (e, kind, make) => step(e, async () => {
    const ready = gathered(e.pc);
    await e.pc.setLocalDescription(await make());
    await ready;
    if (conns.get(e.peer.key) === e) emit(1, e.peer, [kind], enc.encode(e.pc.localDescription.sdp));
  });
  const flush = e => { const b = e.ch[1]; while (e.queue.length && b.readyState === 'open' && b.bufferedAmount < HIGH) b.send(e.queue.shift()); };

  function create(peer, offer, ice) {
    drop(peer.key);
    const pc = new RTCPeerConnection({ iceServers: ice.map(urls => ({ urls })), bundlePolicy: 'max-bundle', rtcpMuxPolicy: 'require' });
    const ch = ['ihn6', 'ihn6-bulk'].map((label, id) => pc.createDataChannel(label, { negotiated: true, id, ordered: true }));
    const e = { peer, pc, ch, queue: [], chain: Promise.resolve() };
    const live = () => conns.get(peer.key) === e;
    ch.forEach((c, i) => {
      c.binaryType = 'arraybuffer';
      c.onopen = () => live() && emit(2, peer, [i]);
      c.onclose = () => live() && emit(3, peer, [i]);
      c.onmessage = m => live() && m.data instanceof ArrayBuffer && emit(4, peer, [i], new Uint8Array(m.data));
    });
    ch[1].bufferedAmountLowThreshold = HIGH / 2;
    ch[1].onbufferedamountlow = () => live() && (flush(e), emit(5, peer));
    pc.onconnectionstatechange = () => live() && emit(6, peer, [Math.max(0, STATES.indexOf(pc.connectionState))]);
    conns.set(peer.key, e);
    if (offer) local(e, 1, () => pc.createOffer());
  }

  return {
    // rtc_create(peer, u8 offer, u32 n, n × str ice)
    [FX | 0x60]: r => { const peer = peerOf(r), offer = r.u8() === 1; create(peer, offer, Array.from({ length: r.u32() }, () => r.str())); },
    // rtc_remote(peer, u8 kind 1 offer | 2 answer, str sdp)
    [FX | 0x61]: r => {
      const e = conns.get(peerOf(r).key), type = r.u8() === 1 ? 'offer' : 'answer', sdp = r.str();
      if (e) step(e, () => e.pc.setRemoteDescription({ type, sdp }));
    },
    // rtc_answer(peer)
    [FX | 0x62]: r => { const e = conns.get(peerOf(r).key); if (e) local(e, 2, () => e.pc.createAnswer()); },
    // rtc_close(peer)
    [FX | 0x63]: r => drop(peerOf(r).key),
    // rtc_send(peer, u8 ch, bytes): the bulk channel queues above 256 KiB buffered
    [FX | 0x64]: r => {
      const e = conns.get(peerOf(r).key), i = r.u8(), bytes = r.copy().buffer, c = e?.ch[i];
      if (c?.readyState !== 'open') return;
      try {
        if (i === 1 && (e.queue.length || c.bufferedAmount >= HIGH)) e.queue.push(bytes); else c.send(bytes);
      } catch (_) { emit(6, e.peer, [4]); }
    },
    // bc_open(str name): BroadcastChannel; messages arrive as event 16 [u32 name length][name][data]
    [FX | 0x65]: r => {
      const name = r.str();
      if (tabs.has(name)) return;
      const bc = new BroadcastChannel(name), head = enc.encode(name), n = new Uint8Array(4);
      new DataView(n.buffer).setUint32(0, head.length, true);
      bc.onmessage = m => m.data instanceof ArrayBuffer && host.message(CHAN, cat([16], n, head, new Uint8Array(m.data)));
      tabs.set(name, bc);
    },
    // bc_send(str name, bytes)
    [FX | 0x66]: r => { const bc = tabs.get(r.str()), bytes = r.copy().buffer; bc?.postMessage(bytes, [bytes]); },
    // bc_close(str name)
    [FX | 0x67]: r => { const name = r.str(); tabs.get(name)?.close(); tabs.delete(name); }
  };
}
