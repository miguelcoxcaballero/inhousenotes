# v5.16.28

Test the in-app updater.

Prepares Android 1.0.28 / 29, verified against 1.0.27, to check that 1.0.27
(the first app with its own UpdateFileProvider) installs updates from the app.
Channel A stays on 1.0.27 until the new APK is public; channel B is unchanged.
