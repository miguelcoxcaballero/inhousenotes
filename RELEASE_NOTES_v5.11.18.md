# Inhouse Notes v5.11.18

## Clean scanner stencil

- Removes the photographed yellow frame, its grey antialiasing and the footer box residue onto corrected white paper before drawing the precise digital stencil.
- Processes out the photographed grey dot grid across the warped page, including small non-rigid alignment errors, so the digital grid replaces it instead of stacking on top.
- Preserves the dark core of black handwriting and saturated red or blue pen strokes while removing the pale neutral template layer.
- Regenerates the four colour reference dots and footer geometry from clean vector artwork.

## Verification

- Visually inspected the supplied problem photo after frame and grid removal.
- Verified an isolated photographed grid dot is removed while a black stroke crossing a grid location survives.
- Passed all 15 deterministic synthetic photos, including perspective, rotation, colour casts, blur and curved-page cases.
