# Inhouse Notes v5.8.3

This patch publishes Android 1.0.9 and activates the detailed mandatory-update progress introduced in v5.8.2.

- Publishes the signed Android 1.0.9 APK as a versioned GitHub release.
- Records the verified APK size in MB so percentage calculations remain available if a server omits its content length.
- Streams real byte counts from the Android updater to the progress card.
- Points both the mandatory updater and the website download button to Android 1.0.9.
- Keeps the update gate locked until Android opens its installation confirmation.
