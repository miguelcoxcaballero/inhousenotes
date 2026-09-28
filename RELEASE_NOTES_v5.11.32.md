# Inhouse Notes v5.11.32

- Made the editor interactive before activating/rasterizing the first PDF page; opening defers first-page work until after the editor gets its first paint.
- Deferred recovery of embedded clean-original PDFs until the editor is open. PDF export waits for recovery so an early save still preserves the original and reversible erasing.
- Limited initial legacy-page hydration to the first page and reduced background migration batches to avoid competing with pen, pan, zoom, and tool changes.
- Made transient missing-page save failures checkpoint pending legacy page data and retry, instead of permanently blocking Drive autosave.
- Prioritizes an active Drive retry in the save indicator so an older failure does not keep showing red while a new attempt is progressing.
