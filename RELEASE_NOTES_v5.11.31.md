# Inhouse Notes v5.11.31

- Removed the full-document PDF prewarm when entering Edit, so opening a file no longer launches a competing render across every page as the user starts working.
- Kept legacy stroke-ID migration on the visible working set first and moved the remaining pages into the existing idle migration path. Legacy IDs use a stable document/page/index identity instead of hashing every point in the stroke.
- Reused unchanged PDF overlay cache entries by page identity and mutation generation, avoiding a full stroke-point fingerprint pass for every untouched page on each Drive save.
- Preserved the existing PDF metadata, vector-stroke recovery, stable Drive revision checks, and full-document upload behavior.
