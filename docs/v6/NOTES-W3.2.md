# W3.2 EDITOR-VIEW — notes for the other units

Branch `v6-w3_2` (from `v6-int` at `185da5c`). Owns `rust/crates/ihn-app/src/editor/{view,layout,viewport,pages}.rs`,
`tests/v6/parity/states/{editor-view,canvas}.mjs`; adds `tests/v6/e2e/internal-editor-view.spec.mjs`.

## What exists now

- `editor::layout` — the page column (5.18.0 `getPageLayoutMetrics`): `Dim`, `Metrics`, `compute`, `metrics`,
  `content_height`, `page_top`, `page_center`, `dims`, `visible_range`, `viewport_visible`, `expand`, the binary
  searches. The page list is set by `editor::pages` (`layout::set_pages`) whenever the rows change.
- `editor::viewport` — zoom/pan state (`app.st.editor_viewport.{zoom, pan_x, pan_y, overscroll_y, pending_create}`),
  `#canvas-container` transform, clamp, wheel (Ctrl = zoom about the mouse), mouse pan, touch pan with axis lock and
  teleport filter, pinch, fling inertia (`jsmath::pow`/`hypot`), pull-to-add (rubber band, indicator, 600 ms snap
  back, 300 ms wheel release, `animatePanTo`), view save (`notebook-view-v1`, 250 ms), 90 ms resolution resync,
  the 100 ms gesture-idle settle (`isGestureInteracting`), `preserveViewOnResize`, window blur release,
  `align_top` / `center_on_top` / `go_to_page`, and the fast-scroll thumb's editor target (`scroll_metrics`,
  `scroll_to`, called by `home::scroll`).
  - `viewport::interacting(app, now)` is 5.18.0 `isViewportInteracting()`.
  - **For W3.5 (input):** call `viewport::claim(app, pointer_id)` from the page canvas' `pointerdown` when the
    pointer is taken for a drawing tool (it runs before the viewport's listener): the viewport then ignores that
    pointer. Set `app.st.editor_viewport.pen_writing` while the pen writes (5.18.0 `isPenWriting`): touches do not
    pan, repaints/staged slices/visible-page passes wait for the lift. The pen's background double tap is yours.
- `editor::pages` — page rows (`.page-row > .page-wrapper` + `.page-number`), at most three active pages with
  `canvas.page-canvas`, the three overlays and the selection box (5.18.0 markup), backing sizes
  (`target_scale`, `backing`, `overlay_scale`; device class from the UA), resolution sync keeping the last frame,
  activation (directly visible at once, the rest in idle callbacks / 90 ms timers), release with a page preview,
  scroll previews (serial queue, sliced), template CSS URLs, photo backgrounds, synchronous and staged repaints
  (generation guard, pen pauses), `add_page`, `changed` (remote/merge changes → rows or repaints).
  - **The canvas state of the app lives here:** `pages::with_gfx(app, |app, gfx| …)` gives the one
    `Cmds`, `Templates` and `PathCache` (NOTES-W2.3); call `pages::flush(app, host)` after encoding.
  - `pages::active(app, page_id) -> Option<&Active>`: canvas/overlay nodes, `Surf`s, backing sizes and scales.
    `pages::overlay_ink(app, host, page, which, has_ink)` = `setOverlayCanvasInk` (0 lasso, 1 cursor, 2 remote).
  - `pages::request_repaint(app, host, page)`; `pages::touch(app, page)` after a local edit (previews stale,
    a running staged repaint restarts); `pages::to_page(rect, w, h, x, y)` and `pages::page_rect` for the
    client → page mapping (measure once per contact).
  - **For W3.3 (PDF backgrounds):** `pages::set_pdf_raster(app, host, page, Some(blob_url) | None)`; until a
    raster is set a PDF page paints nothing under its ink and keeps its preview (5.18.0 `canClearPagePreview`).
  - **For W4.2 (selection):** set `app.st.editor_pages.selection_page` (activation priority, as 5.18.0).
- `editor::view` — `setReadOnlyMode` (`view::set_read_only`, `view::read_only`), toolbar/eraser-menu buttons
  disabled in Read, the toolbar flash, the viewer's toolbar (`display: none`) and tool buttons, the mode toggle,
  the title (`updateDocTitle`, editing with Enter/Escape/blur, the Drive rename `PATCH` and `Rename failed`),
  `#btn-drive-share` visibility. Broadcast `view::sig::MODE` (`read` | `edit`): the eraser menu, pages panel and
  selection close on `read`; the history module sets its buttons on `edit` (entering Edit disables undo/redo as
  an empty history does).
  - Open flow: `shell::sig::VIEW = editor` while `doc::open` or `doc::create` runs → empty column, zoom 1, Read,
    the file name as title, top aligned to A4; `session::sig::OPENED` → rows + `centerViewOnTop`; new documents
    switch to Edit; `CHANGED` (while the editor is visible) → `pages::changed`; `CLOSED` → empty column.

## Verified

- `cargo test --locked --workspace` green (ihn-app 106): backing sizes and overlay scales, layout metrics and
  searches, titles, the **gesture budget** (20 Ctrl+wheel steps and a 2-finger pinch: no canvas resize and ≤ 3
  layout reads each; the resync follows the settled gesture), clamps, Read mode without overscroll, pull-to-add
  creating a page with the 0.35 s pan animation.
- e2e `internal-editor-view.spec.mjs` (new): a v6 container opened through the shared link → 3 rows, active
  canvases 1588×2246 / lasso 1072×1516 / cursor 993×1404 at 100 %, `data-has-page-draw`, centred column
  (`translate(213px, 20px) scale(1)`), zoom 2 re-rasters at scale 4 after the gesture, zoom 0.5 clamps and keeps
  the visible pages active, Read → Edit, pull-to-add adds page 4. `00-boot`, `02-home`, `internal-doc` still pass.
- Parity `--quick --states editor-view,canvas`: see the deviations — every state opens a 5.x fixture, which v6
  cannot import before W3.4; on v6 those opens end on Home, so the states cannot be pixel-equal yet. The scripts
  run on 5.18.0 (A) and produce their captures; rerun them once W3.4 is merged.

## Deviations from the plan

1. **Parity not met yet (blocked on W3.4).** The plan's scene loader opens 5.x fixtures; v6 imports them in W3.4.
   The editor-view and canvas states are written against those fixtures and must be rerun after W3.4 merges.
2. **Files outside the owned list (small, documented):**
   - `doc/open.rs`: `opening_name()` accessor, and `show_view(Editor)` moved after the job is stored, so the editor
     can show the file's name as the title at the start of the open (W3.1's parity note: title `template` vs
     `Untitled`).
   - `doc/create.rs`: `pending_name()` accessor (title and Edit mode of a new document).
   - `home/scroll.rs`: the fast-scroll thumb's editor target (`#canvas-viewport`, 5.18.0
     `installDocumentFastScroller`), delegating to `editor::viewport`.
   - `host/dom.js`: a contenteditable event target reports its `textContent` as the event value (the title has no
     `value`; title editing needs its text on `blur`).
