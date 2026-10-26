# v5.17.0

Live ink saves through Drive.

- Finished strokes are durably queued and sent as small causal edits, independently of PDF generation. Existing WebRTC continues to stream live ink; Drive is the fallback without adding another server.
- Reuses Inhouse Learn's direct-file-ID live transport pattern: an `inhouse notes` Drive folder (or the existing `Inhouse Notes` folder), per-writer journals, serialized writes with a 600 ms minimum interval and hot reads every 450 ms plus network time. Folder and document IDs are account-scoped. These intervals are scheduling targets, not a promise of Google Docs latency or availability on every network.
- PDF metadata and P2P advertise journal IDs, so peers can read them without waiting for Drive search indexing. A stalled index request cannot block known-file reads. Quiet writers back off polling.
- Offline edits survive process restart and the home screen drains the durable outbox without requiring the PDF to be opened. Reserved file IDs avoid duplicate creates after lost responses. Conditional writes reject stale revisions. Receipts acknowledge only the revision actually sent, never newer edits.
- Incoming strokes, edits and erasures use the existing causal merge and local durable checkpoint. Page tombstones prevent historical packets from recreating deleted pages. Journals are retained alongside the PDF rather than replacing the last recoverable copy.
- Save details distinguish confirmed stroke changes from a PDF still updating. The PDF's saved flag is never advanced by a journal receipt.
- Includes v5.16.65's 180 ms local-save idle window, reuse of unchanged page checkpoints, invalidation after new ink and bounded Drive authentication checks.

Validation: transport tests cover two writers, chunk rotation, delayed indexing, lost responses, reload recovery, stale conditional writes, document identity and failed merge acknowledgement; browser tests exercise real editor merges, erasures, durable home recovery and pen-priority delivery. Drive calls are simulated in these tests; production multi-device latency still depends on Google Drive and the connection.

Android uses the refreshed web assets. Native installer and A/B APK catalogs are unchanged.
