# v5.16.54

Restore reliable ink display and writing.

- Revert the v5.16.53 desynchronized page/cursor canvas contexts and direct raw-event paint scheduling after reports of invisible existing and new ink. Return to the v5.16.52 canvas/input path; retain the earlier repaint-continuity and Rust geometry fixes.
- Do not migrate, clear or replace stored documents, stroke journals or offline snapshots. This change addresses the rendering regression; it cannot establish whether a particular user's data was lost or recover data already overwritten elsewhere.
- Add a composited-screenshot regression covering visible ink, checkpoint/reload, identical persisted points and writing again after reopening. Exercise existing pen, zoom, home-save and local-recovery tests.
- Regenerate web/offline and Android bundled resources; frozen installer unchanged.
