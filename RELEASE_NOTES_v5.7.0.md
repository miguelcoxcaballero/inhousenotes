# Inhouse Notes v5.7.0

This release makes peer-to-peer startup and network recovery substantially faster and more deterministic.

## Faster network changes

- Detects confirmed Wi-Fi/mobile-data interface changes and replaces the obsolete route immediately.
- Uses a lightweight 250 ms peer heartbeat as a portable fallback for browsers that do not expose network-change events.
- Retires a silent route after 900 ms instead of waiting for the browser's multi-second ICE timeout.
- Probes ambiguous network hints with a 220 ms baseline and an RTT-aware safety margin, preserving slow routes that are still responsive.
- Keeps reverse recovery enabled so either endpoint can rebuild the connection.

## Faster first connection

- A newly opened device may initiate immediately, regardless of lexical peer ordering.
- Simultaneous offers, including fast-recovery offers, now resolve to one deterministic winner.
- Encrypted Drive signalling is checked every 220 ms while the document is active.
- The initial SDP is published after a 35-90 ms ICE window; later candidates continue through progressive encrypted batches.
- Recovery poll bursts run within the first 800 ms after an offer.

## Safety and compatibility

- Offline transitions preserve retry state and resume immediately when connectivity returns.
- Drive remains the durable fallback while the direct route is being rebuilt.
- No external processing server or TURN service was added.
- The existing compact 9 px status dot and all current UI behavior are unchanged.
