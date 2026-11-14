# W5.3 TIMELINE — notes for the other units

Branch `v6-w5_3` (from `v6-w4_5`, with `v6-w5_1` merged in for `panels::slider`). Owns
`rust/crates/ihn-app/src/{panels/timeline.rs,doc/versions.rs}`, `tests/v6/e2e/16-timeline.spec.mjs`,
`tests/v6/parity/states/timeline.mjs`.

## What exists now

- `doc::versions` (capture trigger, ARCHITECTURE §4.6):
  - Every local commit above the seq the last capture covered marks the document dirty (`session::sig::COMMITTED`,
    compared with this tab's seq in `doc.vv`). A capture that starts a new entry runs after 180 ms of quiet input
    (5.18.0 `DRIVE_BUILD_IDLE_WINDOW_MS`); one that would coalesce into this tab's newest entry runs at most every
    290 s (`THROTTLE_MS`, SYNC §14.6 budget). Pending edits are captured at once on `VIEW` ≠ editor, `pagehide`,
    hidden, Timeline open and before a restore. An automatic capture whose pages equal the newest entry's (a
    Drive-side rename applied by `collab`, an edit undone) only marks the edits covered.
  - `capture(app, host, hint, ts)` = 5.18.0 `captureVersionSnapshot` (`history::plan_capture` → SHA-256 in Rust →
    `blob_new` + `session::put_asset` → `Planned::commit` inside `session::edit`). Author = the Drive profile (name
    fallback e-mail | `You`), device = this tab's actor (hex).
  - Snapshot bytes are cached per document; others are read from their blobs (`blob_read`) — at open the chains the
    next capture needs, everything when the Timeline opens (`load`). `snapshot(app, hash)` materialises.
  - `restore(app, host, target, when, hint)` = `pre-restore` milestone (`Before restoring <when>`; failure →
    `Restore cancelled: a safe recovery point could not be created.`), `plan_restore` as one batch, undo cleared,
    pages refreshed, view centred on the top, then the `restore` / `revert-original` milestone.
  - **`add_v5(&mut Import)`** (lead's request: `Import::versions`): the imported 5.x timeline becomes one more batch
    of the import actor (`VersionAdd` per entry, ids `Id::legacy(5.x id)`, so every device importing the same file
    writes the same versions) plus the snapshot assets (delta chains, checkpoint every 8). Items keep the z the
    import gave them. `doc::v5open` calls it in both paths (main instance and the export worker).
    **W5.6 (migration): call `versions::add_v5(&mut imp)` after `v5::import::from_local` too.**
  - Test commands `doc.versions.state` (entries oldest first with kind/milestone/summary/author/device/depth/delta,
    `cached`, `stored`; counters) and `doc.versions.capture {ts?, milestone?, kind?, summary?}`.
- `panels::timeline`: `#btn-open-timeline` (in the Pages panel), `#btn-close-timeline`, the list (5.18.0 markup:
  months → 1-hour clusters, newest cluster expanded, explicit collapse choices kept per session, `Current`,
  `Restored`, `Reverted`, `Original` tags, the Original entry last), keyboard Enter/Space, preview (white page,
  template / photo background of the live page at the same index / the pdf.js raster of a live page showing the same
  PDF page, images, strokes, diff highlights against the previous display entry), Ctrl+wheel and pinch zoom
  (0.2–3), Revert through `slider::show` with 5.18.0's titles, messages and statuses, `Loading timeline…` while
  snapshots are read, rebuilt when entries change while open. `panels.timeline.state`.
- Small edits outside the owned files: `ihn-core::doc::Tx::doc()` (accessor), `doc::session::referenced_assets`
  includes `history::needed_assets` (version snapshots are loaded at open, kept by asset GC and listed as missing
  for peers), `editor::pages::{image_of, pdf_raster_of}` (accessors), two `add_v5` calls in `doc::v5open`, and
  `tests/v6/e2e/internal-doc.spec.mjs` (an edit now journals two batches: the stroke and its timeline capture).

## Verified

- `cargo test --workspace` green (ihn-app +2 tests: imported timeline → versions with delta chains; civil days).
- e2e `16-timeline` (3 tests): coalescing over two 5-minute windows (same id within 5 min, summary against the
  replaced entry as 5.18.0, a new delta entry after 6 min, identical content skipped, milestone always captured,
  pagehide captures pending edits); the imported `versions-30` history (31 entries, 10 milestones, chains), list,
  Original, current selected, preview highlights, zoom, collapse, restore through the slider (pre-restore + restore
  milestones, 11 strokes), revert to Original; a second device sees the same entries and the current tag.
- Also green: e2e 04-ink, 05-pages, 07-share, 11-two-devices (a)–(e), 12-open-v5, internal-doc.
- Parity `--quick --states timeline` (A = 5.18.0, desktop + phone): all 10 comparisons pixel-equal (`list`,
  `older` with its highlight, `collapsed` cluster and month, `original`, `restore-slider`); DOM/trace warnings only,
  none inside `#timeline-panel`. One run hit the known 3-px share-button flake on phone (`list`); its rerun passed.

## Deviations

1. **Snapshot assets do not travel yet**: assets are host blobs and neither P2P (`AssetWant`) nor relay asset files
   carry them (NOTES-W2.7 dev. 2, NOTES-W2.8 dev. 3). Entries (`VersionAdd`) travel with the operations, so every
   device lists the same timeline, but another device's snapshot can be previewed and restored only once its asset
   arrives (W5.4's export attachments, §14.6). Until then the preview shows `Select a version…` and Revert stays
   disabled for that entry.
2. **Throttle**: a coalescing capture runs at most every 290 s, so the newest entry's time can trail the last edit by
   up to that (5.18.0 rewrote it at every Drive save). Opening the Timeline, leaving and restoring capture at once.
3. **PDF page previews** use the pdf.js raster of a live page that shows the same PDF page (`doc::backgrounds`, LRU
   6); without one the page stays white under the ink (5.18.0 rendered the page with pdf.js into the preview).
4. **Avatar `onerror` fallback** is not ported (an `error` listener per image); entries without a photo use the
   fallback avatar as 5.18.0.
5. **Original entry of PDF documents**: the source PDF's page count is the highest page any live page shows, sizes
   from those pages (5.18.0 kept `cleanOriginalPageSizes` from the clean PDF).
6. `v6-w5_1` was merged into this branch for `panels::slider` (W5.1 is not on `v6-int` yet); the merge is clean.
7. Commit trailer names the model given by the workflow.
