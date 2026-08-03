# Inhouse Notes v5.9.7

## Faster, more reliable peer-to-peer

- Adds a short directed mailbox in Drive metadata so each device can locate the other device's encrypted WebRTC offer without scanning the document's comment history.
- Reuses the metadata response already fetched every 600 ms, then polls only the exact encrypted offer comment for its answer and trickled ICE candidates.
- Installs the ICE listener before gathering begins, preventing Android WebViews from losing the first host candidate and waiting for the full timeout.
- Keeps a failed offer retryable instead of marking it consumed before its encrypted answer has actually reached Drive.
- Learns the active WebRTC candidate-pair RTT and combines it with the browser network RTT to give slow, valid routes enough time to answer health probes.
- Retains paginated encrypted-comment discovery as an automatic compatibility fallback, but runs it less often so it cannot starve the fast requests on a weak link.
- Leaves mailbox entries in place until their short TTL or a confirmed channel open, avoiding a cleanup race that could erase a newer retry.

## Android

- The existing Android `1.0.10` shell loads this web release automatically; no native binary change is required.
