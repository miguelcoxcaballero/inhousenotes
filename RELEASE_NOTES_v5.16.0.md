# v5.16.0

Reliable save, processing and peer transport engines.

Web only: the existing Android APK loads the new content-addressed resources;
no package, signing key, updater binary or A/B catalog is changed.

- Replace local and Drive save admission with session-scoped coalescing lanes.
  Requests arriving during a save wait for the next checkpoint. Preserve force,
  force-upload and lifecycle options while consolidating queued work.
- Use the same queue model for durable offline snapshots, replacing recursive
  capture draining and rejecting superseded account/document snapshots.
- Check document sessions after asynchronous upload stages and release the
  active writer in `finally`, including cancelled and failed attempts.
- Replace document-worker RPC bookkeeping with bounded serial execution,
  active-only deadlines, cancellation and generation-scoped PDF handles.
- Keep multiple source PDFs alive until explicitly disposed; validate PDF object
  ownership, reject stale handles, and release replaced cached sources.
- Extract and rewrite P2P framing with flow control before every frame, bounded
  send/receive queues, strict fragment validation, expiry and peer-close cleanup.
  Preserve compatible signalling, merge rules and wire-v1 interoperability.
- Replace retry-delay listeners with cancellation-aware cleanup; never execute
  an already-cancelled operation or retry an explicit permanent failure.
- Remove duplicated Drive queue flags, the old worker request map/timers, inline
  fragment assembly and implicit deletion of unrelated source PDFs. Retain old
  PDF/IndexedDB readers required to preserve existing documents.

See docs/DOCUMENT_PIPELINE.md for responsibilities and failure invariants.
Validation includes overlapping saves, worker crashes, saturation, malformed
transfers, two source PDFs, stale handles, exact PDF pixels/metadata, offline
conditional writes, concurrent merging and headless Android processing.

Checks: 190 unit tests and the release smoke checks passed. Targeted browser
regressions cover 43 distinct cases. A dense 800-stroke fixture exported in about
733 ms initially and 169 ms with unchanged work cached on the test machine;
this is not an Android benchmark or a before/after device-speed comparison.
