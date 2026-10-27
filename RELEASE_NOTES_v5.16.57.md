# v5.16.57

Follow pen samples without excess smoothing.

- Render pen paths through their measured points with rounded joins, replacing midpoint quadratic averaging in live ink and cached page paths. The highlighter keeps its existing rendering.
- Preserve handwriting samples at lift instead of running distance filtering and RDP simplification. Previously discarded points cannot be reconstructed.
- Append each new batch directly to the bounded live layer. Remove the provisional curved tail and its six-point finalization threshold; skip paint when there are no new samples. Keep standard canvas contexts and one page commit at lift.
- Existing pen paths are displayed through their stored points as well; documents are not rewritten merely by opening them. PDF/thumbnail rendering shares the same path renderer.
- Regenerate web/offline and Android assets; frozen installer unchanged.

Validation covers measured sharp corners in live and stored raster output, unchanged samples, duplicate batches, pen contact/release, visibility after reopen, persistence, zoom and editor performance regressions. No physical-device latency guarantee.
