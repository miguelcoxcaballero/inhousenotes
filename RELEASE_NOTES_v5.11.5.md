# Inhouse Notes v5.11.5

## Real scan-processing animation

- The selected image now shows each real processing phase directly over its
  preview: the four detected corners, traced page edges, perspective mesh,
  geometric straightening and colour correction.
- Corner positions and the animated mesh come from the same detected page quad
  used to generate the exported scan; the animation is not a decorative mock.
- During straightening, the photographed page itself is progressively remapped
  through the detected perspective geometry before the final A4 result appears.
- Colour balancing uses an actual before-and-after wipe over the corrected
  output, making the lighting and white-balance change visible.

## Performance and polish

- Accelerated easing and short, overlapping phases keep the sequence fluid
  while the full-resolution warp runs in parallel.
- The animation canvas is resolution-capped on desktop and lower-memory mobile
  devices, so it does not slow the underlying scan or retain a second large
  photo bitmap.
- Only the currently selected page is animated; queued background pages retain
  the fastest processing path.
- Reduced-motion preferences are respected, and switching pages or encountering
  an error clears the visual layer immediately.

## Regression protection

- The offline scanner browser test now verifies all phases in order and checks
  that corners, mesh, warp and colour stages report real processing metadata.
- The same test confirms that the overlay is removed after processing and that
  no OpenCV, WASM or external server is required.
