# v5.16.27

Fix in-app update installation.

Publish Android 1.0.27 / 28 on channel A after verifying its exact public APK,
package identity, unchanged signing certificate against 1.0.26, the update
provider's own UpdateFileProvider class and SHA-256 in CI (run 39425069887).
Channel B is unchanged. Apps on 1.0.26 and older carry the provider bug in the
APK: install 1.0.27 once with Download APK, then updates work from the app.
