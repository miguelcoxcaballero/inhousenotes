# v5.15.20

Preserve photographed handwriting when tracing editable ink.

Web only; Android loads the scanner update without a new APK.

- Use local ink contrast as well as the darkest nearby sample when selecting
  photographed ink. A dark crossing must not erase its lighter continuation.
  Rust and JavaScript implement the same selection; cached older Rust kernels
  use the current fallback rather than silently changing the geometry.
- Reconstruct the measured silhouette using nested native pen paths. These
  retain curves, varying widths and counters that a constant-width centreline
  and isolated repair dots could not represent. Keep standard-width pens
  where the measured ink supports them, and keep colour boundaries separate.
- Store only ordinary editable pens. Selection, resizing, erasing, undo,
  persistence and PDF reopening use the existing document format. Join
  opposite branches of the same colour and width at their existing junction,
  retaining the original end segments instead of smoothing across the join.
- Bound reconstruction work and output size, yield for cancellation, and
  discard temporary canvases. Round coordinates to hundredths of a scan pixel
  to avoid storing long floating-point interpolation tails.

The fidelity checks now require substantially less missing ink and higher
silhouette accuracy, including per-letter checks rather than only page-wide
averages. Full contour paths use more points than the former two-point repair
discs; the dense-page check retains the same pen-count and serialized-byte
budgets and exercises editing and persistence on the actual generated ink.

Validation: 51 browser regressions, 10 Rust-adapter tests, Rust unit tests,
offline-shell generation and release smoke checks passed. In the variable-width
fixture, every letter retains more than 98.5% of its original ink within a
half-editor-pixel tolerance; all 24 counters and thin tips remain present.
The 900-character editor fixture uses 6,458 pens and 176,535 points (4.52 MB
serialized), including the full envelopes, and preserves them through editing
and PDF reopening. Its pointer burst retains every sample with two paints.
These are desktop test measurements, not an Android-device benchmark.

Existing converted pages are unchanged. Rescan the original to apply the new
tracing. The compressed private photo was used to inspect downstream tracing;
it does not establish that every blurred letter can be recovered, or validate
a fresh Android camera capture. No private photographs are included here.
