# v5.17.18

Faster edit preparation; restore PDF photo sync.

- Capture indexes its immutable document fields once instead of scanning every page's fields for each changed page or stroke chunk. Operation order and serialized bytes remain identical.
- A focused adapter CPU comparison with 512 pages and 32 changed pages fell from 22.17 to 4.32 ms. Browser coverage verifies one full field traversal instead of 19 for 18 operations, without dropping any fields or strokes. These are preparation measurements, not network latency promises.
- Stable PDF pulls again accept photos embedded in the PDF when their original bytes are absent from its editable JSON. Authorization is private and bound to the exact PDF page, field record, merge call and document session; raw journal/P2P payloads still require matching photo bytes.
- Retrying an interrupted PDF checkpoint also restores a missing photo when the page's other data was already merged. Existing PDF raster storage behavior is unchanged.
- Validation: 59 distinct browser cases covering context projection, stroke batching/sync, photos, interrupted checkpoints, offline Home uploads and PDF authorization boundaries; native-asset verification and release smoke.

Web and Android web assets updated; APK/frozen installer unchanged.
