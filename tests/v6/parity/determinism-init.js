// Init script for both apps (TESTING §3.1). Time is owned by the Playwright clock (installed and paused by run.mjs:
// Date, performance.now, timers, rAF and requestIdleCallback); this file makes the rest reproducible:
//   - Math.random, crypto.getRandomValues and crypto.randomUUID come from one seeded xorshift32 stream;
//   - navigator.onLine is fixed to true (offline states fail the connectivity probe through the mock instead);
//   - every source of asynchronous work that the fake clock does not own (fetch, XHR, IndexedDB, workers, image
//     decoding, blobs, digests, wasm compilation, fonts) is counted, so the harness can wait until the page is really
//     idle before it advances the clock again. Nothing here changes what the app observes.
// The seed is prepended by the harness as `self.__IHN_DET_SEED__ = <n>;` (default 0x1badc0de).
//
// Workers: init scripts and the Playwright clock do not reach them, so the same function is installed there too — the
// harness prepends it to worker scripts served over HTTP (run.mjs routes them), and the Worker wrapper below does it
// for blob: workers. Inside a worker `Date` is frozen at `__IHN_DET_EPOCH__` (performance.now stays real).
(function install(g) {
  if (g.__ihnDet) return;
  const inWorker = typeof g.WorkerGlobalScope !== 'undefined' && g instanceof g.WorkerGlobalScope;
  if (inWorker && Number.isFinite(g.__IHN_DET_EPOCH__)) {
    const NativeDate = g.Date, epoch = g.__IHN_DET_EPOCH__;
    const FakeDate = function Date(...args) {
      if (!new.target) return new NativeDate(epoch).toString();
      return args.length ? new NativeDate(...args) : new NativeDate(epoch);
    };
    FakeDate.prototype = NativeDate.prototype;
    FakeDate.now = () => epoch; FakeDate.parse = NativeDate.parse; FakeDate.UTC = NativeDate.UTC;
    g.Date = FakeDate;
  }
  let state = (g.__IHN_DET_SEED__ >>> 0) || 0x1badc0de;
  const next = () => { state ^= state << 13; state >>>= 0; state ^= state >>> 17; state ^= state << 5; state >>>= 0; return state; };
  const det = g.__ihnDet = { busy: 0, events: 0, workers: 0, next };
  const begin = () => { det.busy++; det.events++; };
  const end = () => { det.busy--; det.events++; };
  const track = promise => { begin(); Promise.resolve(promise).then(end, end); return promise; };

  Math.random = () => next() / 4294967296;
  const fill = array => {
    const view = new Uint8Array(array.buffer, array.byteOffset, array.byteLength);
    for (let i = 0; i < view.length; i++) view[i] = next() & 255;
    return array;
  };
  if (g.crypto) {
    try { Object.defineProperty(Object.getPrototypeOf(g.crypto), 'getRandomValues', { configurable: true, writable: true, value: fill }); } catch (error) { g.crypto.getRandomValues = fill; }
    const uuid = () => {
      const b = fill(new Uint8Array(16)); b[6] = (b[6] & 15) | 64; b[8] = (b[8] & 63) | 128;
      const h = Array.from(b, x => x.toString(16).padStart(2, '0')).join('');
      return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
    };
    try { Object.defineProperty(Object.getPrototypeOf(g.crypto), 'randomUUID', { configurable: true, writable: true, value: uuid }); } catch (error) { g.crypto.randomUUID = uuid; }
  }
  if (g.Navigator) { try { Object.defineProperty(g.Navigator.prototype, 'onLine', { configurable: true, get: () => true }); } catch (error) {} }

  // ---- activity accounting ----
  const wrapAsync = (owner, name) => {
    const original = owner && owner[name];
    if (typeof original !== 'function') return;
    owner[name] = function (...args) { return track(original.apply(this, args)); };
  };
  wrapAsync(g, 'fetch');
  wrapAsync(g, 'createImageBitmap');
  if (g.Blob) for (const name of ['arrayBuffer', 'text', 'bytes']) wrapAsync(g.Blob.prototype, name);
  if (g.Response) for (const name of ['arrayBuffer', 'text', 'json', 'blob', 'bytes', 'formData']) wrapAsync(g.Response.prototype, name);
  if (g.crypto && g.crypto.subtle) for (const name of ['digest', 'importKey', 'encrypt', 'decrypt', 'sign', 'verify', 'deriveBits', 'deriveKey', 'generateKey', 'exportKey']) wrapAsync(Object.getPrototypeOf(g.crypto.subtle), name);
  if (g.WebAssembly) for (const name of ['instantiate', 'instantiateStreaming', 'compile', 'compileStreaming']) wrapAsync(g.WebAssembly, name);
  if (g.CompressionStream) { /* streams end in Response/Blob readers, which are tracked */ }
  if (g.HTMLImageElement) wrapAsync(g.HTMLImageElement.prototype, 'decode');
  if (g.HTMLCanvasElement) {
    const toBlob = g.HTMLCanvasElement.prototype.toBlob;
    g.HTMLCanvasElement.prototype.toBlob = function (callback, ...rest) { begin(); return toBlob.call(this, blob => { end(); callback(blob); }, ...rest); };
  }
  if (g.OffscreenCanvas) wrapAsync(g.OffscreenCanvas.prototype, 'convertToBlob');
  const once = (target, names) => {
    begin();
    const done = () => { for (const name of names) target.removeEventListener(name, done); end(); };
    for (const name of names) target.addEventListener(name, done);
  };
  if (g.XMLHttpRequest) {
    const send = g.XMLHttpRequest.prototype.send;
    g.XMLHttpRequest.prototype.send = function (...args) { once(this, ['loadend']); return send.apply(this, args); };
  }
  if (g.FileReader) for (const name of ['readAsArrayBuffer', 'readAsDataURL', 'readAsText', 'readAsBinaryString']) {
    const original = g.FileReader.prototype[name];
    g.FileReader.prototype[name] = function (...args) { once(this, ['loadend']); return original.apply(this, args); };
  }
  if (g.IDBFactory) {
    for (const name of ['open', 'deleteDatabase']) {
      const original = g.IDBFactory.prototype[name];
      g.IDBFactory.prototype[name] = function (...args) { const request = original.apply(this, args); once(request, ['success', 'error']); return request; };
    }
    const transaction = g.IDBDatabase.prototype.transaction;
    g.IDBDatabase.prototype.transaction = function (...args) { const tx = transaction.apply(this, args); once(tx, ['complete', 'abort', 'error']); return tx; };
  }
  if (g.Worker) {
    // A request/response worker is busy while it owes replies. Capped by the harness (a worker may also push freely).
    const NativeWorker = g.Worker, source = install.toString();
    const prelude = () => `self.__IHN_DET_SEED__ = ${(state ^ 0x9e3779b9) >>> 0}; self.__IHN_DET_EPOCH__ = ${inWorker ? g.__IHN_DET_EPOCH__ : Date.now()};\n(${source})(self);\n`;
    // A blob: worker never touches the network: rebuild it with the prelude in front (same origin, synchronous read).
    const deterministic = url => {
      if (!/^blob:/.test(String(url))) return url;
      try {
        const xhr = new XMLHttpRequest(); xhr.open('GET', url, false); xhr.send();
        return URL.createObjectURL(new Blob([prelude(), xhr.responseText], { type: 'text/javascript' }));
      } catch (error) { return url; }
    };
    g.Worker = class extends NativeWorker {
      constructor(url, ...rest) { super(deterministic(url), ...rest); this.addEventListener('message', () => { if (this.__owed > 0) { this.__owed--; det.workers--; } det.events++; }); this.__owed = 0; }
      postMessage(...args) { this.__owed++; det.workers++; det.events++; return super.postMessage(...args); }
      terminate() { det.workers -= this.__owed; this.__owed = 0; return super.terminate(); }
    };
  }
  // The harness adds this script before it installs the clock, so these are the browser's own.
  const nativeRaf = g.requestAnimationFrame ? g.requestAnimationFrame.bind(g) : null;
  const facts = () => {
    let images = 0;
    if (g.document) for (const img of g.document.images) if (!img.complete) images++;
    return { busy: det.busy, workers: det.workers, events: det.events, dirty: det.dirty, images, fonts: g.document && g.document.fonts ? g.document.fonts.status === 'loading' : false,
      ready: g.document ? g.document.readyState : 'complete' };
  };
  // Style recalculation, transition starts, animation events and observer callbacks happen in rendering frames, which
  // the browser schedules in real time. Whenever the document changed (or an animation runs) the harness therefore
  // waits for one real frame before it lets the clock move again, so their effects land in a fixed slice.
  det.dirty = false;
  if (g.document && g.MutationObserver) {
    new g.MutationObserver(() => { det.dirty = true; }).observe(g.document, { subtree: true, childList: true, attributes: true, characterData: true });
    g.addEventListener('scroll', () => { det.dirty = true; }, { capture: true, passive: true });
    g.addEventListener('resize', () => { det.dirty = true; }, { passive: true });
  }
  /** One real task hop, then the page's idle facts. Called by the harness between clock slices. */
  det.probe = () => new Promise(resolve => {
    const channel = new MessageChannel();
    channel.port1.onmessage = () => { channel.port1.close(); resolve(facts()); };
    channel.port2.postMessage(0);
  });
  /** Two real frames (style, layout, observers, paint), then the idle facts. Used before a capture. */
  det.frames = (count = 2) => new Promise(resolve => {
    det.dirty = false;
    if (!nativeRaf || (g.document && g.document.visibilityState === 'hidden')) { det.probe().then(resolve); return; }
    const next = left => nativeRaf(() => left > 1 ? next(left - 1) : det.probe().then(resolve));
    next(count);
  });
  /**
   * CSS animations, transitions and Web Animations follow the fake clock: the harness stops the document timeline
   * (CDP Animation.setPlaybackRate 0) and calls this with every clock slice.
   */
  det.animate = ms => {
    if (!g.document || !g.document.getAnimations) return 0;
    let running = 0;
    for (const animation of g.document.getAnimations()) {
      if (animation.playState !== 'running') continue;
      running++; det.dirty = true;
      try { animation.currentTime = (Number(animation.currentTime) || 0) + ms * (animation.playbackRate || 1); } catch (error) { /* scroll-driven */ }
    }
    return running;
  };
})(self);
