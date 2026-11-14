// W6.3p: memory of a tab while it opens a document, measured the same way for 5.18.0 and v6.
//   node tests/v6/perf/memory.mjs <tree: a v6 checkout with build/, or a 5.18.0 tree> [scene=dense-20] [--ms 12000]
//        [--overlay dir: another build/ of the v6 tree, e.g. one with `--features ihn-app/memstat`]
//
// Every 100 ms, for the page and for each of its workers (CDP auto-attach): `Runtime.getHeapUsage` — the JS heap in
// use plus `backingStorageSize`, the ArrayBuffer backing stores outside the heap (a wasm `memory.buffer`, pdf.js
// buffers, typed arrays). The tab total is their sum. Reported: the peak of that sum during the open, and the steady
// state once the open is quiet (after a forced GC in every isolate). v6 also reports `memory.buffer.byteLength` of
// the main instance. Canvas backing stores and the GPU process are outside both numbers (both apps draw the same
// pages, so they cancel out).
import { chromium } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import { startServer } from '../server.mjs';
import { createMock } from '../mock/google.mjs';
import { addScene, addAccounts, ANA } from '../mock/fixtures.mjs';
import { chromiumExecutable } from '../parity/matrix.mjs';

const args = process.argv.slice(2);
const opt = (name, fallback) => { const i = args.indexOf(`--${name}`); return i < 0 ? fallback : args[i + 1]; };
const [root, scene = 'dense-20'] = args.filter((a, i) => !a.startsWith('--') && !args[i - 1]?.startsWith('--'));
const total = Number(opt('ms', 12000));
const sleep = ms => new Promise(r => setTimeout(r, ms));
const v6 = fs.existsSync(path.join(root, 'build', 'index.html'));
const server = await startServer({ root, overlay: v6 ? path.resolve(opt('overlay', path.join(root, 'build'))) : null });
const browser = await chromium.launch({ executablePath: chromiumExecutable() });
const context = await browser.newContext({ viewport: { width: 1200, height: 900 }, serviceWorkers: 'block' });
const mock = createMock({ drive: { now: () => Date.now() } }); addAccounts(mock);
const file = addScene(mock, scene, { title: 'P.pdf' });
await mock.install(context);
await context.addInitScript(e => { if (!localStorage.getItem('s')) { localStorage.setItem('s', '1'); for (const [k, v] of Object.entries(e)) localStorage.setItem(k, v); } }, mock.session(ANA));
const page = await context.newPage();
page.on('pageerror', e => console.error('pageerror', e.message));
const cdp = await context.newCDPSession(page);

// Workers through the (non-flat) target protocol: one message id space per worker session.
const workers = new Map(); // sessionId → { type, pending: Map }
let nextId = 1;
cdp.on('Target.attachedToTarget', e => { workers.set(e.sessionId, { type: e.targetInfo.type, url: e.targetInfo.url, pending: new Map() }); });
cdp.on('Target.detachedFromTarget', e => workers.delete(e.sessionId));
cdp.on('Target.receivedMessageFromTarget', e => {
  const w = workers.get(e.sessionId); if (!w) return;
  const m = JSON.parse(e.message); const p = w.pending.get(m.id); if (p) { w.pending.delete(m.id); m.error ? p.reject(new Error(m.error.message)) : p.resolve(m.result); }
});
const wsend = (sessionId, method, params = {}) => new Promise((resolve, reject) => {
  const w = workers.get(sessionId); if (!w) return reject(new Error('gone'));
  const id = nextId++; w.pending.set(id, { resolve, reject });
  cdp.send('Target.sendMessageToTarget', { sessionId, message: JSON.stringify({ id, method, params }) }).catch(reject);
  setTimeout(() => { if (w.pending.delete(id)) reject(new Error('timeout')); }, 3000);
});
await cdp.send('Target.setAutoAttach', { autoAttach: true, waitForDebuggerOnStart: false, flatten: false });

