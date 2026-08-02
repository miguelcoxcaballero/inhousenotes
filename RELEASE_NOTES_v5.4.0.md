# Inhouse Notes v5.4.0

This release improves direct collaboration and makes the final Drive save more reliable when leaving the editor or closing the app.

## Peer-to-peer collaboration

- Peers that have acknowledged a complete document now receive compact page deltas instead of another full notebook snapshot after every edit.
- Every delta identifies its confirmed base. A mismatched receiver rejects it and automatically requests a complete snapshot, preserving the existing anti-entropy safety model.
- Full snapshots remain the connection, recovery, relay and conflict fallback.
- Snapshot data is immutable while it is queued, preventing later local mutations from changing an in-flight delivery.
- Mobile background transitions pause Drive polling without destroying healthy WebRTC connections.
- Returning to the app gives suspended data channels a fresh ping grace period before reconnecting them.
- ICE candidate gathering is more tolerant of slower mobile and cross-network connections.
- Leaving the editor briefly flushes the latest live state before the peer graph is closed.

## Exit and Drive durability

- Closing from UI Read mode no longer skips local persistence or the pending-upload marker.
- Hidden and closing pages create a durable IndexedDB checkpoint before attempting the final Drive upload.
- Repeated mobile close events reuse the same version checkpoint instead of starting competing IndexedDB writes.
- Full PDFs are no longer sent with browser `keepalive`, whose request-size limit made large notebook uploads fail immediately.
- A normal background upload is attempted while the page remains alive; if the browser terminates it, the durable pending upload resumes on the next visible, online or signed-in session.
- Opening another document waits for the lifecycle save so one document cannot replace another document's pending state.
