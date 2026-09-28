# Inhouse Notes v5.11.38

## Interactive collaboration under load

- Peers negotiate a separate reliable, ordered interactive data channel for pen/eraser updates and health messages. Full-document snapshots stay on the original channel. Capability negotiation preserves compatibility with older clients; an unavailable interactive stream falls back without tearing down the peer.
- Bulk backpressure is limited to roughly 64 KB instead of 1 MB. A blocked queue no longer treats an elapsed timeout as permission to enqueue more data. Waiters clean up on timeout, drainage, or channel closure.
- Incoming peer data counts as evidence that a probed route is alive, even if an exact pong is delayed. A successful routine watchdog probe no longer forces another complete document transfer.
- Reasserting an already-running collaboration session no longer immediately scans Drive for every stroke and snapshot. Scheduled discovery and explicit network recovery remain active.

## Save continuity

- Pointer input pauses PDF preparation cooperatively instead of aborting and discarding it. Version and session validation still reject an outdated result; page structure mutations explicitly cancel the export before acquiring its lock.
- JSON responses from Drive, including signalling and revision checks, retain their timeout and cancellation until the response body has finished, not only until HTTP headers arrive. Successful binary downloads retain streaming behaviour.

## Verification and limits

Five repeated real-WebRTC tests with three isolated browser contexts passed with 100 ms simulated Drive-request latency. Two live stroke-preview batches arrived in 80–103 ms while the primary channel's send queue was deliberately delayed by two seconds. These are controlled desktop tests, not phone or internet latency guarantees.

The existing no-server architecture still depends on Drive signalling and direct ICE connectivity. This release does not add a relay or guarantee a sub-second connection on every network. Both devices must load this release to use the separate interactive channel.

The Android wrapper is unchanged; it loads the hosted runtime without requiring a new APK.
