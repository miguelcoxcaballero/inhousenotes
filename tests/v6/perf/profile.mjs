// Performance profile core (TESTING §6): the same phases, document and Drive mock for 5.18.0 and v6.
//
//   node tests/v6/perf/profile.mjs --app v5|v6 [--root <tree>] [--overlay build] [--throttle 1|4] [--short]
//        [--label name] [--out file.json]
//
// The document is the `dense-20` fixture (20 pages × 120 strokes × 40 points), opened like a user would: a stored
// session and the shared-link URL `/?drive=<id>&e2e=1`. Phases: `editor-idle` 60 s, `writing` 15 s of CDP pen strokes,
// `after-writing` 30 s, `home-idle` 60 s (`--short`: 30/15/15/30). Reported per phase: requests/min, KB/min, CPU %,
// script %, layout ms, JS heap MB, wasm memory MB (v6), long tasks, frame p95/max/janky; and input-ack p50/p95/max.
// Real time, no determinism scripts: this measures the app, not the harness. W6.2 adds the v6-only measurements
// (remote batches under the pen, boot marks, allocations, second device) and the gates on top of this file.
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { chromium } from '@playwright/test';
import { startServer } from '../server.mjs';
import { createMock } from '../mock/google.mjs';
import { addScene, addAccounts, ANA } from '../mock/fixtures.mjs';
import { chromiumExecutable } from '../parity/matrix.mjs';
import { ensureV5Ref, REPO } from '../parity/run.mjs';

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

