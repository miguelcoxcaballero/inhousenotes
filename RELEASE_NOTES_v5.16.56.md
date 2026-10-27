# v5.16.56

Preserve pending edits and isolate upload receipts.

- Bind resumable sessions to the exact immutable PDF Blob. Equal file sizes and logical document hashes no longer allow a different binary to resume or accept completion of an older PDF. Restart expired 410 sessions as well as 404 sessions.
- Preserve dirty intent before starting a save, including when version counters match. A pending draft must not take the no-upload success path.
- Remote pulls acknowledge only the content version captured before the pull, not edits made during its asynchronous processing.
- Include durable outbox dirtiness in save presentation. Surface pending upload errors as errors with their available detail.
- Atomically keep one last-upload editable recovery draft per document before replacing a confirmed snapshot with its PDF. Stale acknowledgements cannot clear a newer generation.
- Regenerate web/offline and Android resources; frozen installer unchanged. These fixes do not claim recovery of previously lost data or identify the user's unspecified upload error.
