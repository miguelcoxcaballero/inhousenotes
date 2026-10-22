# v5.16.45

Rust binary ink checkpoints reduce save stalls.

- Replace object-per-point page and offline-outbox checkpoints with lossless binary ink blocks. Rust/WASM encodes and decodes IEEE-754 coordinate and pressure bits in a dedicated worker, separate from PDF export.
- Gather and restore points in yielding batches; retain the writing-idle gate and immutable checkpoint/session checks. Keep browser input, canvas drawing and IndexedDB transaction ownership in JavaScript.
- Preserve stroke IDs, colours, widths, pressure presence, signed zero, legacy cover strokes and unusual point metadata. Existing object checkpoints remain readable. Raw binary and cooperative JavaScript decoding cover unavailable WASM/workers and cold offline startup.
- Regenerate Rust, Android and offline assets. The frozen installer is unchanged.

A 300,000-point page at 4x Chromium CPU throttling saved in 621 ms versus 1,243 ms with object checkpoints. The synchronous IndexedDB put fell from 553 ms to 37 ms, with all points recovered. Reproduce with scripts/benchmark-ink-checkpoint.mjs and the local test server; these are controlled desktop measurements, not a device frame-rate guarantee.
