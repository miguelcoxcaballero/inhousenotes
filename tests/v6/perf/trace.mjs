// W6.3p: what each app asks the browser to do for the same scripted scroll or zoom on `dense-page`.
//   node tests/v6/perf/trace.mjs <tree: a v6 checkout with build/, or a 5.18.0 tree> scroll|zoom [--out file.json]
//        [--calls]  (also count canvas 2D calls and DOM mutations: a separate run, the wrappers cost time)
//
// CDP tracing (devtools.timeline, blink, cc, gpu, viz, disabled-by-default-devtools.timeline) around the action;
// trace events are summed by name (count, total ms) for the renderer main thread, the compositor, raster workers and
// the GPU process. Also reported: the canvases alive with their backing-store sizes, layers with will-change or a
// transform, and the `Performance.getMetrics` deltas.
import { chromium } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import { startServer } from '../server.mjs';
import { createMock } from '../mock/google.mjs';
import { addScene, addAccounts, ANA } from '../mock/fixtures.mjs';
import { chromiumExecutable } from '../parity/matrix.mjs';

const args = process.argv.slice(2);
const opt = (name, fallback) => { const i = args.indexOf(`--${name}`); return i < 0 ? fallback : args[i + 1]; };
const [root, action = 'scroll'] = args.filter((a, i) => !a.startsWith('--') && !['--out'].includes(args[i - 1]));
const calls = args.includes('--calls');
const sleep = ms => new Promise(r => setTimeout(r, ms));
const v6 = fs.existsSync(path.join(root, 'build', 'index.html'));
const server = await startServer({ root, overlay: v6 ? path.join(root, 'build') : null });
const browser = await chromium.launch({ executablePath: chromiumExecutable() });
const context = await browser.newContext({ viewport: { width: 1200, height: 900 }, serviceWorkers: 'block' });
const mock = createMock({ drive: { now: () => Date.now() } }); addAccounts(mock);
const file = addScene(mock, 'dense-page', { title: 'P.pdf' });
await mock.install(context);
await context.addInitScript(e => { if (!localStorage.getItem('s')) { localStorage.setItem('s', '1'); for (const [k, v] of Object.entries(e)) localStorage.setItem(k, v); } }, mock.session(ANA));
if (calls) await context.addInitScript(() => {
  const C = self.__calls = { ctx: {}, mut: 0, recs: 0, put: [], draw: [], canvasSize: 0 };
  const proto = CanvasRenderingContext2D.prototype;
  for (const name of Object.getOwnPropertyNames(proto)) {
    const d = Object.getOwnPropertyDescriptor(proto, name);
    if (typeof d.value === 'function') {
      const f = d.value;
      proto[name] = function (...a) {
        C.ctx[name] = (C.ctx[name] || 0) + 1;
        if (name === 'drawImage') C.draw.push(`${a[0]?.width}x${a[0]?.height}${a.length > 3 ? `→${Math.round(a[a.length - 2])}x${Math.round(a[a.length - 1])}` : ''}`);
        if (name === 'putImageData') C.put.push(`${a[0]?.width}x${a[0]?.height}`);
        return f.apply(this, a);
      };
    } else if (d.set) {
      const set = d.set;
      Object.defineProperty(proto, name, { ...d, set(v) { C.ctx[`=${name}`] = (C.ctx[`=${name}`] || 0) + 1; set.call(this, v); } });
    }
  }
  for (const k of ['width', 'height']) {
    const d = Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype, k);
    Object.defineProperty(HTMLCanvasElement.prototype, k, { ...d, set(v) { C.canvasSize++; d.set.call(this, v); } });
  }
  new MutationObserver(list => { C.mut++; C.recs += list.length; }).observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
  C.reset = () => { C.ctx = {}; C.mut = 0; C.recs = 0; C.put = []; C.draw = []; C.canvasSize = 0; };
});
const page = await context.newPage();
await page.goto(`${server.url}/?drive=${file.id}&e2e=1`);
for (let i = 0; i < 600; i++) { if (await page.evaluate(() => !!document.querySelector('canvas.page-canvas')?.offsetParent).catch(() => false)) break; await sleep(50); }
await page.locator('#mode-toggle-btn').first().click();
await sleep(5000);
const r = await page.evaluate(() => { const c = document.querySelector('canvas.page-canvas[data-page="0"]'); c.scrollIntoView({ block: 'center' }); const b = c.getBoundingClientRect(); return { x: b.left, y: b.top, w: b.width, h: b.height }; });
await sleep(1500);
const inventory = () => page.evaluate(() => {
  const canvases = [...document.querySelectorAll('canvas')].map(c => {
    const s = getComputedStyle(c), b = c.getBoundingClientRect();
    return { cls: c.className.baseVal ?? c.className, page: c.dataset.page, px: `${c.width}x${c.height}`, css: `${Math.round(b.width)}x${Math.round(b.height)}`, shown: !!c.offsetParent, wc: s.willChange, tf: s.transform !== 'none' ? s.transform.slice(0, 40) : '', bg: s.backgroundImage.length > 4 ? s.backgroundImage.length : 0 };
  });
  const layers = [...document.querySelectorAll('*')].filter(e => { const s = getComputedStyle(e); return s.willChange !== 'auto' || s.transform !== 'none' || s.contain !== 'none' || (s.position === 'fixed'); }).map(e => `${e.tagName.toLowerCase()}${e.id ? '#' + e.id : ''}${e.className && typeof e.className === 'string' ? '.' + e.className.split(' ').slice(0, 2).join('.') : ''} wc=${getComputedStyle(e).willChange} tf=${getComputedStyle(e).transform.slice(0, 30)}`);
  const megapixels = canvases.reduce((n, c) => { const [w, h] = c.px.split('x').map(Number); return n + w * h; }, 0) / 1e6;
  return { canvases, megapixels: Math.round(megapixels * 100) / 100, layers, nodes: document.getElementsByTagName('*').length };
});
const before = await inventory();
const cdp = await context.newCDPSession(page);
await cdp.send('Performance.enable');
const metrics = async () => Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map(m => [m.name, m.value]));
if (calls) await page.evaluate(() => self.__calls.reset());
const events = [];
cdp.on('Tracing.dataCollected', e => events.push(...e.value));
const done = new Promise(res => cdp.once('Tracing.tracingComplete', res));
const m0 = await metrics();
await cdp.send('Tracing.start', { transferMode: 'ReportEvents', traceConfig: { includedCategories: ['devtools.timeline', 'disabled-by-default-devtools.timeline', 'blink', 'cc', 'gpu', 'viz', 'toplevel', 'v8', 'benchmark', 'disabled-by-default-devtools.timeline.frame'], recordMode: 'recordAsMuchAsPossible' } });
const t0 = Date.now();
await page.mouse.move(r.x + r.w / 2, 450);
if (action === 'scroll') { for (let i = 0; i < 40; i++) { await page.mouse.wheel(0, i < 20 ? 120 : -120); await sleep(16); } await sleep(1000); }
if (action === 'zoom') { await page.keyboard.down('Control'); await page.mouse.wheel(0, -100); await page.keyboard.up('Control'); await sleep(8000); }
await cdp.send('Tracing.end');
await done;
const m1 = await metrics();
const wall = Date.now() - t0;
const after = await inventory();
const callStats = calls ? await page.evaluate(() => { const C = self.__calls; const tally = a => Object.entries(a.reduce((m, k) => (m[k] = (m[k] || 0) + 1, m), {})).sort((x, y) => y[1] - x[1]).slice(0, 12); return { ctx: Object.entries(C.ctx).sort((a, b) => b[1] - a[1]).slice(0, 30), mutations: C.mut, records: C.recs, canvasResizes: C.canvasSize, put: tally(C.put), draw: tally(C.draw) }; }) : null;

