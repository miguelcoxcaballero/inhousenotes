// WebRTC and BroadcastChannel transports (family 3.5 = 0x60..0x6f; SYNC §18.3).
// Stub created by W1.1; the complete executor is implemented by W2.8 (BUILD-PLAN.md).
// Contract (host/main.js): return { [family << 8 | op]: reader => … }; report results with
// host.io(req, status, obj, bytes) or host.message(chan, data, obj).
export default function install(_host) {
  return {};
}
