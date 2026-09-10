# Inhouse Notes v5.11.20

## General lag and jank

- Pen strokes were rendered with one Canvas2D `beginPath()`/`stroke()` call per point instead of one call for the whole stroke. On a page with thousands of points this made every full-page repaint (erasing, pinch-zoom, switching pages) cost tens to hundreds of milliseconds of main-thread work. Strokes are now drawn as a single path, matching the line-join style that was already configured but never actually used because every segment reset the path.
- Drawing a stroke or erasing was re-triggering the peer-to-peer signalling poll (a Google Drive comments fetch) on every single pointer-move event, saturating the poll loop for as long as the pen was down. The collaboration session is now only re-asserted once per stroke/gesture instead of on every move, restoring the intended ~2.5s signalling cadence while drawing.
- Raised the memory-pressure cleanup threshold from 50% to 80% of the heap limit; at 50% it stayed permanently triggered on modest devices, discarding PDF page caches the user was about to scroll back to and forcing them to re-render.

## Android

- The existing Android `1.0.10` shell loads this web release automatically; no native binary change is required.
