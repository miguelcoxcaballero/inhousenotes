// Export worker (ARCHITECTURE §12): a second instance of ihn.wasm with its own memory, created by host/main.js
// (effect family 3.7). Every message is `{bytes, obj}` with bytes = `[u8 role][payload]`. Rust decides everything
// (rust/crates/ihn-app/src/doc/v5open.rs import role 0, export/pdf.rs export role 1); this file runs pdf-lib on its
// plans. Role 0: op 1 runs the 5.x parse in Rust; op 2 is the pdf-lib overlay strip of 5.18.0
// `stripOverlaysFromPdfBytes`. Role 1 (W5.4, ARCHITECTURE §8.2): op 1 hands over a Blob of a job, op 2 is the plan +
// snapshot (Rust answers the assembly plan, op 3, which pdf-lib turns into the file → op 4 with the bytes); op 5
// rebuilds a source PDF from an export (§4.5) → op 6 with the Blob. Classic script: `new Worker(url)`.
'use strict';

const encoder = new TextEncoder(), decoder = new TextDecoder();
const ROLE_IMPORT = 0, OP_STRIP = 2, WORKER = 8;
const ROLE_EXPORT = 1, OP_BLOB = 1, OP_BUILT = 4, OP_REBUILD = 5, OP_REBUILT = 6, OP_META = 7;
const jobs = new Map(); // export job → Map(key → Blob)
let ex = null, memory = null, version = '', ready = null, local = null;

const u8 = (ptr, len) => new Uint8Array(memory.buffer, ptr, len);
const view = () => new DataView(memory.buffer);

