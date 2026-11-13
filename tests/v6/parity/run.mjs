// Pixel-parity harness (TESTING §3) — the release gate.
//
//   node tests/v6/parity/run.mjs [--quick] [--states welcome,home-shell] [--filter text] [--runs 2] [--self]
//        [--a-root <5.18.0 tree>] [--b-root .] [--b-overlay build] [--port-a N] [--port-b N] (default: free ports)
//        [--out build/parity-report] [--workers 1] [--save-all] [--viewports phone-360,desktop-1280]
//
// A is 5.18.0 (default: `git archive 1d6e8a0` extracted to build/v5-ref), B is v6 (this tree + build/ overlay).
// `--self` serves the A tree on both sides (the harness proving itself); `--runs N` repeats every state and also
// compares each side with its own previous run. Exit code 1 when any screenshot differs outside a
// BEHAVIOUR-CHANGES rule.
//
// A state file (tests/v6/parity/states/*.mjs) registers scripts with `parity(name, [options], script)`; the script
// gets a driver `t` and uses only user-level actions, so the same script drives both apps:
//
//   parity('welcome/sheet', { systemDark: true }, async t => {
//     await t.open();                      // signed out
//     await t.click('#btn-welcome-more');
//     await t.advance(400);                // fake time; animations follow it
//   });                                    // captured at the end (or explicitly with t.capture('label'))
import { execSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { chromium } from '@playwright/test';
import { startServer } from '../server.mjs';
import { createMock } from '../mock/google.mjs';
import { addScene, storageDump, restoreStorage } from '../mock/fixtures.mjs';
import { CHROMIUM_FLAGS, chromiumExecutable, contextOptions, cellsFor, cellId, themeStorage, modeQuery, NATIVE_STUB, EPOCH, SEED, VIEWPORTS } from './matrix.mjs';
import { normalizeDom, normalizeTrace } from './normalize.mjs';
import { compareCaptures } from './compare.mjs';
import { writeReport } from './report.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const REPO = path.resolve(HERE, '..', '..', '..');
export const V5_COMMIT = '1d6e8a0';
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

// ---- state registry ------------------------------------------------------------------------------------------------
const registry = new Map();
/**
 * Registers a scripted state.
 * @param {string} name `<area>/<state>`
 * @param {object|Function} [options] `{systemDark, app, only:{viewports,themes,modes}, tags:['B-E1'], expected, drive}`:
 *   `drive(mock)` seeds the mock before load; `tags` are BEHAVIOUR-CHANGES ids; `expected(t)` returns the CSS-pixel
 *   rectangles `[{x,y,width,height}]` in which the tagged difference is allowed (evaluated on B at capture time).
 * @param {(t: Driver) => Promise<void>} script
 */
export function parity(name, options, script) {
  if (typeof options === 'function') { script = options; options = {}; }
  if (registry.has(name)) throw new Error(`Duplicate parity state ${name}`);
  registry.set(name, { name, ...options, script, file: parity.currentFile || '' });
}

/** The 5.18.0 reference tree, extracted once from git. */
export function ensureV5Ref(target = path.join(REPO, 'build', 'v5-ref')) {
  if (!fs.existsSync(path.join(target, 'app-v5.js'))) {
    fs.mkdirSync(target, { recursive: true });
    execSync(`git archive ${V5_COMMIT} | tar -x -C "${target}"`, { cwd: REPO, stdio: ['ignore', 'ignore', 'inherit'] });
  }
  return target;
}

// In a worker the same file runs before the worker's own code, on one line so stack line numbers move by one.
const WORKER_INIT = fs.readFileSync(path.join(HERE, 'determinism-init.js'), 'utf8').replace(/^\s*\/\/.*$/gm, '').replace(/\n\s*/g, ' ');
const DET_INIT = fs.readFileSync(path.join(HERE, 'determinism-init.js'), 'utf8');
const TRACE_INIT = fs.readFileSync(path.join(HERE, 'trace-init.js'), 'utf8');

// ---- driver --------------------------------------------------------------------------------------------------------
/** One app, one matrix cell, one fresh browser context, time owned by the harness. */
export class Driver {
  constructor({ browser, baseUrl, cell, side = 'a', state = {}, trace = true, seed = SEED }) {
    Object.assign(this, { browser, baseUrl, cell, side, state, traceEnabled: trace, seed: seed >>> 0 });
    this.mock = createMock();
    this.mock.gate = true;
    this.captures = [];
    this.problems = [];     // page errors, CSP violations, unmocked requests, never-idle warnings
    this.elapsed = 0;       // fake ms since start
  }

  async start() {
    this.state.drive?.(this.mock, this);
    this.context = await this.browser.newContext(contextOptions(this.cell));
    await this.mock.install(this.context);
    // Fonts and images of the app's own origin load asynchronously: they wait at the mock's gate like network answers.
    await this.context.route(url => /^(127\.0\.0\.1|localhost)$/.test(url.hostname) && /\.(woff2?|ttf|otf|png|jpe?g|gif|webp|svg|ico)$/i.test(url.pathname),
      route => this.mock.gateLocal(route, /\.(woff2?|ttf|otf)$/i.test(new URL(route.request().url()).pathname) ? 'font' : 'image'));
    // Worker scripts get the determinism prelude (frozen Date, seeded randomness); see determinism-init.js.
    await this.context.route(url => /worker[^/]*\.m?js$/i.test(url.pathname) && !/service-worker/.test(url.pathname) && /^(127\.0\.0\.1|localhost)$/.test(url.hostname), async route => {
      try {
        const response = await route.fetch();
        await route.fulfill({ response, body: `self.__IHN_DET_SEED__ = ${(this.seed ^ 0x5bd1e995) >>> 0}; self.__IHN_DET_EPOCH__ = ${EPOCH + this.elapsed};\n${WORKER_INIT}\n${await response.text()}` });
      } catch { await route.abort().catch(() => {}); }
    });
    // Order matters: determinism-init keeps the native rAF, then the clock replaces the timers.
    await this.context.addInitScript(`self.__IHN_DET_SEED__ = ${this.seed};\n${DET_INIT}`);
    await this.context.clock.install({ time: EPOCH });
    await this.context.clock.pauseAt(EPOCH);
    if (this.traceEnabled) await this.context.addInitScript(TRACE_INIT);
    if (this.cell.mode === 'app') await this.context.addInitScript(NATIVE_STUB);
    this.page = await this.context.newPage();
    this.page.on('pageerror', error => this.problems.push(`pageerror: ${error.message}`));
    this.page.on('console', message => { if (message.type() === 'error' && /Content Security Policy|Refused to/.test(message.text())) this.problems.push(`csp: ${message.text().slice(0, 300)}`); });
    this.cdp = await this.context.newCDPSession(this.page);
    await this.cdp.send('Animation.enable');
    await this.cdp.send('Animation.setPlaybackRate', { playbackRate: 0 });
    return this;
  }

  async stop() {
    if (this.mock.unexpected.length) this.problems.push(`unmocked requests: ${[...new Set(this.mock.unexpected)].slice(0, 5).join(', ')}`);
    await this.context?.close().catch(() => {});
  }

  async #facts(frames) {
    try { return await this.page.evaluate(count => self.__ihnDet ? (count ? self.__ihnDet.frames(count) : self.__ihnDet.probe()) : null, frames); }
    catch { return null; }   // navigating
  }
  /**
   * Waits (real time, fake time frozen) until nothing the clock does not own is in flight: mocked requests, fetches,
   * IndexedDB, decoding, fonts, worker replies. `frames` also runs two real frames first (observers, paint).
   */
  async settle({ frames = true } = {}) {
    const started = Date.now(), mock = this.mock;
    let stable = 0, last = -1, workerSince = 0, dirty = false, first = true;
    for (;;) {
      const activity = mock.activity;
      // A changed document gets one real frame (see determinism-init.js); a capture point gets two.
      const facts = await this.#facts(frames && first ? 2 : dirty ? 1 : 0);
      first = false; dirty = !!facts?.dirty;
      // Requests waiting at the gate (or held by a rule) are the only work allowed to be outstanding.
      const waitingFetch = mock.gated.filter(g => g.fetch).length + mock.held.length, waitingImages = mock.gated.filter(g => g.image).length, waitingFonts = mock.gated.filter(g => g.font).length;
      const loading = mock.gated.length - waitingFetch + mock.held.length > 0;
      let idle = !!facts && !mock.pending && facts.busy <= waitingFetch && (!facts.images || waitingImages > 0) && (!facts.fonts || waitingFonts > 0) && (facts.ready === 'complete' || loading)
        && activity === mock.activity && facts.events === last && !facts.dirty;
      if (idle && facts.workers > 0) { workerSince ||= Date.now(); if (Date.now() - workerSince < 3000) idle = false; }
      last = facts ? facts.events : -1;
      if (idle && ++stable >= 2) {
        // The page has nothing left to do: let exactly one network answer in, then wait again. Local work
        // (IndexedDB, decoding) therefore always finishes before the next answer, in both apps, in every run.
        if (!mock.next()) return;
        stable = 0; last = -1;
        continue;
      }
      if (idle) continue;
      stable = 0;
      if (Date.now() - started > 20_000) { this.problems.push(`never idle: ${JSON.stringify(facts)} pending=${mock.pending} gated=${mock.gated.length}`); while (mock.next()); return; }
      await sleep(2);
    }
  }
  /** Advances fake time by `ms` in fixed slices; the page is idle before every slice. Animations follow. */
  async advance(ms, { step = 20 } = {}) {
    for (let left = ms; left > 0; left -= step) {
      const slice = Math.min(step, left);
      await this.settle({ frames: false });
      await this.page.clock.runFor(slice);
      await this.page.evaluate(d => self.__ihnDet && self.__ihnDet.animate(d), slice).catch(() => {});
      this.elapsed += slice;
    }
    await this.settle();
  }
  /** Advances until `predicate()` (in the page) is true, in `step` slices, at most `timeout` fake ms. */
  async until(predicate, { timeout = 10_000, step = 100, arg } = {}) {
    for (let waited = 0; ; waited += step) {
      if (await this.page.evaluate(predicate, arg).catch(() => false)) return waited;
      if (waited >= timeout) throw new Error(`until: not reached in ${timeout} ms of fake time: ${predicate}`);
      await this.advance(step, { step: Math.min(step, 20) });
    }
  }

  /**
   * Loads the app. `account`: email of a mock account whose session is already stored (null = signed out);
   * `scene`: a v5 fixture opened through the shared-link URL `/?drive=<id>&e2e=1` (TESTING §3.1);
   * `storage`: extra localStorage entries; `dump`: a 5.x storage fixture restored before load;
   * `settleMs`: fake time given to boot (the same for both apps); `ready: false` skips the test-API readiness check
   * (first-paint states).
   */
  async open({ account = null, scene = null, sceneOptions = {}, storage = {}, dump = null, query = '', settleMs = 3000, ready: wantReady = true } = {}) {
    await this.page.goto(`${this.baseUrl}/__seed`, { waitUntil: 'commit' });   // 404 page of the app's origin
    const entries = { ...themeStorage(this.cell), ...(account ? this.mock.session(account, { now: EPOCH }) : {}), ...storage };
    await this.page.evaluate(values => { for (const [key, value] of Object.entries(values)) localStorage.setItem(key, value); }, entries);
    if (dump) await restoreStorage(this.page, typeof dump === 'string' ? storageDump(dump) : dump);
    let sceneQuery = '';
    if (scene) { const file = addScene(this.mock, scene, sceneOptions); sceneQuery = `drive=${encodeURIComponent(file.id)}&`; }
    await this.page.goto(`${this.baseUrl}/?${sceneQuery}e2e=1${modeQuery(this.cell)}${query}`, { waitUntil: 'commit' });
    await this.advance(settleMs);
    if (!wantReady) return;
    const ready = await this.page.evaluate(() => {
      if (!self.__IHN_TEST_API__) return 'no test api';
      self.__ihnReady = 'pending';
      Promise.resolve(self.__IHN_TEST_API__.ready()).then(() => { self.__ihnReady = 'ready'; }, error => { self.__ihnReady = `failed: ${error}`; });
      return 'asked';
    });
    if (ready !== 'asked') throw new Error(`open: ${ready} after ${settleMs} ms`);
    await this.settle();
    const state = await this.page.evaluate(() => self.__ihnReady);
    if (state !== 'ready') throw new Error(`open: __IHN_TEST_API__.ready() is ${state} after ${settleMs} ms`);
  }

  /**
   * Runs an async function in the page while the harness keeps the clock moving (a plain `page.evaluate` of code that
   * awaits a timer would never return under the paused clock). For fixture generators and e2e helpers, not for
   * parity states: it reaches into the page.
   */
  async job(fn, arg, { timeout = 300_000, step = 50 } = {}) {
    await this.page.evaluate(([source, value]) => {
      self.__ihnJob = { done: false };
      (0, eval)(`(${source})`)(value).then(result => { self.__ihnJob = { done: true, result }; }, error => { self.__ihnJob = { done: true, error: String(error && error.stack || error) }; });
    }, [fn.toString(), arg]);
    await this.until(() => self.__ihnJob.done, { timeout, step });
    const out = await this.page.evaluate(() => self.__ihnJob);
    if (out.error) throw new Error(`page job failed: ${out.error}`);
    return out.result;
  }

  // ---- user-level actions (each leaves the page idle; none advances time) ----
  async click(selector, options = {}) { await this.page.locator(selector).first().click({ timeout: 5000, ...options }); await this.settle(); }
  async tap(selector) { if (VIEWPORTS[this.cell.viewport].touch) await this.page.locator(selector).first().tap({ timeout: 5000 }); else await this.page.locator(selector).first().click({ timeout: 5000 }); await this.settle(); }
  async hover(selector) { await this.page.locator(selector).first().hover({ timeout: 5000 }); await this.settle(); }
  async press(key) { await this.page.keyboard.press(key); await this.settle(); }
  async type(selector, text) { await this.page.locator(selector).first().fill(text, { timeout: 5000 }); await this.settle(); }
  async wheel(dx, dy) { await this.page.mouse.wheel(dx, dy); await this.settle(); }
  /**
   * A pointer gesture through CDP, like a stylus: `points` are `{x, y, t}` in CSS px and ms from the first sample
   * (recorded streams of tests/v6/fixtures/ink). The clock advances between samples; `lift: false` leaves the pen down.
   */
  async pen(points, { pointerType = 'pen', lift = true, force = 0.5 } = {}) {
    const send = (type, p, buttons) => this.cdp.send('Input.dispatchMouseEvent', { type, x: p.x, y: p.y, button: 'left', buttons, clickCount: 1, pointerType, force: p.p ?? force });
    let last = 0;
    for (const [index, point] of points.entries()) {
      const wait = Math.max(0, Math.round((point.t ?? index * 8) - last)); last += wait;
      if (wait) await this.advance(wait, { step: wait });
      await send(index === 0 ? 'mousePressed' : 'mouseMoved', point, 1);
      await this.settle({ frames: false });
    }
    if (lift) { await send('mouseReleased', points.at(-1), 0); await this.settle(); }
  }

  /** Takes a capture now: screenshot (the gate), normalized DOM and canvas trace (diagnostics). */
  async capture(label = '') {
    await this.settle();
    const png = await this.page.screenshot({ type: 'png', caret: 'hide', animations: 'allow', scale: 'device' });
    const dom = await this.page.evaluate(normalizeDom);
    const trace = this.traceEnabled ? normalizeTrace(await this.page.evaluate(() => self.__ihnTrace ? self.__ihnTrace.take() : null)) : { surfaces: [] };
    let regions = null;
    if (this.state.expected) regions = await this.state.expected(this);
    this.captures.push({ label, png, dom, trace, regions });
  }
}

