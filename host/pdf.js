// pdf.js adapter: lazy load, hardened open, metadata, attachments, page rasters (family 3.4 = 0x50..0x5f).
// Stub created by W1.1; the complete executor is implemented by W3.1 (BUILD-PLAN.md).
// Contract (host/main.js): return { [family << 8 | op]: reader => … }; report results with
// host.io(req, status, obj, bytes) or host.message(chan, data, obj).
export default function install(_host) {
  return {};
}
