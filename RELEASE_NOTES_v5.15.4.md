# v5.15.4

Verify camera capture and editable scan handoff.

Web only; Android loads this update without a new APK.

This maintenance release retains v5.15.3's sharpness and photographed-dot
capture checks, final grid certification, and existing Rust processing.

- Camera regression tests now use physically projected sheets and verify
  native editable pen strokes instead of the former embedded photographs.
- Calibration tests apply a consistent camera response to the page, all four
  colour circles and the yellow frame, and check measured reference accuracy.
- Review tests require rejected scans to keep Add disabled.

Validation includes camera permissions, interrupted video, older WebViews,
retakes, slow encoding, pale backgrounds, faint lighting and colour calibration.
No scanner processing thresholds or algorithms change in this maintenance release.
