# v5.17.3

Faster saves; less idle storage work.

- Advisory peer-discovery updates no longer hold the durable write lease or the live preview sender. A delayed hint cannot freeze subsequent strokes after Drive has accepted the previous revision.
- Hints are serialized and coalesced, preventing an accumulation of requests while discovery metadata is slow. Failures do not change a confirmed save into a failed upload.
- Empty journals check the cross-process outbox every two seconds instead of every 120 ms. New local edits wake the writer immediately, retaining the existing upload rate limit and offline recovery.
- Tests simulate stalled and failed discovery requests, verify consecutive acknowledged revisions, continued unfinished-stroke frames, and prompt wake-up from idle.

Web assets also serve the Android app; this release does not change the APK or its frozen installer.
