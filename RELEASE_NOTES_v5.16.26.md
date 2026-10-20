# v5.16.26

Fix in-app update installation.

Android keeps one provider instance per class. Capacitor registers androidx
FileProvider for .fileprovider and the shared updater registered
.update-provider with the same class, so that instance rejected the update
URIs and Android's installer could not read the APK ("Android could not start
the installation", or an installer that closed on its own). The shared updater
(acd1c37) now registers .update-provider with its own UpdateFileProvider class,
and the signed-APK check requires it.

The 5.16.23 web routing through installAppUpdate is reverted: the shell
rewrites that method to call installInhouseUpdate, so it changed nothing.

Prepares Android 1.0.27 / 28, verified against 1.0.26. Apps on 1.0.26 and
older carry the bug in the APK: install 1.0.27 once with Download APK, then
updates work from the app. Channel A stays on 1.0.26 until the new APK is
public; channel B is unchanged.
