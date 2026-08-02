# Inhouse Notes v5.4.2

This release keeps collaboration responsive when a device changes network.

## Fast network migration

- A Wi-Fi, mobile-data or browser-online transition immediately probes every open peer route.
- ICE gathering restarts on the new interface while the existing route remains usable.
- A responsive route is preserved and receives a forced current-state delivery.
- A dead route is replaced after a 550 ms probe instead of waiting for the normal multi-second watchdog.
- Drive signalling is polled in a short recovery burst so both sides see replacement offers quickly.
- Edits made during migration remain in the current document state and are force-delivered when the first replacement channel opens.

## Live handwriting

- Pen and highlighter points continue to stream while the stroke is being drawn.
- The completed stroke remains the recovery source if a preview packet is lost during migration.
