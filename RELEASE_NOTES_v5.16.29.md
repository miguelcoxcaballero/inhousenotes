# v5.16.29

Test the in-app updater.

Publish Android 1.0.28 / 29 on channel A after verifying its exact public APK,
package identity, unchanged signing certificate against 1.0.27, the update
provider's own UpdateFileProvider class and SHA-256 in CI (run 39440082839).
Channel B is unchanged.
