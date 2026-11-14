# W5.4 EXPORT-LIFECYCLE — notes for W5.5, W5.6 and integration

Branch `v6-w5_4` (from `v6-w4_5`). Owns `ihn-app/src/export/*`, `drive/revisions.rs`, the export role of
`host/export-worker.js`, `tests/v6/e2e/{08-save-reopen,09-offline,10-exit-upload}.spec.mjs`,
`tests/v6/parity/states/export.mjs`.

## What exists now

- `export::pdf`: `start(Purpose)` builds the open document's PDF (one build at a time, queued). Main side: template
  JPEG 0.85 at scale 2.5 (3.5 for a user export of a document not in Drive), every blob posted to the worker
  (`[1][1]`), then plan + snapshot (`[1][2]`). Worker Rust (`export::pdf::worker`) → one `pdfink` content stream per
  page (`[1][3]`, stays in the worker); worker JS assembles with pdf-lib (source pages copied unchanged, template /
  photo drawn on a white page, the ink as Form XObject `/IHN6Ink` behind `q /IHN6Ink Do Q`, original content wrapped
  in `q … Q`, images attached `IHN6-<hex>`, Info Title/Subject/Creator of 5.18.0, Keywords
  `STROKES_Z:!;IHN6:` + base64(zlib(snapshot))) → bytes back (`[1][4]`).
  Source rebuild (ARCHITECTURE §4.5): when a PDF-background source is missing (`backgrounds::take_missing`, or a
  build that found one missing) the latest export is downloaded, its snapshot read for the pages that carry the
  source, and the worker (`[1][5]` → `[1][6]`) copies those pages, drops the ink stream and `/IHN6Ink`, and fills the
  other page numbers with blank pages: page n of the blob is page n of the source, so **no remap table** is needed.
  Stored with `session::put_asset` under the same hash, then `backgrounds::asset_arrived`.
- `export::upload`: first create (multipart POST with the reserved id into the doc's parents, 409 = created) on
  `OPENED` / `COMMITTED` / online, backoff 2 s…5 min; then `flags.created`, `KnownExport`/`LastExport`,
  `collab::created`. Desk answers (`desk()`, called by `collab::after`, also for a leaving engine): `build` →
  `built`; `merge` → classify by Keywords: V6 = `load_snapshot` + attachments, Plain over an existing document =
  recovered sibling copy then `merged(empty)`; the recovery found at open (`v5open::take_recovery`) is written
  before any build/merge and its head counts as merged. Name push (§14.7) after each `desk.done`. `overlay()` shows a
  running create as Building/Uploading and a recovered copy as Preserving in `SyncStatus.export`.
- `export::user`: export modal (prefill = `DocName`, focus+select next frame, Enter/Escape/overlay), confirmed name
  sanitized and written as `DocName` unless Read mode; editor: forced export (`desk.force`) then the head download;
  viewer: head download; not created: local HQ build. Delivery: bridge v2 (`beginPdfExport` → `native_blob`
  240,000 chars → `finishPdfExport` → `handleInhousePdfExportResult` ≤ 30 s) / `navigator.share` / `<a download>`
  with 5.18.0's texts. Test command `export.user.export {name}`.
- `export::lifecycle`: `#btn-drive-home` click/Enter/Space (not on the title) → `Saving on this device…` while a batch
  is uncommitted (degraded storage: until sealed), then Home, `relay_pending` persisted when own batches are
  unsealed, the card badge updated (`home::doc_changed`), compaction. `pagehide`/hidden → compaction;
  `beforeunload` prompts only while a leaving engine exports.
- `export::headless`: runner over every account document with `relay_pending`/`export_pending`/not created: quiet
  session → `OPENED` (engine + create) → forced export → settle → close; `?offline-worker` reports
  `InhouseBackground.complete({busy,pending,errors})`; in the foreground it runs 3 s after the connection comes back
  while Home is shown (an open from Home stops it).
- Small seams outside the unit: `collab::after` calls `export::upload::{desk, overlay}` (3 lines),
  `collab::Run::leaving()`, `doc::v5open::worker_send` (shares the worker start flag).

## Verified

- e2e 08 (4 tests), 09, 10 pass; 00, 04, 07, 11, 12 still pass (21 tests). `cargo test --workspace` green.
- Parity `--quick --states export`: 4 comparisons, pixel-equal (DOM warnings only).
- Parity `--quick --states editor-tools` (rerun asked by the lead): still 4 pixel diffs; the save dot ones
  (`pen-lift`, `highlighter@committed`, `single-tap-dot@live`, desktop) are **N-1 timing**, not a missing upload:
  the states capture 400 ms after the lift, 5.18.0 has uploaded by then, v6's Alone relay flush runs 5 s after the
  last batch (SYNC §9.2). With `advance(6000)` `pen-lift` is pixel-equal (checked, not committed: the state file is
  W3.5's). `undo-redo@redone` phone is the floating-toolbar issue NOTES-W3.4 already describes.

## Deviations

1. **5.x revision merged during a session** (desk `merge` with Keywords `STROKES_`): answered `None` (the export
   backs off); the next open of the document imports it through `doc::v5open`. Reusing the importer outside the open
   path needed a refactor of W3.4's module.
2. **Relay PUT** stays W4.5's multipart PATCH for every size (no resumable upload above 4 MiB).
3. **No remap**: rebuilt sources are padded with blank pages instead of `assets.remap` (render path unchanged).
4. **Hidden 60 s canvas release** is not done here (the engine suspends its transports itself).
5. `10-exit-upload` fires the page's `pagehide` before closing it: requests of a closed page never reach
   Playwright's routes. `#btn-export-pdf` is clicked by `dispatchEvent` (it sits in the pages panel).
6. Version-snapshot attachments (§4.6) are not attached (W5.3's assets).
7. Found, not mine: `editor/view.rs` `finish_title` PATCHes the Drive name only when the document is **not**
   created (inverted test); the export's name push (§14.7) corrects the Drive name at the next export.
   Engine status shows `uploading` while offline with unsealed batches (relay write considered in flight).
8. pdf.js logs `XObject should be a stream` once when rendering a rebuilt/annotated source page (rendering is
   correct); not investigated further.
9. wasm 1,650,192 → 1,719,861 bytes raw (report-only gate); first-party JS 119 KB (over the 100 KB budget,
   report-only; export-worker.js +170 lines).
