// Normalization of the two diagnostics of TESTING §3.2: the DOM (runs in the page) and the canvas trace (Node).

/**
 * Runs in the page (`page.evaluate(normalizeDom)`): returns the document as one node per line.
 * - attributes sorted by name; `style` re-serialised through a scratch element's CSSOM;
 * - `<script>` elements (and so `#inhouse-native-assets`) removed;
 * - `?v=` / `native=` queries stripped from `<link>`/`<meta>` URLs;
 * - `blob:` URLs → `blob:<n>` in order of appearance; `data:image/*` URLs → `data:sha256:<hex>`;
 * - values of `data-page-id` and `data-version-id` → `id<n>` in order of first appearance (N-11);
 * - live facts that markup does not show are added as pseudo attributes: `:value`, `:checked`, `:scroll`.
 */
export async function normalizeDom() {
  const scratch = document.createElement('div');
  const blobs = new Map(), ids = new Map(), datas = new Map();
  const blob = url => { if (!blobs.has(url)) blobs.set(url, `blob:${blobs.size}`); return blobs.get(url); };
  const idOf = value => { if (!ids.has(value)) ids.set(value, `id${ids.size}`); return ids.get(value); };
  const text = value => String(value).replace(/blob:[^\s"')]+/g, blob).replace(/data:image\/[^\s"')]+/g, url => { if (!datas.has(url)) datas.set(url, `\u0000data${datas.size}\u0000`); return datas.get(url); });
  const escape = value => value.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/\n/g, '&#10;');
  const lines = [];
  const walk = (node, depth) => {
    const pad = '  '.repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) { const t = node.nodeValue.replace(/\s+/g, ' ').trim(); if (t) lines.push(`${pad}"${text(t)}"`); return; }
    if (node.nodeType === Node.COMMENT_NODE) return;
    if (node.nodeType !== Node.ELEMENT_NODE) return;
    const tag = node.localName;
    if (tag === 'script') return;
    const attrs = [];
    for (const attr of node.attributes) {
      let value = attr.value;
      if (attr.name === 'style') { scratch.setAttribute('style', value); value = scratch.style.cssText; }
      else if (attr.name === 'data-page-id' || attr.name === 'data-version-id') value = idOf(value);
      else if ((tag === 'link' || tag === 'meta') && /[?&](v|native)=/.test(value)) value = value.replace(/\?.*$/, '');
      attrs.push([attr.name, text(value)]);
    }
    if (tag === 'input' || tag === 'textarea' || tag === 'select') {
      attrs.push([':value', text(node.value)]);
      if (node.type === 'checkbox' || node.type === 'radio') attrs.push([':checked', String(node.checked)]);
    }
    if (node.scrollTop || node.scrollLeft) attrs.push([':scroll', `${node.scrollLeft},${node.scrollTop}`]);
    attrs.sort((a, b) => a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0);
    lines.push(`${pad}<${tag}${attrs.map(([k, v]) => ` ${k}="${escape(v)}"`).join('')}>`);
    if (tag === 'style') { const css = node.textContent.trim(); if (css) lines.push(`${pad}  "${text(css.replace(/\s+/g, ' '))}"`); return; }
    for (const child of node.childNodes) walk(child, depth + 1);
    if (node.shadowRoot) for (const child of node.shadowRoot.childNodes) walk(child, depth + 1);
  };
  walk(document.documentElement, 0);
  let out = lines.join('\n');
  for (const [url, mark] of datas) {
    const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(url));
    out = out.split(mark).join('data:sha256:' + Array.from(new Uint8Array(digest), b => b.toString(16).padStart(2, '0')).join(''));
  }
  return out;
}

