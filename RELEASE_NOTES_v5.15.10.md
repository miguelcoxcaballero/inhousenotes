# v5.15.10

Responsive pen input and consistent scanned pens.

Web only; Android loads this update without a new APK.

- Retain raw/coalesced pen positions and release samples, handle empty Android
  coalesced lists, ignore foreign pointers and keep capture across canvas edges.
- Paint once per animation frame with batched curves. Bound pen-up simplification
  and defer whole-page repaints and storage snapshots while the pen is down,
  including when it is stationary. Preserve eraser feedback and exit saves.
- Resolve scan colour from reliable pigment across each continuous pen. Keep
  near-standard writing at the editor's standard width across shaded sections,
  with light smoothing and preserved thin/thick pens and real crossings.

Validation: 21 editor/storage/PDF browser checks, including 3000-pen pages,
stationary input, empty/raw/coalesced events, foreign pointers and local reload;
12 scan vector checks for colour, width, smoothing, Rust/fallback parity, editing
and photographed import. Release storage-generation checks pass.
