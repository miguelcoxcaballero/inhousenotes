# Inhouse Notes v5.11.14

## Geometry-verified scanner reconstruction

- Uses the four printed colour circles to establish the target page's orientation and coordinate system, even when other sheets overlap it.
- Traces the complete yellow frame and the two horizontal rails of both lower calibration boxes without confusing their vertical dividers with the page border.
- Locates the physical page corners independently from the printed stencil and rejects incomplete or degenerate edge fits.
- Straightens perspective and local paper curvature with a dense mesh constrained by the outer frame and both lower rails.
- Restores paper white and the printed red, black, blue, green and yellow references without introducing shadow patches.
- Uses a fast first pass for ordinary scans and an adaptive precision pass for strong curl, blur, weak markers or incomplete evidence.

## Verification

- Visually verified the frame, lower boxes, paper crop and corrected output in all six supplied photos.
- Added 15 deterministic synthetic photos covering 3D warps, both landscape orientations, upside-down pages, shadows, blur, exposure changes and colour casts.
- Tests physical corners, all four outer yellow rails, both lower box rails, final mesh alignment, paper white and all four ink colours.
- Passed 100 unit and smoke checks plus all 20 browser tests.
