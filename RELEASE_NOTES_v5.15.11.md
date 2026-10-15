# v5.15.11

Detect faded frames by their printed layout.

Web only; Android loads this update without a new APK.

- Recover pale, grey and colour-shifted rails from narrow local contrast when
  ordinary yellow detection is insufficient. Require the four ordered colour
  circles, all measured frame sides and both separate lower box strips.
- Follow current-frame evidence during tracking, retaining brightness, focus
  and dot-grid checks. Sample between the box dividers so their intersections
  do not pull the two faint horizontal rails together.
- Recheck non-yellow rails in the captured photograph and use the measured
  calibration strip to exclude box corners from the 3D bullet-grid fit.

Validation: 27 detector unit checks; 29 scanner browser checks covering four
curved grey frames, tracking, a complete grey-frame camera-to-editable scan,
colour-cast/dim scenes, blur, clipped and missing grids, and both worker and
fallback processing. Final output dots remain independently verified.
