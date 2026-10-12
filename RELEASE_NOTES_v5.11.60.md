# Inhouse Notes v5.11.60

Lighting-aware scanning and grid-guided 3D reconstruction.

The live camera and still-image processor measure yellow ink relative to the paper's local white balance. This recovers the frame under cold and warm lighting while retaining the checks for the page perimeter, thin printed rails and calibration boxes. Colour cleanup removes the same cast before whitening paper, preserving pencil and coloured writing.

The 3D reconstruction traces the observed rows and columns of grid dots as curves. Their positions and tangents constrain the physical paper surface, alongside the yellow border. Missing or occluded runs remain gaps. The measured calibration strip is excluded from the interior grid so JPEG artefacts at its corners cannot add false rows. Independent observations check the reconstruction before it is used; unsupported geometry retains the existing fallback.

The new screen recording supplied the blue-light regression samples. Synthetic curved pages separately check geometry and writing preservation. The model estimates a smooth developable sheet from a single image; it does not measure calibrated depth or reconstruct arbitrary folds.

Android 1.0.16 (versionCode 17) loads these scanner changes from the website and already includes the camera permission. No APK reinstall or update catalog change is required. This release has not been tested on a physical Android device.
