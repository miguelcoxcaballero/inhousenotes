// internal-peer (BUILD-PLAN W2.8, e2e-internal `peer.selftest`): the shipped host/peer.js executor in Chromium —
// a loopback WebRTC connection between two of its own connections (offer/answer with the gathering rule, both
// negotiated channels, ordered control and bulk traffic with backpressure, close without stray events) and a
// BroadcastChannel echo between two pages. Records are built here with the out-buffer layout host/main.js
// hands to executors; events are decoded with the layouts of docs/v6/NOTES-W2.8.md.
import { expect, test } from '@playwright/test';

const PAGE = '/__peer.html';

async function harness(page) {
  await page.route(`**${PAGE}`, route => route.fulfill({ contentType: 'text/html', body: '<!doctype html><body></body>' }));
  await page.goto(PAGE);
  await page.evaluate(async () => {
    const mod = await import('/host/peer.js');
    const enc = new TextEncoder();
    const events = [];
    // A Reader over one record payload, with the methods of host/main.js's Reader.
    class Reader {
      constructor(bytes) { this.buf = bytes; this.view = new DataView(bytes.buffer); this.p = 0; }
      u8() { return this.buf[this.p++]; }
      u32() { const v = this.view.getUint32(this.p, true); this.p += 4; return v; }
      bytes() { const n = this.u32(); const v = this.buf.subarray(this.p, this.p + n); this.p += n; return v; }
      copy() { return this.bytes().slice(); }
      str() { return new TextDecoder().decode(this.bytes()); }
    }
    const decoder = new TextDecoder();
    const decode = bytes => {
      const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
      if (bytes[0] === 16) {
        const n = view.getUint32(1, true);
        return { ev: 16, name: decoder.decode(bytes.subarray(5, 5 + n)), data: bytes.slice(5 + n) };
      }
      const peer = { lo: view.getUint32(1, true), hi: view.getUint32(5, true) }, ev = bytes[0], rest = bytes.subarray(9);
      if (ev === 1) return { ev, peer, kind: rest[0], sdp: decoder.decode(rest.subarray(1)) };
      if (ev === 4) return { ev, peer, ch: rest[0], data: rest.slice(1) };
      return { ev, peer, ch: rest[0], state: rest[0] };
    };
    const ops = mod.default({ message: (chan, data) => events.push({ chan, ...decode(data) }) });
    // fields: ['u8', v] | ['u32', v] | ['str', s] | ['bytes', Uint8Array]
    const run = (op, ...fields) => {
      const parts = fields.map(([t, v]) => {
        if (t === 'u8') return Uint8Array.of(v);
        if (t === 'u32') { const b = new Uint8Array(4); new DataView(b.buffer).setUint32(0, v, true); return b; }
        const raw = t === 'str' ? enc.encode(v) : v;
        const b = new Uint8Array(4 + raw.length);
        new DataView(b.buffer).setUint32(0, raw.length, true);
        b.set(raw, 4);
        return b;
      });
      const all = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
      let p = 0;
      for (const part of parts) { all.set(part, p); p += part.length; }
      ops[(3 << 8) | op](new Reader(all));
    };
    const wait = async (pred, ms = 10_000) => {
      const end = performance.now() + ms;
      while (!pred()) {
        if (performance.now() > end) throw new Error(`timeout: ${JSON.stringify(events.map(e => [e.ev, e.peer?.lo, e.ch ?? e.kind ?? e.state]))}`);
        await new Promise(resolve => setTimeout(resolve, 10));
      }
    };
    window.__peer = { ops, run, events, wait, enc };
  });
}

