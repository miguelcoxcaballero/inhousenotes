# v5.16.62

Rust document scheduler; fewer background wakeups.

- Rewrite eligibility, quiet-period deadlines, session rejection, FIFO admission and bounded resume planning in Rust/WASM. The host bridge only supplies interaction state, resolves promises, handles browser cancellation and operates one shared alarm.
- Replace each document task's 12–24 ms polling loop with the shared queue. Wake on pen/gesture completion, interruption, document changes and visibility/exit transitions; keep a single bounded fallback check for mutable local-save abort flags and native input state.
- Resume up to eight paused tasks per batch and recheck input before the next batch. Preserve the 1.2-second writing pause, immediate active-worker/upload interruption, Android background sync cancellation and durable Home checkpoint.
- Keep a matching fallback for unavailable or older WASM; move queued jobs to Rust when loading completes. Bounded native queue overflow also preserves every waiter in the fallback.
- Regenerate Rust, web/offline and Android assets; frozen installer unchanged.

Validation: Rust native tests; actual WASM/fallback equivalence, cancellation, stale sessions, memory growth, capacity overflow and delayed initialization; browser writing, shared-wakeup measurement, checkpoint/reopen, Home navigation, background sync and worker lifetime tests. With 64 paused jobs, the controlled 1.2-second test needs at most 14 timer callbacks instead of roughly 3,200 independent 24 ms polls. This measures scheduling overhead, not physical tablet input-to-display latency.

Persistence, DOM/Canvas, Worker termination and network APIs remain host operations; document contents and stroke geometry are unchanged.
