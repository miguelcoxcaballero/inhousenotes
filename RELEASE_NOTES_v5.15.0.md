# v5.15.0

Editable scanned handwriting.

Web only; Android uses the updated web app without a new APK.

- New corrected scans become ordinary editor pen strokes in the exact black,
  blue and red writing palette. Stroke colours are opaque; the normal canvas
  edge antialiasing is shared with handwritten digital pens.
- Rust extracts centre-lines and widths from cleaned ink. A yielding JavaScript
  fallback supports browsers without WebAssembly. Preview and insertion use
  the same strokes over the native template, with no photograph underneath.
- Scanned pens support selection, resizing, recolouring, partial/stroke erasing,
  undo, local persistence, collaboration identities and editable PDF metadata.
- Existing documents and earlier image-based scans retain their saved content.

Validation covers Rust/fallback parity, faint ink, curves, loops, punctuation,
template separation, normal editing tools, local reload, PDF reopening, scanner
workers and cancellation. Centre-lines reconstruct visible ink, not the unknown
original order of pen movements through handwritten intersections.
