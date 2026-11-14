// Export worker (ARCHITECTURE §12): a second instance of ihn.wasm with its own memory, created by host/main.js
// (effect family 3.7). Every message is `{bytes, obj}` with bytes = `[u8 role][payload]`; the instance answers with
// WORKER_POST records, posted back as `{bytes, obj}`. Role 0 (import, W3.4): op 1 runs the 5.x parse in Rust;
// op 2 is the pdf-lib overlay strip of 5.18.0 `stripOverlaysFromPdfBytes`, done here because it needs pdf-lib.
// W5.4 adds the export role. Classic script: it is started with `new Worker(url)`.
'use strict';

const encoder = new TextEncoder(), decoder = new TextDecoder();
const ROLE_IMPORT = 0, OP_STRIP = 2, WORKER = 8;
let ex = null, memory = null, version = '', ready = null;

const u8 = (ptr, len) => new Uint8Array(memory.buffer, ptr, len);
const view = () => new DataView(memory.buffer);

// The out-buffer of the worker instance: only WORKER_POST (FX 0x71: u32 obj, bytes) means something here.
function flush() {
  const len = ex.ihn_out_len();
  let p = ex.ihn_out_ptr();
  const end = p + len, v = view();
  while (p < end) {
    const size = v.getUint32(p + 2, true);
    if (v.getUint8(p) === 3 && v.getUint8(p + 1) === 0x71) {
      const n = v.getUint32(p + 10, true), bytes = u8(p + 14, n).slice();
      self.postMessage({ bytes }, [bytes.buffer]);
    }
    p += 6 + size;
  }
  ex.ihn_out_reset();
}

function call(fn, bytes) {
  const ptr = ex.ihn_alloc(bytes.length);
  u8(ptr, bytes.length).set(bytes);
  try { fn(ptr, bytes.length); } finally { ex.ihn_free(ptr, bytes.length); }
  flush();
}

const env = {
  host_flush: () => flush(),
  host_rect: () => {},
  host_metric: () => 0,
  host_now: () => performance.now(),
  host_time: () => Date.now(),
  host_random: (ptr, len) => { crypto.getRandomValues(u8(ptr, len)); },
  host_ls: () => -1,
  host_intl: () => -1,
  host_log: (level, ptr, len) => console[['debug', 'info', 'warn', 'error'][level] || 'log'](decoder.decode(u8(ptr, len)))
};

async function init(module) {
  const instance = await WebAssembly.instantiate(module, { env });
  ex = instance.exports;
  memory = ex.memory;
  // Config: version, url, user agent, language, flags = WORKER, viewport, dpr, touch points.
  const strings = [version, String(self.location.href), '', ''].map(s => encoder.encode(s));
  const out = new Uint8Array(strings.reduce((n, s) => n + 4 + s.length, 0) + 32);
  const dv = new DataView(out.buffer);
  let p = 0;
  for (const s of strings) { dv.setUint32(p, s.length, true); out.set(s, p + 4); p += 4 + s.length; }
  dv.setUint32(p, WORKER, true);
  call(ex.ihn_init, out);
}

// v5: app-v5.js:stripOverlaysFromPdfBytes — removes each page's overlay XObject (by name, else by object number).
// Request: [0][2][u32 req][u32 n][n × (u32 page, u8 kind, u32 number | str name)]; answer: [0][2][u32 req][u8 ok]
// [f64 size] with obj = the stripped Blob.
async function strip(bytes, blob) {
  const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const req = v.getUint32(2, true), refs = new Map();
  let p = 10;
  for (let i = v.getUint32(6, true); i > 0; i--) {
    const page = v.getUint32(p, true), kind = v.getUint8(p + 4);
    if (kind === 0) { refs.set(page, { refNum: v.getUint32(p + 5, true) }); p += 9; } else {
      const n = v.getUint32(p + 5, true);
      refs.set(page, { name: decoder.decode(bytes.subarray(p + 9, p + 9 + n)) });
      p += 9 + n;
    }
  }
  let out = null;
  try {
    if (!self.PDFLib) importScripts(new URL(`../vendor/pdf-lib/pdf-lib.min.js?v=${version}`, self.location.href).href);
    const { PDFDocument, PDFName } = self.PDFLib;
    const doc = await PDFDocument.load(await blob.arrayBuffer(), { ignoreEncryption: true, updateMetadata: false });
    const pages = doc.getPages();
    let removed = 0;
    for (const [index, info] of refs) {
      const xobjects = pages[index]?.node?.lookup(PDFName.of('Resources'))?.lookup?.(PDFName.of('XObject'));
      if (!xobjects || typeof xobjects.delete !== 'function') continue;
      if (info.name && xobjects.has(PDFName.of(info.name))) { xobjects.delete(PDFName.of(info.name)); removed++; continue; }
      if (Number.isFinite(info.refNum)) {
        const hit = xobjects.entries().find(([, value]) => (value?.objectNumber ?? value?.tag?.objectNumber) === info.refNum);
        if (hit) { xobjects.delete(hit[0]); removed++; }
      }
    }
    if (removed) out = new Blob([await doc.save({ useObjectStreams: true, addDefaultPage: false })], { type: 'application/pdf' });
  } catch (error) {
    console.warn('stripOverlaysFromPdfBytes failed:', error);
  }
  const answer = new Uint8Array(15), av = new DataView(answer.buffer);
  answer[0] = ROLE_IMPORT; answer[1] = OP_STRIP; av.setUint32(2, req, true); answer[6] = out ? 1 : 0;
  av.setFloat64(7, out ? out.size : 0, true);
  self.postMessage({ bytes: answer, obj: out }, [answer.buffer]);
}

self.onmessage = event => {
  const data = event.data || {};
  if (data.type === 'init') {
    version = data.version || '';
    ready = init(data.module);
    return;
  }
  const bytes = data.bytes instanceof Uint8Array ? data.bytes : new Uint8Array(data.bytes || 0);
  ready.then(() => {
    if (bytes[0] === ROLE_IMPORT && bytes[1] === OP_STRIP) return strip(bytes, data.obj);
    call(ex.ihn_worker, bytes);
    // An uncaught error reaches the page as the worker's `error` event (CHAN.WORKER_ERROR).
  }).catch(error => setTimeout(() => { throw error; }));
};