/** Runs the profile once and returns the report object. */
export async function profile({ app = 'v5', root, overlay = null, throttle = 1, short = false, label = app, log = () => {} } = {}) {
  root = root || (app === 'v5' ? ensureV5Ref() : REPO);
  if (app === 'v6' && overlay === null && fs.existsSync(path.join(REPO, 'build', 'index.html'))) overlay = path.join(REPO, 'build');
  const server = await startServer({ root, overlay });
  const browser = await chromium.launch({ executablePath: chromiumExecutable() });
  try {
    const context = await browser.newContext({ viewport: { width: 1200, height: 900 }, serviceWorkers: 'block' });
    const mock = createMock({ drive: { now: () => Date.now() } });
    addAccounts(mock);
    const file = addScene(mock, 'dense-20', { title: 'Prof.pdf' });
    await mock.install(context);
    await context.addInitScript(entries => { if (!localStorage.getItem('ihn-test-seeded')) { localStorage.setItem('ihn-test-seeded', '1'); for (const [k, v] of Object.entries(entries)) localStorage.setItem(k, v); } }, mock.session(ANA));
    const page = await context.newPage();
    const errors = []; page.on('pageerror', error => errors.push(error.message));
    await page.goto(`${server.url}/?drive=${file.id}&e2e=1`);
    await page.waitForFunction(() => self.__IHN_TEST_API__, null, { timeout: 30_000 });
    await page.evaluate(() => self.__IHN_TEST_API__.ready());
    await page.waitForSelector('canvas.page-canvas', { state: 'visible', timeout: 60_000 });
    // A document opened from its link starts in Read mode; the profile measures an editing session.
    await page.locator('#mode-toggle-btn').first().click();
    await page.waitForFunction(() => { const pen = document.querySelector('[data-tool="pen"]'); return pen && !pen.disabled; }, null, { timeout: 15_000 });
    await sleep(8000);                                   // open settles: later pages, first save decisions
    const opened = await page.evaluate(() => ({ canvases: document.querySelectorAll('canvas.page-canvas').length, pages: document.querySelectorAll('.page-wrapper').length }));
    log(`opened ${JSON.stringify(opened)} after ${mock.calls.length} requests`);

    const cdp = await context.newCDPSession(page);
    await cdp.send('Performance.enable');
    if (throttle > 1) await cdp.send('Emulation.setCPUThrottlingRate', { rate: throttle });
    const metrics = async () => Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map(m => [m.name, m.value]));
    await page.evaluate(() => {
      self.__prof = { long: 0, longMs: 0, frames: [], last: 0 };
      new PerformanceObserver(list => { for (const entry of list.getEntries()) { __prof.long++; __prof.longMs += entry.duration; } }).observe({ type: 'longtask', buffered: false });
      const tick = t => { if (__prof.last) __prof.frames.push(t - __prof.last); __prof.last = t; requestAnimationFrame(tick); };
      requestAnimationFrame(tick);
    });
    // v6 reports its WebAssembly memories through the test API (`perf.memory` → {main, exportWorker} in bytes).
    const wasmMemory = () => page.evaluate(async () => { try { const m = await self.__IHN_TEST_API__.cmd?.('perf.memory'); return m ? { main: m.main || 0, exportWorker: m.exportWorker || 0 } : null; } catch { return null; } });

    const keyOf = call => `${call.method} ${call.host === 'www.googleapis.com' ? '' : call.host}${call.path.replace(/files\/[^/]+/, 'files/:id').replace(/(comments|revisions|permissions)\/[^/]+/, '$1/:id')}${/[?&]alt=media/.test(call.url) ? '?alt=media' : ''}${/[?&]q=/.test(call.url) ? '?q' : ''}`;
    const byKey = new Map();
    async function measure(phase, ms, during) {
      const first = mock.calls.length, m0 = await metrics();
      await page.evaluate(() => { __prof.long = 0; __prof.longMs = 0; __prof.frames = []; __prof.last = 0; });
      const t0 = Date.now();
      if (during) await during();
      const left = ms - (Date.now() - t0);
      if (left > 0) await sleep(left);
      const m1 = await metrics(), secs = (Date.now() - t0) / 1000, calls = mock.calls.slice(first);
      for (const call of calls) byKey.set(`${phase} ${keyOf(call)}`, (byKey.get(`${phase} ${keyOf(call)}`) || 0) + 1);
      const frames = await page.evaluate(() => { const f = __prof.frames.slice().sort((a, b) => a - b); return { long: __prof.long, longMs: Math.round(__prof.longMs), frames: f.length, p95: Math.round(f[Math.floor(f.length * 0.95)] || 0), max: Math.round(f.at(-1) || 0), janky: f.filter(x => x > 34).length }; });
      const wasm = await wasmMemory();
      const row = { phase, reqPerMin: +(calls.length / secs * 60).toFixed(1), kbPerMin: +(calls.reduce((n, c) => n + (c.bytes || 0), 0) / 1024 / secs * 60).toFixed(1),
        cpuPct: +((m1.TaskDuration - m0.TaskDuration) / secs * 100).toFixed(1), scriptPct: +((m1.ScriptDuration - m0.ScriptDuration) / secs * 100).toFixed(1),
        layoutMs: Math.round((m1.LayoutDuration - m0.LayoutDuration) * 1000), heapMB: Math.round(m1.JSHeapUsedSize / 1048576),
        wasmMB: wasm ? +((wasm.main + wasm.exportWorker) / 1048576).toFixed(1) : null, cpuSeconds: +(m1.TaskDuration - m0.TaskDuration).toFixed(2), ...frames };
      log(JSON.stringify(row));
      return row;
    }

    const results = [], inputAck = [];
    results.push(await measure('editor-idle', short ? 30_000 : 60_000));
    await page.locator('[data-tool="pen"]').first().click();
    results.push(await measure('writing', 15_000, async () => {
      const r = await page.evaluate(() => { const c = document.querySelector('canvas.page-canvas[data-page="0"]').getBoundingClientRect(); return { x: c.left, y: c.top, w: c.width, h: c.height }; });
      const t0 = Date.now(), mouse = (type, x, y, buttons) => cdp.send('Input.dispatchMouseEvent', { type, x, y, button: 'left', buttons, clickCount: 1, pointerType: 'pen' });
      for (let n = 0; Date.now() - t0 < 15_000; n++) {
        const sx = r.x + r.w * (0.1 + (n % 6) * 0.13), sy = r.y + r.h * (0.15 + (Math.floor(n / 6) % 8) * 0.09);
        await mouse('mousePressed', sx, sy, 1);
        for (let k = 1; k <= 30; k++) { const t = Date.now(); await mouse('mouseMoved', sx + k * 2.2, sy + Math.sin(k / 3) * 12, 1); inputAck.push(Date.now() - t); }
        await mouse('mouseReleased', sx + 66, sy, 0);
        await sleep(120);
      }
    }));
    results.push(await measure('after-writing', short ? 15_000 : 30_000));
    await page.locator('#btn-drive-home').first().click();
    await sleep(3000);
    results.push(await measure('home-idle', short ? 30_000 : 60_000));

    inputAck.sort((a, b) => a - b);
    return { label, app, throttle, short, opened, results, errors,
      inputAckMs: { p50: inputAck[Math.floor(inputAck.length / 2)], p95: inputAck[Math.floor(inputAck.length * 0.95)], max: inputAck.at(-1), n: inputAck.length },
      topRequests: [...byKey].sort((a, b) => b[1] - a[1]).slice(0, 60) };
  } finally { await browser.close(); await server.close(); }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const args = process.argv.slice(2), flag = name => args.includes(`--${name}`), opt = (name, fallback) => { const i = args.indexOf(`--${name}`); return i < 0 ? fallback : args[i + 1]; };
  profile({ app: opt('app', 'v5'), root: opt('root') && path.resolve(opt('root')), overlay: opt('overlay') ? path.resolve(opt('overlay')) : null, throttle: Number(opt('throttle', process.env.THROTTLE || 1)), short: flag('short') || !!process.env.SHORT,
    label: opt('label', opt('app', 'v5')), log: line => console.error(line) }).then(report => {
    const text = JSON.stringify(report, null, 1);
    if (opt('out')) fs.writeFileSync(path.resolve(opt('out')), text + '\n'); else console.log(text);
  }, error => { console.error(error); process.exit(2); });
}