// The out-buffer of the worker instance (abi.rs `fx`): 0x71 `u32 obj, bytes` → post `{bytes}`; 0x75 `str type,
// bytes data, bytes msg` → post `{bytes: msg, obj: Blob}`; 0x76 = an answer for this file (`local`).
function flush() {
  const v = view(), end = ex.ihn_out_ptr() + ex.ihn_out_len();
  for (let p = ex.ihn_out_ptr(); p < end; p += 6 + v.getUint32(p + 2, true)) {
    const op = v.getUint8(p + 1), r = reader(u8(p + 6, v.getUint32(p + 2, true)), 0);
    if (v.getUint8(p) !== 3) continue;
    if (op === 0x76) local = r.rest().slice();
    else if (op === 0x75) {
      const type = r.str(), obj = new Blob([r.bin()], { type }), bytes = r.bin().slice();
      self.postMessage({ bytes, obj }, [bytes.buffer]);
    } else if (op === 0x71) {
      r.u32();
      const bytes = r.bin().slice();
      self.postMessage({ bytes }, [bytes.buffer]);
    }
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

async function init(module, module2) {
  // 6.2: the second part of ihn.wasm (scripts/split-wasm.mjs), instantiated against this instance when first needed.
  let second = null;
  env.host_split = () => { second ||= new WebAssembly.Instance(module2, { env, p: ex }); };
  const instance = await WebAssembly.instantiate(module, { env });
  ex = instance.exports;
  memory = ex.memory;
  // Config (abi.rs `Config`): four empty strings, flags = WORKER (Rust creates no `App` then), zeros.
  const config = new Uint8Array(48);
  config[16] = WORKER;
  call(ex.ihn_init, config);
}

// v5: app-v5.js:stripOverlaysFromPdfBytes — removes each page's overlay XObject (by name, else by object number).
// Request: [0][2][u32 req][u32 n][n × (u32 page, u8 kind, u32 number | str name)], one per page; answer: [0][2]
// [u32 req][u8 ok][f64 size] with obj = the stripped Blob.
function pdfLib() {
  if (!self.PDFLib) importScripts(new URL(`../vendor/pdf-lib/pdf-lib.min.js?v=${version}`, self.location.href).href);
  return self.PDFLib;
}

async function strip(bytes, blob) {
  const r = reader(bytes, 2), req = r.u32(), refs = [];
  for (let n = r.u32(); n > 0; n--) refs.push([r.u32(), r.u8() ? { name: r.str() } : { refNum: r.u32() }]);
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
  const r = {
    u8: () => v.getUint8(p++),
    u32: () => { const x = v.getUint32(p, true); p += 4; return x; },
    f64: () => { const x = v.getFloat64(p, true); p += 8; return x; },
    bin: () => { const n = v.getUint32(p, true); p += 4 + n; return bytes.subarray(p - n, p); },
    str: () => decoder.decode(r.bin()),
    rest: () => bytes.subarray(p)
  };
  return r;
}

function answer(op, job, ok, tail) {
  const out = new Uint8Array(7 + (tail ? tail.length : 0)), v = new DataView(out.buffer);
  out[0] = ROLE_EXPORT; out[1] = op; v.setUint32(2, job, true); out[6] = ok ? 1 : 0;
  if (tail) out.set(tail, 7);
  return out;
}

const isPng = b => b.length > 4 && b[0] === 0x89 && b[1] === 0x50 && b[2] === 0x4e && b[3] === 0x47;

// The assembly plan of `export::pdf::worker` (`assemble_plan`) → the PDF (ARCHITECTURE §8.2).
async function assemble(bytes) {
  const r = reader(bytes, 2), job = r.u32(), blobs = jobs.get(job) || new Map();
  jobs.delete(job);
  try {
    const { PDFDocument, PDFName, rgb } = pdfLib();
    const out = await PDFDocument.create();
    const ink = r.str(), gs = {}, sources = [], pages = [], attachments = [];
    for (let n = r.u32(); n > 0; n--) { const name = r.str(), alpha = r.f64(); gs[name] = { Type: 'ExtGState', CA: alpha, ca: alpha, BM: r.str() }; }
    for (let n = r.u32(); n > 0; n--) {
      const key = r.u32(), uses = [];
      for (let m = r.u32(); m > 0; m--) uses.push([r.u32(), r.u32()]);
      sources.push([key, uses]);
    }
    for (let n = r.u32(); n > 0; n--) {
      const page = { w: r.f64(), h: r.f64(), draw: r.u8(), key: r.u32(), bw: r.f64(), bh: r.f64(), content: r.str(), images: [] };
      for (let k = r.u32(); k > 0; k--) page.images.push(r.u32());
      pages.push(page);
    }
    for (let n = r.u32(); n > 0; n--) attachments.push([r.str(), r.u32()]);
    const snapshot = r.rest();
    const bytesOf = async key => new Uint8Array(await blobs.get(key).arrayBuffer());
    // Source pages: copied with their content objects unchanged (one copy per use).
    for (const [key, uses] of sources) {
      const src = await PDFDocument.load(await bytesOf(key), { ignoreEncryption: true, updateMetadata: false });
      const valid = uses.filter(use => use[1] < src.getPageCount());
      const list = await out.copyPages(src, valid.map(use => use[1]));
      valid.forEach((use, i) => { pages[use[0]].copied = list[i]; });
    }
    const images = new Map();
    const image = async key => {
      if (!images.has(key)) {
        const b = await bytesOf(key);
        images.set(key, isPng(b) ? await out.embedPng(b) : await out.embedJpg(b));
      }
      return images.get(key);
    };
    for (const p of pages) {
      let page;
      if (p.copied) {
        page = out.addPage(p.copied);
      } else {
        page = out.addPage([p.w, p.h]);
        const { width, height } = page.getSize();
        if (p.draw & 1) page.drawRectangle({ x: 0, y: 0, width, height, color: rgb(1, 1, 1) });
        if (p.draw & 2 && blobs.has(p.key)) page.drawImage(await image(p.key), { x: 0, y: 0, width, height });
      }
      if (!p.content) continue;
      const { width, height } = page.getSize(), box = page.getMediaBox();
      const xobjects = {};
      for (let k = 0; k < p.images.length; k++) xobjects[`Im${k}`] = (await image(p.images[k])).ref;
      const form = out.context.flateStream(p.content, {
        Type: 'XObject', Subtype: 'Form', BBox: [0, 0, p.bw, p.bh], Matrix: [width / p.bw, 0, 0, height / p.bh, box.x, box.y],
        Resources: { ExtGState: gs, XObject: xobjects }
      });
      const node = page.node;
      node.normalize();
      node.setXObject(PDFName.of('IHN6Ink'), out.context.register(form));
      // The page's own content runs inside q … Q, so its graphics state cannot reach the ink.
      node.wrapContentStreams(out.context.register(out.context.stream('q')), out.context.register(out.context.stream('Q')));
      node.addContentStream(out.context.register(out.context.stream(ink)));
    }
    for (const [name, key] of attachments) {
      if (!blobs.has(key)) continue;
      const b = await bytesOf(key);
      await out.attach(b, name, { mimeType: isPng(b) ? 'image/png' : 'image/jpeg' });
    }
    const zlib = new Uint8Array(await new Response(new Blob([snapshot]).stream().pipeThrough(new CompressionStream('deflate'))).arrayBuffer());
    call(ex.ihn_worker, answer(OP_META, job, true, zlib));
    const meta = reader(local, 0);
    local = null;
    out.setTitle(meta.str());
    out.setSubject(meta.str());
    out.setCreator(meta.str());
    out.setKeywords([meta.str()]);
    const pdf = await out.save({ useObjectStreams: true, addDefaultPage: false });
    const msg = answer(OP_BUILT, job, true, pdf);
    self.postMessage({ bytes: msg }, [msg.buffer]);
  } catch (error) {
    console.warn('PDF export failed:', error);
    const msg = answer(OP_BUILT, job, false, encoder.encode(String(error?.message || error)));
    self.postMessage({ bytes: msg }, [msg.buffer]);
  }
}

// ARCHITECTURE §4.5, the plan of `export::pdf::rebuild_plan`: the export's pages that carry a source, with the ink
// stream and `/IHN6Ink` removed, as page numbers 1..max (blank pages where nothing is copied).
async function rebuild(bytes, blob) {
  const r = reader(bytes, 2), job = r.u32(), max = r.u32(), blank = [r.f64(), r.f64()], ink = r.str(), pairs = [];
  for (let n = r.u32(); n > 0; n--) pairs.push([r.u32(), r.u32()]);
  let result = null;
  try {
    const { PDFDocument, PDFName } = pdfLib();
    const exported = await PDFDocument.load(await blob.arrayBuffer(), { ignoreEncryption: true, updateMetadata: false });
    const out = await PDFDocument.create();
    const valid = pairs.filter(pair => pair[1] < exported.getPageCount());
    const list = await out.copyPages(exported, valid.map(pair => pair[1]));
    const copies = new Map(valid.map((pair, i) => [pair[0], list[i]]));
    for (let n = 1; n <= max; n++) {
      const copy = copies.get(n);
      if (!copy) { out.addPage(blank); continue; }
      const page = out.addPage(copy), node = page.node;
      node.normalize();
      const contents = node.Contents();
      const last = contents?.size?.() ? out.context.lookup(contents.get(contents.size() - 1)) : null;
      const raw = last?.getContents?.() || last?.contents;
      if (raw && decoder.decode(raw).trim() === ink) contents.remove(contents.size() - 1);
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
    ready = init(data.module, data.module2);
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
    if (local) { const plan = local; local = null; assemble(plan); }
    // An uncaught error reaches the page as the worker's `error` event (CHAN.WORKER_ERROR).
  }).catch(error => setTimeout(() => { throw error; }));
};
