# Inhouse Notes v5.11.54 · A

Reliable Android update results.

Android 1.0.16 (versionCode 17) prepares the shared updater fix for installation results. Opening the Android installer is not treated as a successful update: success must reflect the installed app version. The package and signing key remain unchanged.

The release workflow checks the APK version, package, camera permission and signing continuity against Android 1.0.15 before publishing. It then downloads the public APK, compares its exact bytes and verifies it again before publishing the matching channel A catalog. Channel B remains unchanged.

This release has not been tested on a physical Android device. Native installation behavior still requires device validation. The camera and minimal save interface from v5.11.51 are unchanged.
