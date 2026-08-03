# Inhouse Notes v5.9.1

## Faster document opening

- Recent Drive documents reopen from the latest locally cached, Drive-confirmed PDF.
- Their already-decoded stroke metadata is restored without repeating `STROKES_Z` decompression and analysis.
- Drive validates the exact Drive revision in the background; newer collaborative work is merged without blocking entry to the editor.
- Every successful Drive upload refreshes the fast-open snapshot, and storage is bounded to six recent documents.

## Android launch screen

- The bundled native shell now fills the WebView viewport and centers the Inhouse Notes mark exactly on phones and tablets.

## Android update

- Publishes the signed Android `1.0.10` APK (`versionCode 11`, 3,159,597 bytes) as a required update.
