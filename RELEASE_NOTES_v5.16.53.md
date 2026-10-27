# v5.16.53

Reduce live pen display latency.

- Draw fresh raw pen samples immediately instead of always waiting for requestAnimationFrame. Limit direct updates to 120 per second; retain a frame callback for queued samples and pointermove-only browsers.
- Avoid drawing the same tail twice when raw input is followed by duplicate pointermove events, or when a scheduled frame has no new points.
- Request low-latency (desynchronized) contexts for page ink and the live cursor layer before their first paint. Browsers that do not support the hint keep their usual context.
- Regenerate web/offline and Android bundled resources; frozen installer unchanged.
- Preserve coalesced input, release points, persistence and staged page repaint behavior. Canvas scheduling stays in the browser; Rust cannot replace browser frame presentation.

Validation: real editor pen-input tests cover immediate raw paint, frame-batched fallback, 200 duplicate samples with no extra paints, curved coalesced input, release recovery and offline persistence; repaint continuity tests cover dense pages and zoom. These checks do not establish a physical tablet refresh-rate guarantee.
