// Inhouse Notes 6 host: loads ihn.wasm, applies its out-buffer and forwards events (ARCHITECTURE §3).
// JavaScript here only touches the platform; every decision is made in Rust. The numbers mirror
// rust/crates/ihn-app/src/abi.rs (ABI v6).
//
// Executor modules (host/<name>.js) export `default function install(host)` returning `{ [family << 8 | op]: fn }`;
// each handler receives the record's Reader. They report results with `host.io(req, status, obj, bytes)` and
// `host.message(chan, data, obj)`.

const IHN_ABI = 6;
const IMPORTS = ['host_flush', 'host_rect', 'host_metric', 'host_now', 'host_time', 'host_random', 'host_ls', 'host_intl', 'host_log'];
const KIND = { DOM: 1, POINTER: 2, TIMER: 4, IDLE: 5, IO: 6, LIFECYCLE: 7, MESSAGE: 8, TEST: 9 };
const MODE = { APP: 1, HEADLESS: 2, E2E: 4, WORKER: 8, ONLINE: 16, DARK: 32, HIDDEN: 64, REDUCED_MOTION: 128 };
const LIFE = { VISIBILITY: 1, PAGEHIDE: 2, BEFOREUNLOAD: 3, ONLINE: 4, OFFLINE: 5, RESIZE: 6, FULLSCREEN: 7, SCHEME: 8, BLUR: 9, FOCUS: 10, PAGESHOW: 11, STORAGE: 12, RESUME: 13, MOTION: 14 };
const CHAN = { WORKER: 3, WORKER_ERROR: 4, SERVICE_WORKER: 5 };
const FX = 3 << 8;
const EXECUTORS = ['dom', 'canvas', 'idb', 'fetch', 'pdf', 'peer', 'platform'];

const encoder = new TextEncoder();
const decoder = new TextDecoder();

// Cursor over one out-buffer record. Views are re-created by `host.view()` when wasm memory grows.
class Reader {
  constructor(host) { this.host = host; this.p = 0; }
  u8() { return this.host.view().getUint8(this.p++); }
  u32() { const v = this.host.view().getUint32(this.p, true); this.p += 4; return v; }
  i32() { const v = this.host.view().getInt32(this.p, true); this.p += 4; return v; }
  f32() { const v = this.host.view().getFloat32(this.p, true); this.p += 4; return v; }
  f64() { const v = this.host.view().getFloat64(this.p, true); this.p += 8; return v; }
  // A view into wasm memory, valid until the next call into wasm; `copy()` for anything kept longer.
  bytes() { const n = this.u32(); const v = this.host.u8(this.p, n); this.p += n; return v; }
  copy() { return this.bytes().slice(); }
  str() { return decoder.decode(this.bytes()); }
  ostr() {
    if (this.host.view().getUint32(this.p, true) === 0xffffffff) { this.p += 4; return null; }
    return this.str();
  }
}

// Event payload writer over a scratch buffer inside wasm memory.
class Writer {
  constructor(host) { this.host = host; this.base = 0; this.cap = 0; this.p = 0; }
  begin(size) {
    if (size > this.cap) {
      if (this.cap) this.host.ex.ihn_free(this.base, this.cap);
      this.cap = Math.max(size, 4096);
      this.base = this.host.ex.ihn_alloc(this.cap);
    }
    this.p = this.base;
    return this;
  }
  u8(v) { this.host.view().setUint8(this.p++, v); return this; }
  u32(v) { this.host.view().setUint32(this.p, v, true); this.p += 4; return this; }
  i32(v) { this.host.view().setInt32(this.p, v, true); this.p += 4; return this; }
  f32(v) { this.host.view().setFloat32(this.p, v, true); this.p += 4; return this; }
  f64(v) { this.host.view().setFloat64(this.p, v, true); this.p += 8; return this; }
  raw(bytes) { this.host.u8(this.p, bytes.length).set(bytes); this.p += bytes.length; return this; }
  str(s) {
    const { written } = encoder.encodeInto(s, this.host.u8(this.p + 4, s.length * 3));
    return this.u32(written).skip(written);
  }
  skip(n) { this.p += n; return this; }
  // A scratch grown for one large payload (a downloaded PDF, a worker batch) goes back to wasm after the call.
  shrink() {
    if (this.cap <= 256 << 10) return;
    this.host.ex.ihn_free(this.base, this.cap);
    this.base = this.cap = 0;
  }
  get length() { return this.p - this.base; }
}

