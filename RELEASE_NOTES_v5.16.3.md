# v5.16.3

Reliable in-app Android installation.

Prepare signed Android 1.0.21 / 22 with shared updater 1.2.4. Preserve the current
attempt through Android permission process death, continue automatically after
granting permission, open consent in the foreground and keep failures retryable.
Update stays in the app; GitHub remains an explicit recovery option.

Require a passing runtime installation check for the exact shared updater
revision before publishing an APK. Android 11 and Android 16 both passed real
permission denial/grant, cancellation, retry and PackageManager version upgrade.
The Android 11 run reproduced and recovered from permission-induced process death.

Web popup checks: 5 passed. Shared web checks: 28 passed. Native policy and catalog
checks: 31 passed. The public catalog stays on its previous verified artifact
until the new signed APK is public and verified. B remains unchanged.