// Threads by name.
const threads = new Map(), procs = new Map();
for (const e of events) {
  if (e.ph === 'M' && e.name === 'thread_name') threads.set(`${e.pid}:${e.tid}`, e.args.name);
  if (e.ph === 'M' && e.name === 'process_name') procs.set(e.pid, e.args.name);
}
const sum = new Map();
for (const e of events) {
  if (e.ph !== 'X' && e.ph !== 'B') continue;
  const th = threads.get(`${e.pid}:${e.tid}`) || '?';
  const proc = procs.get(e.pid) || '?';
  const key = `${proc.replace(/ Process$/, '')}/${th.replace(/\d+$/, '')} ${e.name}`;
  const s = sum.get(key) || { n: 0, ms: 0 };
  s.n++; s.ms += (e.dur || 0) / 1000;
  sum.set(key, s);
}
const top = [...sum].filter(([, s]) => s.ms > 0.5 || s.n > 20).sort((a, b) => b[1].ms - a[1].ms).slice(0, 70).map(([k, s]) => [k, s.n, Math.round(s.ms * 10) / 10]);
const report = { app: v6 ? 'v6' : '5.x', action, wallMs: wall, taskMs: Math.round((m1.TaskDuration - m0.TaskDuration) * 1000), scriptMs: Math.round((m1.ScriptDuration - m0.ScriptDuration) * 1000), layoutMs: Math.round((m1.LayoutDuration - m0.LayoutDuration) * 1000), styleMs: Math.round((m1.RecalcStyleDuration - m0.RecalcStyleDuration) * 1000), layouts: m1.LayoutCount - m0.LayoutCount, styles: m1.RecalcStyleCount - m0.RecalcStyleCount, before, after, calls: callStats, top };
const text = JSON.stringify(report, null, 1);
if (opt('out')) fs.writeFileSync(opt('out'), text); else console.log(text);
await browser.close(); await server.close();
