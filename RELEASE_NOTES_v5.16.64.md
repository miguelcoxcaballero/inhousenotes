# v5.16.64

Smoother zoom, scrolling and editor interactions.

- Give touch, wheel, keyboard and editor gestures the same immediate background-work interruption as pen input. Pause PDF/checkpoint workers, uploads, hidden sync and Android background sync once when interaction starts; extend one idle deadline on subsequent input.
- Keep background work paused while a pointer remains held, during pinch/pan, inertia, page dragging and selection transforms. Do not run a forced visible-page/canvas refresh in the middle of a held gesture. Resume native synchronization after cooldown even when no further input occurs.
- Reuse viewport geometry across wheel/pinch/pan, preview-range queries and the fast-scroll control. Defer canvas resolution changes until interaction settles and apply inertia transforms in the current animation frame.
- Keep cached page previews visible while deferring new rasterization. Cancel active preview PDF work on interaction, retain its queued page, and regenerate when idle.
- Release viewport pointers and pending motion on focus loss, document changes and Home navigation, preventing an interrupted gesture from leaving saving paused indefinitely.
- Regenerate web/offline and Android bundled assets; frozen installer unchanged.

Validation: browser tests for actual touch/wheel/keyboard interruption, stationary held pointers, geometry reads and canvas resize counts, preview retention/recovery, native sync resumption and focus loss; pen, Home checkpoint, offline previews and processing lifecycle regressions; unit and release checks. A controlled 50-frame wheel/pinch sequence used two viewport layout reads and zero canvas resizes during interaction. This is not a device FPS guarantee.
