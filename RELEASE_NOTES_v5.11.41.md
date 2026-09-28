# Inhouse Notes v5.11.41

- Fixed idle document synchronization for a newly opened local tab. BroadcastChannel now requests the current document on join, so two windows reconcile their existing strokes without requiring another edit.
- A joining device's targeted state request no longer narrows a pending local edit to that device only. Existing peers and local tabs still receive the update.
- Healthy connections now compare cached document hashes in their heartbeats and repair divergent state without reconnecting. Failed applies request a retry immediately; a missing receipt retries on the same route once queued bytes have drained. Snapshot preparation keeps a low-rate recovery timer after its fast retries instead of waiting for the next edit.
- Final remote pen strokes invalidate the prepared PDF and document generation and schedule durable saving, even if the full safety snapshot never arrives. Updates merged from Drive are relayed to live peers too.
- Preserved already-shared page IDs during initial synchronization. Unnecessary legacy ID changes could strand pen and eraser packets in flight.
- Added real WebRTC regression coverage for late local tabs, final-stroke durability, missing transfers, different starting versions, simultaneous edits and offline reconnection. Signalling is simulated in these browser tests; they do not certify every mobile network or Google Drive account.

The Android shell is unchanged; this is a hosted web/app update. After saving pending changes, reload both clients to use the same version. Connection and transfer times still depend on network conditions and document size.
