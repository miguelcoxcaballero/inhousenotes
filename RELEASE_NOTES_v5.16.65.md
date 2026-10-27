# v5.16.65

Faster local saves; bounded Drive checks.

- Reduce the post-input save quiet period from 1.2 seconds to 180 ms. A held pen or gesture still blocks processing; new input still interrupts heavy work. Start prepared PDF and upload scheduling without the extra quarter-second delay.
- Reuse immutable packed offline page snapshots within an editor session and mutation generation, with an LRU page/estimated-memory bound. Changed ink, metadata, page ownership and sessions invalidate reuse; persisted snapshots still include every page.
- Invalidate per-page PDF metadata and overlay caches when appending a pen operation to an existing stroke array, so subsequent saves cannot reuse the previous page's ink.
- Include token acquisition in Drive request deadlines and cancellation. A stalled authorization no longer sits outside the timeout while the UI says Checking Drive.
- Accept verified identical PDF heads across metadata-only version advances while preserving newer metadata. Continue rejecting genuinely older PDF heads.
- Regenerate web/offline and Android assets. Frozen installer unchanged.

Validation: real pen input followed by IndexedDB journal inspection measured roughly 187–192 ms after release, versus 1,161 ms before; deadline/late-request, unchanged-page reuse, mutation invalidation and post-save ink tests; gesture/pen priority, offline sync, Home checkpoint and save-truth regressions. These are local-save measurements, not a promise of zero network latency. The separate stroke-by-stroke Drive transport is being added in the following feature release.
