# v6.2.0

- Faster first load: the app's WebAssembly is split in two. What opens the app and shows Home is 261 KB (gzip)
  instead of 600 KB; the rest (editor, export, sync) downloads in the background right after Home appears, so
  documents open as before. If something is needed earlier, it loads on the spot.
- Web assets updated; Android APK and frozen installer unchanged.
