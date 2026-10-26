# v5.16.60

Reduce pen-event overhead and redundant frames.

- Read coalesced hardware samples directly, without copying the event array or allocating temporary coordinate objects. Preserve ordering, release samples, foreign-pointer rejection and distinct positions sharing a timestamp.
- Use squared-distance comparisons with the same existing pen/highlighter thresholds.
- Schedule pen drawing and collaboration preview only when new points were appended. Cache the device-specific idle delay once per writing interval instead of detecting the device on every move/raw event.
- Preserve definitive incremental page drawing, captured geometry, background priority and persistence behavior. Regenerate web/offline and Android assets; frozen installer unchanged.

Validation: repeated/frozen coalesced batches produce no extra frames, preview sends, device detections, Array.from copies or Math.hypot calls; existing pen contact, raw/move deduplication, persistence/reopen, fidelity and background priority checks. This reduces input work, not a measured physical-tablet FPS guarantee.
