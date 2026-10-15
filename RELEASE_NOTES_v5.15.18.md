# v5.15.18

Calibrate ink against local neutral references.

Web only; Android loads the scanner changes without a new APK.

- Measure both the white paper and the grey bullet dots to distinguish a
  local coloured reflection from the ink's pigment. White balance alone
  could leave black pen strokes blue or red under additive reflected light.
- Require agreement among neighbouring neutral dots before applying this
  correction. Handwriting across a reference dot cannot calibrate the page
  by itself. Preserve muted blue and dark red when neutral evidence is valid.
- Keep measured luminance unchanged when correcting colour, including
  out-of-gamut values, so colour calibration does not expand or erase ink.
- Run the pixel correction in Rust/Wasm with the same JavaScript fallback.
  Older cached kernels safely fall back instead of ignoring new inputs.

Validation covers strong local blue/red reflections, thin JPEG-compressed
black, pencil, muted blue and dark red strokes, contaminated reference dots,
missing or inconsistent footer markers, and out-of-gamut luminance. Rust and
JavaScript agree. Private-photo checks compare final native pen colours;
these isolate colour processing and do not certify camera geometry on the
compressed attachment. No user photos are committed.

This release addresses colour calibration. A separate handwriting topology
fix is being validated for fragmented or distorted editable strokes. Existing
converted pages are unchanged; rescan the original to apply this correction.
