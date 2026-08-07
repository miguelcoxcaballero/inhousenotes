# Inhouse Notes v5.11.17

## Reliable scanner queue

- Opens the first newly imported page immediately so its real processing animation is visible instead of leaving it at `Queued...`.
- Prioritises a queued page when it is selected and automatically resumes pending work after app or browser lifecycle interruptions.
- Prevents duplicate queue entries and rechecks work added during the final queue transition.

## Observed border geometry

- Rejects border fits that require invented corners far outside the photographed image.
- Scores the physical paper silhouette and printed-frame evidence together instead of always preferring a weaker marker-only axis.
- Uses the observed paper corners when a frame rail is genuinely missing, avoiding mirrored pages, false diagonal paths and dark background wedges.

## Stable colour correction

- Separates smooth paper illumination from strokes, pencil and grid detail before correcting exposure.
- Cleans obvious desk or folder pixels pulled into the narrow template margin without removing real page shadows.
- Keeps the four printed colour references vivid while normalising dark, overexposed and colour-cast photos.

## Verification

- Visually inspected the supplied problem photos and their detected-border overlays and corrected outputs.
- Passed all 15 deterministic synthetic photos, including dark camera, overexposure, perspective, rotation, blur and curved-page cases.
- Passed 100 unit and smoke checks plus all 20 browser end-to-end tests.
