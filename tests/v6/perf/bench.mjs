// Focused benchmarks, 5.18.0 vs v6 (W6.2): the same UI actions, Drive mock and fixtures for both apps.
//
//   node tests/v6/perf/bench.mjs --app v5|v6 [--root <tree>] [--overlay build] [--throttle 1|4] [--runs 3]
//        [--only boot,open,pen,erase,lasso,scroll,zoom,pinch,export] [--out file.json]
//
// Nothing app-specific is read: listener and rAF times come from wrapping `addEventListener` and
// `requestAnimationFrame` before either app loads, CPU from CDP `Performance.getMetrics`. Reported (ms unless noted):
//   boot      navigation → first Home card visible (signed in, small library); v6 also wasm compile+instantiate
//   open      navigation → first page canvas visible for `dense-20` (a 5.x PDF, 20 pages × 120 strokes × 40 points);
//             heap MB (JS heap after GC) and wasm MB after the open settles
//   pen       on `dense-page` (2400 strokes × 100 points on page 0): listener ms per pointermove, CPU ms per
//             move, rAF ms per frame p95, frame interval p95
//   erase / lasso  the same for the eraser and a lasso loop + release on the dense page
//   scroll / zoom / pinch  wheel, ctrl+wheel and two-finger pinch: frame interval p95/max, CPU ms per step
//   export    Export PDF of dense-20 → download event
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { chromium } from '@playwright/test';
import { startServer } from '../server.mjs';
import { createMock } from '../mock/google.mjs';
import { addScene, addAccounts, fixtures, ANA } from '../mock/fixtures.mjs';
import { chromiumExecutable } from '../parity/matrix.mjs';
import { ensureV5Ref, REPO } from '../parity/run.mjs';

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const median = xs => { const s = xs.slice().sort((a, b) => a - b); return s.length ? s[Math.floor((s.length - 1) / 2)] : null; };
const pct = (xs, p) => { const s = xs.slice().sort((a, b) => a - b); return s.length ? s[Math.min(s.length - 1, Math.floor(s.length * p))] : null; };
const r2 = x => x == null ? null : Math.round(x * 100) / 100;

// Runs in the page before any app script.
function instrument() {
  const B = self.__bench = { events: [], frames: new Map(), intervals: [], last: 0 };
  const TYPES = new Set(['pointerdown', 'pointermove', 'pointerup', 'pointerrawupdate', 'pointercancel', 'wheel', 'touchstart', 'touchmove', 'touchend', 'scroll', 'mousemove', 'mousedown', 'mouseup']);
  const seen = new WeakMap(), wrapped = new WeakMap();
  const add = EventTarget.prototype.addEventListener, remove = EventTarget.prototype.removeEventListener;
  const wrap = fn => {
    let w = wrapped.get(fn);
    if (!w) {
      w = function (event) {
        const t = performance.now();
        try { return typeof fn === 'function' ? fn.call(this, event) : fn.handleEvent(event); } finally {
          let entry = seen.get(event);
          if (!entry) { entry = { type: event.type, ms: 0 }; seen.set(event, entry); B.events.push(entry); }
          entry.ms += performance.now() - t;
        }
      };
      wrapped.set(fn, w);
    }
    return w;
  };
  EventTarget.prototype.addEventListener = function (type, fn, options) { return add.call(this, type, fn && TYPES.has(type) ? wrap(fn) : fn, options); };
  EventTarget.prototype.removeEventListener = function (type, fn, options) { return remove.call(this, type, fn && TYPES.has(type) ? (wrapped.get(fn) || fn) : fn, options); };
  const raf = window.requestAnimationFrame.bind(window);
  window.requestAnimationFrame = cb => raf(t => { const s = performance.now(); try { cb(t); } finally { B.frames.set(t, (B.frames.get(t) || 0) + performance.now() - s); } });
  const tick = t => { if (B.last) B.intervals.push(t - B.last); B.last = t; raf(tick); };
  raf(tick);
  B.reset = () => { B.events = []; B.frames = new Map(); B.intervals = []; B.last = 0; };
}

