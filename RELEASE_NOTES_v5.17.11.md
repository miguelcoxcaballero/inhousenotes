# v5.17.11

Edit ink without resending its photo.

- Changes to an existing stroke's width, color or geometry use the same compact stroke operation already supported by older receivers when its page, photo, fields and document context are unchanged.
- New pages, recovery, new capture-only strokes, tombstones, changed images and metadata retain the full page payload. The optimization does not omit photo bytes from a winning background revision or introduce a new wire format.
- Stroke identity, causal stamps and point geometry freeze before asynchronous persistence. Superseding edits are recaptured, and save receipts only acknowledge the persisted/uploaded revision.
- Added regressions for each edited-stroke property on a 1 MiB photo, actual two-replica reception and IndexedDB persistence, full-payload fallbacks, and an edit during persistence.
- Validation: 27 browser regressions, native-asset checks and release smoke passed. The edited-stroke fixtures send 260–284 bytes; photo bytes, geometry and causal revisions are preserved. Payload size is measured independently of real-network latency.

Web and Android web assets updated; APK/frozen installer unchanged.