const strSize = (...strings) => strings.reduce((n, s) => n + 4 + s.length * 3, 0);

function createHost(config) {
  const host = {
    config,
    ex: null,
    memory: null,
    module: null,
    ops: Object.create(null),
    objs: new Map(),
    nextObj: 1,
    inCall: false,
    queue: [],
    dataView: null,
    // --- wasm memory ---
    view() {
      if (!this.dataView || this.dataView.buffer !== this.memory.buffer) this.dataView = new DataView(this.memory.buffer);
      return this.dataView;
    },
    u8(ptr, len) { return new Uint8Array(this.memory.buffer, ptr, len); },
    // --- JS object handles handed to Rust (Blobs, file lists, PDF documents…) ---
    hold(obj) {
      if (obj == null) return 0;
      const id = this.nextObj++;
      this.objs.set(id, obj);
      return id;
    },
    obj(id) { return this.objs.get(id); },
    // --- calls into wasm ---
    // Runs `fn` now, or after the current call returns when an event fires while Rust is running (a DOM op that
    // dispatches focus/blur/scroll synchronously). Queued events cannot be cancelled: their result is 0.
    enter(fn) {
      if (this.inCall) { this.queue.push(fn); return 0; }
      this.inCall = true;
      let result = 0;
      try {
        result = fn() || 0;
        this.applyOut();
        while (this.queue.length) { this.queue.shift()(); this.applyOut(); }
      } finally {
        this.inCall = false;
        this.queue.length = 0;
        this.writer.shrink();
      }
      return result;
    },
    // Completion of an asynchronous effect.
    io(req, status, obj = 0, bytes = null) {
      this.enter(() => {
        const w = this.writer.begin(12 + (bytes ? bytes.length : 0)).u32(req).i32(status).u32(obj);
        if (bytes) w.raw(bytes);
        return this.ex.ihn_event(KIND.IO, w.base, w.length);
      });
    },
    // A message from outside the page; `data` is a string or bytes.
    message(chan, data = '', obj = 0) {
      this.enter(() => {
        const bytes = typeof data === 'string' ? encoder.encode(data) : data;
        const w = this.writer.begin(5 + bytes.length).u8(chan).u32(obj).raw(bytes);
        return this.ex.ihn_event(KIND.MESSAGE, w.base, w.length);
      });
    },
    life(code, a = 0, b = 0, c = 0, s = '') {
      return this.enter(() => {
        const w = this.writer.begin(25 + strSize(s)).u8(code).f64(a).f64(b).f64(c).str(s);
        return this.ex.ihn_event(KIND.LIFECYCLE, w.base, w.length);
      });
    },
    // --- out-buffer ---
    applyOut() {
      const len = this.ex.ihn_out_len();
      if (!len) return;
      const reader = this.reader;
      const saved = reader.p;
      let p = this.ex.ihn_out_ptr();
      const end = p + len;
      try {
        while (p < end) {
          const view = this.view();
          const key = view.getUint8(p) << 8 | view.getUint8(p + 1);
          const size = view.getUint32(p + 2, true);
          const handler = this.ops[key];
          if (!handler) throw new Error(`ihn: op ${key >> 8}.0x${(key & 255).toString(16)} has no executor`);
          reader.p = p + 6;
          handler(reader);
          p += 6 + size;
        }
      } finally {
        reader.p = saved;
        this.ex.ihn_out_reset();
      }
    }
  };
  host.reader = new Reader(host);
  host.writer = new Writer(host);
  return host;
}

