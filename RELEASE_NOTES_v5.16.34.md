# v5.16.34

Reliable offline retry and document opening.

- Pending edits in the open document retry after connectivity returns or another
  editor releases the sync lease. A failed startup checkpoint no longer disables
  the outbox runner for the entire session.
- Home shows waiting, preparing, merging, uploading and retry states; the editor
  save indicator now includes offline-outbox uploads. Background worker progress
  is forwarded to the visible app, without inventing upload percentages.
- Local and remote folder refreshes share one deterministic order, including
  pending local aliases and files with identical modification times.
- Opening from Home no longer checkpoints stale pages from the previous editor.
  A broken clean local PDF falls back to Drive; dirty drafts stay protected.
  Failed imports release the opening lock and display a visible error. Optional
  folder classification no longer delays an already decoded PDF.

The existing APK loads this web update. No Android installer or APK channel
catalog changes are required.

Validation: 41 browser tests pass across offline storage, reconnect, worker merge,
Home navigation and save indicators. The five focused regressions were rerun
against the final opening change; 28 relevant unit tests and release smoke pass.
Offline/native asset hashes have been regenerated.
