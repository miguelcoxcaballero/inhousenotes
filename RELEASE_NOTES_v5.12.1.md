# Inhouse Notes 5.12.1

- Reuse the remote PDF already archived by the offline outbox instead of downloading the entire document again to merge it. Revision checks and conditional writes still prevent overwriting a newer remote edit.
- Verify the headless processor with animation frames disabled, matching a background Android WebView; it preserves both devices’ strokes and explicit deletions with one download and one upload.
- Deployment now checks the public version and application file after Pages updates, using uncached requests from the build runner.

This web update uses Android 1.0.17 (versionCode 18) already offered in A. No additional APK is required. B remains at 1.0.16. Browser tests do not establish speed or scheduling behavior on a physical Android device.
