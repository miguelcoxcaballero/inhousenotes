// Canvas command replayer (ARCHITECTURE §3.3 family 2, §6): a dumb executor of rust/crates/ihn-core/src/render/cmd.rs.
// One record `[2][1]` carries a batch of `[u8 op][arguments]` commands (f64 numbers, u32 handles, u32-length UTF-8
// strings). Rust decides every call; this file only makes it. Results: host.io(req, status, obj, bytes).
// Other executors reach the tables through host.surface(id) (a canvas) and host.image(id) (an HTMLImageElement).

const CAPS = ['butt', 'round', 'square'];
const JOINS = ['round', 'miter', 'bevel'];
const COMPOSITES = ['source-over', 'multiply', 'destination-out'];
// Must equal `cmd::op` (a cargo test reads this table).
export const OP = {
  CREATE: 1, BIND: 2, DROP: 3, WIDTH: 4, HEIGHT: 5, CTX: 6, SELECT: 7, SAVE: 8, RESTORE: 9, SET_TRANSFORM: 10, SCALE: 11,
  TRANSLATE: 12, ROTATE: 13, FILL_STYLE: 14, STROKE_STYLE: 15, LINE_WIDTH: 16, LINE_CAP: 17, LINE_JOIN: 18,
  GLOBAL_ALPHA: 19, COMPOSITE: 20, LINE_DASH: 21, BEGIN_PATH: 22, MOVE_TO: 23, LINE_TO: 24, BEZIER_TO: 25, QUAD_TO: 26,
  ARC: 27, ROUND_RECT: 28, RECT: 29, CLIP: 30, STROKE: 31, FILL: 32, CLEAR_RECT: 33, FILL_RECT: 34, STROKE_RECT: 35,
  DRAW_IMAGE: 36, PATH_BEGIN: 37, PATH_END: 38, STROKE_PATH: 39, PATH_DROP: 40, FILTER: 41, SHADOW_COLOR: 42,
  SHADOW_BLUR: 43, RADIAL_FILL: 44, TO_DATA_URL: 45, TO_BLOB: 46, IMAGE_URL: 47, IMAGE_BLOB: 48, IMAGE_DROP: 49,
  FONT: 50, TEXT_ALIGN: 51, TEXT_BASELINE: 52, FILL_TEXT: 53, READ_PIXELS: 54
};

