// Canvas call trace (TESTING §3.4). Init script for both apps: wraps every drawing method and property setter of
// CanvasRenderingContext2D / OffscreenCanvasRenderingContext2D, Path2D, CanvasGradient, and the size setters,
// getContext, toDataURL, toBlob of HTMLCanvasElement / OffscreenCanvas.
//
// Raw log entry: [surface, name, ...args]. Surfaces are numbered by first appearance (c0, c1, …), paths p0…,
// gradients g0…, patterns t0…, bitmaps b0…. Argument encoding (exact, JSON-safe):
//   number            → "~" + 16 hex digits of its IEEE-754 bits            (so −0, NaN and 1 ULP differences show)
//   string            → the string itself (no canvas string starts with "~")
//   boolean/null      → as is; undefined → null
//   canvas            → {"surface":"c3"}        image → {"image":"<hash of src>","w":…,"h":…}
//   ImageBitmap       → {"bitmap":"b0","w":…,"h":…}   ImageData → {"data":"<hash>","w":…,"h":…}
//   Path2D            → {"path":"p4","n":<ops recorded so far>}   gradient/pattern → {"ref":"g1"}
//   DOMMatrix         → {"matrix":[a,b,c,d,e,f]}   arrays/objects → element-wise
// `__ihnTrace.take()` returns {surfaces, paths, ops}; normalize.mjs turns it into per-surface op streams.
// Canvases drawn inside workers are not traced (init scripts do not run there); their output reaches the page as
// bitmaps or blobs whose bytes the pixel comparison covers.
(() => {
  const g = self;
  if (g.__ihnTrace) return;
  const f64 = new Float64Array(1), u32 = new Uint32Array(f64.buffer);
  const HEX = Array.from({ length: 256 }, (_, i) => i.toString(16).padStart(2, '0'));
  const num = value => {
    f64[0] = value;
    const hi = u32[1], lo = u32[0];
    return '~' + HEX[hi >>> 24] + HEX[(hi >>> 16) & 255] + HEX[(hi >>> 8) & 255] + HEX[hi & 255] + HEX[lo >>> 24] + HEX[(lo >>> 16) & 255] + HEX[(lo >>> 8) & 255] + HEX[lo & 255];
  };
  // cyrb53 over UTF-16 code units or bytes: fast, synchronous, 53 bits — enough to tell two images apart.
  const hash = data => {
    let h1 = 0xdeadbeef, h2 = 0x41c6ce57;
    const text = typeof data === 'string';
    for (let i = 0; i < data.length; i++) {
      const c = text ? data.charCodeAt(i) : data[i];
      h1 = Math.imul(h1 ^ c, 2654435761); h2 = Math.imul(h2 ^ c, 1597334677);
    }
    h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507) ^ Math.imul(h2 ^ (h2 >>> 13), 3266489909);
    h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507) ^ Math.imul(h1 ^ (h1 >>> 13), 3266489909);
    return (4294967296 * (2097151 & h2) + (h1 >>> 0)).toString(16).padStart(14, '0');
  };

  const trace = g.__ihnTrace = { enabled: true, timing: false, ops: [], times: [], surfaces: [], used: new Map(), refs: 0 };
  let pathCount = 0;
  const surfaceIds = new WeakMap(), pathIds = new WeakMap(), refIds = new WeakMap(), bitmapIds = new WeakMap(), srcHashes = new Map();
  let bitmaps = 0;
  const surfaceOf = canvas => {
    let id = surfaceIds.get(canvas);
    if (id === undefined) {
      id = 'c' + trace.surfaces.length;
      surfaceIds.set(canvas, id);
      trace.surfaces.push({ id, kind: g.HTMLCanvasElement && canvas instanceof g.HTMLCanvasElement ? 'canvas' : 'offscreen', ref: new WeakRef(canvas) });
    }
    return id;
  };
  const pathOf = path => {
    let entry = pathIds.get(path);
    if (!entry) { entry = { id: 'p' + pathCount++, ops: [] }; pathIds.set(path, entry); }
    return entry;
  };
  const isCanvas = v => (g.HTMLCanvasElement && v instanceof g.HTMLCanvasElement) || (g.OffscreenCanvas && v instanceof g.OffscreenCanvas);
  const enc = v => {
    if (typeof v === 'number') return num(v);
    if (typeof v === 'string' || typeof v === 'boolean' || v === null) return v;
    if (v === undefined) return null;
    if (Array.isArray(v) || (ArrayBuffer.isView(v) && !(v instanceof DataView))) return Array.from(v, enc);
    if (isCanvas(v)) return { surface: surfaceOf(v) };
    if (g.Path2D && v instanceof g.Path2D) { const p = pathOf(v); trace.used.set(p.id, p); return { path: p.id, n: p.ops.length }; }
    if (refIds.has(v)) return { ref: refIds.get(v) };
    if (g.HTMLImageElement && v instanceof g.HTMLImageElement) {
      const src = v.currentSrc || v.src || '';
      let h = srcHashes.get(src);
      if (h === undefined) { h = /^blob:/.test(src) ? 'blob' : hash(src); if (srcHashes.size < 4096) srcHashes.set(src, h); }
      return { image: h, w: v.naturalWidth, h: v.naturalHeight };
    }
    if (g.ImageBitmap && v instanceof g.ImageBitmap) { let id = bitmapIds.get(v); if (!id) { id = 'b' + bitmaps++; bitmapIds.set(v, id); } return { bitmap: id, w: v.width, h: v.height }; }
    if (g.ImageData && v instanceof g.ImageData) return { data: hash(v.data), w: v.width, h: v.height };
    if (g.DOMMatrixReadOnly && v instanceof g.DOMMatrixReadOnly) return { matrix: [v.a, v.b, v.c, v.d, v.e, v.f].map(num) };
    if (g.HTMLVideoElement && v instanceof g.HTMLVideoElement) return { video: true, w: v.videoWidth, h: v.videoHeight };
    if (g.SVGImageElement && v instanceof g.SVGImageElement) return { image: hash(v.href.baseVal || ''), w: v.width.baseVal.value, h: v.height.baseVal.value };
    if (typeof v === 'object') { const out = {}; for (const key of Object.keys(v).sort()) out[key] = enc(v[key]); return out; }
    return String(v);
  };
  const push = entry => { trace.ops.push(entry); if (trace.timing) trace.times.push(Date.now()); };

  const READS = new Set(['getImageData', 'measureText', 'getTransform', 'getLineDash', 'isPointInPath', 'isPointInStroke', 'getContextAttributes', 'createImageData', 'isContextLost']);
  const FACTORIES = { createLinearGradient: 'g', createRadialGradient: 'g', createConicGradient: 'g', createPattern: 't' };
  const wrapContext = proto => {
    if (!proto) return;
    for (const name of Object.getOwnPropertyNames(proto)) {
      const d = Object.getOwnPropertyDescriptor(proto, name);
      if (!d || !d.configurable || name === 'constructor' || name === 'canvas') continue;
      if (typeof d.value === 'function') {
        if (READS.has(name)) continue;
        const original = d.value, prefix = FACTORIES[name];
        d.value = prefix ? function (...args) {
          const made = original.apply(this, args);
          if (trace.enabled && made) { const id = prefix + trace.refs++; refIds.set(made, id); push([id, name, surfaceOf(this.canvas), ...args.map(enc)]); }
          return made;
        } : function (...args) {
          if (trace.enabled) push([surfaceOf(this.canvas), name, ...args.map(enc)]);
          return original.apply(this, args);
        };
        Object.defineProperty(proto, name, d);
      } else if (d.set) {
        const set = d.set;
        d.set = function (value) { if (trace.enabled) push([surfaceOf(this.canvas), 'set:' + name, enc(value)]); return set.call(this, value); };
        Object.defineProperty(proto, name, d);
      }
    }
  };
  wrapContext(g.CanvasRenderingContext2D && g.CanvasRenderingContext2D.prototype);
  wrapContext(g.OffscreenCanvasRenderingContext2D && g.OffscreenCanvasRenderingContext2D.prototype);

  for (const [ctor, prefix] of [[g.CanvasGradient, 'g'], [g.CanvasPattern, 't']]) {
    if (!ctor) continue;
    for (const name of Object.getOwnPropertyNames(ctor.prototype)) {
      const original = ctor.prototype[name];
      if (name === 'constructor' || typeof original !== 'function') continue;
      ctor.prototype[name] = function (...args) { if (trace.enabled) push([refIds.get(this) || prefix + '?', name, ...args.map(enc)]); return original.apply(this, args); };
    }
  }

  if (g.Path2D) {
    const Native = g.Path2D;
    const Traced = function Path2D(...args) {
      const path = new Native(...args), entry = pathOf(path);
      if (args[0] instanceof Native) entry.ops.push(...pathOf(args[0]).ops);
      else if (typeof args[0] === 'string') entry.ops.push(['svg', args[0]]);
      return path;
    };
    Traced.prototype = Native.prototype;
    Object.defineProperty(Native.prototype, 'constructor', { value: Traced, configurable: true, writable: true });
    for (const name of Object.getOwnPropertyNames(Native.prototype)) {
      const original = Native.prototype[name];
      if (name === 'constructor' || typeof original !== 'function') continue;
      Native.prototype[name] = function (...args) {
        if (trace.enabled) { const entry = pathOf(this); if (name === 'addPath' && args[0] instanceof Native) entry.ops.push(['addPath', pathOf(args[0]).ops.slice(), enc(args[1])]); else entry.ops.push([name, ...args.map(enc)]); }
        return original.apply(this, args);
      };
    }
    g.Path2D = Traced;
  }

  const wrapCanvas = (proto, methods) => {
    if (!proto) return;
    for (const name of ['width', 'height']) {
      const d = Object.getOwnPropertyDescriptor(proto, name);
      if (!d || !d.set) continue;
      const set = d.set;
      d.set = function (value) { if (trace.enabled) push([surfaceOf(this), 'set:' + name, enc(Number(value))]); return set.call(this, value); };
      Object.defineProperty(proto, name, d);
    }
    for (const name of methods) {
      const original = proto[name];
      if (typeof original !== 'function') continue;
      proto[name] = function (...args) {
        if (trace.enabled) push([surfaceOf(this), name, ...args.filter(a => typeof a !== 'function').map(enc)]);
        return original.apply(this, args);
      };
    }
  };
  wrapCanvas(g.HTMLCanvasElement && g.HTMLCanvasElement.prototype, ['getContext', 'toDataURL', 'toBlob', 'transferControlToOffscreen']);
  wrapCanvas(g.OffscreenCanvas && g.OffscreenCanvas.prototype, ['getContext', 'convertToBlob', 'transferToImageBitmap']);

  // A stable name for a DOM canvas: its id, else its classes plus `data-page`, plus its position among equals.
  const label = canvas => {
    if (!canvas || !canvas.isConnected) return null;
    if (canvas.id) return '#' + canvas.id;
    const base = 'canvas' + Array.from(canvas.classList).sort().map(c => '.' + c).join('') + (canvas.dataset.page !== undefined ? `[page=${canvas.dataset.page}]` : '');
    let index = 0;
    for (const other of canvas.ownerDocument.querySelectorAll('canvas')) {
      if (other === canvas) break;
      if (!other.id && label.base(other) === base) index++;
    }
    return index ? `${base}#${index}` : base;
  };
  label.base = canvas => 'canvas' + Array.from(canvas.classList).sort().map(c => '.' + c).join('') + (canvas.dataset.page !== undefined ? `[page=${canvas.dataset.page}]` : '');

  /** Snapshot of the log. `clear` starts a new log afterwards (surface, path and ref numbering continue). */
  trace.take = ({ clear = false } = {}) => {
    const out = {
      surfaces: trace.surfaces.map(s => { const canvas = s.ref.deref(); return { id: s.id, kind: s.kind, label: s.kind === 'canvas' ? label(canvas) : null, width: canvas ? canvas.width : null, height: canvas ? canvas.height : null }; }),
      paths: Object.fromEntries(Array.from(trace.used.values(), p => [p.id, p.ops.slice()])),
      ops: trace.ops.slice(), times: trace.timing ? trace.times.slice() : undefined
    };
    if (clear) trace.reset();
    return out;
  };
  trace.reset = () => { trace.ops = []; trace.times = []; trace.used = new Map(); };
  trace.count = () => trace.ops.length;
})();
