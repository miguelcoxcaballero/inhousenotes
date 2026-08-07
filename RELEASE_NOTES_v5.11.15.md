# Inhouse Notes v5.11.15

## Reliable neutral-frame scanning

- Recovers the printed yellow frame when low saturation, overexposure or a colour cast makes it appear neutral grey in the photo.
- Uses the lower square rails and all four colour markers to establish orientation and constrain the frame before choosing any corners.
- Rejects handwriting, dot-grid columns and unrelated paper edges instead of accepting them as stencil borders.
- Follows curved border evidence continuously and straightens it with the existing mesh rather than forcing an inaccurate rectangular crop.
- Keeps colour correction calibrated from the four marker circles even when the frame itself has lost its yellow hue.

## Processing animation

- Shows the real detected frame and square evidence directly over the source preview in bright yellow.
- Grows the evidence from the lower calibration strip and smoothly merges it into the fitted frame, rails and square dividers.
- Keeps the detected stencil visible through corner, mesh and warp phases, then reveals the corrected colour result.
- Limits display evidence adaptively so the animation stays smooth on low-memory mobile devices.

## Verification

- Visually verified the fitted frame, lower boxes and corrected output in all six supplied photos.
- Passed all 15 deterministic synthetic photos, including a near-grey frame with perspective, curvature and colour distortion.
- The near-grey regression case measures 1.34% corner error and 0.82% maximum border-curve error.
- Passed 100 unit and smoke checks, all 17 browser resilience tests and the complete 15-case geometry matrix.
