// pdf.js adapter (effect family 3.4 = 0x50..0x5f): lazy load, hardened open, metadata, attachments, page sizes and
// page rasters. A dumb executor of rust/crates/ihn-app/src/doc/open.rs (`pdf` module): Rust decides which blob,
// which page and which scale; nothing here knows about documents.
//
// Results: host.io(req, status, obj, bytes). status 0 = done; -1 = failed (bytes = the message); -2 = the blob
// breaks the size/type limit; -3 = the page count is outside the limit (limits and texts come with the open).
const FX = 3 << 8;
const encoder = new TextEncoder();

// `[u32 n][bytes]` for strings, f64 for numbers, {u32} for counts.
function pack(...fields) {
  const parts = fields.map(field => (typeof field === 'string' ? encoder.encode(field) : field));
  const size = parts.reduce((n, part) => n + (part instanceof Uint8Array ? 4 + part.length : typeof part === 'number' ? 8 : 4), 0);
  const out = new Uint8Array(size), view = new DataView(out.buffer);
  let p = 0;
  for (const part of parts) {
    if (typeof part === 'number') { view.setFloat64(p, part, true); p += 8; continue; }
    if (!(part instanceof Uint8Array)) { view.setUint32(p, part.u32, true); p += 4; continue; }
    view.setUint32(p, part.length, true);
    out.set(part, p + 4);
    p += 4 + part.length;
  }
  return out;
}

