# Inhouse Notes v5.11.35

- PDF page copying, overlay rasterization, PNG encoding, compression, document hashing and Timeline archive construction now run in a background worker on supported browsers. Processing remains on the device; no new server or service receives document content.
- Dense handwritten pages repaint in short, cancellable slices. Completed repaints cannot overwrite strokes added while rendering, or replace a page from a different document session.
- Local page checkpoints clone strokes incrementally, with generation checks before and after the IndexedDB commit.
- Fixed a circular wait between a queued save and deferred clean-original recovery. Saving explicitly starts the prerequisite recovery.
- An upload already in progress is no longer cancelled when the user touches the screen or writes. Pointer hover no longer repeatedly pauses processing.
- Saving immediately after reopening restores embedded Timeline history before appending a new version. Repeated archive generation and unnecessary deep copies are avoided, with bounded archive retention handled off-thread.
- Added browser regression coverage for dense-page responsiveness, concurrent ink/repaint safety, rendering equivalence, uninterrupted uploads, and immediate reopen/save history preservation.

## Validation

On a synthetic desktop Chromium fixture with 2,400 strokes and 240,000 points, PDF preparation fell from approximately 6.6 seconds to 3.4 seconds. The largest observed main-thread task fell from approximately 2.9 seconds to 72 ms. Dense repaint setup took roughly 3–7 ms before yielding. These measurements do not include real Drive network upload time and are not a guarantee for every device or document.

The Android native wrapper is unchanged; it loads this updated web runtime. No new APK is required.
