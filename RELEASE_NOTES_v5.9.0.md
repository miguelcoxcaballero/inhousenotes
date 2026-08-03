# Inhouse Notes v5.9.0

## Faster document opening

- Keeps a bounded local cache of the latest Drive-confirmed PDF for recently opened documents.
- Reuses the already-decoded stroke metadata on subsequent opens, avoiding both a Drive download and repeated `STROKES_Z` analysis.
- Validates the exact Drive revision immediately in the background and merges newer remote work without blocking the editor.
- Refreshes the fast-open cache after every successful Drive upload, so local reopening stays aligned with saved changes.
- Limits the cache to the six most recently used documents.

## Android launch polish

- The bundled launch shell now occupies the full native viewport and centers the Inhouse Notes mark both vertically and horizontally.

## Android build

- Android version: `1.0.10` (`versionCode 11`).
