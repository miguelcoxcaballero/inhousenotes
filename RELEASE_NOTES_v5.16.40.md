# v5.16.40

Recover flattened PDFs and unblock sync.

- Home uploads no longer remain in `merge-unavailable` when Drive contains a flattened PDF without editable collaboration metadata.
- An identical remote/base PDF needs no stroke merge. A genuinely different, non-mergeable remote is preserved as a sibling `recovered Drive version` PDF before uploading local edits. Both originals also remain in local recoveries.
- Recovery IDs survive retries and lost responses. Failed preservation or a changed remote revision prevents replacement. Normal editable conflicts still use the existing merge and conditional write.
- P2P offer/answer writes use the bounded metadata deadline instead of the default 60 seconds. Signalling processes up to four comments concurrently and continues later pages after individual failures.
- No TURN service is configured: restrictive NAT/firewalls can still prevent direct WebRTC. Drive remains the fallback; universal network connectivity is not claimed.
- Android uses the updated web/offline assets. No installer or update channel changes.

Validation: real-PDF hidden Home processor with missing metadata; baseline byte equality; lost recovery response; preservation denied; existing offline/reconnect tests; collaboration and peer transport tests.
