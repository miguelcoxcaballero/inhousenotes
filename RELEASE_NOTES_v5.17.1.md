# v5.17.1

- Fix PDF saves remaining on Checking Drive during the actual single-request upload.
- Stop our own save-presence writes invalidating the PDF's conditional write token. Retry metadata-only conflicts using the same PDF and a fresh token; a different PDF head still requires a merge.
- Home outbox compares PDF heads when obtaining its v2 write token, so presence/version updates during preparation no longer force an endless rebuild cycle.
- Retain download deadlines and pen cancellation after response headers arrive. A stalled PDF body now releases its request and reports a retryable error instead of hanging checking/preparation forever.
- Verified with complete editor saves, simulated presence races, stalled response streams, and offline outbox uploads. Android installer unchanged.