export const launchBrowser = () => chromium.launch({ executablePath: chromiumExecutable(), args: CHROMIUM_FLAGS });

/**
 * Runs one state in one cell against one app, in a browser process of its own: Chromium's process-wide caches (decoded
 * and scaled images above all) otherwise make a capture depend on what ran before it — seen as ±1 levels in a scaled
 * thumbnail once the image cache budget was exhausted. Returns `{captures, problems, error}`.
 */
export async function runState({ baseUrl, state, cell, side }) {
  let error = null, t = null, browser = null;
  try {
    browser = await launchBrowser();
    t = new Driver({ browser, baseUrl, cell, side, state });
    await t.start();
    await state.script(t);
    if (!t.captures.length) await t.capture('');
  } catch (caught) { error = caught; }
  await t?.stop();
  await browser?.close().catch(() => {});
  return { captures: t?.captures || [], problems: t?.problems || [], error };
}

function judge(state, a, b, dpr) {
  const comparison = compareCaptures(a, b);
  if (comparison.pixelsEqual) return { status: comparison.dom || comparison.trace ? 'warn' : 'pass', comparison };
  const regions = b.regions || a.regions;
  if (state.tags?.length && regions && comparison.pixels.box) {
    // Expected difference: every differing pixel must lie inside the rule's rectangles (TESTING §3.5).
    const box = comparison.pixels.box, inside = regions.some(r => box.x >= Math.floor(r.x * dpr) && box.y >= Math.floor(r.y * dpr) && box.x + box.width <= Math.ceil((r.x + r.width) * dpr) && box.y + box.height <= Math.ceil((r.y + r.height) * dpr));
    if (inside) return { status: 'expected', comparison };
  }
  return { status: 'fail', comparison };
}

