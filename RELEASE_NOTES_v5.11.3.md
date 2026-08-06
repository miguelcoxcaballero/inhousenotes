# Inhouse Notes v5.11.3

## Faster, more reliable scanning

- Replaced the broad yellow test that could mistake wooden desks and warm
  backgrounds for the Inhouse stencil with strict hue and saturation checks.
- Added robust line fitting for partially hidden, broken, or perspective-skewed
  stencil borders and rejects geometrically impossible crops.
- Added a lightweight paper-edge fallback when the yellow stencil cannot be
  recovered, including strongly inclined and partly clipped sheets.
- Keeps analysis at a small working resolution and generates the full A4 image
  only once; real reference photos complete perspective and colour processing
  in roughly half a second on the test machine.

## Natural colour processing

- Validates calibration marks before using them, so a shadow, note, or damaged
  dot can no longer wash out the entire page.
- Uses bounded paper white balance as the safe default and preserves red, blue,
  green, and black handwriting.
- Neutralizes stencil pixels with the local paper brightness instead of drawing
  conspicuous pure-white bands.

## Regression protection

- Added browser coverage for a perspective sheet on the warm background that
  triggered the original false detection.
- The scanner continues to work completely offline without OpenCV or WASM.
