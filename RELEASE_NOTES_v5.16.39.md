# v5.16.39

Fix Drive uploads from Home.

- Conditional uploads using a Drive v2 ETag now use PUT (files.update). The
  previous PATCH method is metadata-only in v2 and does not support media.
  The v3 upload path retains PATCH. Revision checks, If-Match and recovery
  copies remain in place.
- Connectivity probes now contact Drive instead of a potentially cached or
  intercepted app asset. An authenticated Drive response clears stale offline
  state, and an older failed probe cannot override that newer evidence.
- Tapping a failed document’s upload badge shows the actual error and phase,
  with Copy error and Retry actions. An offline label no longer hides a failure.

API contract checked against Google's Drive v2 discovery document:
https://www.googleapis.com/discovery/v1/apis/drive/v2/rest
files.patch: PATCH, no media upload; files.update: PUT, media upload supported.

Validation: regressions reject PATCH on v2 media endpoints, drain queued uploads,
exercise automatic Home synchronization, preserve concurrent edits and verify
stale-network recovery and error details. Offline/native assets regenerated for
existing Android APKs; no installer changes.
