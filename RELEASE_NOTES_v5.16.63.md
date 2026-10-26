# v5.16.63

Faster Drive saves and cross-device updates.

- Export eligible pen overlays as compact PDF vector paths in Rust/WASM inside the processing worker. Preserve every input sample, width and color, editable metadata and removable overlays. Dense or tightly cropped ink, images, highlighters and area erasers retain the existing lossless raster path; unavailable or older WASM also falls back safely.
- Send existing PDFs up to 4 MiB in one conditional media request, eliminating the resumable-session handshake. Larger PDFs retain resumable chunks. Require the exact document's completed server receipt before acknowledging a save.
- Bind uploads to the revision actually checked and merged through Drive v2 ETags. A concurrent edit rejects the upload and queues a fresh merge; it never silently overwrites the newer remote copy. Discard stale upload sessions on conflicts and never reuse a session across write revisions.
- Remove the extra remote-save waiting loop. Notify connected peers and tabs after a confirmed write so they can immediately check Drive; hints are throttled, scoped to the open document, and never replace authenticated revision checks or content merging. Periodic Drive checks remain the fallback.
- Preserve the writing pause, immediate processing/upload interruption and durable Home checkpoint. Propagate worker cancellation through export steps instead of starting expensive compatibility fallbacks while the pen is active.
- Regenerate web/offline and Android bundled assets; frozen Android installer unchanged.

Validation: Rust native and real WASM tests; PDF pixel/color/position and removable-overlay checks; sparse/dense export measurements; conditional upload, receipt, concurrent-edit and cancellation tests; Home/offline background sync and real pen-priority regressions. Network and Drive response times still determine cloud latency; saving resumes after the existing 1.2-second writing pause. No claim of instantaneous cloud durability or direct peer connectivity on networks that block it.

Controlled warm-cache browser measurement, 30 pen strokes distributed over one page (1,800 samples): PDF preparation took 28–30 ms with vectors versus 72–76 ms with raster overlays, with a slightly smaller PDF. This measures preparation only, not Drive/network latency or tablet hardware. Dense 48,000-sample pages keep the raster/cache path.
