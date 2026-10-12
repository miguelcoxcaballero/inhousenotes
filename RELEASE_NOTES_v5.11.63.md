# Inhouse Notes v5.11.63

Straighter scans, clearer writing and the native editor template.

Photographed bullet centres are refined on the original image. Once the physical page model passes independent checks, reliable measured dots anchor the correction to the printed bullet positions. The warp includes those anchors explicitly and compensates for the way small photographed dots are resampled. Missing or ambiguous dots remain unreported. Processing guides follow the actual image transformation; alignment is checked on the corrected photograph before cleanup or template replacement.

Illumination correction uses the paper around observed bullets to distinguish shadows from writing and filled drawings. Local contrast restores darker pen and pencil strokes while preserving coloured ink and punctuation.

Scanned writing is now a transparent PNG image on a normal editor page. The editor and scan preview share the exact same native template renderer, including its dot gradients, yellow frame, footer boxes and opaque coloured markers. The template is no longer photographed or JPEG-compressed into the inserted scan. Both layers survive saving, reopening and PDF export/import. Uncorrected photos keep their original import behaviour.

Android 1.0.16 (versionCode 17) loads these changes from the website. No APK reinstall or update catalog change is required. Physical Android camera performance has not been measured for this release.