// Effects 3.1 (timers, rAF, idle), 3.7 (export worker) and the core ops.
function coreOps(host) {
  const timers = new Map();
  const idles = new Map();
  let raf = 0;
  let worker = null;
  let testResult = null;
  const clearTimer = id => {
    const timer = timers.get(id);
    if (!timer) return;
    (timer.repeat ? clearInterval : clearTimeout)(timer.handle);
    timers.delete(id);
  };
  const fire = (kind, size, fill) => host.enter(() => {
    const w = fill(host.writer.begin(size));
    return host.ex.ihn_event(kind, w.base, w.length);
  });
  const requestIdle = window.requestIdleCallback
    ? (fn, timeout) => requestIdleCallback(fn, timeout > 0 ? { timeout } : undefined)
    : fn => setTimeout(() => fn({ didTimeout: true, timeRemaining: () => 0 }), 1);
  const cancelIdle = window.cancelIdleCallback || clearTimeout;
  host.testResult = () => { const value = testResult; testResult = null; return value; };
  host.ready = new Promise(resolve => { host.markReady = resolve; });
  return {
    [FX | 0x01]: r => host.objs.delete(r.u32()),
    [FX | 0x02]: r => { testResult = r.str(); },
    [FX | 0x03]: () => host.markReady(true),
    [FX | 0x10]: r => {
      const id = r.u32(), ms = r.f64(), repeat = r.u8() !== 0;
      clearTimer(id);
      const run = () => {
        if (!repeat) timers.delete(id);
        fire(KIND.TIMER, 4, w => w.u32(id));
      };
      timers.set(id, { repeat, handle: (repeat ? setInterval : setTimeout)(run, ms) });
    },
    [FX | 0x11]: r => clearTimer(r.u32()),
    [FX | 0x12]: () => {
      if (!raf) raf = requestAnimationFrame(t => { raf = 0; host.enter(() => host.ex.ihn_frame(t)); });
    },
    [FX | 0x13]: () => { cancelAnimationFrame(raf); raf = 0; },
    [FX | 0x14]: r => {
      const id = r.u32(), timeout = r.f64();
      if (idles.has(id)) cancelIdle(idles.get(id));
      idles.set(id, requestIdle(deadline => {
        idles.delete(id);
        fire(KIND.IDLE, 13, w => w.u32(id).f64(deadline.timeRemaining()).u8(deadline.didTimeout ? 1 : 0));
      }, timeout));
    },
    [FX | 0x15]: r => { const id = r.u32(); if (idles.has(id)) cancelIdle(idles.get(id)); idles.delete(id); },
    // The export worker gets the compiled module and creates its own instance and memory (ARCHITECTURE §1).
    [FX | 0x70]: () => {
      worker?.terminate();
      worker = new Worker(window.InhouseAssetUrl(`host/export-worker.js?v=${host.config.version}`));
      worker.onmessage = event => host.message(CHAN.WORKER, event.data?.bytes || '', host.hold(event.data?.obj));
      worker.onerror = event => host.message(CHAN.WORKER_ERROR, String(event.message || 'Worker failed'));
      worker.postMessage({ type: 'init', module: host.module, version: host.config.version, url: location.href });
    },
    [FX | 0x71]: r => {
      const obj = host.obj(r.u32()), bytes = r.copy();
      worker?.postMessage({ bytes, obj }, [bytes.buffer]);
    },
    [FX | 0x72]: () => { worker?.terminate(); worker = null; },
    // Migration (ARCHITECTURE §10.2): ask the service worker which windows are v6 → JSON {windows, v6}; -1 without
    // an answer within 3 s.
    [FX | 0x73]: r => {
      const req = r.u32();
      let done = false;
      const reply = (status, text) => { if (!done) { done = true; host.io(req, status, 0, encoder.encode(text)); } };
      setTimeout(() => reply(-1, ''), 3000);
      navigator.serviceWorker?.ready.then(registration => {
        const channel = new MessageChannel();
        channel.port1.onmessage = event => reply(0, JSON.stringify(event.data ?? null));
        registration.active?.postMessage({ type: 'ihn6-clients' }, [channel.port2]);
      }, () => reply(-1, ''));
    },
    // Deletes the Cache Storage caches whose name starts with the prefix.
    [FX | 0x74]: r => {
      const req = r.u32(), prefix = r.str();
      (async () => { if (typeof caches !== 'undefined') for (const key of await caches.keys()) if (key.startsWith(prefix)) await caches.delete(key); })()
        .then(() => host.io(req, 0), () => host.io(req, -1));
    }
  };
}

