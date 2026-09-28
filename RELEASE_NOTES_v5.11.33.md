# Inhouse Notes v5.11.33

- Removed unnecessary cloning of up to three complete stroke pages while opening a document; only the first hydrated page prepares compatibility covers, and older-stroke normalization yields in small batches.
- Drive document opens no longer trigger a background PDF rebuild/upload just because the import path initialized compatibility metadata. Legacy PDFs are migrated the next time the user makes and saves a real edit.
- Changed Drive PDF writes to resumable uploads with server-confirmed byte offsets, bounded retry, and progress reporting, so a dropped mobile request continues from the acknowledged range instead of restarting the entire file.
- The save details now distinguish PDF preparation, revision verification, and actual Drive upload progress, rather than labeling every in-flight phase as “Uploading”.
