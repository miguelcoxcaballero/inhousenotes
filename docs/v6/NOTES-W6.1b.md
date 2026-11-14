# W6.1b PARITY-SWEEP (editor) — notes for integration

Branch `v6-w6_1b` (from `v6-int` at `776700d`). Sweep agent B: the editor states (`editor-view`, `canvas`,
`editor-tools`, `toolbar`, `erasers`, `selection`, `images`, `pages`, `pdf-backgrounds`) and the editor e2e specs.
The report is `tests/v6/parity/REPORT-B.md`.

## Code fixes

- **`doc::session` — batches only after their IDB commit (SYNC §6 invariant 1).** Own batches whose append is in
  flight (`Session::unstored`) are hidden from the sync engine: `Replica::version_vector` answers `doc.vv` without them
  (`wire_vv`, rebuilt at every `with_replica` and after `receive`/`load_snapshot`) and `batches_since` serves only
  what that vector holds. The commit (`appended`) clears them and `sig::COMMITTED` hands them to the engine as before.
  `session::sync_status` adds them to `unsealed_local` while an engine is attached (they are unsealed, the engine just
  does not see them yet); `collab.state.unsealed` uses it. Degraded storage is unchanged (no append, outbox at once).
- **`collab` — a batch committed after the Leave** (append still in flight when Home was opened) is added to the
  leaving engine's detached replica and handed to it (`COMMITTED` no longer ignored while leaving).
- **`collab` — remote live erase previews are drawn** (v5 `getRemoteLiveErasePreviewStrokes` /
  `applyRemoteLiveErasePreview`): `collab::erase_view` applies every remote eraser gesture of a page (hidden strokes
  removed, fragments inserted where the first replaced stroke was) inside `pages::with_view`, so repaints, damage
  repaints and thumbnails all see it; `paint_overlays` repaints the pages whose gesture changed, appeared or ended
  (`erase_sig`), `clear_overlays` restores them. `collab.state` gains `erasing`. e2e 11 (a) checks that A shows the
  erase before B lifts and that the preview is gone after.
- **`editor::eraser` / `editor::selection` — full repaint after the gesture.** 5.18.0's damage repaints end with
  `page.needsRedraw = true` ("final gesture rendering still refreshes the complete cache"), so the next visible-pages
  pass repaints the whole page; the clipped repaints' antialiasing differs from a full repaint by 1–2 levels. v6 kept
  the clipped result; now both end with `pages::request_repaint`.
- **`editor::pages` — `applyPagePreview` on every ordered page** (5.18.0 `updateVisiblePages`), also an active, painted
  one: the wrapper keeps the preview/template under the canvas (visible in the antialiased edge rows of a photo page).

## Harness (shared with sweep agent A)

- `determinism-init.js`: an animation that a timer starts at the end of a clock slice no longer receives the time of
  that whole slice (its phase depended on where the slice began, i.e. on unrelated timers of the page). This was the
  toolbar `toolbar-fade` restart after the snap tilt (`toolbar/snap-*`, Δ4 over the whole toolbar).
- `run.mjs judge`: expected regions are checked per differing pixel (was: the bounding box of all differences inside a
  single region), so several rules can be combined in one state (`support/editor.mjs` `rules()`).
- `run.mjs Driver.pen`: `press: false` continues a contact that a previous call left down (a second `mousePressed`
  during a press made 5.18.0 drop the selection transform's history entry: undo stayed disabled).
- `tests/v6/parity/support/editor.mjs`: `VERSION` (version label), `SAVE_DOT` (N-1: only the save dot may differ
  in states captured within ~6 s of an edit), `rules()`.

## Deviations

1. N-1 is handled with the `SAVE_DOT` expected region rather than advancing 6 s in every state: many states capture
   mid-gesture (`@live`, `@mid`, `@undo-enabled`), where waiting would change what is captured. BEHAVIOUR-CHANGES N-1
   lists the rule.
2. Files outside the editor/session owned by the lead's brief: `collab/mod.rs` (erase previews, COMMITTED while
   leaving, test command) and the parity harness (`determinism-init.js`, `run.mjs`, `compare.mjs`).
3. Remaining open differences (phone-412 page-edge row, phone share-button edge, photo-page edge rows, dense-page AA) and the partial full matrix are in `tests/v6/parity/REPORT-B.md`.
