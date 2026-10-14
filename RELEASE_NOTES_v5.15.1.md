# v5.15.1

Cleaner scan ink, smoother editing.

Web only; Android loads this update without a new APK.

- Rust reconstructs one centre-line from the ink silhouette before assigning
  palette colours. Coverage-weighted colour voting and short-run filtering
  prevent dark cores and chromatic camera fringes becoming separate outlines.
- Pen widths close to the editor's standard 2 px snap to it at either scan
  resolution. Cross-section measurements preserve distinct fine/thick strokes
  and are bounded at intersections to avoid inflated junctions.
- The scanner uses shorter labels, compact animated progress and faster reveal
  transitions, with reduced-motion support. Previews swap only after decoding;
  the dots, yellow border and four reference circles stay visible throughout
  processing, using the native template on corrected progress images.
- Editable scan pages reuse cached pen paths and hit-test bounds. Partial erasing
  copies only changed pens into undo history; selection drags update once per
  frame and repaint their affected area. Background repaint work yields while
  the pen is active. Ink and preview encoding run concurrently.

Validation: Rust/JavaScript parity, mixed-colour fringe and crossing fixtures,
standard width at two resolutions, full photographed-page import, native editing,
local/PDF round trips, persistent processing references, reduced motion, parallel
worker completion/cancellation, dense-page partial erasing and undo/redo.
Existing saved ink is preserved; improved colour reconstruction applies to new scans.