export default function install(host) {
  const surfs = new Map(), ctxs = new Map(), paths = new Map(), images = new Map(), urls = new Map();
  const decoder = new TextDecoder(), encoder = new TextEncoder();
  // Commands for a surface that is gone (its page left the document) are swallowed.
  const VOID = new Proxy(function () {}, { get: () => VOID, set: () => true, apply: () => VOID });
  const lost = () => VOID;
  let ctx = null;
  host.surface = id => surfs.get(id) || null;
  host.image = id => images.get(id) || null;

  const f64s = (...values) => new Uint8Array(new Float64Array(values).buffer);
  function load(image, req, src, owned) {
    const img = new Image();
    images.set(image, img);
    if (owned) urls.set(image, src);
    img.onload = () => host.io(req, 0, 0, f64s(img.naturalWidth, img.naturalHeight));
    img.onerror = () => host.io(req, 1);
    img.src = src;
  }

  function batch(reader) {
    const dv = host.view();
    let p = reader.p;
    const end = p + dv.getUint32(p - 4, true);
    const u8 = () => dv.getUint8(p++);
    const u32 = () => { const v = dv.getUint32(p, true); p += 4; return v; };
    const f = () => { const v = dv.getFloat64(p, true); p += 8; return v; };
    const str = () => { const n = u32(); const text = decoder.decode(host.u8(p, n)); p += n; return text; };
    let target = ctx || lost();
    while (p < end) {
      switch (dv.getUint8(p++)) {
        case OP.CREATE: {
          const id = u32();
          if (u8() === 0) surfs.set(id, document.createElement('canvas'));
          else {
            const w = u32(), h = u32();
            if (typeof OffscreenCanvas === 'function') surfs.set(id, new OffscreenCanvas(w, h));
            else { const canvas = document.createElement('canvas'); canvas.width = w; canvas.height = h; surfs.set(id, canvas); }
          }
          break;
        }
        case OP.BIND: { const id = u32(), node = host.node(u32()); if (node) surfs.set(id, node); break; }
        case OP.DROP: { const id = u32(); if (ctxs.get(id) === ctx) { ctx = null; target = lost(); } surfs.delete(id); ctxs.delete(id); break; }
        case OP.WIDTH: { const canvas = surfs.get(u32()), px = u32(); if (canvas) canvas.width = px; break; }
        case OP.HEIGHT: { const canvas = surfs.get(u32()), px = u32(); if (canvas) canvas.height = px; break; }
        case OP.CTX: {
          const id = u32(), canvas = surfs.get(id), cpu = u8();
          ctx = canvas ? (cpu ? canvas.getContext('2d', { willReadFrequently: true }) : canvas.getContext('2d')) : null;
          if (ctx) ctxs.set(id, ctx);
          target = ctx || lost();
          break;
        }
        case OP.SELECT: ctx = ctxs.get(u32()) || null; target = ctx || lost(); break;
        case OP.SAVE: target.save(); break;
        case OP.RESTORE: target.restore(); break;
        case OP.SET_TRANSFORM: target.setTransform(f(), f(), f(), f(), f(), f()); break;
        case OP.SCALE: target.scale(f(), f()); break;
        case OP.TRANSLATE: target.translate(f(), f()); break;
        case OP.ROTATE: target.rotate(f()); break;
        case OP.FILL_STYLE: target.fillStyle = str(); break;
        case OP.STROKE_STYLE: target.strokeStyle = str(); break;
        case OP.LINE_WIDTH: target.lineWidth = f(); break;
        case OP.LINE_CAP: target.lineCap = CAPS[u8()]; break;
        case OP.LINE_JOIN: target.lineJoin = JOINS[u8()]; break;
        case OP.GLOBAL_ALPHA: target.globalAlpha = f(); break;
        case OP.COMPOSITE: target.globalCompositeOperation = COMPOSITES[u8()]; break;
        case OP.LINE_DASH: { const dash = []; for (let n = u8(); n > 0; n -= 1) dash.push(f()); target.setLineDash(dash); break; }
        case OP.BEGIN_PATH: target.beginPath(); break;
        case OP.MOVE_TO: target.moveTo(f(), f()); break;
        case OP.LINE_TO: target.lineTo(f(), f()); break;
        case OP.BEZIER_TO: target.bezierCurveTo(f(), f(), f(), f(), f(), f()); break;
        case OP.QUAD_TO: target.quadraticCurveTo(f(), f(), f(), f()); break;
        case OP.ARC: target.arc(f(), f(), f(), f(), f()); break;
        case OP.ROUND_RECT: target.roundRect(f(), f(), f(), f(), f()); break;
        case OP.RECT: target.rect(f(), f(), f(), f()); break;
        case OP.CLIP: target.clip(); break;
        case OP.STROKE: target.stroke(); break;
        case OP.FILL: target.fill(); break;
        case OP.CLEAR_RECT: target.clearRect(f(), f(), f(), f()); break;
        case OP.FILL_RECT: target.fillRect(f(), f(), f(), f()); break;
        case OP.STROKE_RECT: target.strokeRect(f(), f(), f(), f()); break;
        case OP.DRAW_IMAGE: {
          const source = u8() === 0 ? surfs.get(u32()) : images.get(u32()), argc = u8();
          const a = []; for (let i = 0; i < argc; i += 1) a.push(f());
          // A zero-sized canvas or a broken image throws; 5.18.0 never draws one, and nothing else may be lost.
          try {
            if (!source) break;
            if (argc === 2) target.drawImage(source, a[0], a[1]);
            else if (argc === 4) target.drawImage(source, a[0], a[1], a[2], a[3]);
            else target.drawImage(source, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]);
          } catch (_) { /* skipped */ }
          break;
        }
        case OP.PATH_BEGIN: target = new Path2D(); paths.set(u32(), target); break;
        case OP.PATH_END: target = ctx || lost(); break;
        case OP.STROKE_PATH: { const path = paths.get(u32()); if (path) target.stroke(path); break; }
        case OP.PATH_DROP: paths.delete(u32()); break;
        case OP.FILTER: target.filter = str(); break;
        case OP.SHADOW_COLOR: target.shadowColor = str(); break;
        case OP.SHADOW_BLUR: target.shadowBlur = f(); break;
        case OP.RADIAL_FILL: {
          const gradient = target.createRadialGradient(f(), f(), f(), f(), f(), f());
          for (let n = u8(); n > 0; n -= 1) { const at = f(); gradient.addColorStop(at, str()); }
          target.fillStyle = gradient;
          break;
        }
        case OP.TO_DATA_URL: {
          const canvas = surfs.get(u32()), req = u32(), type = str(), quality = f();
          let url = null;
          try { url = canvas && canvas.toDataURL ? canvas.toDataURL(type, quality) : null; } catch (_) { /* tainted or lost */ }
          host.io(req, url ? 0 : 1, 0, url ? encoder.encode(url) : null);
          break;
        }
        case OP.TO_BLOB: {
          const canvas = surfs.get(u32()), req = u32(), type = str(), quality = f();
          const done = blob => host.io(req, blob ? 0 : 1, host.hold(blob), blob ? f64s(blob.size) : null);
          try {
            if (canvas && canvas.toBlob) canvas.toBlob(done, type, quality);
            else if (canvas) canvas.convertToBlob({ type, quality }).then(done, () => done(null));
            else done(null);
          } catch (_) { done(null); }
          break;
        }
        case OP.IMAGE_URL: { const image = u32(), req = u32(); load(image, req, str(), false); break; }
        case OP.IMAGE_BLOB: {
          const image = u32(), req = u32(), blob = host.obj(u32());
          if (blob) load(image, req, URL.createObjectURL(blob), true); else host.io(req, 1);
          break;
        }
        case OP.IMAGE_DROP: { const image = u32(); if (urls.has(image)) URL.revokeObjectURL(urls.get(image)); urls.delete(image); images.delete(image); break; }
        case OP.FONT: target.font = str(); break;
        case OP.TEXT_ALIGN: target.textAlign = str(); break;
        case OP.TEXT_BASELINE: target.textBaseline = str(); break;
        case OP.FILL_TEXT: { const text = str(); target.fillText(text, f(), f()); break; }
        case OP.READ_PIXELS: { const x = f(), y = f(), w = f(), h = f(); try { target.getImageData(x, y, w, h); } catch (_) { /* nothing to materialise */ } break; }
        default: throw new Error(`ihn: canvas command ${dv.getUint8(p - 1)} at ${p - 1 - reader.p}`);
      }
    }
  }

  return { [2 << 8 | 1]: batch };
}
