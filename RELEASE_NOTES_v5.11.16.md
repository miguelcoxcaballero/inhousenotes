# Inhouse Notes v5.11.16

## Reliable neutral-frame scanning

- Recovers the printed yellow frame when low saturation, overexposure or a colour cast makes it appear neutral grey in the photo.
- Uses the lower square rails and all four colour markers to establish orientation and constrain the frame before choosing any corners.
- Rejects handwriting, dot-grid columns and unrelated paper edges instead of accepting them as stencil borders.
- Shows the real detected border and square evidence over the preview, then merges it into the fitted stencil before mesh, warp and colour correction.

## Portable production validation

- Keeps the neutral-ridge and marker-guided processing paths under explicit bounded execution limits on desktop, mobile and slower Linux runners.
- Expresses final warp tolerance as the exact two-pixel raster boundary used by the validation canvas, avoiding platform-specific rounding failures without loosening visible-quality requirements.
- Preserves the sub-pixel mesh precision for complete and strongly curved chromatic frames.

## Verification

- Visually verified the fitted frame, lower boxes and corrected output in all six supplied photos.
- Passed all 15 deterministic synthetic photos, including a near-grey frame with perspective, curvature and colour distortion.
- The near-grey regression case measures 1.34% corner error and 0.82% maximum border-curve error.
- Passed 100 unit and smoke checks, all browser resilience and collaboration-chaos tests, and the complete geometry matrix locally.
