# W4.2 SELECTION — notes for the other units

Branch `v6-w4_2` (from `v6-int` at `58039fc`). Owns `rust/crates/ihn-app/src/editor/selection.rs`,
`tests/v6/e2e/04c-select.spec.mjs`, `tests/v6/parity/states/selection.mjs`.

## What exists now

- `selection::Lasso`: the `input::ToolHandler` of the lasso. Pointer events append the dispatched position to a
  buffer reserved at boot (5.18.0 pushes one point per event; the release adds none), and the dashed path is drawn
  once per frame (`render::overlay::lasso_path`). At the lift, more than two points → `Doc::lasso` (images whose
  rotated rectangle meets the polygon, strokes with a point inside, draw order); nothing found clears the selection.
- Selection box (5.18.0 `updateSelectionBox`): classes `hidden` / `has-selection` / `has-image`, `left/top/width/height`
  from the bounds, the actions above or below, outlines on the lasso overlay. The children of the box shown are
  queried once per box node; one `pointerdown` listener on `#canvas-container` handles them (swatch, colour button,
  duplicate, trash, rotate handle, 8 scale handles, the box body = move) with `preventDefault + stopPropagation`.
- Transforms: a drag adds `document` pointermove/up/cancel listeners for its duration only. Pointer moves keep the
  newest position; once per frame the selected items are transformed from the model (`geom::Affine`, `jsmath`
  sin/cos/atan2/hypot) into a preview, the damaged rectangle is repainted with the preview in place of the originals
  (`render::damage`; whole page when images move or the canvas has no complete paint), and the box and outlines
  follow. **The model is not touched until the lift:** then `Tx::transform` = one batch, one undo step (nothing is
  written when the preview equals the model).
- Cross-page drag (move mode, centre beyond −50 px or height + 50 px): one batch at that moment — the transformed
  bodies shifted by the crossed page's real height, `ItemPut` to the target page with `Z::TOP` (drawn over everything
  there, e.g. a newer highlighter), one undo step (undo restores the original page, z and body). The drag continues on
  the target page. If the target has no canvases yet, `pages::schedule_visible` is asked and the next move retries.
- Duplicate (+18/+18 clamped to the real page with a 6 px margin, copies become the selection, tool → lasso),
  recolour (palette colours only, strokes only, selection kept), delete (status `Selection deleted` / `Item
  deleted`), each one batch + one step.
- **API for other units:** `selection::clear(app, host)` (v5 `clearSelection`), `selection::select(app, host, page,
  &ids)` (W4.3: select an inserted image, then `input::set_tool(Lasso)`), `selection::selected(app)`.
  `app.st.editor_pages.selection_page` is kept in step (the selection page keeps its canvases first).
- The selection is cleared when a selected item is changed or removed by anything else (undo/redo, another device:
  `session::sig::STATE` / `CHANGED` compare the body stamps) and when the session closes.
- Test API: `editor.selection.state` → `{page, strokes, images, bounds, transforming, lasso, commits}`.

## Files outside the owned list

- `editor/input.rs`: the lasso arm in `handler`; `selection::clear` at every non-lasso drawing pointerdown and on tool
  clicks other than the lasso (as W3.5's notes ask). **Merge with W4.1:** both change the `handler` match; keep
  `Tool::EraserStroke | Tool::EraserArea => Some(&eraser::Eraser)` and `Tool::Lasso => Some(&super::selection::Lasso)`.
- `editor/pages.rs`: `with_view` made `pub` (the same one-line change as W4.1).

## Verified

- `cargo test -p ihn-app` green (123; 5 selection tests: lasso hit/miss, move/scale/rotate one batch each and undo,
  preview without model change until the lift, duplicate clamped on an A5 page + recolour + delete, cross-page drag
  on A5 pages with the moved stroke over a newer highlighter, undo back to page 1).
- e2e `04c-select` (v6 fixture container, mouse with Draw with finger): lasso → box style `left: 100px; top: 100px;
  width: 200px; height: 160px`, move previewed then one commit, corner scale, rotate, recolour via the menu,
  duplicate (+18), delete, image in the lasso (`has-image`), cross-page drag at 50 % zoom onto page 2 with the pen
  pixel identical over and beside a newer highlighter, undo back to page 1. `04-ink`, `00-boot`, `internal-canvas`,
  `internal-editor-view` still pass.
- `npm run build` (report-only): ihn.wasm 1,026,051 bytes raw / 414,365 gzip.
- Parity `--self --quick --states selection`: 8 states (11 captures) run and match in 5.18.0.

## Deviations

1. **Parity B blocked on W3.4** (as W3.2/W3.5/W4.1): the states open the 5.x fixture `r0`; rerun
   `--quick --states selection` once the 5.x import is merged. Desktop/tablet only (the lasso button is below the
   window on phones in 5.18.0). No non-A4 state yet (B-E1/B-E2 are covered by cargo tests on A5 pages); `r1`'s A5
   page 2 is the candidate scene.
2. **B-E2 read as "the crossed page's real height"** (not height + gap): 5.18.0 shifts by 1123 and A4 must stay
   pixel-identical, so the gap is not added; 5.18.0's jump by the gap on the next move (start position re-based by
   ±height while the pointer is measured on the new page) is kept for the same reason.
3. A cross-page drag is one batch: the transform made before the crossing and the move are undone together
   (5.18.0 records only the move and undo returns the items to their transformed place beyond the old page's edge).
4. After an undo/redo touching a selected item the selection is cleared (5.18.0 clears it for every selection
   action except a transform, whose undo leaves a stale box).
5. Lasso path drawn per frame and transforms applied per frame (5.18.0: path per event, transform per frame); the
   paused states are identical.
6. 5.18.0 leaves the old page's outlines on its lasso overlay after a cross-page drag; kept (visible in parity).
