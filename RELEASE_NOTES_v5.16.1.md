# v5.16.1

Reliable in-app Android installation.

- Keep Update inside the native bridge, including older installed shells.
- Shared updater 1.2.4 opens Android consent from the visible installation screen.
- Resume automatically after granting install permission; keep errors and cancellation visible with an internal retry.
- Recheck SHA-256 while staging each installation and retain installed-version verification.
- Do not report an active native installation as failed when the web view regains focus.

Android 1.0.20 / 21 is prepared for signed CI build. A remains on the previously
verified APK until the new public artifact is verified. B remains unchanged.
Older APKs keep their existing native code until an installation succeeds.
