# v5.15.3

Verify sharpness and page dots before capture.

Web only; Android loads this update without a new APK.

- Automatic capture waits for three independently verified camera frames over
  at least 300 ms. Each must contain sharp ink edges and a measured bullet
  lattice distributed throughout the page, rather than only a yellow border.
- The exact last verified frame is preserved for capture. Phone movement after
  that instant does not replace it with an unverified image. Manual captures
  and backup frames also require photographed dots and sufficient sharpness.
- A persistent background worker checks the lattice without blocking the live
  preview. Checks are bounded and cancelled when closing or restarting the
  camera; slower checks do not indefinitely reset the evidence window.
- Camera scans require a validated physical dot surface and independently
  verified dots in the corrected raster before enabling Add. Failed automatic
  batches retry without inserting the original photograph.
- Existing Rust processing, native template colours and editable pen strokes
  remain in place. Photo imports retain their existing compatibility.

Validation covers curved and rotated sheets, exposure and colour casts, blur,
clipped highlights, noise, missing dots, stalled video, movement, slow checks,
cancellation, burst review and insertion into the editor. These checks reduce
failed captures; they cannot guarantee recovery of unreadable photographs.
