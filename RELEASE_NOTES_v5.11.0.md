# Inhouse Notes v5.11.0

This release brings the complete Inhouse Scanner workflow into the document editor.

## Scan pages inside a document

- **Scan Page** now appears directly below **+ Photo** in Manage Pages.
- Capture a new page with the rear camera or select one or more photos from the gallery.
- The scanner keeps its automatic processing, live processing previews, manual and automatic crop tools, pinch zoom, page reference/stencil mode, page deletion, and drag-to-reorder workflow.
- Processed pages can still be exported as a standalone PDF and the printable scanner stencil remains available.

## Reliable document insertion

- **Add to document** waits until every selected image has finished processing and then inserts the complete scan in the chosen order.
- Multi-page scans are applied as one atomic page-structure operation after the current page, so an interrupted handoff cannot leave a partial batch.
- Existing pages, strokes, page identities, and their order are preserved while the IndexedDB page store is shifted safely for the whole batch.
- New scan pages use the normal local checkpoint, Drive save, Timeline, and collaboration metadata paths, so they behave like every other document page on other devices.

## Embedded and mobile-safe

- The scanner runs as a same-origin full-screen editor, keeping its layout and processing code isolated from the notes editor.
- Parent/frame messages require the exact origin, frame, and one-time scan session identifier before page data is accepted.
- Page count, image size, and total batch size are bounded before data is imported.
- Closing the document, entering read-only mode, or switching documents invalidates the active scanner session.

The Android native wrapper remains version 1.0.10 because it loads this web release directly and no native code or signed APK content changed.
