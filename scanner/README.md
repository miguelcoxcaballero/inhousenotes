# Scanner

The embedded scanner opens `live-camera.js` immediately. It requests a rear camera without audio, analyses frames at a maximum edge of 480 pixels, and draws observed yellow-frame and calibration-strip geometry over an uncropped video preview. `processing/liveDetection.js` also uses the paper perimeter to support faint or broken rails.

Automatic capture requires a complete, stable frame across at least seven distinct video frames and one second, sufficient confidence, exposure and sharpness. Three source frames are retained for review; detailed perspective/colour correction uses the existing `processing/lightweight.js` only after capture. The user confirms before the existing session-checked scanner bridge inserts a page. Originals stay in memory until close or retake; no video recording or upload is used for detection.

Camera access requires HTTPS (or localhost) and user permission. The parent iframe grants `allow="camera"`. Android WebView additionally needs a signed app declaring `android.permission.CAMERA`; older APKs without it can still import a photo. Extremely faded or occluded frames may require manual capture and review.

Run `npm run test:unit` and `npx playwright test tests/e2e/live-scanner.spec.mjs tests/e2e/scanner-live-detection.spec.mjs tests/e2e/scanner-geometry.spec.mjs` for detector, camera lifecycle, insertion and perspective regressions. Camera tests use real Canvas MediaStreams with synthetic printed-page footage; physical-device testing remains useful for autofocus and lighting differences.
