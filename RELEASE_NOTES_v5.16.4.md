# v5.16.4

Reliable in-app Android installation.

Publish Android 1.0.21 / 22 on channel A after verifying its exact public APK,
package identity, unchanged signing certificate, SHA-256 and 239 bundled assets.
Channel B is unchanged.

- Update uses the native installer instead of automatically navigating to GitHub.
- Recover the current attempt when Android terminates the app while granting install permission.
- Resume after Settings, present consent from the visible Activity, and retain actionable cancellation/errors with retry.
- Reuse the downloaded APK and verify SHA-256 again before each install session.
- Confirm success only when PackageManager reports the expected installed version.
- Require successful real Android installation tests before APK publication.

Validated: real upgrades on Android 11 and Android 16, including denied/granted
permission, Android 11 process death, cancellation and retry; 28 shared web tests,
31 native-policy/catalog tests and 5 Notes browser integration tests.

The native fixes take effect when this APK is installed; the remotely delivered
web change cannot replace the native code of an already-installed older APK.
No physical Samsung device was available for this validation.
