# Inhouse Notes v5.11.4

## Marker-guided page geometry

- The scanner now identifies the complete repeated squares at the bottom of the
  Inhouse template and uses their two endpoints to determine page width, scale,
  and the lower corners.
- The red, black, blue, and green colour circles validate the strip centre and
  orientation; isolated coloured handwriting can no longer move a corner.
- The asymmetric marker pattern resolves the true top and bottom even when a
  photo is rotated 90 degrees or the ordinary yellow border appears plausible in
  the wrong orientation.
- Missing border segments can be reconstructed from the calibrated template
  geometry when another sheet, a shadow, or the edge of the photo hides them.

## Reliability and speed

- Competing yellow lines from sheets underneath are rejected unless they agree
  with both ends of the calibration strip and the perspective of the page.
- The four supplied cluttered, inclined, and rotated reference photos now use
  the marker-guided path and return consistent stencil corners.
- Detection stays local and dependency-free; full perspective and colour
  processing remains below one second on the reference test machine.

## Regression protection

- Added a browser test containing a rotated page with two deliberately obscured
  border sections. It must recover the A4 corners from the squares and circles.
- All processing continues to work offline without OpenCV, WASM, or a server.
