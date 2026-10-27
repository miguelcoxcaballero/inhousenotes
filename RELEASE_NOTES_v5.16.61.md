# v5.16.61

Pause document work while writing.

- Stop active PDF/checkpoint workers, PDF preparation and foreground/background document requests on an accepted writing contact. Retain dirty state for retry; do not interrupt on pen hover.
- Defer journal writes, page compression and upload chunks until 1.2 seconds after writing stops. A held pen keeps the pause active. Drain the journal once per burst instead of creating a waiter for each letter.
- Hidden sync editors respect foreground input. Android WorkManager sync is cancelled once per writing burst and rescheduled after idle. A pending exit upload no longer bypasses input priority when the editor is reopened.
- Home stops PDF preparation before committing the local checkpoint, so a paused export cannot hold its page-store lock against saving on exit. Hidden-page lifecycle saves may run without waiting for stale gesture timers.
- Regenerate web/offline and Android assets; frozen installer unchanged.

Validation: browser pen contacts interrupt a blocked worker and active upload, workers restart after release, no page/journal writes during a six-letter burst at 4x CPU throttling, automatic journal resume, reopen recovery, Home checkpoint and background sync regression coverage.

New ink remains in memory during the pause and becomes durable after the idle checkpoint or successful exit checkpoint. An already-issued atomic storage transaction may finish; cancelling a request cannot retract bytes already accepted by Drive. No physical-tablet FPS or hard real-time preemption guarantee.
