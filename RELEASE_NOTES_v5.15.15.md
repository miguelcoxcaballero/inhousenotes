# v5.15.15

Preserve fine scanned handwriting and stroke endings.

Web only; Android loads this update without a new APK.

- Recover faint ink using contrast across the pen and continuity along it.
  A darker nearby stroke no longer erases a fine parallel line or connection.
  Keep the first background valley between strokes and reject isolated noise.
- Protect supported diagonal tips before centre-line thinning. Reduce curve
  displacement and simplification, and sample native pens more closely to
  preserve small turns when rendering and erasing.
- Apply the connected pen's measured width to short junction chunks created
  by width splitting, so an otherwise normal pen cannot end in a thick blob.
- Keep the extraction in Rust/Wasm with matching JavaScript fallback output.
  Scans remain ordinary editable pens in the editor palette.

Validation: new continuity regressions at two resolutions retain over 99.8%
of sampled fine handwriting beside darker ink, without filling the gap between
parallel strokes; Rust and JavaScript produce identical pens. The photographed
small-letter regression preserves all 54 counters and all sampled letter cores.
Native editing, erasing, reload, PDF, standard/thin/thick widths, colour fringes,
lighting, capture cancellation and mandatory grid validation are also checked.

The supplied compressed photograph was compared through the downstream ink
stages separately: it does not pass the independent final dot-grid certificate.
That camera gate is unchanged. Existing converted pages are unchanged; rescan
the original to recover details already lost in an older conversion.
