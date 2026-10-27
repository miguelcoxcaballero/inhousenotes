# v5.17.13

Retry interrupted sync without losing ink or photos.

- Receiving the same edit again now completes any failed page-body or metadata checkpoint before acknowledging it. Early false returns and thrown storage writes retain the pending work.
- A complete local save can repair an interrupted merge without waiting for another peer packet. Old receipts cannot clear that pending state, and document switches keep failures isolated.
- Interrupted multi-page photo edits keep each background register paired with its actual photo. Replay repairs missing photo bytes; mismatched payloads cannot replace a correct photo or publish an inconsistent checkpoint.
- Recovery invalidates stale PDF caches and queues Drive synchronization without claiming a PDF upload succeeded. Healthy identical edits still perform no page or metadata writes.

- Validation: 283 unit checks (including rebuilt native assets), release smoke, 57 integration checks, then 11 focused recovery and 18 photo regressions against the final recovery changes.

Web and Android web assets updated; APK/frozen installer unchanged.
