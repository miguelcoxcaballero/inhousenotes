# Document persistence and transport

## Ownership

- `runtime-core-v5.js`: cancellable network retries, coalescing job lanes and a
  bounded, serial worker RPC client. These mechanisms do not know editor state.
- `app-v5.js`: editor adapters. Local checkpoints and Drive saves have separate
  lanes, keyed by document session. Local page bodies and operation logs commit
  before checkpoint metadata. Cloud saves still reconcile the latest remote
  revision and use conditional uploads.
- `offline-library-v5.js`: durable offline snapshots. Concurrent requests share
  the next checkpoint, with account/document/session checks before persistence.
- `offline-sync-v5.js`: the shared foreground/background outbox runner, protected
  by a lease and conditional writes. A failed upload does not acknowledge edits.
- `document-processing-v5.js`: PDF handles scoped to a worker generation and
  explicit disposal. A worker restart invalidates every old handle.
- `document-processing-worker-v5.js`: PDF and image operations; object ownership
  checked before mutation. Loading another source no longer deletes open PDFs.
- `peer-transport-v5.js`: compatible v1 framing, backpressure, bounded fragment
  reception, expiry and cleanup. It does not decide whether changes are merged.
- `live-collaboration-v5.js`: discovery, peer lifecycle, acknowledgements and the
  editor merge adapter. Receipt is separate from successful merge acknowledgement.

## Invariants

1. A request made while a save is running waits for a subsequent checkpoint.
   Requests in that next batch coalesce; a forced save cannot be weakened.
2. A completed operation from another document cannot mark the current one saved.
3. A processor executes one RPC at a time. The execution deadline starts when
   work begins; queued requests do not time out just because an earlier job runs.
4. A crash, active cancellation or timeout terminates the worker, rejects accepted
   pending work, and invalidates its generation. Queued cancellation affects only
   that request. The next request can create a fresh worker.
5. The processor accepts at most 64 queued jobs and 32 open PDF documents. PDF
   callers must dispose completed documents in `finally`; the source cache owns
   its retained source. Active exports pin that source until completion, so cache
   replacement or document reset cannot dispose a source still in use.
6. P2P flow control applies before the start frame and every chunk. The editor
   caps each peer's send queue at four deliveries. Oversaturation triggers the
   existing retry/full-state reconciliation path.
7. Incoming transfers reserve at most 50 million UTF-16 code units in aggregate,
   at most 16 transfers globally and four per peer. Invalid lengths, inconsistent
   duplicates and overruns are rejected. Partial transfers expire after 60 seconds
   and are released on peer/session closure. A single wire frame is capped at 2 MiB
   of code units before parsing; control messages retain the existing schema.

## Compatibility and removals

Wire v1, PDF metadata, IndexedDB records, merge stamps and deletion tombstones
remain readable. Old document migration is data compatibility, not dead code.
The duplicated Drive force/keepalive flags, recursive local-save draining,
recursive offline-capture draining, ad-hoc worker pending map/timers, inline P2P
fragment assembler and implicit deletion of all prior source PDFs were replaced.

## Validation

Unit tests inject storage failures, overlapping saves, worker crashes, stale
responses, cancellation, timeouts, queue saturation and malformed fragments.
Browser tests cover two simultaneous source PDFs, foreign/stale handles, exact
rendered pixels, metadata, cached exports, offline conditional uploads and merging
in the Android headless worker. Speed measurements from a desktop test runner are
not Android-device latency guarantees.

## Leaving the editor

Home requires local durability for actual changes, including the account-scoped
per-document snapshot used by the outbox. It does not wait for Drive authentication,
network uploads or peer acknowledgements. Storage failure keeps the editor open.
Exit jobs recheck their document session after waiting so they cannot save the next
document. An unchanged cached document can leave immediately; cache materialization
and an expired Drive token alone do not indicate unsaved content.