function imports(host) {
  const storage = op => (op & 4 ? sessionStorage : localStorage);
  const json = text => (text ? JSON.parse(text) : undefined);
  const metric = (handle, which) => {
    const node = host.node(handle), vv = window.visualViewport;
    switch (which) {
      case 0: return node?.scrollTop ?? 0;
      case 1: return node?.scrollLeft ?? 0;
      case 2: return node?.scrollHeight ?? 0;
      case 3: return node?.scrollWidth ?? 0;
      case 4: return node?.clientWidth ?? 0;
      case 5: return node?.clientHeight ?? 0;
      case 6: return node?.offsetTop ?? 0;
      case 7: return node?.offsetLeft ?? 0;
      case 8: return node?.offsetWidth ?? 0;
      case 9: return node?.offsetHeight ?? 0;
      case 10: return window.innerWidth;
      case 11: return window.innerHeight;
      case 12: return window.devicePixelRatio || 1;
      case 13: return vv ? vv.width : window.innerWidth;
      case 14: return vv ? vv.height : window.innerHeight;
      case 15: return vv ? vv.offsetLeft : 0;
      case 16: return vv ? vv.offsetTop : 0;
      case 17: return vv ? vv.scale : 1;
      case 18: return node?.naturalWidth ?? 0;
      case 19: return node?.naturalHeight ?? 0;
      case 20: return node?.isConnected ? 1 : 0;
      case 21: return window.scrollX;
      case 22: return window.scrollY;
      default: return 0;
    }
  };
  const intl = (op, fields) => {
    switch (op) {
      case 0: {
        const date = new Date(Number(fields[0])), locales = json(fields[2]), options = json(fields[3]);
        if (fields[1] === 'time') return date.toLocaleTimeString(locales, options);
        return fields[1] === 'date' ? date.toLocaleDateString(locales, options) : date.toLocaleString(locales, options);
      }
      case 1: return String(new Intl.Collator(json(fields[2]), json(fields[3])).compare(fields[0], fields[1]));
      case 2: return fields.join('\0').toLocaleLowerCase();
      case 3: return fields.join('\0').toLocaleUpperCase();
      case 4: return Number(fields[0]).toLocaleString(json(fields[1]), json(fields[2]));
      default: return null;
    }
  };
  return {
    host_flush: () => host.applyOut(),
    host_rect: (handle, out) => {
      host.applyOut();
      const rect = host.node(handle)?.getBoundingClientRect?.();
      const view = host.view();
      [rect?.x, rect?.y, rect?.width, rect?.height].forEach((v, i) => view.setFloat64(out + i * 8, v || 0, true));
    },
    host_metric: (handle, which) => { host.applyOut(); return metric(handle, which); },
    host_now: () => performance.now(),
    host_time: () => Date.now(),
    host_random: (ptr, len) => { crypto.getRandomValues(host.u8(ptr, len)); },
    host_ls: (op, kptr, klen, vptr, vlen) => {
      try {
        const key = decoder.decode(host.u8(kptr, klen));
        if ((op & 3) === 1) { storage(op).setItem(key, decoder.decode(host.u8(vptr, vlen))); return 0; }
        if ((op & 3) === 2) { storage(op).removeItem(key); return 0; }
        const value = storage(op).getItem(key);
        if (value === null) return -1;
        const bytes = encoder.encode(value);
        if (bytes.length <= vlen) host.u8(vptr, bytes.length).set(bytes);
        return bytes.length;
      } catch (_) { return -1; }
    },
    // Ops 0..15 are Intl; ops 16.. are `Host::ask` (computed style, media queries, hit test…), answered by
    // host/dom.js after the pending records are applied.
    host_intl: (op, ptr, len) => {
      try {
        if (op >= 16) host.applyOut();
        const fields = decoder.decode(host.u8(ptr, len)).split('\0');
        const result = op >= 16 ? host.ask(op, fields) : intl(op, fields);
        if (result === null) return -1;
        return encoder.encodeInto(result, host.u8(ptr, Math.max(len, 1024))).written;
      } catch (_) { return -1; }
    },
    host_log: (level, ptr, len) => {
      console[['debug', 'info', 'warn', 'error'][level] || 'log'](decoder.decode(host.u8(ptr, len)));
    }
  };
}

