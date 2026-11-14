# W4.1 ERASERS — notes for the other units

Branch `v6-w4_1` (from `v6-int` at `58039fc`). Owns `rust/crates/ihn-app/src/editor/eraser.rs`,
`tests/v6/e2e/04b-erase.spec.mjs`, `tests/v6/parity/states/erasers.mjs`.

## What exists now

- `editor::eraser::Eraser` — the `input::ToolHandler` of `eraser-stroke` and `eraser-area` (arm added in
  `input::handler`). Pointer events only append positions (5.18.0 `collectEraserPositions`: spacing
  `max(0.8, 0.18·eraserWidth)`, the event's last sample always kept) to a buffer reserved at boot and request a
  frame; the pointerup's samples are ignored, as in 5.18.0.
- Per frame: each position is tested as the segment swept from the previous one (B-E6; `geom::Sweep`,
  `polyline_hit`, `split_ranges`), the result is a pending *cut* (item → surviving point ranges of the original;
  none for the stroke eraser), the damaged rectangle (`render::damage`, v5 `recordEraserDamage` padding) is repainted
  from the model minus the cut even while the pen is down (a canvas without a complete paint is repainted whole), and
  the eraser cursor is drawn on the cursor overlay. The model is not touched until the lift.
- At lift: one `session::edit` batch (`Tx::erase` / `Tx::split_ranges`, fragments at the original z: B-E4), one
  history step (`history::record`; points = fragment points), the erased items' `Path2D`s dropped, the cursor
  cleared. A stroke another device changed during the gesture (body stamp differs) is left alone and the page is
  repainted. Read mode mid-gesture (`discard`) or a deleted page abandons the cut and repaints the page from the model.
- **W4.5 (collab):** `eraser::set_live_sink(app, true)` then `eraser::drain_live(app)` → `LivePreview::Erase
  {gesture, page, hidden, fragments}` (the complete current set, at most every 50 ms; fragment keys are
  `cut index << 32 | k`) and `EraseEnd` before the batch (also on abandon).
- Test API: `editor.eraser.state` → `{active, cut, frames, repaints, commits, stroke, width}`;
  `editor.eraser.mode` (`"area"` / `"stroke"`) sets `input.eraser_stroke` and selects the eraser (what W4.4's
  `#eraser-stroke-toggle` does; used by the e2e spec because the menu is not on `v6-int` yet).

## Verified

- `cargo test -p ihn-app` green (121; 3 eraser tests: swept stroke erase with the model unchanged until the lift,
  repaint and cursor before it, undo/redo at the original z; area erase into fragments below a newer highlighter,
  undo restoring the stroke there, a gesture cutting its own fragment again = one batch/one step; Read mode
  mid-gesture discards, live packets `Erase` … `EraseEnd`).
- e2e `04b-erase.spec.mjs` (CDP stylus on the v6 fixture container): stroke eraser paused mid-gesture → cut 1,
  the bitmap at the erased line is back to the template, cursor pixels on the overlay, model unchanged; lift → one
  item fewer, cursor cleared; undo restores the pixels; a single 120 px sample jump across a line erases it (B-E6);
  area eraser through a pen line under a highlighter → +1 item (two fragments), undo → the pixel equals the one
  before the erase (pen under the highlighter, B-E4). `04-ink`, `00-boot`, `internal-*` still pass.
- `npm run build` (report-only): 990,499 bytes raw / 400,760 gzip.
- Parity `--self --quick --states erasers`: all 4 states (6 captures) run and match in 5.18.0.

## Deviations

1. **Parity B blocked on W3.4** (as W3.2/W3.5/W4.4): the states open the 5.x fixture `r0`; v6 ends on Home, so B fails
   at the mode toggle ("Element is not visible"). `erasers/area*` also need W4.4's eraser menu on B. Rerun
   `--quick --states erasers` after W3.4 and W4.4 merge. `erasers/area-undo` is tagged B-E4 and
   `erasers/fast-swipe` B-E6 (expected region: the pen curve's band). Desktop/tablet only, like `toolbar/eraser-menu`.
2. **Files outside the owned list:** `editor/pages.rs` — `with_view` made `pub` (the damage repaint needs the page
   view with the cut applied); `editor/input.rs` — the eraser arm in `handler` (as W3.5's notes ask) and its doc line.
3. The cursor and the hit tests run once per animation frame instead of per pointer event (5.18.0 draws the cursor
   per event and repaints per frame); the frame shows the same state, pointer events stay allocation-free.
4. Positions skipped by 5.18.0's spacing filter are still covered by the sweep between kept positions up to
   `0.18·width` (< the radius); the spacing filter is kept so paused states match 5.18.0's positions.
5. After the lift the page row is not marked `needsRedraw` (5.18.0 marks it so a later full repaint refreshes the
   cache): the damage repaint already equals a full repaint, so the extra repaint is skipped unless something else
   asked for one during the gesture.
6. Read mode switched on mid-gesture restores the erased strokes on screen (5.18.0 leaves them removed locally
   without saving or undo, a display/model mismatch).
