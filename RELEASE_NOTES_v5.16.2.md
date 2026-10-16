# v5.16.2

Recover Android installation after permission changes.

- Preserve the current retry across process termination when Android grants install permission.
- Recheck permission and continue without relying on a callback from the old process.
- Keep consent and actionable failures in the foreground; verify the staged APK checksum again.
- Synchronize the same canonical installer revision into all five consumers.

Android 1.0.20 / 21 remains prepared for signed CI build. Catalog A continues to
point to the previous verified APK until the new artifact is published and verified.
B remains unchanged. Runtime regression covers denied permission, granting it,
process death on Android 11, cancellation, retry and an actual version upgrade.
