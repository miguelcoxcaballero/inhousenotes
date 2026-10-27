# v5.16.59

Prioritize pen input over document jobs.

- Gate PDF worker dispatch and result consumption on an input-idle background turn, before structured cloning and before main-thread continuations. Use scheduler.postTask background priority where available; keep epoch/session/abort validation across waits.
- Preserve the separate durable ink-checkpoint queue and allow processor cleanup without waiting on pen activity.
- Pause resumable uploads before initiation, between chunks and before completion handling. Keep accepted bytes and in-flight network requests; resume the same session after interaction settles.
- Recheck input after PDF-building yields instead of proceeding immediately into the next processing slice.
- Regenerate web/offline and Android assets; frozen installer unchanged.

Validation: worker dispatch/result gates, independent checkpoint queue, exact upload ranges across a pause, abort during wait, lifecycle/reset ownership, existing upload retry/save-truth and editor/PDF tests. This is cooperative prioritization, not a physical-device latency guarantee; an already executing synchronous operation cannot be preempted by JavaScript.