/**
 * Raw `__ihnTrace.take()` → per-surface op streams that can be compared between apps and against Rust:
 * - surfaces are named by their DOM label (`#id`, `canvas.class[page=n]`), detached/offscreen ones `offscreen#<k>` in
 *   order of first use;
 * - `stroke(path)` / `fill(path)` / `clip(path)` are expanded to the path's construction ops at the time of the call
 *   (`path:<op>` entries), so a cached Path2D and a freshly built path compare equal iff the geometry is bit-identical;
 * - gradients and patterns are inlined where they are assigned; bitmap identities are dropped (size kept);
 * - yields between repaint slices leave no mark: only the per-surface order counts.
 */
export function normalizeTrace(raw) {
  if (!raw) return { surfaces: [] };
  const names = new Map(); let detached = 0;
  for (const s of raw.surfaces) names.set(s.id, s.label || null);
  const nameOf = id => { let n = names.get(id); if (!n) { n = `offscreen#${detached++}`; names.set(id, n); } return n; };
  const refs = new Map(), streams = new Map(), meta = new Map(raw.surfaces.map(s => [s.id, s]));
  const stream = id => { const name = nameOf(id); let s = streams.get(name); if (!s) { const m = meta.get(id) || {}; s = { label: name, kind: m.kind || 'canvas', width: m.width ?? null, height: m.height ?? null, ops: [] }; streams.set(name, s); } return s; };
  const arg = v => {
    if (!v || typeof v !== 'object') return v;
    if (Array.isArray(v)) return v.map(arg);
    if (v.surface) return { surface: nameOf(v.surface) };
    if (v.ref) { const r = refs.get(v.ref); return r ? { [r.kind]: [r.create, ...r.calls] } : { ref: '?' }; }
    if (v.bitmap) return { bitmap: true, w: v.w, h: v.h };
    return v;
  };
  const expand = (ops, out) => { for (const op of ops) { if (op[0] === 'addPath') { out.push(['path:addPath:begin', op[2]]); expand(op[1], out); out.push(['path:addPath:end']); } else out.push([`path:${op[0]}`, ...op.slice(1)]); } };
  for (const entry of raw.ops) {
    const [target, name, ...args] = entry;
    if (/^[gt]\d+$/.test(target) || /^[gt]\?$/.test(target)) {
      if (/^create/.test(name)) refs.set(target, { kind: target[0] === 'g' ? 'gradient' : 'pattern', create: [name, ...args.slice(1).map(arg)], calls: [] });
      else refs.get(target)?.calls.push([name, ...args.map(arg)]);
      continue;
    }
    const s = stream(target);
    const pathArg = args.find(a => a && typeof a === 'object' && a.path);
    if (pathArg) {
      s.ops.push(['path:begin']);
      expand((raw.paths[pathArg.path] || []).slice(0, pathArg.n), s.ops);
      s.ops.push([`${name}(path)`, ...args.filter(a => a !== pathArg).map(arg)]);
    } else s.ops.push([name, ...args.map(arg)]);
  }
  return { surfaces: [...streams.values()].sort((a, b) => a.label < b.label ? -1 : a.label > b.label ? 1 : 0) };
}

/** Deterministic text form of a normalized trace: one op per line (diffable, and what the Rust goldens read). */
export function stringifyTrace(normalized, header = {}) {
  const surfaces = normalized.surfaces.map(s => `    {"label":${JSON.stringify(s.label)},"kind":${JSON.stringify(s.kind)},"width":${JSON.stringify(s.width)},"height":${JSON.stringify(s.height)},"ops":[\n${s.ops.map(op => '      ' + JSON.stringify(op)).join(',\n')}\n    ]}`);
  const head = Object.entries(header).map(([k, v]) => `  ${JSON.stringify(k)}: ${JSON.stringify(v)},\n`).join('');
  return `{\n${head}  "surfaces": [\n${surfaces.join(',\n')}\n  ]\n}\n`;
}

/** Decodes a `~hex` number of a trace back to a JS number (for humans reading a diff). */
export function traceNumber(text) {
  const view = new DataView(new ArrayBuffer(8));
  view.setBigUint64(0, BigInt('0x' + text.slice(1)));
  return view.getFloat64(0);
}
