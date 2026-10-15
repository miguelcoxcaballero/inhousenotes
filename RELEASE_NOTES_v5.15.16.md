# v5.15.16

Keep captures; preserve fine handwriting.

Web only; Android loads this update without a new APK.

- Stop automatically reopening the camera when every captured photo fails
  correction. Keep the photos and their failure reason visible; Retake is
  explicit, and an unverified photo still cannot be added to the document.
- Recover faint dots when a well-populated but incomplete lattice previously
  prevented the low-contrast pass. Anchor a few missing top rows using both
  lateral borders and the opposite horizontal border, with unique grid phase.
- If the physical surface cannot explain a crease, try a smooth map of the
  measured lattice, validated against held-out dots and their tangents. This
  fallback estimates flat page coordinates, not calibrated 3D depth. Revisit
  faint source dots, constrain the observed perimeter and retain the mandatory
  independent final-raster alignment check. Retry a saved photo at higher
  detection resolution once when the coarse observation is insufficient.
- Normalize the sampling distance across oblique pen directions in Rust/Wasm
  and JavaScript. Faint cursive turns beside darker strokes retain more of
  their original shape, without broadening the ink threshold or increasing
  simplification. Native editing, palette colours and standard widths remain.

Validation covers the capture/review/Retake flow, missing and partial grids,
uneven dot contrast, creased sheets, known interior writing, camera geometry,
editable ink, letter counters, thin/standard/thick pens and Rust/JS parity.
The new oblique-writing fixture retains 99.2% of sampled centreline positions,
up from 97.9%; the photographed-letter fixture retains all 54 counters.
The broader legacy geometry suite still fails its grey `faded-yellow` fixture
at border detection, with the same corner error reproduced on v5.15.15. That
known case is not counted as passing and no geometry tolerance was relaxed.

Private diagnostic: a captured-photo crop from the supplied recording completes
correction and passes final dot alignment. Other compressed attachments can
still be rejected; these checks are not a substitute for testing the native
camera on the user's phone. No user images are included in the repository.
Previously converted pages are unchanged; rescan to recover details discarded
by an older conversion.
