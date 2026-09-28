# Inhouse Notes v5.11.34

- Timeline history remains compressed while the editor is in use and is restored only when Timeline is opened, avoiding a large post-open inflate and JSON parse on the main thread.
- Deferred legacy page migration now works one page per turn, waits through real user activity, and yields to local/Drive saves rather than competing with pen, pan, and zoom input.
- PDF repair and clean-original swaps now wait for an extended idle window, preventing a second PDF.js reopen from interrupting the newly opened editor.
- Legacy eraser-cover strokes are no longer cloned or redrawn while clean-PDF recovery is pending; if recovery fails, masks are created lazily for loaded pages.
- Open-cache records no longer duplicate raw stroke arrays alongside compressed metadata, reducing IndexedDB write and memory pressure.
- Added regression coverage for lazy Timeline restoration and migration yielding under active use.
