# W7 — pen writing (pressure ink, side-button eraser, predicted tip)

Branch `v6-pen`, base `main` 488f6d7 (published 6.0.0). Not published. Publishing it is a new version (6.0.1 or
6.1.0) after the owner approves `BEHAVIOUR-CHANGES.md` §7. This is an original implementation; no third-party
or proprietary code was used.

## Why writing felt flat

`Body::Stroke.p` (the pen pressure of every point) was stored and synced, but `render::StrokeRef` had no pressure
field, so every pen stroke was drawn with one `lineWidth`. There was no prediction of the pen position either.

## Ink (`rust/crates/ihn-core/src/render/ink.rs`)

- Centreline: the same centripetal Catmull–Rom spline as before (`pen::segment`), sampled every `STEP` = 1.5 page
  px (at most 32 samples per segment).
- Width: radius = width / 2 × `factor(p̄)`, `factor(p) = MIN_FACTOR + SPAN · p^GAMMA` = 0.4 + 1.5 · p^0.8.
  p̄ is a running average of the pressure (`PRESSURE_SMOOTHING` = 0.35). The average is causal, so the live stroke
  and the committed stroke get the same radii and the ink does not change at lift. Radii are interpolated
  between points.
- A stroke counts as pressure-sensitive when its pressure varies by more than `PRESSURE_RANGE` = 0.04. Otherwise
  (mouse, touch, documents written without pressure) every radius is width / 2, as before.
- Outline: one closed "ribbon" per run (left edge forward, round end cap, right edge back, round start cap),
  filled with the nonzero rule. Every piece of a ribbon winds the same way, so self-crossings never leave holes.
  Turns sharper than about 100° (`SPLIT_COS`) split the ribbon, so the outer side gets a round join.
- Live drawing: finalized segments are filled on the page bitmap piece by piece (round caps overlap, so no seam);
  the newest segment and the predicted points are filled on the tip layer, which is cleared every frame.
- PDF export: the same outline as a filled path (`f`); arcs become cubic Béziers of at most 90°.
- New canvas ops: `FILL_PATH` (55, `ctx.fill(path2d)`), `CLOSE_PATH` (56). Cached `Path2D` per stroke as before.
  A cover stroke (5.x) is drawn wider than its item, so its outline is not cached.
- Tests comparing call traces with 5.18.0 recordings set `ink::LEGACY` (test builds only) so they still check
  everything else bit for bit; new tests cover the outline, the PDF and the page repaint.

Tuning: change the constants at the top of `ink.rs`. A larger `SPAN` gives more contrast; a smaller
`PRESSURE_SMOOTHING` gives smoother width changes with more lag.

## Side button (`rust/crates/ihn-app/src/editor/input.rs`)

- A pen contact whose `buttons` has bit 2 (side button) or 32 (eraser end) uses the eraser mode of the eraser menu
  (`contact_tool`). The toolbar tool does not change.
- Chrome reports the side button pressed in the air as a `pointerdown` with `buttons == 2` and no pressure. That
  down starts nothing. The tip touching then arrives as a chorded `pointermove` (`buttons == 3`), which starts the
  eraser contact. For pen moves with the side button `host/dom.js` sends the target path, so Rust can find the page.
- Lifting the tip with the button still held (`buttons == 2`, pressure 0) ends the contact. Only the tip bit
  counts for this check (before, `buttons == 0` was required).
- `contextmenu` on the page is cancelled after a side-button contact or during a pen contact.

## Predicted tip

`host/dom.js` sends up to 4 `getPredictedEvents()` samples with every pen `pointermove`. `editor/pen.rs` keeps them
until the next event, and `render::live::tip` draws them with the last real width. They are never stored, synced or
sent as live previews. A `desynchronized` canvas was not used: it can tear or flicker in a scrolled, zoomed
container on some GPUs, and it could not be checked on a real phone here.

## Verification

- `cargo test --workspace`: 177 + 146 (1 ignored) + 49 (new: ink outline, pressure, PDF outline, page fill, side
  button and eraser end, side button pressed in the air, hover with side button, predicted points not stored).
- e2e `04d-pen-pressure`: widths ordered by pressure on the real page bitmap, constant pressure keeps the width,
  side-button erase with the CDP stylus (`pointerType: 'pen'`, `force`, chorded buttons), toolbar unchanged.
- Size: `ihn.wasm` 1,341,706 bytes / 598,680 gzip (main: 1,334,340 / 589,605 by `gzip -9`), JS 121,863 bytes.

## Not done / open

- Not checked on a real S Pen device: pressure curve and side-button event order should be confirmed there.
- A fragment left by the area eraser restarts the pressure average, so its width can differ slightly from the
  stroke it came from; a fragment whose remaining pressure no longer varies is drawn at the chosen width.
- Selection outlines (the dashed halo) still follow the centreline.
