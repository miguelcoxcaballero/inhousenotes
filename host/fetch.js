// fetch with deadlines, resumable and keepalive uploads, blob handles, SHA-256 and deflate (families 3.3 = 0x30..0x4f, 3.8 = 0x78..0x7f, 3.10 = 0x90..0x9f).
// Stub created by W1.1; the complete executor is implemented by W1.3 (BUILD-PLAN.md).
// Contract (host/main.js): return { [family << 8 | op]: reader => … }; report results with
// host.io(req, status, obj, bytes) or host.message(chan, data, obj).
export default function install(_host) {
  return {};
}
