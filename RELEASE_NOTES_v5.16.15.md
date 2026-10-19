# v5.16.15

Folder lid and zoom move as one.

- The lid no longer turns on its own before the zoom begins. Lid, zoom and interior fade now start together on one native timeline with a shared smooth acceleration, so there is no hitch between the two phases.
- All scene preparation happens before the first visible frame, never in the middle of the motion. Opening is 600 ms end to end.
- Web only. The Android A catalog remains on the previous verified APK; no new native build is required.
