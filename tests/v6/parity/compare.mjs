// Comparison of two captures (TESTING §3.2): pixels at tolerance 0 (the gate), DOM and canvas trace (diagnostics).
// PNG decoding/encoding is done here with node:zlib (no image dependency): 8-bit truecolour, with or without alpha,
// non-interlaced — what Chromium screenshots are.
import zlib from 'node:zlib';

const CRC = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
const crc32 = buf => { let c = 0xffffffff; for (let i = 0; i < buf.length; i++) c = CRC[(c ^ buf[i]) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };

/** @returns {{width:number,height:number,data:Buffer}} RGBA, 4 bytes per pixel */
export function decodePng(png) {
  if (png.readUInt32BE(0) !== 0x89504e47) throw new Error('Not a PNG');
  let at = 8, width = 0, height = 0, depth = 0, type = 0, interlace = 0; const idat = [];
  while (at < png.length) {
    const length = png.readUInt32BE(at), kind = png.toString('latin1', at + 4, at + 8), data = png.subarray(at + 8, at + 8 + length);
    if (kind === 'IHDR') { width = data.readUInt32BE(0); height = data.readUInt32BE(4); depth = data[8]; type = data[9]; interlace = data[12]; }
    else if (kind === 'IDAT') idat.push(data);
    else if (kind === 'IEND') break;
    at += 12 + length;
  }
  const channels = type === 6 ? 4 : type === 2 ? 3 : type === 0 ? 1 : type === 4 ? 2 : 0;
  if (depth !== 8 || !channels || interlace) throw new Error(`Unsupported PNG (depth ${depth}, colour type ${type}, interlace ${interlace})`);
  const raw = zlib.inflateSync(Buffer.concat(idat)), stride = width * channels, out = Buffer.alloc(width * height * 4);
  let prev = Buffer.alloc(stride);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)], line = Buffer.from(raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1)));
    for (let x = 0; x < stride; x++) {
      const a = x >= channels ? line[x - channels] : 0, b = prev[x], c = x >= channels ? prev[x - channels] : 0;
      let add = 0;
      if (filter === 1) add = a; else if (filter === 2) add = b; else if (filter === 3) add = (a + b) >> 1;
      else if (filter === 4) { const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c); add = pa <= pb && pa <= pc ? a : pb <= pc ? b : c; }
      line[x] = (line[x] + add) & 255;
    }
    for (let x = 0; x < width; x++) {
      const o = (y * width + x) * 4, i = x * channels;
      if (channels >= 3) { out[o] = line[i]; out[o + 1] = line[i + 1]; out[o + 2] = line[i + 2]; out[o + 3] = channels === 4 ? line[i + 3] : 255; }
      else { out[o] = out[o + 1] = out[o + 2] = line[i]; out[o + 3] = channels === 2 ? line[i + 1] : 255; }
    }
    prev = line;
  }
  return { width, height, data: out };
}

export function encodePng({ width, height, data }) {
  const chunk = (kind, body) => { const out = Buffer.alloc(12 + body.length); out.writeUInt32BE(body.length, 0); out.write(kind, 4, 'latin1'); body.copy(out, 8); out.writeUInt32BE(crc32(out.subarray(4, 8 + body.length)), 8 + body.length); return out; };
  const head = Buffer.alloc(13); head.writeUInt32BE(width, 0); head.writeUInt32BE(height, 4); head[8] = 8; head[9] = 6;
  const raw = Buffer.alloc(height * (width * 4 + 1));
  for (let y = 0; y < height; y++) data.copy(raw, y * (width * 4 + 1) + 1, y * width * 4, (y + 1) * width * 4);
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', head), chunk('IDAT', zlib.deflateSync(raw, { level: 6 })), chunk('IEND', Buffer.alloc(0))]);
}

/**
 * Pixel comparison at tolerance 0 on decoded RGBA.
 * @returns {{equal:boolean, different:number, total:number, box:null|{x:number,y:number,width:number,height:number}, maxDelta:number, diff:()=>Buffer}}
 */
