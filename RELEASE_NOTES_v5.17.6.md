# v5.17.6

Responsive peer reads; fewer durable merges.

- Journal and live-preview readers now keep up to four independent requests active. A stalled device cannot hold successive frames/edits from responsive devices. Per-source ordering, serialized document merges and cancellation remain enforced.
- Consecutive strokes use bounded batches (32 strokes / 8192 points) through the same durable merge path. The 65-stroke regression now requires three document merges rather than 65, with identical persisted points.
- Repeated stroke IDs keep revision ordering; deletion tombstones remain authoritative. A single oversized stroke keeps its existing handling.
- An in-flight page capture now prevents a premature 'Strokes saved' receipt. Completing capture refreshes the status.
- Tests cover stalled sources, concurrency limits, leaving during pending merges, actual persistence, revision conflicts, deletions, point budgets and receipt timing.

Updated web and Android web assets; no APK or frozen-installer changes. Real Drive latency remains network-dependent.
