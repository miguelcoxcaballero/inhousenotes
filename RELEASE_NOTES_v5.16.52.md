# v5.16.52

Accelerate long-stroke eraser queries with Rust.

- Move repeated polyline/point hit testing for strokes of 256+ points to Rust/WASM. Keep existing bounds filtering and the JavaScript path for short strokes or unavailable WASM.
- Retain unchanged coordinates in a generation-scoped LRU bounded to 64 entries / 4 MiB. Any document mutation/session change invalidates geometry before reuse. No point simplification or changes to stored ink.
- Keep input, rendering and persistence in their existing hosts. Regenerate Rust, web/offline/Android assets; frozen installer unchanged.

Validation: five native Rust tests; 600 randomized geometry comparisons, coordinate invalidation, eviction, JS fallback and integrated editor erase. A controlled Chromium run of 1,200 mixed hit/miss queries on 3,000 points measured 119.9 ms JS vs 19 ms Rust, plus 1.5 ms initial preparation. Reproduce with tests/e2e/editor-rust-hit.spec.mjs. This is a kernel/adapter benchmark, not an editor frame-rate guarantee.
