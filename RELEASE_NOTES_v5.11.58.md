# Inhouse Notes v5.11.58

Curved-page scanning with a visible, responsive preview.

The live scanner follows measured curved yellow borders and calibration marks, tolerates short detection gaps, and selects a sharp frame from the capture burst. The captured photo appears immediately while background detection, curved-surface straightening and colour correction run. Changing frames or scanning again cancels obsolete processing.

The processor uses the printed guides and measured calibration-cell spacing to flatten a curved surface, with a dense inverse sampling mesh that preserves page content. Perspective correction also accounts for the changing scale across a tilted page. Shadows are reduced while preserving pencil, punctuation, coloured ink and grey diagram fills. This is guide-based dewarping from a single camera view; hidden text and folds outside that view cannot be reconstructed.

Android 1.0.16 (versionCode 17) continues to load the scanner from the website and already includes the camera permission. The web correction reaches that installed APK without reinstalling it; its signed binary and A/B catalogs remain unchanged.

Validation covers curved geometry, image quality, camera lifecycle, review and document insertion. Independent camera projections of cylindrical and S-shaped sheets check interior writing as well as the printed frame; missing calibration marks, distracting yellow lines and invalid folded meshes also have regression coverage. The supplied screen recording was used to reproduce the empty preview and processing delay. This release has not been tested on a physical Android device.
