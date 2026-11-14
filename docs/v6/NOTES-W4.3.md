# W4.3 IMAGES — notes for the other units

Branch `v6-w4_3` (from `v6-int` at `58039fc`). Owns `rust/crates/ihn-app/src/editor/images.rs`,
`tests/v6/e2e/06-images.spec.mjs`, `tests/v6/parity/states/images.mjs`.

## What exists now

- `editor::images` — 5.18.0 `insertImageBlobsAtClientPoint` / `compressImageForDocument` / `addImageToPage`
  (spec/editor.md §5.7). Listeners: document `paste`, `#canvas-viewport` `dragover`/`drop`, and the last pointer
  position (document `pointerdown` capture, plus `mousemove` capture for hover — a DOM event, so no coalesced list is
  encoded per move). A paste/drop takes the event's image files with `fx::files_take` inside the event (256 object
  handles reserved per event) and queues one batch for the page whose wrapper is nearest to the point (the last
  pointer position for a paste, else the viewport centre). Files run one at a time: decode (`Cmds::image_blob`) →
  PNG ≤ 4 MiB at scale 1 kept byte-exact (the File is the asset blob), else one draw on a processing canvas at
  `min(1, 3200/max(w,h), √(8 MiPx/(w·h)))` and one encode (PNG stays PNG, else JPEG 0.94 on white) → `fx::digest`
  → `session::put_asset` (skipped, and the blob released, when the hash is already stored: one asset per hash) →
  one `add_image` batch, one undo step (`history::record`), `selection::clear` + `selection::select(page, &[id])`,
  `input::set_tool(Lasso)`, page repaint. Status "Image added" / "N images added" at the end of the batch.
  The queue is dropped (handles released) on `session::sig::CLOSED`.
- Test API `editor.images.*`: `state` → `{added, queued, busy, last}`; `list` → live images `[{page, id, asset (hex),
  cx, cy, w, h, rotation}]`; `transform {sx, sy, rot, tx, ty}` → the selection-box commit (one batch, one undo step)
  applied to the newest placed image about its centre (used by e2e 06 until W4.2's box is merged).

## Verified

- `cargo test --workspace` green (ihn-app 122, incl. 4 new): scale/keep rules, placement offsets and clamping on A4
  and on a smaller page (B-E1), paste of PNG/JPEG/duplicate PNG → 3 images, 2 assets, 3 undo steps, lasso tool,
  status; drop on the nearest page with page-size conversion, undecodable file skipped; read mode ignores
  dragover/drop; queue abandoned on close.
- e2e `06-images`: real `ClipboardEvent('paste')` / `DragEvent('drop')` with `DataTransfer` files — read mode ignores
  the paste; PNG asset = SHA-256 of the file bytes; the same PNG dropped again adds an image but no asset; JPEG
  re-encoded; offsets `+18`; transform to a rotated, half-size image (pixels follow); undo/redo; reopen gives the same
  snapshot, the same images, no missing asset, painted. `00-boot`, `04-ink`, `internal-doc` still pass.
- Parity `--quick --states images`: on B all three cells fail at the mode toggle ("Element is not visible": the
  5.x fixture `r0` needs W3.4, as W3.2/W3.5 reported); `--self` passes 3/3 and A's captures show the pasted, dropped
  and rotated images. Rerun after W3.4 and W4.2 merge.
- `npm run build` (report-only): ihn.wasm 986,300 bytes raw / 398,012 gzip.

## Deviations

1. **`host/dom.js` (W1.1's) one-line change:** the `DT_FILES` flag now also reads `clipboardData.types` for a paste.
   The paste handler must decide `preventDefault` synchronously and the file list arrives only as an effect
   completion; without the flag every paste (text in the title or the share fields included) would have to be
   prevented or none. Doc comment of `abi::ev::DT_FILES` updated. A paste whose clipboard holds files that are not
   images is prevented (5.18.0 lets it through); nothing is inserted.
2. **`editor/selection.rs` placeholders:** `clear` and `select` with W4.2's signatures (empty bodies) so this branch
   builds before W4.2 merges. At the merge take W4.2's file (it replaces the stub).
3. **Smoothing quality:** 5.18.0 sets `imageSmoothingQuality = 'high'` on the processing canvas; `host/canvas.js` has no
   op for it, so the default (`low`) is used. Only images that are scaled down (> 3200 px or > 8 MP) differ.
4. The `alpha: false` context option for JPEG is not reproduced (the canvas is filled white first, so the JPEG is the
   same).
5. Placement and clamping use the page's real size (B-E1), including the 0.6 × width / 0.45 × height size limit.
6. 5.18.0's `'No usable image in clipboard'` status is never shown (it needs the `'clipboard'` source, which no
   caller passes); not ported.
