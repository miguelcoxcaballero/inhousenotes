# Inhouse Notes v5.9.8

## Safe, fast return to Home

- Keeps the editor behind a short branded saving screen until the exact latest document content version is confirmed in Drive.
- Runs the durable local checkpoint, Google session refresh and final live-collaboration flush in parallel to minimise the wait.
- Reuses the prepared PDF from autosave whenever it already matches the latest edit, avoiding an unnecessary rebuild before upload.
- Preserves the instant path when there is nothing left to save.
- If Drive cannot confirm the upload, the editor remains open and the local recovery checkpoint stays available instead of showing a potentially stale Home screen.

## Android

- The existing Android `1.0.10` shell loads this web release automatically; no native binary change is required.
