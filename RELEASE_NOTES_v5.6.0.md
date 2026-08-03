# Inhouse Notes v5.6.0

This release redesigns peer-to-peer negotiation to be both faster and more
resilient on slow, changing or unusual networks.

## Progressive WebRTC negotiation

- Offers and answers are published after a short 80–180 ms initial window
  instead of waiting up to several seconds for ICE gathering to finish.
- Host, local-network and public STUN candidates that arrive later are sent in
  encrypted Drive reply batches and applied to the existing connection.
- Candidates received before the remote answer are held safely and applied as
  soon as the remote description is ready.
- Repeated Drive polls deduplicate candidates, so the same route is never added
  twice.

## Recovery and reliability

- Failed candidate batches remain queued and retry with bounded backoff.
- Candidate activity extends a connecting peer's deadline, preventing a useful
  negotiation from being destroyed by a fixed timeout while still capping a
  genuinely stuck attempt at 30 seconds.
- Posting an offer starts an immediate answer-poll burst rather than waiting for
  the normal signalling interval.
- The encrypted Drive PDF workflow remains the durable fallback whenever a
  browser, firewall or carrier network does not permit a direct WebRTC route.

The save-status indicator keeps the original compact 9 px dot design restored
in v5.5.1.
