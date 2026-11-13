// GIS, InhouseNative bridges, updater, app mode, file chooser, clipboard, share, download, fullscreen, history (family 3.6 = 0xa0..0xdf). Sets the app-mode bit (1) in host.config.flags during install (ARCHITECTURE §3.7).
// Stub created by W1.1; the complete executor is implemented by W1.3 (BUILD-PLAN.md).
// Contract (host/main.js): return { [family << 8 | op]: reader => … }; report results with
// host.io(req, status, obj, bytes) or host.message(chan, data, obj).
export default function install(_host) {
  return {};
}
