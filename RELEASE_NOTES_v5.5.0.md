# Inhouse Notes v5.5.0

This release makes live collaboration recover faster, prevents several save
races, and replaces the ambiguous coloured dot with a clearer document-status
indicator.

## Faster, self-healing peer-to-peer connections

- A network change can now be recovered by either endpoint after first contact.
  The system no longer waits for one predetermined device to notice the broken
  route.
- Simultaneous recovery offers use deterministic glare resolution, so two
  devices reconnect instead of leaving one another stuck in `connecting`.
- Signal polls requested while another Drive-comments request is running are
  queued and drained immediately afterwards rather than silently discarded.
- A transient failure while answering an offer leaves that offer retryable.
- ICE gathering now has a 2.6 second normal ceiling and a 900 ms fast-recovery
  ceiling, with a short candidate-settle window that still allows STUN routes to
  arrive.
- Failure while creating the RTC data channel can no longer leave a phantom
  peer blocking later retries.

## Safer saving

- A local/Drive content-version mismatch is always treated as pending work,
  even if a transient path cleared an intermediate dirty flag.
- Leaving an interaction re-arms Drive sync whenever the uploaded version is
  behind, closing a race where a change could otherwise wait for the next edit.
- A forced or scheduled Drive save restores the dirty state from the version
  invariant before deciding whether there is anything to upload.

## Clearer document status

- The old colour-only dot now uses meaningful symbols for up to date, saving,
  uploading, receiving and error states, with a full accessible label.
- The status panel separately reports this device, the live connection, each
  collaborator and the durable Drive copy.
- Direct, reconnecting and solo states are reported explicitly, including peer
  count and latency when available.