test('peer.selftest: loopback RTC connection through the executor', async ({ page }) => {
  test.setTimeout(60_000);
  await harness(page);
  const result = await page.evaluate(async () => {
    const { run, events, wait } = window.__peer;
    // Peer A (lo 0x100, hi 1) is "the other side" of peer B (lo 0x200, hi 1): one page, two connections.
    const A = [['u32', 0x100], ['u32', 1]], B = [['u32', 0x200], ['u32', 1]];
    const of = (e, lo) => e.peer && e.peer.lo === lo && e.peer.hi === 1;
    const t0 = performance.now();
    run(0x60, ...A, ['u8', 1], ['u32', 0]);
    await wait(() => events.some(e => e.ev === 1 && of(e, 0x100)));
    const gatherMs = performance.now() - t0;
    const offer = events.find(e => e.ev === 1 && of(e, 0x100));
    run(0x60, ...B, ['u8', 0], ['u32', 0]);
    run(0x61, ...B, ['u8', 1], ['str', offer.sdp]);
    run(0x62, ...B);
    await wait(() => events.some(e => e.ev === 1 && of(e, 0x200)));
    const answer = events.find(e => e.ev === 1 && of(e, 0x200));
    run(0x61, ...A, ['u8', 2], ['str', answer.sdp]);
    const opened = lo => [0, 1].every(ch => events.some(e => e.ev === 2 && of(e, lo) && e.ch === ch));
    await wait(() => opened(0x100) && opened(0x200));
    // Control: a small frame each way. Bulk: 40 × 60 KiB in order (exceeds the 256 KiB high-water mark).
    run(0x64, ...A, ['u8', 0], ['bytes', Uint8Array.of(0x20, 1, 2, 3, 4)]);
    run(0x64, ...B, ['u8', 0], ['bytes', Uint8Array.of(0x21, 1, 2, 3, 4)]);
    const N = 40, SIZE = 60 << 10;
    for (let i = 0; i < N; i++) run(0x64, ...A, ['u8', 1], ['bytes', new Uint8Array(SIZE).fill(i)]);
    await wait(() => events.filter(e => e.ev === 4 && of(e, 0x200) && e.ch === 1).length === N, 20_000);
    const bulk = events.filter(e => e.ev === 4 && of(e, 0x200) && e.ch === 1);
    const ctlB = events.find(e => e.ev === 4 && of(e, 0x200) && e.ch === 0);
    const ctlA = events.find(e => e.ev === 4 && of(e, 0x100) && e.ch === 0);
    const drained = events.some(e => e.ev === 5 && of(e, 0x100));
    // Close A: B sees its channels close; A reports nothing more.
    const before = events.filter(e => of(e, 0x100)).length;
    run(0x63, ...A);
    await wait(() => events.some(e => e.ev === 3 && of(e, 0x200)));
    await new Promise(resolve => setTimeout(resolve, 300));
    return {
      offerKind: offer.kind, answerKind: answer.kind, offerHasData: /m=application/.test(offer.sdp), gatherMs,
      ctlB: Array.from(ctlB.data), ctlA: Array.from(ctlA.data),
      inOrder: bulk.every((e, i) => e.data.length === SIZE && e.data[0] === i && e.data[SIZE - 1] === i),
      drained, connected: events.some(e => e.ev === 6 && of(e, 0x200) && e.state === 2),
      afterClose: events.filter(e => of(e, 0x100)).length - before
    };
  });
  expect(result.offerKind).toBe(1);
  expect(result.answerKind).toBe(2);
  expect(result.offerHasData).toBe(true);
  expect(result.gatherMs).toBeLessThan(1500);
  expect(result.ctlB).toEqual([0x20, 1, 2, 3, 4]);
  expect(result.ctlA).toEqual([0x21, 1, 2, 3, 4]);
  expect(result.inOrder).toBe(true);
  expect(result.drained).toBe(true);
  expect(result.connected).toBe(true);
  expect(result.afterClose).toBe(0);
});

test('peer.selftest: BroadcastChannel echo between two tabs', async ({ page, context }) => {
  const other = await context.newPage();
  await harness(page);
  await harness(other);
  const name = 'ihn6:0123456789abcdef:doc-1';
  for (const p of [page, other]) await p.evaluate(n => window.__peer.run(0x65, ['str', n]), name);
  // The other tab echoes every tab message back with its first byte incremented.
  await other.evaluate(n => {
    const { run, events } = window.__peer;
    const push = events.push.bind(events);
    events.push = e => {
      push(e);
      if (e.ev === 16) run(0x66, ['str', n], ['bytes', Uint8Array.from(e.data, (b, i) => (i ? b : b + 1))]);
      return events.length;
    };
  }, name);
  const result = await page.evaluate(async n => {
    const { run, events, wait } = window.__peer;
    run(0x66, ['str', n], ['bytes', Uint8Array.of(0x02, 9, 8, 7)]);
    await wait(() => events.some(e => e.ev === 16));
    const echo = events.filter(e => e.ev === 16);
    run(0x67, ['str', n]);
    run(0x66, ['str', n], ['bytes', Uint8Array.of(0x02)]); // closed: nothing is sent
    await new Promise(resolve => setTimeout(resolve, 200));
    return { echo: echo.map(e => [e.name, Array.from(e.data)]), total: events.filter(e => e.ev === 16).length };
  }, name);
  // Only the echo arrives (a BroadcastChannel never delivers to its own sender).
  expect(result.echo).toEqual([[name, [0x03, 9, 8, 7]]]);
  expect(result.total).toBe(1);
  const seen = await other.evaluate(() => window.__peer.events.filter(e => e.ev === 16).length);
  expect(seen).toBe(1);
});
