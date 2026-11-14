# W6.1c PARITY-SWEEP (third round) — notes

Branch `v6-w6_1c` (from `v6-int` at `bc2e028`). Report: `tests/v6/parity/REPORT.md`, generated with
`node tests/v6/parity/report.mjs --summary <run dirs>…` (new CLI: one summary over the per-file runs).

## Environment

The first full e2e run had 58 failures, nearly all `Target crashed` / `ERR_INSUFFICIENT_RESOURCES`: the disk was full
(133 MB free). The cargo `rust/target` directories of the merged wave 1–5 worktrees in the scratchpad (regenerable
build caches, untouched for hours) were removed (~10 GB). Not a code problem.

## Fixes (each a real cause)

| Symptom | Cause | Fix |
|---|---|---|
| Title rename of a document in Drive never reached Drive | `editor/view.rs finish_title` tested `flags.created` inverted | PATCH when the document is in Drive; the pushed name is recorded (`export::upload::pushed_name`) so the poll does not take it for an outside rename and no export pushes it again. e2e 03 checks the PATCH |
| e2e 10 (and the product) — a stroke made right before closing did not reach the relay | SYNC §6 invariant 1 hides a batch from the engine until its IndexedDB commit; a commit landing after `pagehide` met a keepalive flush that had already been reset | `collab`: a batch committed while hidden / after `pagehide` re-arms the keepalive flush |
| e2e 06 asset count 4 ≠ 3 | the timeline capture stores its snapshot as an asset | the test counts image assets (session assets minus stored timeline snapshots) |
| e2e 08 export-to-user: no ink in the download | the test's stroke at page y = 700 was below the 800 px viewport (CDP drops it) | stroke at y = 600; 08/10 stroke helpers wait for a drawn page and for the ink to be counted |
| internal-doc selftest / reserved id | expectations older than W6.1 (a checkpoint is a local save → `All changes saved`; a new document's create starts at once online → `building`) | expectations follow the current (5.18.0-matching) behaviour |
| parity `open/failed`: status line ends on counts (A) vs the error (B) | under the harness every Drive answer lands at one fake ms; the line shows whichever flow needs fewer storage round trips. With a real network (traced) both apps end on `Could not open this document. Try again.` | the state holds Drive's answers for the document until Home's lists arrived (`hold`), the real order |
| phone-412 page-edge row (Δ1, full width) | 5.18.0's global capture `pointerdown`/`touchstart`/`wheel`/`keydown` (`interruptProcessingForUserInput`) is a gesture in the editor; its settle runs a visible-pages pass (`applyPagePreview`). v6 only recorded activity, so a tap on the phone's mode-toggle group ran no pass | `shell`: document capture `pointerdown` listener; these inputs call `viewport::note_interaction` while the editor is shown (save indicator excluded) |
| `canvas/dense-page@repaint` page-corner pixels (Δ2) | four differences in the open's page pipeline: (1) `renderAllPages({deferVisibleUpdate: true})` + the 80 ms pass of the open — v6 forced a pass; (2) the open's page list applied right after the first render rebuilt the rows with a forced pass; (3) `scheduleCanvasResolutionRetry` (no active canvas at resync → two-frame forced pass + resync) was missing; (4) `attachPageCanvases` clears the wrapper preview when the paint starts, v6 when it ends; (5) staged repaint sliced by count (160 strokes) instead of 5.18.0's 4 ms budget | `pages::rebuild_opened`, first render of an empty row list deferred, `viewport` retry frames (`schedule_visible_from_frame`: a pass asked from another frame callback runs in the next frame), `clear_preview` in `attach`, staged slices of 40-stroke chunks drawn at once (`host.flush`) until 4 ms are used; scroll-preview slices likewise until more than 3 ms (5.18.0's budgets; under the harness's frozen clock both apps then finish a slice in one go, as 5.18.0 always did there) |
| parity `open/loading`: save dot `paused` (B) vs `saved` (A) during a held open | the record of a Drive document is marked created only when the open's metadata arrives; `sync_status` reported `not_created` meanwhile | `session::sync_status`: nothing pending while `doc::open::busy` |
| phone icon edges, 2–3 px Δ ≤ 20 (share icon, new-folder icon), intermittent | a raster flake of Chromium at the clip edge of inline SVG icons whose stroke caps touch their own box: `--runs` showed 5.18.0 differing from itself (A run 1 ≠ run 2) as well as v6 | harness (`run.mjs` `#freshRaster`): `svg:not(:root) { overflow: visible }` in both apps before the capture; 2 runs × light/dark of `editor-view` + `canvas` on phone-360 then passed 72/72 (they flaked several times per run before) |

## Open (not fixed)

- **`home/folder-zoom-open@50`, Δ1 over the folder view (stable, both viewports)**: 5.18.0's `joinMotion` runs after
  `zoom.ready` and builds the folder view's fade from `zoom.currentTime` with `fade.startTime = zoom.startTime`; v6
  joins at its prepare timer with `now − click` as elapsed and a negative delay. Here: offset 0.04333 vs 0.045 and
  local time 299 vs 300 ms → opacity 0.6929 vs 0.6947 at the 50 % capture (25/75/100 % match). Matching it needs an
  "animation ready" completion from the host (an ABI addition in `host/dom.js` + `home/folders.rs`); not done in this
  round (a metric-only attempt fell back because the zoom has not started at v6's join; reverted).
- **`pages/photo-modal@added`, page-corner pixels**: 5.18.0's wrapper of the new photo page still shows the template,
  v6 already its scroll preview (pre-existing, REPORT-B item 3): order of the new page's preview and the last visible
  pass after the insert.

## Deviations

1. The full matrix (5 viewports × 2 themes, ~470 cells at ≈35 s) does not fit the time available; the quick matrix
   was run file by file (`REPORT.md` lists which files and matrices were run).
2. `tests/v6/parity/report.mjs` gained the `--summary` CLI so the final report is generated from per-file runs.
3. Files outside app code touched: `export/upload.rs` (`pushed_name`), `collab/mod.rs` (keepalive after pagehide).
