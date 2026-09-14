# Inhouse Notes v5.11.21

## Erasing on a PDF could leave a trace, sometimes permanently

- **The bug:** saving ink on top of an imported PDF is supposed to keep the original PDF pages untouched and store strokes as an independent, editable layer, so erasing a stroke removes it cleanly. A save could silently fall back to a raster pipeline that flattens every current stroke directly into the page's pixels instead. Once that happened, the eraser could no longer truly remove a stroke — it painted an opaque white patch over the burnt-in pixels to approximate erasing, which doesn't reliably hide anti-aliased edges or strokes over a non-white background, and can't be undone.
- **Root cause:** the safe, non-destructive save path needs a copy of the original PDF's bytes. Capturing that copy was capped at 60MB — 4MB stricter than the app's own 64MB upload limit — and gave up silently (no warning, no retry) on any capture failure, including a one-off network hiccup. Any of those triggered the destructive fallback on a document's very first save, not just on old files.
- **The fix:**
  - The capture cap now matches the real 64MB upload limit exactly, so any PDF the app accepts at all keeps its clean original.
  - A failed capture is retried once before being treated as unrecoverable.
  - If the safe save path fails, it's now retried once before falling back, since a single transient error (not a real capture problem) was enough to trigger the destructive path before.
  - If a save is ever still forced onto the raster fallback, the document is now marked honestly right away — the save indicator shows a warning instead of silently looking fine until the next time you tried to erase something and found it didn't work.
  - That marking self-heals: the next time a save succeeds through the safe path, the document goes back to true, traceless erasing.
- **What this doesn't fix:** if a PDF was already saved through the destructive path before this update — by an older app version, or by hitting this bug — the original pixels underneath are genuinely gone. There is no hidden backup of the pre-annotation PDF to restore from in that case. Reopening the file will keep using the white-cover approximation for erasing on that specific document; there's no way around that for content already affected. Documents saved from here on should not hit this at all.

## Tests

- Added `tests/pdf-clean-original-cap.test.mjs` to pin the capture cap to `securityCore.MAX_PDF_BYTES` so the two can't silently drift apart again.
- Added `tests/e2e/pdf-nondestructive-erase.spec.mjs`: a clean save never falls back to legacy baking, a transient failure is retried instead of degrading, a persistent failure is marked honestly, and that marking self-heals once the safe path works again.

## Android

- The existing Android `1.0.10` shell loads this web release automatically; no native binary change is required.