function lifecycle(host) {
  const on = (target, type, fn, options) => target.addEventListener(type, fn, options);
  on(document, 'visibilitychange', () => host.life(LIFE.VISIBILITY, document.visibilityState === 'hidden' ? 1 : 0));
  on(window, 'pagehide', event => host.life(LIFE.PAGEHIDE, event.persisted ? 1 : 0));
  on(window, 'pageshow', event => host.life(LIFE.PAGESHOW, event.persisted ? 1 : 0));
  on(window, 'beforeunload', event => {
    if (host.life(LIFE.BEFOREUNLOAD) & 1) { event.preventDefault(); event.returnValue = ''; }
  });
  on(window, 'online', () => host.life(LIFE.ONLINE));
  on(window, 'offline', () => host.life(LIFE.OFFLINE));
  on(window, 'resize', () => host.life(LIFE.RESIZE, innerWidth, innerHeight, devicePixelRatio || 1));
  on(window, 'blur', () => host.life(LIFE.BLUR));
  on(window, 'focus', () => host.life(LIFE.FOCUS));
  on(window, 'storage', event => { if (event.storageArea === localStorage) host.life(LIFE.STORAGE, 0, 0, 0, event.key || ''); });
  on(document, 'resume', () => host.life(LIFE.RESUME));
  for (const type of ['fullscreenchange', 'webkitfullscreenchange']) {
    on(document, type, () => host.life(LIFE.FULLSCREEN, document.fullscreenElement || document.webkitFullscreenElement ? 1 : 0));
  }
  const scheme = matchMedia('(prefers-color-scheme: dark)');
  scheme.addEventListener?.('change', () => host.life(LIFE.SCHEME, scheme.matches ? 1 : 0));
  const motion = matchMedia('(prefers-reduced-motion: reduce)');
  motion.addEventListener?.('change', () => host.life(LIFE.MOTION, motion.matches ? 1 : 0));
}

// The v6 service worker lives at the URL every 5.x page registered (ARCHITECTURE §12). Each v6 window announces
// itself so the migration can tell when no 5.x client is left (§10.2).
function serviceWorker(host) {
  const sw = navigator.serviceWorker;
  if (!sw || host.config.flags & MODE.HEADLESS) return;
  sw.register('/service-worker.js', { updateViaCache: 'none' }).catch(() => {});
  const hello = () => sw.ready.then(registration => registration.active?.postMessage('ihn6-hello'), () => {});
  hello();
  sw.addEventListener('controllerchange', hello);
  sw.addEventListener('message', event => host.message(CHAN.SERVICE_WORKER, JSON.stringify(event.data ?? null)));
}

