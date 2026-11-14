# W3.3 PDF-BACKGROUNDS — notes for the other units

Branch `v6-w3_3` (from `v6-int` at `58039fc`). Owns `rust/crates/ihn-app/src/doc/backgrounds.rs`; adds
`tests/v6/parity/states/pdf-backgrounds.mjs`.

## What exists now

- `doc::backgrounds` — 5.18.0's PDF background pipeline on top of `host/pdf.js` (W3.1) and `editor::pages` (W3.2):
  - `scale_for(pages, size)` = `getPdfBackgroundScale` (500+ pages 1.2, 200+ 1.5, >80 MB 0.9, >30 MB 1.25,
    >10 MB or 30+ pages 1.6, else 3); fixed per source when it is opened, from the document's page count and
    `Session::export_size` (5.18.0 used the Drive file's `numPages` and size); rendered at `max(0.6, scale)`.
  - `apply(app, host, page)` — called by `pages::apply_background` for every PDF page: a cached raster is touched
    (and handed back if the editor dropped it, e.g. after `pages::rebuild`), else the page is queued.
  - Queue: only active pages, one render in flight, the next after an animation frame; nothing while
    `viewport::interacting` (resume via `requestIdleCallback`, timeout 600); a result that lands within 240 ms of
    input is discarded and the page requeued at the front (5.18.0 `requeuePdfPageRenderAfterInteraction`).
  - LRU: at most 6 rasters; the least recently used non-active ones are dropped (URL revoked, blob released,
    `pages::set_pdf_raster(None)`).
  - Session `OPENED` (another document) / `CLOSED` → everything released, sources closed (`pdf::close`), in-flight
    answers released when they arrive.
  - **For W5.4 / W4.5:** a source asset the session lacks is listed by `take_missing(app)` (rebuild it from the
    latest export, ARCHITECTURE §4.5); after `session::put_asset` call `asset_arrived(app, host, hash)`.
  - Test command `doc.backgrounds.state` → `{cache, queue, rendering, sources:[{open,failed,scale}], missing, renders}`.
- `editor/pages.rs` (W3.2's, 2 small edits): the hook in `apply_background`, and `pub fn has_pdf_raster`.

## Verified

- `cargo test --workspace` green (ihn-app 123 incl. 5 new: scale policy, one page per frame + input deferral and
  idle resume, LRU sparing active pages, close releases, missing source listed).
- On `v6-w3_3` + `v6-w3_4` merged locally (not committed): e2e `00-boot`, `12-open-v5`, `internal-editor-view`
  12/12 pass. Parity `--quick --states pdf-backgrounds` (10 comparisons): zoom 1/2, pan during/after on desktop and
  phone pixel-equal (DOM/trace warnings only). Two cells differ, neither in this module:
  - `pan@during` phone: 3 px Δ7 at the share button's edge — the known flake (NOTES-W3.4); a rerun passed.
  - `agenda-diary@scrolled` phone: 1 px Δ1 inside an agenda/diary template line (185 vs 184 grey), stable —
    template raster (W2.3/W3.2); recorded for W6.1.

## Deviations

1. Parity needs W3.4 (5.x fixtures); measured on a local merge of both branches.
2. The parity state file is new (the plan lists no state file for this unit).
3. No memory-pressure eviction (5.18.0 `checkMemoryPressure` reads non-standard `performance.memory`); the LRU 6
   bound is kept.
4. No render cancellation: `host/pdf.js` has no cancel op; a render overtaken by input finishes and is discarded
   (same pixels as 5.18.0's cancelled render task).
5. The source rebuild request is a seam (`take_missing`/`asset_arrived`), the worker role is W5.4's.
6. Commit trailer names the model given by the workflow.
