# v5.16.41

Pen contact recovery and responsive offline checkpoints.

- End the active pen stroke when Android delivers hover after a missing release. Never add the hover position to the ink.
- A new pen contact starts a separate stroke, including changed pointer IDs. Document-level release/cancel events also close a stroke whose canvas lost its capture.
- Avoid collecting coalesced pen samples twice per movement.
- Build offline snapshots in short, yielding batches, pausing for writing and rejecting a snapshot if its document/session changes. Reuse the cached preview instead of synchronously repainting every stroke during each checkpoint.
- Android uses regenerated web/offline assets. No installer changes or additional relay servers.

Validation includes curved/coalesced pen samples, missed releases, new pointer IDs, outside-canvas release, stationary pens, 3,000-stroke pages, checkpoint cancellation and existing offline/background-upload coverage.
