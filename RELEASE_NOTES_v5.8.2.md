# Inhouse Notes v5.8.2

This release adds detailed, real-time progress to mandatory Android updates.

- Shows a clear download percentage and a smooth progress bar.
- Displays transferred and total APK size in MB.
- Streams real byte counts from Android while the APK is written to local update storage.
- Throttles progress callbacks so the UI stays fluid during fast downloads.
- Moves to exactly 100% before Android opens the installation confirmation.
- Improves the update card while keeping the existing Inhouse Notes visual language.