async function loadStates(names) {
  const dir = path.join(HERE, 'states');
  const files = names?.length ? names.map(n => fs.existsSync(n) ? path.resolve(n) : path.join(dir, n.endsWith('.mjs') ? n : `${n}.mjs`)) : fs.readdirSync(dir).filter(f => f.endsWith('.mjs')).sort().map(f => path.join(dir, f));
  for (const file of files) { parity.currentFile = path.basename(file, '.mjs'); await import(pathToFileURL(file).href); }
  parity.currentFile = '';
  return [...registry.values()];
}

async function main() {
  const args = process.argv.slice(2);
  const flag = name => args.includes(`--${name}`), opt = (name, fallback) => { const i = args.indexOf(`--${name}`); return i < 0 ? fallback : args[i + 1]; };
  const quick = flag('quick'), self = flag('self'), runs = Number(opt('runs', '1')), workers = Math.max(1, Number(opt('workers', '1')));
  const aRoot = path.resolve(opt('a-root', '') || ensureV5Ref()), bRoot = self ? aRoot : path.resolve(opt('b-root', REPO));
  const bOverlay = self ? null : path.resolve(opt('b-overlay', path.join(REPO, 'build')));
  const outDir = path.resolve(opt('out', path.join(REPO, 'build', 'parity-report')));
  const states = (await loadStates(opt('states', '').split(',').filter(Boolean))).filter(s => !opt('filter') || s.name.includes(opt('filter')));
  if (!states.length) { console.error('No parity states selected.'); process.exit(2); }

  const started = Date.now();
  const serverA = await startServer({ root: aRoot, port: Number(opt('port-a', 0)) });
  const serverB = await startServer({ root: bRoot, overlay: bOverlay && fs.existsSync(bOverlay) ? bOverlay : null, port: Number(opt('port-b', 0)) });
  const results = [], jobs = [];
  const onlyViewports = opt('viewports', '').split(',').filter(Boolean);
  for (const state of states) for (const cell of cellsFor(state, { quick })) if (!onlyViewports.length || onlyViewports.includes(cell.viewport)) jobs.push({ state, cell });

  const runJob = async ({ state, cell }) => {
    const id = cellId(cell), dpr = VIEWPORTS[cell.viewport].dpr, previous = {};
    for (let run = 0; run < runs; run++) {
      const a = await runState({ baseUrl: serverA.url, state, cell, side: 'a' });
      const b = await runState({ baseUrl: serverB.url, state, cell, side: 'b' });
      const base = { state: state.name, cell: runs > 1 ? `${id}.run${run + 1}` : id, tags: state.tags || [] };
      if (a.error || b.error) {
        results.push({ ...base, kind: 'parity', status: 'error', message: [a.error && `A: ${a.error.message.split('\n')[0]}`, b.error && `B: ${b.error.message.split('\n')[0]}`].filter(Boolean).join(' / '), a: a.captures.at(-1), b: b.captures.at(-1) });
      } else if (a.captures.length !== b.captures.length) {
        results.push({ ...base, kind: 'parity', status: 'error', message: `capture count differs: ${a.captures.length} vs ${b.captures.length}` });
      } else {
        a.captures.forEach((capture, index) => {
          const verdict = judge(state, capture, b.captures[index], dpr), problems = [...new Set([...a.problems.map(p => `A ${p}`), ...b.problems.map(p => `B ${p}`)])];
          results.push({ ...base, capture: capture.label, kind: 'parity', ...verdict, problems: index === 0 ? problems : [], a: capture, b: b.captures[index] });
        });
      }
      for (const [side, now] of [['a', a], ['b', b]]) {
        const before = previous[side];
        if (before && !now.error && !before.error) now.captures.forEach((capture, index) => {
          if (!before.captures[index]) return;
          const comparison = compareCaptures(before.captures[index], capture);
          results.push({ ...base, capture: capture.label, kind: 'stability', sides: `${side.toUpperCase()} run ${run} vs ${run + 1}`, status: comparison.pixelsEqual ? (comparison.dom || comparison.trace ? 'warn' : 'pass') : 'fail', comparison, a: before.captures[index], b: capture });
        });
        previous[side] = now;
      }
    }
    const mine = results.filter(r => r.state === state.name && r.cell.startsWith(id));
    const bad = mine.filter(r => r.status === 'fail' || r.status === 'error').length;
    console.log(`${bad ? 'FAIL' : 'ok  '} ${state.name} [${id}] ${mine.length} comparison${mine.length === 1 ? '' : 's'}${bad ? ` — ${mine.find(r => r.status === 'fail' || r.status === 'error').message || 'pixels differ'}` : ''}`);
  };
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(workers, jobs.length) }, async () => { while (next < jobs.length) await runJob(jobs[next++]); }));

  await serverA.close(); await serverB.close();
  const summary = writeReport(outDir, results, { title: self ? '5.18.0 against itself' : '5.18.0 vs v6', a: `5.18.0 (${path.relative(REPO, aRoot) || '.'})`, b: self ? `5.18.0 (${path.relative(REPO, bRoot) || '.'}, second origin)` : `v6 (${path.relative(REPO, bRoot) || '.'}${bOverlay ? ' + ' + path.relative(REPO, bOverlay) : ''})`, quick, runs, saveAll: flag('save-all'), startedAt: new Date(started).toISOString(), durationMs: Date.now() - started });
  console.log(`\n${summary.failed ? 'FAIL' : 'PASS'}: ${summary.passed} pass, ${summary.failed} fail/error, ${summary.expected} expected, ${summary.warned} warnings — ${path.relative(process.cwd(), summary.file)}`);
  process.exit(summary.failed ? 1 : 0);
}

// No top-level await: state files import `parity` from this module while main() is running.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main().catch(error => { console.error(error); process.exit(2); });