export default function install(host) {
  let library = null;
  const asset = path => window.InhouseAssetUrl(path);
  // pdf.js 3.11.174 is a classic script that defines `pdfjsLib`; it is fetched on first need (N-9).
  const load = () => (library ||= new Promise((resolve, reject) => {
    if (window.pdfjsLib) return resolve(window.pdfjsLib);
    const script = document.createElement('script');
    script.src = asset(`vendor/pdfjs/pdf.min.js?v=${host.config.version}`);
    script.onload = () => (window.pdfjsLib ? resolve(window.pdfjsLib) : reject(new Error('PDF.js is unavailable')));
    script.onerror = () => { library = null; script.remove(); reject(new Error('PDF.js is unavailable')); };
    document.head.appendChild(script);
  }).then(pdfjs => {
    pdfjs.GlobalWorkerOptions.workerSrc = asset('vendor/pdfjs/pdf.worker.min.js');
    return pdfjs;
  }));
  const fail = (req, error, status = -1) => host.io(req, status, 0, encoder.encode(String(error?.message || error || '')));
  const base = () => new URL('vendor/pdfjs', location.href).href;

  // security-core-v5.js: assertSafePdfBlob + hardenPdfDocumentParams + loadPdfDocument, with Rust's limits.
  async function open(req, blob, limit) {
    if (!(blob instanceof Blob) || !(blob.size > 0) || blob.size > limit.bytes || (blob.type && !limit.type.test(blob.type))) {
      return fail(req, limit.large, -2);
    }
    const pdfjs = await load();
    const task = pdfjs.getDocument({
      data: new Uint8Array(await blob.arrayBuffer()),
      cMapUrl: `${base()}/cmaps/`, cMapPacked: true, standardFontDataUrl: `${base()}/standard_fonts/`,
      isEvalSupported: false, maxImageSize: limit.pixels
    });
    const doc = await task.promise;
    if (!Number.isInteger(doc?.numPages) || doc.numPages < 1 || doc.numPages > limit.pages) {
      try { await task.destroy(); } catch (_) { /* already gone */ }
      return fail(req, limit.many, -3);
    }
    const [metadata, attachments] = await Promise.all([doc.getMetadata().catch(() => null), doc.getAttachments().catch(() => null)]);
    const keywords = typeof metadata?.info?.Keywords === 'string' ? metadata.info.Keywords : '';
    const files = Object.entries(attachments || {}).map(([key, value]) => [String(value?.filename || key), value?.content?.length || 0]);
    host.io(req, 0, host.hold({ doc, attachments: attachments || {} }), pack({ u32: doc.numPages }, keywords, { u32: files.length }, ...files.flat()));
  }

  async function render(req, entry, number, scale, fill, type, quality) {
    const page = await entry.doc.getPage(number);
    const viewport = page.getViewport({ scale });
    const canvas = document.createElement('canvas');
    canvas.width = viewport.width;
    canvas.height = viewport.height;
    const context = canvas.getContext('2d');
    context.fillStyle = fill;
    context.fillRect(0, 0, canvas.width, canvas.height);
    // Hidden pages have no animation frames; pdf.js display rendering waits for one (5.18.0 rule).
    await page.render({ canvasContext: context, viewport, intent: document.hidden ? 'print' : 'display' }).promise;
    const blob = await new Promise(resolve => canvas.toBlob(resolve, type, quality));
    const size = [canvas.width, canvas.height];
    canvas.width = canvas.height = 0;
    page.cleanup?.();
    if (!blob) return fail(req, 'The page could not be encoded');
    host.io(req, 0, host.hold(blob), pack(URL.createObjectURL(blob), ...size));
  }

  const entry = (req, id) => host.obj(id) || (fail(req, 'No such PDF'), null);
  return {
    // pdf_load(u32 req) → 0 once pdf.js is ready (prewarm).
    [FX | 0x50]: r => { const req = r.u32(); load().then(() => host.io(req, 0), error => fail(req, error)); },
    // pdf_open(u32 req, u32 blob, f64 bytes, str type, f64 pixels, u32 pages, str large, str many) → obj = the
    // document, `u32 pages, str keywords, u32 n, n × (str name, f64 size)`.
    [FX | 0x51]: r => {
      const req = r.u32(), blob = host.obj(r.u32()), limit = { bytes: r.f64(), type: new RegExp(r.str(), 'i'), pixels: r.f64(), pages: r.u32(), large: r.str(), many: r.str() };
      open(req, blob, limit).catch(error => fail(req, error));
    },
    // pdf_pages(u32 req, u32 doc, u32 first, u32 count) → `count × (f64 width, f64 height)` at scale 1 (points).
    [FX | 0x52]: r => {
      const req = r.u32(), e = entry(req, r.u32()), first = r.u32(), count = r.u32();
      if (!e) return;
      Promise.all(Array.from({ length: Math.max(0, Math.min(count, e.doc.numPages - first + 1)) }, (_, i) => e.doc.getPage(first + i)))
        .then(pages => host.io(req, 0, 0, pack(...pages.flatMap(page => { const v = page.getViewport({ scale: 1 }); return [v.width, v.height]; }))), error => fail(req, error));
    },
    // pdf_attachment(u32 req, u32 doc, str name) → obj = a Blob of the embedded file, `f64 size`.
    [FX | 0x53]: r => {
      const req = r.u32(), e = entry(req, r.u32()), name = r.str();
      if (!e) return;
      const found = Object.entries(e.attachments).find(([key, value]) => (value?.filename || key) === name)?.[1];
      if (!found?.content) return fail(req, 'No such attachment');
      const blob = new Blob([found.content]);
      host.io(req, 0, host.hold(blob), pack(blob.size));
    },
    // pdf_render(u32 req, u32 doc, u32 page, f64 scale, str fill, str type, f64 quality) → obj = the image Blob,
    // `str blob URL, f64 width, f64 height`.
    [FX | 0x54]: r => {
      const req = r.u32(), e = entry(req, r.u32()), number = r.u32(), scale = r.f64(), fill = r.str(), type = r.str(), quality = r.f64();
      if (e) render(req, e, number, scale, fill, type, quality).catch(error => fail(req, error));
    },
    // pdf_close(u32 doc): destroys the document and frees the handle.
    [FX | 0x55]: r => {
      const id = r.u32(), e = host.obj(id);
      host.objs.delete(id);
      void e?.doc?.destroy?.().catch?.(() => {});
    }
  };
}
