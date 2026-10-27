# v5.17.7

Faster cold saves and less pen-up work.

- First journal uploads reserve their ID while locating the Drive folder. Both tasks settle under the write lease; failures cannot leave a delayed reservation racing a retry. Existing media updates skip folder lookup.
- Journal and preview startup share a folder lookup per account/store; direct hints bypass a stalled initial search. Deleted preview-folder caches are invalidated so retry can recover.
- Pen-up avoids a redundant full stroke clone. Synchronous journal serialization still freezes all point/style/stamp data before any asynchronous work, with local/history copies unchanged.
- History compaction scores each action once and removes one prefix. A representative grouped fixture drops from 108,000 to 20,000 property reads with identical undo history and thresholds (480 model comparisons).
- Regression tests cover delayed setup, lease lifetime, cancelled folder lookups, account boundaries, late mutation of a 4000-point stroke, and history retention.

Web and Android web assets updated. APK/frozen installer unchanged. Network response time remains outside the local scheduling budget.

An older pen-burst test expected no operation-log writes after 250 ms pauses; that expectation also failed on the published baseline, whose local idle window is 180 ms. It now verifies completed strokes are checkpointed between contacts, zero operation-log writes occur while the pen is down, page snapshots remain deferred, and all six strokes survive reload.
