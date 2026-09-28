# Inhouse Notes v5.11.28

## Fluid pen and touch input

- Live pen packets sanitize only the unsent tail; the final recovery packet still contains the complete stroke.
- Receiving devices append the new live points to the overlay instead of copying the whole stroke and repainting every previous point for each packet.
- The pen and eraser preview clear only their previous stroke or cursor bounds; highlighter previews paint newly sampled segments incrementally.
- Viewport movement remains frame-coalesced, while canvas resolution and expensive page activation wait until interaction settles.
- The v5.11.27 pen-down and save scheduling improvements remain included.

## Android

- The existing Android 1.0.10 shell loads this hosted web release automatically; no native APK change is required.
