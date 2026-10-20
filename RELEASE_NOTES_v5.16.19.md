# v5.16.19

Fix in-app update installation.

Prepare signed Android 1.0.24 / 25 with the shared updater at d313e8a. In-app
updates failed on Samsung phones with "Android could not start the
installation" because the updater used a PackageInstaller session. The updater
now hands the verified APK to Android's package installer, asks for the install
permission first and confirms success from the installed version on return.
Download integrity, package, version and signer checks are unchanged.

Installed 1.0.23 and older apps still carry the old updater: install 1.0.24
once with Download APK; later updates work in the app. The A catalog stays on
its previous verified APK until the new signed APK is public and verified. B
remains unchanged.
