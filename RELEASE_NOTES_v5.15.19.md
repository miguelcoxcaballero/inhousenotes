# v5.15.19

Recover missing handwritten contours as editable ink.

Web only; Android loads this update without a new APK. Includes the local
neutral-reference colour calibration published in v5.15.18.

- Preserve the original editable pen axes and recover substantial curves,
  lobes and endings omitted by their constant-width approximation. Additional
  short native pens must fit inside the measured ink silhouette, retaining
  letter counters and the existing standard-width strokes.
- Derive repair colour only from connected native ink. Do not borrow a pen
  colour across white paper or paint over a different coloured stroke.
- Select the supported contours in Rust/Wasm, with an identical cancellable
  JavaScript fallback for older cached kernels. Limit candidates, output pens
  and points, and release the temporary render canvas immediately.
- Keep the result as ordinary editable pens, with no photographic layer.

Validation includes camera-resolution JPEG fixtures with small letters and
variable pen widths, adjacent coloured components, large filled drawings,
weak oblique crossings and native editor/PDF integration. Rust and JavaScript
produce identical reconstruction geometry. The original 54 letter counters
and the additional 24 counters and fine endings remain intact.

In the variable-width fixture, black-ink recovery improves from 91.72% to
96.72%; the lowest individual-letter recovery rises from 85.65% to 94.09%. These compare
against the generated original with a half-editor-pixel tolerance. The private
photo gains 470 supported details with approximately 5% more points. Its
downstream reconstruction improves in all five inspected regions, but does
not establish complete recovery of ambiguous or blurred photographed letters.
Private checks isolate processing after the warp; they do not certify camera
geometry on the compressed attachment. No user photos are committed.

The measured private-photo contour selector took about 68 ms in Rust versus
867 ms in the yielding JavaScript fallback on this test machine; this is one
processing stage, not the entire scan or an Android-device benchmark.

Existing converted pages are unchanged. Rescan the original page to apply
these improvements; missing source information cannot be recovered from old
editable strokes alone.

Release checks: 48 browser regressions, 10 core/adapter tests, Rust unit tests,
offline build and release smoke checks passed. The dense editor fixture keeps
8,685 native pens, including 3,452 repaired contours, through selection,
erasing, undo, reload and PDF reopening without a photographic layer.
