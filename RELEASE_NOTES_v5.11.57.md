# Inhouse Notes v5.11.57 · A and B

Open the GitHub release page.

Download APK now opens the specific release page on GitHub, where the user selects the APK under Assets. This avoids starting the binary download immediately in a browser tab, which remained stuck at 100% on the reported Android device. The destination is derived from the selected channel’s validated APK URL, never from GitHub’s generic latest release.

Android 1.0.16 (versionCode 17) remains the exact verified APK offered in A and B. Binary, signature and catalog metadata are unchanged. The installed app version remains the source of confirmation after an update.

The shared native cancellation, retry and self-update flow passed on an Android 15 emulator. This release has not been tested on a physical Android device.
