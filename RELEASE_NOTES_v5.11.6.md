# Inhouse Notes v5.11.6

## Complete yellow-frame detection

- The scanner now follows the complete yellow frame between its four corners
  instead of reducing every page to four straight lines.
- Nineteen robust samples per side track curved edges while rejecting isolated
  yellow handwriting, nearby sheets and the repeated squares at the bottom.
- Missing or hidden frame sections are interpolated from neighbouring samples
  and the calibrated template geometry, preserving the existing marker-guided
  fallback for cluttered and rotated photos.

## Curvature-aware straightening

- A two-dimensional Coons mesh models all four detected frame curves and maps
  them to their exact canonical positions on the A4 page.
- The final warp uses a 12 by 18 mesh, correcting page curl in both directions
  rather than applying only a four-corner perspective crop.
- The original fast quadrilateral warp remains available when no reliable
  yellow frame is present.

## Minimal processing animation

- A new “Detecting yellow frame” phase traces the actual curved paths used by
  the scanner before the mesh appears.
- The animation now uses thin Google-blue lines, small unnumbered corner
  handles, restrained opacity and no neon glows or oversized effects.
- The animated mesh uses the same curve mapping as the exported result, then
  visibly straightens while the real photo is warped.
- Colour correction ends with a simple before-and-after divider instead of the
  previous bright gradient sweep.

## Regression protection

- Added a browser test with a strongly curved synthetic frame. It verifies that
  curvature is detected and that the warped yellow border is straight to within
  four output pixels.
- Existing offline, warm-background, rotated and partly hidden-frame tests
  continue to pass without OpenCV, WASM or external processing.
