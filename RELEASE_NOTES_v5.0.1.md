# Inhouse Notes v5.0.1

This release focuses on making large notebooks feel immediate without changing the PDF-only Drive document model.

## Faster PDF opening

- Drive downloads are passed directly to PDF.js instead of copying the complete PDF before page 1 can open.
- Stored page dimensions are reused and missing dimensions are refined in small background batches.
- Only the visible working set is persisted before the editor opens; remaining pages migrate through idle tasks.
- Timeline restoration and original-byte caching run after the document is already usable.
- Cached clean originals are preferred before inspecting or reconstructing an embedded original.

## Faster stroke analysis

- Compressed stroke metadata and timeline JSON are inflated and parsed in a Web Worker.
- Parsed embedded metadata is cached by payload signature for fast repeated access.
- Remote collaboration pulls use the same off-main-thread stroke parser.

## Faster Drive saving

- The PDF for the next content revision is prepared shortly after the pen or touch gesture ends.
- A complete prepared PDF is cached by document session and content revision, so autosave and exit reuse the exact same blob.
- PDF builds are serialized to prevent duplicate background and foreground builds.
- The Drive upload profile now matches the existing incremental page cache and produces a smaller, faster sync PDF while retaining exact vector stroke metadata.
- The idle window before save preparation is shorter, so a quick “write, then leave” action normally finds the PDF already built or uploaded.

## Data integrity

- Revision checks, remote merging, session validation, Drive hydration, timeline snapshots and PDF-only storage remain enabled.
- Any edit or remote merge invalidates a prepared PDF immediately, preventing an older revision from being uploaded.
