# v5.16.23

Fix in-app updates on Android 1.0.23 and older.

Those apps carry an install-session bridge that fails on some phones with
"Android could not start the installation". The web updater now sends them
through the app's original installAppUpdate bridge, which downloads the APK
and opens Android's package installer (ACTION_VIEW), as Inhouse Photos does.
Android 1.0.24 and newer keep the shared updater's verified ACTION_VIEW path.
No new APK: the fix is served with the web app.
