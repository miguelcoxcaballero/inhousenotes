# Inhouse Notes v5.11.30

## Faster document opening and editor mode changes

- Opening a document builds its page previews lazily instead of rasterizing every page before the editor is usable.
- Visible-page lookup now uses cached page geometry and binary search; it no longer measures every page wrapper in the DOM.
- Zoomed-out views activate only the nearest pages up to the canvas memory limit.
- Switching to Edit avoids forcing a page/layout rescan, and PDF export prewarming is delayed until the editor has been idle.
- PDF import yields to the browser more frequently while normalizing large documents.
- The Android 1.0.10 shell loads this hosted web release; no native APK changes are required.
