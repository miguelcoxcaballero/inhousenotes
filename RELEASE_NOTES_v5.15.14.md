# v5.15.14

Preserve scanned handwriting and small letter counters.

Web only; Android loads this update without a new APK.

- Measure pen geometry from paper-relative luminance before boosting pigment.
  Colour calibration no longer fattens the silhouette used for editable pens.
  Rust collects the coverage during the existing colour pass, with the same
  result in the JavaScript fallback.
- Separate the measured ink core from optical/JPEG fringes at page scale.
  Keep faint pencil and respect template cleanup when extracting centre-lines.
- Use connected, measurable sections of the same-colour pen to size ambiguous
  short junction branches. Preserve standard widths, smooth curves, standalone
  punctuation and genuinely thick pens.

Validation: compared the supplied photograph before/after ink conversion. The
compressed attachment does not pass the independent dot-grid certificate, so
its downstream stages were inspected separately without changing that gate.
A synthetic photographed small-letter regression retains all 54 counters and
all sampled letter cores in Rust and JS; the old conversion filled all 54.
Colour invariance, faint pencil, stencil removal, native editing/reload/PDF,
JPEG halos, mixed lighting, paper contrast and camera processing are checked.
Existing converted pages are unchanged; the fix applies to new scans.
