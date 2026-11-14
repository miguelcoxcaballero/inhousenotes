// Export worker (ARCHITECTURE §12): a second instance of ihn.wasm with its own memory, created by host/main.js
// (effect family 3.7). Every message is `{bytes, obj}` with bytes = `[u8 role][payload]`; the instance answers with
// WORKER_POST records, posted back as `{bytes, obj}`. Role 0 (import, W3.4): op 1 runs the 5.x parse in Rust;
// op 2 is the pdf-lib overlay strip of 5.18.0 `stripOverlaysFromPdfBytes`, done here because it needs pdf-lib.
// Role 1 (export, W5.4, ARCHITECTURE §8.2): op 1 hands over a Blob of a job, op 2 is the plan + snapshot (Rust turns
// it into the ink content streams and posts op 3, which stays here: pdf-lib assembles the file → op 4 with the
// bytes); op 5 rebuilds a source PDF from an export (§4.5) → op 6 with the Blob. Classic script: `new Worker(url)`.
'use strict';

const encoder = new TextEncoder(), decoder = new TextDecoder();
const ROLE_IMPORT = 0, OP_STRIP = 2, WORKER = 8;
const ROLE_EXPORT = 1, OP_BLOB = 1, OP_ASSEMBLE = 3, OP_BUILT = 4, OP_REBUILD = 5, OP_REBUILT = 6;
const jobs = new Map(); // export job → Map(key → Blob)
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
      if (bytes[0] === ROLE_EXPORT && bytes[1] === OP_ASSEMBLE) queueMicrotask(() => assemble(bytes));
      else self.postMessage({ bytes }, [bytes.buffer]);
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
function pdfLib() {
  if (!self.PDFLib) importScripts(new URL(`../vendor/pdf-lib/pdf-lib.min.js?v=${version}`, self.location.href).href);
  return self.PDFLib;
}

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
    const { PDFDocument, PDFName } = pdfLib();
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

// ---- export role (W5.4) ------------------------------------------------------------------------------------------

function reader(bytes, at) {
  const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let p = at;
  return {
    u8: () => v.getUint8(p++),
    u32: () => { const x = v.getUint32(p, true); p += 4; return x; },
    f64: () => { const x = v.getFloat64(p, true); p += 8; return x; },
    str: () => { const n = v.getUint32(p, true); const s = decoder.decode(bytes.subarray(p + 4, p + 4 + n)); p += 4 + n; return s; },
    rest: () => bytes.subarray(p)
  };
}

function answer(op, job, ok, tail) {
  const out = new Uint8Array(7 + (tail ? tail.length : 0)), v = new DataView(out.buffer);
  out[0] = ROLE_EXPORT; out[1] = op; v.setUint32(2, job, true); out[6] = ok ? 1 : 0;
  if (tail) out.set(tail, 7);
  return out;
}

const isPng = b => b.length > 4 && b[0] === 0x89 && b[1] === 0x50 && b[2] === 0x4e && b[3] === 0x47;

