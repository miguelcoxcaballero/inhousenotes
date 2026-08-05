# Inhouse Notes v5.11.1

## Scanner reliability and performance

- Removed the OpenCV/WASM startup dependency that could leave Android and web
  sessions permanently stuck on “Loading Core”.
- Replaced the old multi-pass pipeline with a local Canvas implementation that
  detects the yellow stencil at low resolution, straightens the page once, and
  processes only a bounded final A4 bitmap.
- Calibrates red, black, blue, green, yellow, and white references with one
  stable color transform, then removes the printed calibration marks.
- Keeps manual crop as the fallback whenever stencil confidence is insufficient.
- Loads page sorting and PDF export libraries asynchronously, so neither can
  delay opening the scanner.

## Responsiveness

- Removed repeated full-resolution JPEG previews and the repeated config fetch
  from each scanned page.
- Limits oversized camera bitmaps before analysis and yields during color work
  to keep touch and drawing input responsive.
- Unloads the scanner frame immediately when it closes, releasing its canvases
  and memory before returning to the editor.

## Verification

- Added an offline browser regression test that opens the embedded scanner,
  processes a synthetic yellow stencil, and confirms OpenCV is not present.