export function comparePixels(pngA, pngB) {
  if (pngA.equals(pngB)) return { equal: true, different: 0, total: 0, box: null, maxDelta: 0, diff: () => pngA };
  const a = decodePng(pngA), b = decodePng(pngB);
  if (a.width !== b.width || a.height !== b.height) return { equal: false, different: -1, total: a.width * a.height, box: null, maxDelta: 255, sizes: [`${a.width}x${a.height}`, `${b.width}x${b.height}`], diff: () => pngB };
  let different = 0, x0 = a.width, y0 = a.height, x1 = -1, y1 = -1, maxDelta = 0;
  const marks = [];
  for (let i = 0, p = 0; i < a.data.length; i += 4, p++) {
    if (a.data[i] === b.data[i] && a.data[i + 1] === b.data[i + 1] && a.data[i + 2] === b.data[i + 2] && a.data[i + 3] === b.data[i + 3]) continue;
    different++; marks.push(i);
    const x = p % a.width, y = (p / a.width) | 0;
    if (x < x0) x0 = x; if (x > x1) x1 = x; if (y < y0) y0 = y; if (y > y1) y1 = y;
    for (let k = 0; k < 4; k++) maxDelta = Math.max(maxDelta, Math.abs(a.data[i + k] - b.data[i + k]));
  }
  return {
    equal: different === 0, different, total: a.width * a.height, maxDelta, width: a.width, marks,
    box: different ? { x: x0, y: y0, width: x1 - x0 + 1, height: y1 - y0 + 1 } : null,
    /** A (dimmed) with every differing pixel in red. */
    diff: () => { const out = Buffer.from(a.data); for (let i = 0; i < out.length; i += 4) { out[i] = 170 + (out[i] >> 2); out[i + 1] = 170 + (out[i + 1] >> 2); out[i + 2] = 170 + (out[i + 2] >> 2); out[i + 3] = 255; } for (const i of marks) { out[i] = 255; out[i + 1] = 0; out[i + 2] = 0; } return encodePng({ width: a.width, height: a.height, data: out }); }
  };
}

/** Line diff good enough to locate a cause: common prefix/suffix removed, then the differing window (capped). */
export function diffLines(textA, textB, { context = 2, limit = 60 } = {}) {
  if (textA === textB) return null;
  const a = textA.split('\n'), b = textB.split('\n');
  let start = 0; while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length, endB = b.length; while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) { endA--; endB--; }
  const cut = line => line.length > 400 ? line.slice(0, 400) + '…' : line;
  const out = [`@@ line ${start + 1}: -${endA - start} +${endB - start} (of ${a.length} / ${b.length})`];
  for (let i = Math.max(0, start - context); i < start; i++) out.push('  ' + cut(a[i]));
  for (let i = start; i < Math.min(endA, start + limit); i++) out.push('- ' + cut(a[i]));
  if (endA - start > limit) out.push(`- … ${endA - start - limit} more`);
  for (let i = start; i < Math.min(endB, start + limit); i++) out.push('+ ' + cut(b[i]));
  if (endB - start > limit) out.push(`+ … ${endB - start - limit} more`);
  return out.join('\n');
}

/** Per-surface comparison of two normalized traces: first diverging op of each surface. Null when identical. */
export function compareTraces(a, b) {
  const byLabel = t => new Map((t?.surfaces || []).map(s => [s.label, s]));
  const A = byLabel(a), B = byLabel(b), out = [];
  for (const label of new Set([...A.keys(), ...B.keys()])) {
    const x = A.get(label), y = B.get(label);
    if (!x || !y) { out.push(`${label}: only in ${x ? 'A' : 'B'} (${(x || y).ops.length} ops)`); continue; }
    const n = Math.min(x.ops.length, y.ops.length);
    let i = 0; while (i < n && JSON.stringify(x.ops[i]) === JSON.stringify(y.ops[i])) i++;
    if (i === n && x.ops.length === y.ops.length) continue;
    out.push(`${label}: diverges at op ${i} of ${x.ops.length} / ${y.ops.length}\n  A: ${JSON.stringify(x.ops[i] ?? null)}\n  B: ${JSON.stringify(y.ops[i] ?? null)}`);
  }
  return out.length ? out.join('\n') : null;
}

/** Full comparison of two captures `{png, dom, trace}`. `pixelsEqual` is the gate; the rest locates causes. */
export function compareCaptures(a, b) {
  const pixels = comparePixels(a.png, b.png);
  return { pixelsEqual: pixels.equal, pixels, dom: diffLines(a.dom || '', b.dom || ''), trace: compareTraces(a.trace, b.trace) };
}
