# v5.17.10

Faster live startup and queued ink delivery.

- Live preview folder lookup and ID reservation run together, holding the writer lock until both settle. Existing files skip folder lookup. The first upload includes the newest complete geometry accumulated during startup, instead of an obsolete empty frame.
- Successful journal uploads wake the next pending pass after releasing the storage lease, while retaining the 120 ms minimum between write starts and existing idle/error backoff. A controlled-clock regression sends the next queued upload at 130 ms instead of waiting for the 240 ms periodic tick.
- Retired journal reads cannot reactivate themselves through a late response, error or queued merge. Genuine predecessor revisions can reactivate a fresh read generation.
- Recovery distinguishes ownership: uploading another tab's pending journal keeps its current and future edits readable, and recovering another document does not poll it as part of the open document.
- Regressions cover concurrent startup, failure/cancellation, immutable in-flight frames, latest startup geometry, payload limits, retired-read races, shared-store recovery, rate limits and retry behavior. Preview timing tests use simulated transport, not a promise about Google Drive network latency.
- Validation: 56 journal, preview and native-asset unit checks; 17 browser cases covering independent replicas, photos, recovery, immutable stroke capture and previews before pen-up; release smoke checks passed.

Web and Android web assets updated; APK/frozen installer unchanged.
