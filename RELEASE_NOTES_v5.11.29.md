# Inhouse Notes v5.11.29

## Fluid editor navigation

- Pan and pinch cache page bounds and viewport geometry for the duration of a gesture, removing a full-document page scan and layout read from every animation frame.
- Canvas resolution changes and visible-page reconciliation no longer run during active pan/pinch frames; they are reconciled once movement settles.
- Gesture activity refreshes its idle timer without repeatedly clearing/recreating it or re-running save interruption work on every pointer sample.
- The Android 1.0.10 shell continues to load the hosted web release; no native APK changes are required.