function base64(bytes) {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

const INK_CALL = 'q /IHN6Ink Do Q';
// `render::pdfink::GS`: name, opacity, blend mode.
const GS = [['GSn', 1, 'Normal'], ['GSh1', 0.22, 'Multiply'], ['GSh2', 0.1, 'Multiply'], ['GSh3', 0.24, 'Multiply']];

// The assembly plan of `export::pdf::worker` → the PDF (ARCHITECTURE §8.2).
async function assemble(bytes) {
  const r = reader(bytes, 2), job = r.u32(), blobs = jobs.get(job) || new Map();
  jobs.delete(job);
  try {
    const { PDFDocument, PDFName, rgb } = pdfLib();
    const out = await PDFDocument.create();
    const pages = [];
    for (let n = r.u32(); n > 0; n--) {
      const page = { w: r.f64(), h: r.f64(), bg: r.u8(), key: r.u32(), src: r.u32(), content: r.str(), images: [] };
      for (let k = r.u32(); k > 0; k--) page.images.push(r.u32());
      pages.push(page);
    }
    const attachments = [];
    for (let n = r.u32(); n > 0; n--) attachments.push([r.str(), r.u32()]);
    const snapshot = r.rest();
    const bytesOf = async key => new Uint8Array(await blobs.get(key).arrayBuffer());
    // Source pages: copied with their content objects unchanged (one copy per use).
    const copied = new Map();
    for (const key of new Set(pages.filter(p => p.bg === 1).map(p => p.key))) {
      const src = await PDFDocument.load(await bytesOf(key), { ignoreEncryption: true, updateMetadata: false });
      const uses = pages.filter(p => p.bg === 1 && p.key === key);
      const valid = uses.filter(p => p.src >= 1 && p.src <= src.getPageCount());
      const list = await out.copyPages(src, valid.map(p => p.src - 1));
      valid.forEach((p, i) => { p.copied = list[i]; });
    }
    const images = new Map();
    const image = async key => {
      if (!images.has(key)) {
        const b = await bytesOf(key);
        images.set(key, isPng(b) ? await out.embedPng(b) : await out.embedJpg(b));
      }
      return images.get(key);
    };
    const gs = {};
    for (const [name, alpha, blend] of GS) gs[name] = { Type: 'ExtGState', CA: alpha, ca: alpha, BM: blend };
    for (const p of pages) {
      let page;
      if (p.copied) {
        page = out.addPage(p.copied);
      } else {
        // v5: buildPdfBlobWithPdfLib — white page, then the template JPEG or the photo over the whole page.
        page = out.addPage([p.w * 0.75, p.h * 0.75]);
        const { width, height } = page.getSize();
        page.drawRectangle({ x: 0, y: 0, width, height, color: rgb(1, 1, 1) });
        if ((p.bg === 0 || p.bg === 2) && blobs.has(p.key)) page.drawImage(await image(p.key), { x: 0, y: 0, width, height });
      }
      if (!p.content) continue;
      const { width, height } = page.getSize(), box = page.getMediaBox();
      const xobjects = {};
      for (let k = 0; k < p.images.length; k++) xobjects[`Im${k}`] = (await image(p.images[k])).ref;
      const form = out.context.flateStream(p.content, {
        Type: 'XObject', Subtype: 'Form', BBox: [0, 0, p.w, p.h], Matrix: [width / p.w, 0, 0, height / p.h, box.x, box.y],
        Resources: { ExtGState: gs, XObject: xobjects }
      });
      const node = page.node;
      node.normalize();
      node.setXObject(PDFName.of('IHN6Ink'), out.context.register(form));
      // The page's own content runs inside q … Q, so its graphics state cannot reach the ink.
      node.wrapContentStreams(out.context.register(out.context.stream('q')), out.context.register(out.context.stream('Q')));
      node.addContentStream(out.context.register(out.context.stream(INK_CALL)));
    }
    for (const [name, key] of attachments) {
      if (!blobs.has(key)) continue;
      const b = await bytesOf(key);
      await out.attach(b, name, { mimeType: isPng(b) ? 'image/png' : 'image/jpeg' });
    }
    const zlib = new Uint8Array(await new Response(new Blob([snapshot]).stream().pipeThrough(new CompressionStream('deflate'))).arrayBuffer());
    out.setTitle('Cuaderno Digital');
    out.setSubject('Notas con S-Pen');
    out.setCreator('Cuaderno Digital App');
    out.setKeywords([`STROKES_Z:!;IHN6:${base64(zlib)}`]);
    const pdf = await out.save({ useObjectStreams: true, addDefaultPage: false });
    const msg = answer(OP_BUILT, job, true, pdf);
    self.postMessage({ bytes: msg }, [msg.buffer]);
  } catch (error) {
    console.warn('PDF export failed:', error);
    const msg = answer(OP_BUILT, job, false, encoder.encode(String(error?.message || error)));
    self.postMessage({ bytes: msg }, [msg.buffer]);
  }
}

// ARCHITECTURE §4.5: the export's pages that carry a source, with the ink stream and `/IHN6Ink` removed; page n of
// the result is page n of the source (other numbers are blank pages that nothing shows).
async function rebuild(bytes, blob) {
  const r = reader(bytes, 2), job = r.u32(), carried = new Map();
  for (let n = r.u32(); n > 0; n--) { const at = r.u32(); carried.set(r.u32(), at); }
  let result = null;
  try {
    const { PDFDocument, PDFName } = pdfLib();
    const exported = await PDFDocument.load(await blob.arrayBuffer(), { ignoreEncryption: true, updateMetadata: false });
    const out = await PDFDocument.create();
    const max = Math.max(...carried.keys());
    const numbers = [...carried.keys()].filter(n => carried.get(n) < exported.getPageCount());
    const list = await out.copyPages(exported, numbers.map(n => carried.get(n)));
    const copies = new Map(numbers.map((n, i) => [n, list[i]]));
    for (let n = 1; n <= max; n++) {
      const copy = copies.get(n);
      if (!copy) { out.addPage([595.28, 841.89]); continue; }
      const page = out.addPage(copy), node = page.node;
      node.normalize();
      const contents = node.Contents();
      const last = contents?.size?.() ? out.context.lookup(contents.get(contents.size() - 1)) : null;
      const raw = last?.getContents?.() || last?.contents;
      if (raw && decoder.decode(raw).trim() === INK_CALL) contents.remove(contents.size() - 1);
      node.Resources()?.lookup(PDFName.of('XObject'))?.delete?.(PDFName.of('IHN6Ink'));
    }
    result = new Blob([await out.save({ useObjectStreams: true, addDefaultPage: false })], { type: 'application/pdf' });
  } catch (error) {
    console.warn('Source rebuild failed:', error);
  }
  const msg = answer(OP_REBUILT, job, !!result);
  self.postMessage({ bytes: msg, obj: result }, [msg.buffer]);
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
    if (bytes[0] === ROLE_EXPORT && bytes[1] === OP_BLOB) {
      const job = new DataView(bytes.buffer, bytes.byteOffset).getUint32(2, true), key = new DataView(bytes.buffer, bytes.byteOffset).getUint32(6, true);
      if (!jobs.has(job)) jobs.set(job, new Map());
      if (data.obj) jobs.get(job).set(key, data.obj);
      return;
    }
    if (bytes[0] === ROLE_EXPORT && bytes[1] === OP_REBUILD) return rebuild(bytes, data.obj);
    call(ex.ihn_worker, bytes);
    // An uncaught error reaches the page as the worker's `error` event (CHAN.WORKER_ERROR).
  }).catch(error => setTimeout(() => { throw error; }));
};
