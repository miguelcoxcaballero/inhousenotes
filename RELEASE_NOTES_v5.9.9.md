# Inhouse Notes v5.9.9

## Lighter PDFs, faster opening

- Keeps strokes and inserted images at the same render resolution while replacing transparent full-page layers with pixel-aligned cropped overlays.
- Enables compressed PDF object streams and reuses identical template or custom backgrounds across pages instead of embedding duplicate copies.
- Uses the compact text-preserving PDF path for native notebook pages as well as imported PDFs, while retaining the existing safe raster fallback.
- Shows the first working pages before IndexedDB persistence finishes, then stores the remaining data safely in idle batches.
- Skips legacy duplicate analysis for PDFs already normalized by this release and avoids unnecessary clean-original recovery for template-only notebooks.
- Reopens the document just saved from its exact in-memory uploaded Blob when available, avoiding another Drive download or local database read.

## Android

- The existing Android `1.0.10` shell loads this web release automatically; no native binary change is required.