3. **No loading thumbnail** (`.loading-doc-preview` from the card's `thumbnailLink`): `doc::open::File` has no
   thumbnail link. Add it there and call `pages`/`view` when it is known.
4. **Saved view state at boot** (`applyViewState(pendingViewState)` when the editor is visible at boot) is not
   applied: every open resets to zoom 1 and centres the column afterwards, as 5.18.0's open does.
5. **Device class**: the touch-Mac test uses the viewport width (no `screen.width` in `Config`).
6. **Staged repaint / scroll-preview slices are counted** (160 strokes) instead of timed (4 ms / 3 ms), as W2.3
   prescribes; the final pixels are the same.
7. **Remote changes are taken only while the editor is visible** (`session::take_changes` on `CHANGED`), so other
   consumers and the session's own tests still see them otherwise; reopening rebuilds from the model anyway.
8. **Signal code collision found (not fixed, not mine):** `home::sig::OPEN_FILE/SYNC_ERROR/RETRY_SYNC` use
   `0x40..0x42`, the same codes as `session::sig::OPENED/CHANGED/COMMITTED`. Every module that follows the session
   signals also receives Home's (e.g. a card click arrives as `OPENED` with JSON data). The editor guards `OPENED`
   by comparing the data with the open session's id; the owners of `home::sig` should move to a free range.
9. **Size**: `npm run build` (report-only) gives 850,917 bytes raw / 339,861 gzip (`v6-int` at `185da5c`: 735,885 raw);
   the +115 KB is mostly `ihn-core::render`, linked into the app for the first time by this unit.
10. **Commit trailer** names the model given by the workflow.
11. **Parity run** (`--quick --states editor-view,canvas`, 26 cells): 0 pass. On B every scene open ends on Home
    (5.x fixture, W3.4 missing), so the B scripts either find no `#canvas-viewport` (the wheel helpers now fall back
    to the window centre) or no mode toggle; A (5.18.0) runs every script and captures. On phones the mode toggle
    (floating bar ≤ 720 px) fails Playwright's actionability check in 5.18.0 too, so the scripts click it with
    `force`. Rerun after W3.4.
12. `01-signin` "offline: NETWORK signal…" fails on this branch and on `v6-int` (`185da5c`) alike (Home status
    text); not caused by this unit.
