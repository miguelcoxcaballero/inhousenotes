# Inhouse Notes v5.9.10

## Production reliability and security

- Prevents a blocked IndexedDB upgrade in another tab from freezing document opening; the app now falls back quickly and retries storage later.
- Makes page checkpoints atomic from the user's perspective by saving page bodies before publishing their metadata and retaining edits made during an in-flight save.
- Makes remote merges and close-time checkpoints atomic too, so an interrupted page write cannot expose mismatched metadata or a partially persisted collaboration update.
- Recovers cleanly from failed or synchronously rejected storage transactions instead of leaving saving locked or losing queued stroke operations.
- Bounds Android update checks, Calendar requests and PDF metadata workers so a failed dependency cannot keep the interface waiting forever.
- Shares concurrent Google token refreshes so one request cannot overwrite another callback, and cancels stale authentication after sign-out.
- Bounds PDF assembly, image decoding and canvas encoding so a damaged image or worker cannot leave opening, scanning or export permanently busy.
- Makes scanner configuration and PDF export fail safely, with a visible retryable error instead of a stuck or unhandled operation.
- Prevents overlapping exports from corrupting the Android hand-off and bounds the native PDF encoding step.
- Rejects oversized or explosively compressed embedded PDF metadata and attachments, and never repeats a timed-out worker parse on the UI thread.
- Scopes PDF metadata caches to the active document and fingerprints the full payload distribution, preventing same-sized edits from reopening stale strokes or timeline data.
- Releases temporary Blob URLs after failed, superseded, or large PDF imports instead of leaking memory across repeated opens.
- Safely renders Drive folder names, Drive bin file names and scanner upload names, and allowlists persisted calendar-panel markup, without interpreting untrusted content as active HTML.
- Handles failed background page loads and lifecycle Drive saves without unhandled promise rejections.

## Android

- The existing Android `1.0.10` shell loads this web release automatically; no native binary change is required.
