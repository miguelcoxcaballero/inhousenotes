# v5.16.37

Fix background PDF sync; prioritize manual saves.

- Hidden Home/Android processors render PDF backgrounds without waiting for
  animation frames, which are suspended in hidden views. This unblocks PDF
  reconstruction during merges and export without opening the editor.
- Background selection compares against the original local field register, so
  an accepted remote background is actually applied to the uploaded PDF.
- An outbox merge leaves upload ownership with the outbox instead of scheduling
  a second autosave. Merge failures retain the underlying cause; PDFs without
  editable remote metadata keep both recovery copies rather than overwriting.
- An offline save blocked by another upload retries automatically after the
  lease is released, without relying on another Home refresh or button press.
- A manual save reserves priority across the foreground and background outbox.
  An already-started upload can finish; remaining background documents yield
  before starting another upload, and Save now waits for the lease to release.
- Only folders needed by the selected document are synchronized ahead of it.
- Priority is scoped to the account and document session, renewed while waiting,
  released on completion or navigation, and expires after an interrupted session.
  Automatic background synchronization resumes afterwards.

Validation: browser regressions cover real PDF merges in hidden frames with no
animation frames, both replicas’ strokes, photo backgrounds, conditional Drive
writes without exposed ETags, in-flight upload ordering, priority expiry and
save-status interactions. Offline/native assets
regenerated for the existing Android APK; no installer changes.
