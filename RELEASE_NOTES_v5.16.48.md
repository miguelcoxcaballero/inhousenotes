# v5.16.48

Smooth page dragging with edge scrolling.

- Drag a page towards the top or bottom of the page list to scroll continuously, even with a stationary pointer. Speed increases towards the edge and uses elapsed time rather than frame counts.
- A lightweight thumbnail follows the pointer; a single insertion line resolves gaps between pages as well as page bodies. Row geometry is measured once and destination lookup uses binary search.
- Keep the list's scroll position after reordering and use a brief landing animation that respects reduced-motion settings. Remove broad transition-all styling from page rows.
- Capture the active pointer, ignore secondary contacts, and clean up on Escape, cancellation, panel close, window blur/resize, list rebuild or document change. Retain page-ID targeting and the existing durable reorder transaction.
- Regenerate web/offline/Android resource manifests. The frozen installer is unchanged.

Validation: browser tests cover continuous edge scrolling in both directions, dropping with saved ink preserved, touch pointer isolation, cancellation and repeated panel opening.
