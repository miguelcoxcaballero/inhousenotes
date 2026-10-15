# v5.15.13

Keep black ink neutral under mixed lighting.

Web only; Android loads this update without a new APK.

- Measure local paper colour around verified bullet dots, then remove changing
  light casts with a Rust pixel kernel and matching JavaScript fallback. Keep
  the footer pigments for colour calibration and preserve real red/blue ink.
- Recognize locally tinted paper around the four reference circles. Recover
  the live frame from its thin rails and both lower strips when yellow shifts
  under a second light source.
- Recheck luminance ridges on the captured photograph even if the video used
  colour detection. Retain independently measured photo evidence when camera
  white balance changes; missing frames and unverified grids remain rejected.

Validation includes mixed blue/warm/magenta illumination through editable pen
extraction, Rust/fallback agreement, JPEG halo and paper/shadow regressions,
live and full-scan geometry, and rejection of missing frames or grids.
