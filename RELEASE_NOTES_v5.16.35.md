# v5.16.35

Offline uploads, stable recents and fast scrolling.

- Preserve HTTP status codes through the real Drive adapter so reserved offline
  IDs returning 404 proceed to creation instead of retrying forever.
- When Drive v3 does not expose an ETag, retrieve the v2 JSON ETag with its
  revision number and write through the same conditional API. Refuse mismatched revisions and retain If-Match protection,
  merge recovery copies and generation checks throughout.
- Put waiting/uploading/retry feedback on each document card, removing the
  global queue counter. Show upload failure explanations in save details and
  refresh pending badges after successful foreground saves.
- Cached and network Recents share alias deduplication and deterministic ordering.
  Local opening times take precedence over background upload timestamps.
- Add a draggable fast-scroll thumb for Home, folders and the editor, adapted
  from Inhouse Photos mobile _CircularThumb in timeline/scrubber.widget.dart:
  48 px height, rounded left edge, paired white arrows and elevation. Keyboard
  navigation and reduced-motion preferences are supported.

The existing Android APK loads the updated web assets. No installer or channel catalog
changes. Offline/native asset hashes regenerated. Browser regressions cover
real-adapter 404 creation, missing ETags, concurrent revisions, draining eight
queued documents, stable recents, card progress and fast-scroll interactions.