// Rust 1.90 emits reference-types and multivalue; Android System WebView < 96 cannot compile them.
function wasmSupported() {
  try {
    return typeof WebAssembly === 'object' && WebAssembly.validate(new Uint8Array([
      0, 97, 115, 109, 1, 0, 0, 0, // header
      1, 6, 1, 96, 0, 2, 127, 127, // type: () -> (i32, i32)
      4, 4, 1, 111, 0, 0 // table of externref
    ]));
  } catch (_) { return false; }
}

async function loadWasm(url, env) {
  try {
    return await WebAssembly.instantiateStreaming(fetch(url), env);
  } catch (error) {
    // A server without the application/wasm type: compile from the bytes.
    const response = await fetch(url);
    if (!response.ok) throw error;
    return WebAssembly.instantiate(await response.arrayBuffer(), env);
  }
}

function readConfig() {
  // v5: app-v5.js § 4.2 — the inert version block is the single source of truth.
  const source = document.getElementById('inhouse-app-version')?.textContent || '';
  const version = source.match(/APP_VERSION\s*=\s*['"](\d+\.\d+\.\d+)['"]/)?.[1];
  if (!version) throw new Error('The public app version is missing from index.html');
  const params = new URLSearchParams(location.search);
  const local = ['127.0.0.1', 'localhost', '[::1]'].includes(location.hostname);
  return { version, params, e2e: local && params.get('e2e') === '1', flags: 0 };
}

async function boot() {
  const config = readConfig();
  if (!wasmSupported()) {
    const status = document.getElementById('drive-status');
    if (status) status.textContent = 'Update Android System WebView to use Inhouse Notes.';
    document.getElementById('drive-home')?.classList.remove('hidden');
    document.getElementById('welcome-view')?.classList.add('hidden');
    return;
  }
  const host = createHost(config);
  const asset = path => window.InhouseAssetUrl(`${path}?v=${config.version}`);
  performance.mark('ihn-boot-start');
  const [wasm, ...executors] = await Promise.all([
    // `?e2e=1` on localhost loads the test build: the same code plus the test API (scripts/build-wasm.mjs).
    loadWasm(asset(config.e2e ? 'wasm/ihn-test.wasm' : 'wasm/ihn.wasm'), { env: imports(host) }),
    ...EXECUTORS.map(name => import(asset(`host/${name}.js`)))
  ]);
  performance.mark('ihn-wasm-ready');
  host.module = wasm.module;
  host.ex = wasm.instance.exports;
  host.memory = host.ex.memory;
  if (host.ex.ihn_abi() !== IHN_ABI) throw new Error(`ihn.wasm ABI ${host.ex.ihn_abi()} does not match host ABI ${IHN_ABI}`);
  Object.assign(host.ops, coreOps(host));
  for (const executor of executors) Object.assign(host.ops, executor.default(host));
  // App mode is decided by host/platform.js (ARCHITECTURE §3.7), which sets `config.flags` bits during install.
  config.flags |= (config.e2e ? MODE.E2E : 0)
    | (config.params.has('offline-worker') ? MODE.HEADLESS : 0)
    | (navigator.onLine === false ? 0 : MODE.ONLINE)
    | (matchMedia('(prefers-color-scheme: dark)').matches ? MODE.DARK : 0)
    | (matchMedia('(prefers-reduced-motion: reduce)').matches ? MODE.REDUCED_MOTION : 0)
    | (document.visibilityState === 'hidden' ? MODE.HIDDEN : 0);
  if (config.e2e) (await import(asset('host/test-api.js'))).default(host, { KIND, IMPORTS });
  lifecycle(host);
  host.enter(() => {
    const strings = [config.version, location.href, navigator.userAgent || '', navigator.language || ''];
    const w = host.writer.begin(strSize(...strings) + 32);
    for (const s of strings) w.str(s);
    w.u32(config.flags).f64(innerWidth).f64(innerHeight).f64(devicePixelRatio || 1).u32(navigator.maxTouchPoints || 0);
    host.ex.ihn_init(w.base, w.length);
  });
  performance.mark('ihn-ready');
  serviceWorker(host);
}

boot().catch(error => console.error(error));
