# Inhouse Notes v5.11.49 · A

Redesigned update popup.

The shared updater now follows Material Design 3: a flat bottom sheet with the new version, its main change, and Later / Update. Older changes and the channel are under Details. Settings show the A/B channel as a plain list. Channel rules, the 14-day reminder and the version A confirmation are unchanged.

The Android APK remains 1.0.12; the popup ships with the web app. Existing regression coverage remains: BroadcastChannel reconciliation, cached document hashes and real WebRTC tests.
