# Inhouse Notes v5.9.6

## Reliable direct collaboration

- Replaces a P2P route only after two progressive health checks, preventing a busy app or one delayed response from destroying a valid connection.
- Lets a visible tab immediately take collaboration leadership from a hidden tab and releases that ownership when a document closes.
- Keeps initial negotiation alive long enough for Drive answers and trickled ICE candidates to arrive, while retaining bounded recovery timeouts.
- Publishes a fresh rendezvous every 1.5 seconds while searching and polls signalling at a sustainable sub-second cadence.
- Gives ICE gathering enough time to include useful candidates in the first offer instead of relying entirely on delayed trickle replies.
- Replays a short realtime backlog of in-progress pen and eraser packets when a peer connects, then sends the authoritative document snapshot as recovery.
- Keeps known peers available longer through temporary presence gaps so reconnection continues without reopening the document.

## Android

- The existing Android `1.0.10` shell loads this web release automatically; no native binary change is required.
