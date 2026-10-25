# v5.16.50

Recover local saves without false failure alerts.

- Distinguish superseded page snapshots and page-order locks from storage failures. These checkpoints remain pending and retry automatically instead of showing Needs action.
- Keep retrying genuine storage failures with bounded exponential backoff (up to 30 seconds), rather than giving up after three retries. Retry waits safely through active writing and remains scoped to its document session.
- Do not confirm a full checkpoint when stroke operations failed or new edits arrived during metadata commit. Actual IndexedDB errors remain visible until durable saving succeeds.
- Regenerate web/offline/Android resources. The frozen installer is unchanged.

Validation: local status tests cover recovery after eight failures and session isolation; browser tests inject edits during packing/metadata commit, page-order contention and an actual IndexedDB write exception, then verify the latest ink is persisted automatically.
