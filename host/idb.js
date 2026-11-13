// IndexedDB transaction executor, Web Locks and the read-only 5.x dumper (families 3.2 = 0x20..0x2f and 3.9 = 0x80..0x8f).
// Stub created by W1.1; the complete executor is implemented by W2.4 (BUILD-PLAN.md).
// Contract (host/main.js): return { [family << 8 | op]: reader => … }; report results with
// host.io(req, status, obj, bytes) or host.message(chan, data, obj).
export default function install(_host) {
  return {};
}
