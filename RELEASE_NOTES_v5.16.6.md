# v5.16.6

- Returning Home waits for a durable local copy, not a Drive upload or sign-in. Pending edits stay in the per-document outbox and retain the existing merge protections. A failed local checkpoint keeps the editor open.
- Opening an unchanged document stays green: preparing its local page cache or renewing Drive access is not an edit. Explicit saves finish deferred page writes before checking the checkpoint.
- Late exit uploads cannot mark the next document dirty. Local-only checkpoints no longer create pending-upload markers for unchanged files.
- Removed hover action bars. The existing three-dot menu now uses the same icons, including the offline-copy state.

Web and Android-hosted app assets updated; the signed Android APK remains 1.0.21. No Android channel catalog changes.
