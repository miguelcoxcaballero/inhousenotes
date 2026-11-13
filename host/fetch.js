// Effect families 3.3 fetch/blob (0x30..0x4f), 3.8 digest (0x78) and 3.10 deflate (0x90): a dumb executor of
// rust/crates/ihn-app/src/shell/fx.rs. Rust builds every request (URL, headers, body, deadlines) and decides what a
// result means; nothing here knows about Drive.
//
// Results: host.io(req, status, obj, bytes). status >= 0 is success (the HTTP status for a fetch); failures are
// -1 failed (bytes = the error message), -2 deadline before the response headers, -3 aborted by Rust,
// -5 deadline or idle timeout while the body was being read.
// Object handles >= 0x80000000 are chosen by Rust (blob_new, blob_slice, …); lower ones come from host.hold().

const FX = 3 << 8;
const encoder = new TextEncoder();

// `[u32 n][bytes]…` for strings, raw little-endian f64 for numbers.
function pack(...fields) {
  const parts = fields.map(field => (typeof field === 'number' ? field : encoder.encode(field)));
  const out = new Uint8Array(parts.reduce((n, part) => n + (typeof part === 'number' ? 8 : 4 + part.length), 0));
  const view = new DataView(out.buffer);
  let p = 0;
  for (const part of parts) {
    if (typeof part === 'number') { view.setFloat64(p, part, true); p += 8; continue; }
    view.setUint32(p, part.length, true);
    out.set(part, p + 4);
    p += 4 + part.length;
  }
  return out;
}

