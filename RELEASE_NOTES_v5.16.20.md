# v5.16.20

Fix in-app update installation.

Publish Android 1.0.24 / 25 on channel A after verifying its exact public APK,
package identity, unchanged signing certificate against 1.0.23 and SHA-256 in
CI (run 39353294277). Channel B is unchanged.

- The shared updater (d313e8a) hands the verified APK to Android's package
  installer again instead of a PackageInstaller session, which failed on
  Samsung phones with "Android could not start the installation".
- Installed 1.0.23 and older apps still carry the old updater: install 1.0.24
  once with Download APK; later updates work in the app.
