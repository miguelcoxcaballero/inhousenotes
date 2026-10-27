# v5.16.51

Keep ink visible while zooming and repainting.

- Preserve the last complete canvas bitmap during backing-resolution changes. Zoom refinement no longer exposes a cleared canvas while dense ink is rendered in slices.
- Coalesce identical pending paints instead of cancelling and restarting them. New ink, dimensions, image readiness and remote erase previews still invalidate the paint.
- Route individual long strokes through staged page repainting as well as pages with many strokes.
- Regenerate web/offline/Android assets; frozen installer unchanged.

Validation: pixel checks verify visible ink immediately after resize and during delayed repaint; repaint identity tests verify reuse and invalidation. Pen regression suite verifies input continuity and deferred background work.