export async function bench({ app = 'v5', root, overlay = null, throttle = 1, runs = 3, only = null, log = () => {} } = {}) {
  root = root || (app === 'v5' ? ensureV5Ref() : REPO);
  if (app === 'v6' && overlay === null && fs.existsSync(path.join(root, 'build', 'index.html'))) overlay = path.join(root, 'build');
  const server = await startServer({ root, overlay });
  const browser = await chromium.launch({ executablePath: chromiumExecutable() });
  const want = name => !only || only.includes(name);
  const out = { app, throttle };

  async function session({ scene, touch = false, library = false } = {}) {
    const context = await browser.newContext({ viewport: { width: 1200, height: 900 }, serviceWorkers: 'block', hasTouch: touch, acceptDownloads: true });
    const mock = createMock({ drive: { now: () => Date.now() } });
    addAccounts(mock);
    if (library) fixtures.library(mock);
    const file = scene ? addScene(mock, scene, { title: `${scene}.pdf` }) : null;
    await mock.install(context);
    await context.addInitScript(entries => { if (!localStorage.getItem('ihn-test-seeded')) { localStorage.setItem('ihn-test-seeded', '1'); for (const [k, v] of Object.entries(entries)) localStorage.setItem(k, v); } }, mock.session(ANA));
    if (!process.env.NO_INSTR) await context.addInitScript(instrument);
    const page = await context.newPage();
    const errors = []; page.on('pageerror', error => errors.push(error.message));
    const cdp = await context.newCDPSession(page);
    await cdp.send('Performance.enable');
    if (throttle > 1) await cdp.send('Emulation.setCPUThrottlingRate', { rate: throttle });
    const metrics = async () => Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map(m => [m.name, m.value]));
    return { context, mock, file, page, cdp, metrics, errors };
  }
  async function openEditor(s, { edit = true } = {}) {
    const t0 = Date.now();
    await s.page.goto(`${server.url}/?drive=${s.file.id}&e2e=1`);
    const first = await s.page.waitForFunction(() => { const c = document.querySelector('canvas.page-canvas'); return c && c.offsetParent && c.width > 0 ? performance.now() : 0; }, null, { timeout: 120_000, polling: 'raf' }).then(h => h.jsonValue());
    const wall = Date.now() - t0;
    if (edit) {
      await s.page.locator('#mode-toggle-btn').first().click();
      await s.page.waitForFunction(() => { const pen = document.querySelector('[data-tool="pen"]'); return pen && !pen.disabled; }, null, { timeout: 30_000 });
    }
    return { first: Math.round(first), wall };
  }
  const settle = async s => { for (let i = 0; i < 40; i++) { const a = await s.metrics(); await sleep(500); const b = await s.metrics(); if (b.TaskDuration - a.TaskDuration < 0.02) return; } };
  async function canvasRect(s, n = 0) {
    return s.page.evaluate(n => { const c = document.querySelector(`canvas.page-canvas[data-page="${n}"]`); c.scrollIntoView({ block: 'center' }); const r = c.getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height }; }, n);
  }
  // Runs `drive` and reports listener time per event of `type`, CPU per step, rAF work and frame intervals.
  async function phase(s, type, steps, drive) {
    await s.page.evaluate(() => __bench.reset());
    const m0 = await s.metrics();
    const t0 = Date.now();
    await drive();
    await sleep(600);
    const m1 = await s.metrics();
    const data = await s.page.evaluate(type => ({ ev: __bench.events.filter(e => e.type === type).map(e => e.ms), all: __bench.events.reduce((n, e) => n + e.ms, 0), frames: [...__bench.frames.values()], intervals: __bench.intervals.slice(1) }), type);
    return { [`${type}Ms`]: r2(data.ev.reduce((a, b) => a + b, 0) / Math.max(1, data.ev.length)), [`${type}P95`]: r2(pct(data.ev, 0.95)), n: data.ev.length, listenersMs: Math.round(data.all),
      cpuPerStep: r2((m1.TaskDuration - m0.TaskDuration) * 1000 / steps), scriptPerStep: r2((m1.ScriptDuration - m0.ScriptDuration) * 1000 / steps),
      rafP95: r2(pct(data.frames, 0.95)), rafMax: r2(Math.max(0, ...data.frames)), frameP95: r2(pct(data.intervals, 0.95)), frameMax: r2(Math.max(0, ...data.intervals)), janky: data.intervals.filter(x => x > 34).length, secs: (Date.now() - t0) / 1000 };
  }
  const mouse = (s, type, x, y, buttons) => s.cdp.send('Input.dispatchMouseEvent', { type, x, y, button: 'left', buttons, clickCount: 1, pointerType: 'pen' });
  async function strokes(s, r, count, points, { dx = 2.2, dy = 12 } = {}) {
    for (let n = 0; n < count; n++) {
      const sx = r.x + r.w * (0.1 + (n % 6) * 0.13), sy = r.y + r.h * (0.15 + (Math.floor(n / 6) % 8) * 0.09);
      await mouse(s, 'mousePressed', sx, sy, 1);
      for (let k = 1; k <= points; k++) await mouse(s, 'mouseMoved', sx + k * dx, sy + Math.sin(k / 3) * dy, 1);
      await mouse(s, 'mouseReleased', sx + points * dx, sy, 0);
      await sleep(60);
    }
  }

  try {
    if (want('boot')) {
      const rows = [];
      for (let i = 0; i < runs; i++) {
        const s = await session({ library: true });
        await s.page.goto(`${server.url}/?e2e=1`);
        const home = await s.page.waitForFunction(() => { const c = document.querySelector('#drive-recents .drive-card'); return c && c.offsetParent ? performance.now() : 0; }, null, { timeout: 60_000, polling: 'raf' }).then(h => h.jsonValue());
        const marks = await s.page.evaluate(() => Object.fromEntries(performance.getEntriesByType('mark').map(m => [m.name, m.startTime])));
        const res = await s.page.evaluate(() => { const e = performance.getEntriesByType('resource'); return { requests: e.length, kb: Math.round(e.reduce((n, r) => n + (r.encodedBodySize || r.transferSize || 0), 0) / 1024) }; });
        let instantiate = null;
        if (app === 'v6') instantiate = await s.page.evaluate(async () => {
          const bytes = await (await fetch('/wasm/ihn.wasm')).arrayBuffer();
          const t0 = performance.now(); const module = await WebAssembly.compile(bytes); const t1 = performance.now();
          const env = {}; for (const i of WebAssembly.Module.imports(module)) env[i.name] = () => 0;
          await WebAssembly.instantiate(module, { env }); const t2 = performance.now();
          return { compile: t1 - t0, instantiate: t2 - t1 };
        });
        rows.push({ home, marks, res, instantiate });
        await s.context.close();
      }
      out.boot = { homeMs: Math.round(median(rows.map(r => r.home))), requests: rows[0].res.requests, kb: rows[0].res.kb,
        wasmReadyMs: app === 'v6' ? r2(median(rows.map(r => r.marks['ihn-wasm-ready'] - r.marks['ihn-boot-start']))) : null,
        compileMs: app === 'v6' ? r2(median(rows.map(r => r.instantiate.compile))) : null, instantiateMs: app === 'v6' ? r2(median(rows.map(r => r.instantiate.instantiate))) : null };
      log(`boot ${JSON.stringify(out.boot)}`);
    }
    if (want('open') || want('export')) {
      const rows = [];
      for (let i = 0; i < (want('open') ? runs : 1); i++) {
        const s = await session({ scene: 'dense-20' });
        const opened = await openEditor(s, { edit: false });
        const pages = await s.page.waitForFunction(() => document.querySelectorAll('.page-wrapper').length >= 20 ? performance.now() : 0, null, { timeout: 60_000, polling: 'raf' }).then(h => h.jsonValue());
        await settle(s);
        await s.cdp.send('HeapProfiler.collectGarbage');
        const m = await s.metrics();
        const wasm = await s.page.evaluate(() => self.__IHN_TEST_API__?.wasmMemoryBytes?.() || 0);
        const row = { first: opened.first, pages: Math.round(pages), heapMB: r2(m.JSHeapUsedSize / 1048576), wasmMB: r2(wasm / 1048576), nodes: m.Nodes };
        if (want('export') && i === 0) {
          await s.page.locator('#mode-toggle-btn').first().click().catch(() => {});
          await sleep(500);
          const t0 = Date.now();
          const download = s.page.waitForEvent('download', { timeout: 120_000 });
          await s.page.locator('#btn-export-pdf').first().click();
          await s.page.locator('#modal-export-confirm').click();
          const d = await download;
          row.exportMs = Date.now() - t0;
          row.exportKB = Math.round(fs.statSync(await d.path()).size / 1024);
          await s.cdp.send('HeapProfiler.collectGarbage');
          const m2 = await s.metrics();
          row.heapAfterExportMB = r2(m2.JSHeapUsedSize / 1048576);
          row.wasmAfterExportMB = r2((await s.page.evaluate(() => self.__IHN_TEST_API__?.wasmMemoryBytes?.() || 0)) / 1048576);
        }
        row.errors = s.errors.length;
        rows.push(row);
        await s.context.close();
      }
      out.open = { firstCanvasMs: median(rows.map(r => r.first)), allPagesMs: median(rows.map(r => r.pages)), heapMB: median(rows.map(r => r.heapMB)), wasmMB: median(rows.map(r => r.wasmMB)), nodes: rows[0].nodes, errors: rows[0].errors };
      if (rows[0].exportMs != null) out.export = { ms: rows[0].exportMs, kb: rows[0].exportKB, heapAfterMB: rows[0].heapAfterExportMB, wasmAfterMB: rows[0].wasmAfterExportMB };
      log(`open ${JSON.stringify(out.open)} export ${JSON.stringify(out.export)}`);
    }
    if (['pen', 'erase', 'lasso', 'scroll', 'zoom'].some(want)) {
      const s = await session({ scene: 'dense-page' });
      await openEditor(s);
      await settle(s);
      const r = await canvasRect(s, 0);
      if (want('pen')) {
        await s.page.locator('[data-tool="pen"]').first().click();
        await sleep(300);
        out.pen = await phase(s, 'pointermove', 12 * 30, () => strokes(s, r, 12, 30));
        log(`pen ${JSON.stringify(out.pen)}`);
        await settle(s);
      }
      if (want('erase')) {
        await s.page.locator('[data-tool="eraser"]').first().click();
        await sleep(300);
        out.erase = await phase(s, 'pointermove', 6 * 30, () => strokes(s, r, 6, 30, { dx: 6, dy: 30 }));
        log(`erase ${JSON.stringify(out.erase)}`);
        await settle(s);
      }
      if (want('lasso')) {
        await s.page.locator('[data-tool="lasso"]').first().click();
        await sleep(300);
        const cx = r.x + r.w * 0.5, cy = r.y + r.h * 0.4, R = Math.min(r.w, r.h) * 0.25;
        out.lasso = await phase(s, 'pointermove', 48, async () => {
          await mouse(s, 'mousePressed', cx + R, cy, 1);
          for (let k = 1; k <= 48; k++) await mouse(s, 'mouseMoved', cx + R * Math.cos(k / 48 * 2 * Math.PI), cy + R * Math.sin(k / 48 * 2 * Math.PI), 1);
          const t = Date.now();
          await mouse(s, 'mouseReleased', cx + R, cy, 0);
          out.lassoReleaseMs = Date.now() - t;
        });
        await s.page.keyboard.press('Escape').catch(() => {});
        log(`lasso ${JSON.stringify(out.lasso)}`);
        await settle(s);
      }
      if (want('scroll')) {
        await s.page.mouse.move(r.x + r.w / 2, 450);
        out.scroll = await phase(s, 'wheel', 40, async () => { for (let i = 0; i < 40; i++) { await s.page.mouse.wheel(0, i < 20 ? 120 : -120); await sleep(16); } });
        log(`scroll ${JSON.stringify(out.scroll)}`);
        await settle(s);
      }
      if (want('zoom')) {
        await s.page.mouse.move(r.x + r.w / 2, 450);
        await s.page.keyboard.down('Control');
        out.zoom = await phase(s, 'wheel', 20, async () => { for (let i = 0; i < 20; i++) { await s.page.mouse.wheel(0, i < 10 ? -100 : 100); await sleep(32); } });
        await s.page.keyboard.up('Control');
        log(`zoom ${JSON.stringify(out.zoom)}`);
        await settle(s);
        // One zoom step and the full repaint of the dense page at the new scale, until the CPU is idle again.
        const m0 = await s.metrics();
        await s.page.keyboard.down('Control'); await s.page.mouse.wheel(0, -100); await s.page.keyboard.up('Control');
        await settle(s);
        const m1 = await s.metrics();
        out.repaint = { cpuMs: Math.round((m1.TaskDuration - m0.TaskDuration) * 1000) };
        log(`repaint ${JSON.stringify(out.repaint)}`);
      }
      out.denseErrors = s.errors.slice(0, 3);
      await s.context.close();
    }
    if (want('pinch')) {
      const s = await session({ scene: 'dense-page', touch: true });
      await openEditor(s);
      await settle(s);
      const r = await canvasRect(s, 0);
      const cx = r.x + r.w / 2, cy = 450;
      const touch = (type, points) => s.cdp.send('Input.dispatchTouchEvent', { type, touchPoints: points.map(([x, y], id) => ({ x, y, id })) });
      out.pinch = await phase(s, 'touchmove', 60, async () => {
        for (let round = 0; round < 2; round++) {
          await touch('touchStart', [[cx - 60, cy], [cx + 60, cy]]);
          for (let k = 1; k <= 30; k++) { const d = 60 + (round ? 30 - k : k) * 4; await touch('touchMove', [[cx - d, cy], [cx + d, cy]]); await sleep(16); }
          await touch('touchEnd', []);
          await sleep(300);
        }
      });
      log(`pinch ${JSON.stringify(out.pinch)}`);
      await s.context.close();
    }
    return out;
  } finally { await browser.close(); await server.close(); }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const args = process.argv.slice(2), opt = (name, fallback) => { const i = args.indexOf(`--${name}`); return i < 0 ? fallback : args[i + 1]; };
  bench({ app: opt('app', 'v5'), root: opt('root') && path.resolve(opt('root')), overlay: opt('overlay') ? path.resolve(opt('overlay')) : null,
    throttle: Number(opt('throttle', 1)), runs: Number(opt('runs', 3)), only: opt('only') ? opt('only').split(',') : null, log: line => console.error(line) }).then(report => {
    const text = JSON.stringify(report, null, 1);
    if (opt('out')) fs.writeFileSync(path.resolve(opt('out')), text + '\n'); else console.log(text);
  }, error => { console.error(error); process.exit(2); });
}
