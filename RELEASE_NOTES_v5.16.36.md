# v5.16.36

Page previews while scrolling; unified add menu.

- Prewarm small previews of PDF and handwritten pages, prioritizing the visible
  area independently of expensive full-resolution rendering. Decode image
  backgrounds before compositing, yield between ink batches, and pause for pen
  input. Reuse previews when releasing canvases and persist them without
  rewriting document strokes. Session, structure and generation guards reject
  stale asynchronous results.
- Replace New PDF with the same plus control on desktop, tablet and mobile.
  Its dropdown offers Create PDF and Upload file, with the existing Drive folder
  picker used to select a destination and filename.
- Selected PDFs are stored durably in the offline outbox before the picker
  closes, retaining original bytes and destination. Other file types use a
  resumable online upload with their original MIME type; failures keep the
  selected file available for retry.

Existing Android APKs receive these web assets. No installer changes.
Validation includes real PDF thumbnail pixels, unloaded ink previews, stale
preview cancellation, desktop/tablet/mobile menus, offline PDF upload queue,
non-PDF resumable uploads, offline navigation and dense editor performance.