const usage = u => ({ heap: u.usedSize, backing: u.backingStorageSize || 0 });
async function sample(gc = false) {
  if (gc) await cdp.send('HeapProfiler.collectGarbage').catch(() => {});
  const main = usage(await cdp.send('Runtime.getHeapUsage'));
  const ws = [];
  for (const [id, w] of workers) {
    try {
      if (gc) await wsend(id, 'HeapProfiler.collectGarbage');
      ws.push({ type: w.type, url: w.url.split('/').pop(), ...usage(await wsend(id, 'Runtime.getHeapUsage')) });
    } catch { /* starting or gone */ }
  }
  const wasm = v6 ? await page.evaluate(() => self.__IHN_TEST_API__?.wasmMemoryBytes?.() || 0).catch(() => 0) : 0;
  const sum = main.heap + main.backing + ws.reduce((n, w) => n + w.heap + w.backing, 0);
  return { main, ws, wasm, sum };
}
const MB = b => Math.round(b / 10485.76) / 100;
const t0 = Date.now();
await page.goto(`${server.url}/?drive=${file.id}&e2e=1`);
let peak = { sum: 0 }, opened = 0;
while (Date.now() - t0 < total) {
  const s = await sample().catch(() => null);
  if (s && s.sum > peak.sum) peak = { ...s, at: Date.now() - t0 };
  if (!opened && await page.evaluate(() => document.querySelectorAll('.page-wrapper canvas.page-canvas').length > 0).catch(() => false)) opened = Date.now() - t0;
  await sleep(100);
}
const steady = await sample(true);
// Workers still attached at the end (a worker that did not answer within 3 s would be missing from the sum).
const attached = [...workers.values()].map(w => `${w.type}:${w.url.split('/').pop().split('?')[0]}`);
if (attached.length !== steady.ws.length) console.error('WARNING: attached workers', attached, 'sampled', steady.ws.length);
if (v6 && process.env.MEM_STATE) console.log(JSON.stringify(await page.evaluate(() => self.__IHN_TEST_API__.cmd('doc.session.state'))).slice(0, 1500));
const memstat = v6 ? await page.evaluate(() => self.__IHN_TEST_API__.cmd('app.mem')).catch(() => null) : null;
const fmt = s => ({ totalMB: MB(s.sum), mainHeapMB: MB(s.main.heap), mainBackingMB: MB(s.main.backing), wasmMB: MB(s.wasm), workers: s.ws.map(w => `${w.type}:${w.url} heap ${MB(w.heap)} backing ${MB(w.backing)}`) });
console.log(JSON.stringify({ app: v6 ? 'v6' : '5.x', scene, openedMs: opened, peakAtMs: peak.at, peak: fmt(peak), steady: fmt(steady), attached }, null, 1));
if (memstat?.total != null) {
  // Tag → module name (the `modules!` table of rust/crates/ihn-app/src/app.rs, in order).
  const src = fs.readFileSync(path.join(root, 'rust/crates/ihn-app/src/app.rs'), 'utf8');
  const names = [...src.slice(src.indexOf('modules! {')).matchAll(/^\s+[A-Z]\w* \w+ "([\w.]+)";/gm)].map(m => m[1]);
  const name = t => names[t] || (t === 126 ? 'outside' : t === 125 ? 'app' : t >= 64 && names[t - 64] ? `after:${names[t - 64]}` : `tag${t}`);
  const list = a => a.map(([t, b]) => `${name(t)} ${MB(b)}`).join(', ');
  console.log(`memstat live ${MB(memstat.total)} MB, peak ${MB(memstat.peak)} MB\n  live: ${list(memstat.live)}\n  at peak: ${list(memstat.atPeak)}\n  growth (MB after: live, alloc by): ${(memstat.grow || []).map(([p, live, n, t]) => `${MB(p * 65536)}: ${MB(live)} ${n >= 2 ** 31 ? 'realloc ' : ''}${Math.round((n % 2 ** 31) / 1024)}K ${name(t)}`).join(' | ')}\n  blocks: ${memstat.big.map(([n, t]) => `${name(t)}:${Math.round(n / 1024)}K`).join(' ')}`);
}
await browser.close(); await server.close();