export default function install(host) {
  const running = new Map();
  const fail = (req, error, status = -1) => host.io(req, status, 0, encoder.encode(String(error?.message || error || '')));
  const slice = (blob, start, end) => (blob && (start > 0 || end >= 0) ? blob.slice(start, end < 0 ? blob.size : end) : blob);
  // A byte source: `u8 kind` 0 none, 1 inline bytes, 2 `u32 blob, f64 start, f64 end` (end < 0: to the end).
  const source = r => {
    const kind = r.u8();
    if (kind === 1) return new Blob([r.copy()]);
    return kind === 2 ? slice(host.obj(r.u32()), r.f64(), r.f64()) : undefined;
  };
  const bytes = async blob => new Uint8Array(await blob.arrayBuffer());

  async function run(req, method, url, headerText, flags, deadline, idle, body) {
    const controller = new AbortController();
    let reason = -1, inBody = false, idleTimer = 0;
    const stop = code => { if (reason === -1) reason = code; controller.abort(); };
    const timer = deadline > 0 ? setTimeout(() => stop(inBody ? -5 : -2), deadline) : 0;
    running.set(req, () => stop(-3));
    try {
      const headers = new Headers();
      for (const line of headerText.split('\n')) {
        const at = line.indexOf(': ');
        if (at > 0) headers.append(line.slice(0, at), line.slice(at + 2));
      }
      const init = { method, headers, body, signal: controller.signal, keepalive: !!(flags & 1) };
      if (flags & 4) init.cache = 'no-store';
      if (flags & 8) init.credentials = 'omit';
      const response = await fetch(url, init);
      inBody = true;
      let head = '';
      response.headers.forEach((value, name) => { head += `${name}: ${value}\n`; });
      const chunks = [];
      if (flags & 16) void response.body?.cancel().catch(() => {});
      else if (response.body) {
        const reader = response.body.getReader();
        for (;;) {
          if (idle > 0) { clearTimeout(idleTimer); idleTimer = setTimeout(() => stop(-5), idle); }
          const chunk = await reader.read();
          if (chunk.done) break;
          chunks.push(chunk.value);
        }
      }
      const blob = new Blob(chunks, { type: response.headers.get('Content-Type') || '' });
      if (flags & 2) host.io(req, response.status, host.hold(blob), pack(head, blob.size));
      else {
        const prefix = pack(head), out = new Uint8Array(prefix.length + blob.size);
        out.set(prefix);
        out.set(await bytes(blob), prefix.length);
        host.io(req, response.status, 0, out);
      }
    } catch (error) {
      fail(req, error, reason);
    } finally {
      clearTimeout(timer);
      clearTimeout(idleTimer);
      running.delete(req);
    }
  }

  return {
    // fetch(u32 req, str method, str url, str headers "name: value\n"…, u32 flags, f64 deadline_ms, f64 idle_ms, source)
    // flags: 1 keepalive, 2 body as a Blob handle, 4 cache no-store, 8 credentials omit, 16 status and headers only.
    // → status, `str headers, body…` (flag 2: obj = Blob, `str headers, f64 size`).
    [FX | 0x30]: r => { void run(r.u32(), r.str(), r.str(), r.str(), r.u32(), r.f64(), r.f64(), source(r)); },
    [FX | 0x31]: r => running.get(r.u32())?.(),
    // blob_new(u32 obj, str type, bytes)
    [FX | 0x38]: r => { const obj = r.u32(), type = r.str(); host.objs.set(obj, new Blob([r.copy()], { type })); },
    // blob_slice(u32 obj, u32 src, f64 start, f64 end, str type)
    [FX | 0x39]: r => {
      const obj = r.u32(), src = host.obj(r.u32()), start = r.f64(), end = r.f64(), type = r.str();
      if (src) host.objs.set(obj, src.slice(start, end < 0 ? src.size : end, type || src.type));
    },
    // blob_concat(u32 obj, str type, u32 n, n × u32 src)
    [FX | 0x3a]: r => {
      const obj = r.u32(), type = r.str(), parts = [];
      for (let n = r.u32(); n > 0; n -= 1) parts.push(host.obj(r.u32()));
      host.objs.set(obj, new Blob(parts.filter(Boolean), { type }));
    },
    // blob_read(u32 req, u32 src, f64 start, f64 end) → 0, bytes
    [FX | 0x3b]: r => {
      const req = r.u32(), blob = slice(host.obj(r.u32()), r.f64(), r.f64());
      if (!blob) return fail(req, 'No such blob');
      bytes(blob).then(data => host.io(req, 0, 0, data), error => fail(req, error));
    },
    // blob_url(u32 req, u32 src) → 0, the object URL
    [FX | 0x3c]: r => {
      const req = r.u32(), blob = host.obj(r.u32());
      if (!blob) return fail(req, 'No such blob');
      host.io(req, 0, 0, encoder.encode(URL.createObjectURL(blob)));
    },
    [FX | 0x3d]: r => URL.revokeObjectURL(r.str()),
    // blob_info(u32 req, u32 src) → 0, `f64 size, str type, str name, f64 lastModified`
    [FX | 0x3e]: r => {
      const req = r.u32(), blob = host.obj(r.u32());
      if (!blob) return fail(req, 'No such blob');
      host.io(req, 0, 0, pack(blob.size, blob.type || '', blob.name || '', blob.lastModified || 0));
    },
    // digest(u32 req, source) → 0, the 32 bytes of SHA-256
    [FX | 0x78]: r => {
      const req = r.u32(), blob = source(r);
      if (!blob) return fail(req, 'No such blob');
      blob.arrayBuffer().then(data => crypto.subtle.digest('SHA-256', data))
        .then(hash => host.io(req, 0, 0, new Uint8Array(hash)), error => fail(req, error));
    },
    // deflate(u32 req, u8 flags, source): flags 1 inflate, 2 zlib wrapper (else raw), 4 result as a Blob handle.
    [FX | 0x90]: r => {
      const req = r.u32(), flags = r.u8(), blob = source(r);
      if (!blob) return fail(req, 'No such blob');
      const Stream = flags & 1 ? DecompressionStream : CompressionStream;
      new Response(blob.stream().pipeThrough(new Stream(flags & 2 ? 'deflate' : 'deflate-raw'))).blob().then(async result => {
        if (flags & 4) host.io(req, 0, host.hold(result), pack(result.size));
        else host.io(req, 0, 0, await bytes(result));
      }).catch(error => fail(req, error));
    }
  };
}
