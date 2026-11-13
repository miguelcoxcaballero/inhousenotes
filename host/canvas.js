// Canvas command replayer, Path2D and image handle tables (ARCHITECTURE §3.3 family 2, §6). Ops: 2 << 8 | op.
// Stub created by W1.1; the complete executor is implemented by W2.3 (BUILD-PLAN.md).
// Contract (host/main.js): return { [family << 8 | op]: reader => … }; report results with
// host.io(req, status, obj, bytes) or host.message(chan, data, obj).
export default function install(_host) {
  return {};
}
