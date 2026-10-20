# Inhouse services updater

## STOP: the Android installer is FROZEN

The Android installer works and is frozen at tag `installer-known-good` (verified on a real Samsung phone on 2026-10-20). Do not edit, refactor, reformat, rename, move or delete `android/PhotosUpdateInstaller.kt`, `android/UpdateFileProvider.java` or `tools/register_android.py`, their vendored copies in the apps, or the guards listed in `FROZEN_INSTALLER.md`. Do not move or delete tag `installer-known-good` or branch `frozen-installer`. Never register `.update-provider` with `androidx.core.content.FileProvider`, and never replace `ACTION_VIEW` with PackageInstaller sessions or `ACTION_INSTALL_PACKAGE`: each of those broke in-app updates. Only the user, in their own words, can unfreeze it. Read `FROZEN_INSTALLER.md` first.

Mandatory guidance for every AI and developer. This repository is the canonical updater for Inhouse YouTube, Instagram, Notes, Photos, Read and Learn (the `APPS` list in `tools/sync.py`).

- Every new app change and automatic publication goes to **A** unless the user explicitly requests B. A receives **all A and B releases**; B receives **only explicitly authorized B releases**.
- Publish or promote to **B** only after an **explicit request from the user**. Passing tests, elapsed time, a scheduled job or an AI's judgment is not authorization. B is the reviewed release channel; never describe it as a guarantee of zero bugs.
- Add a tiny user-facing changelog to every release: preferably 2–10 words, at most 120 characters. The updater shows at most three entries and a compact skipped-version count.
- Never duplicate or independently fix updater code in a consumer app. Edit this repository first, test it, commit it, then run `tools/sync.py` for ALL six apps. Commit the lock and identical shared files in each app. App-specific configuration and host wiring are the only allowed differences.
- Every app must include these rules in its own `AGENTS.md`, `UPDATER.md` and updater integration comments so an AI working only in that repo knows where to look.
- Updates are always optional. Later defers automatic prompts for 14 days per channel; settings and manual checks remain usable. Changing channels never uninstalls, clears data or downgrades the APK.
- Use monotonically increasing Android versionCode across both channels, the existing package ID and permanent signing key. Never commit signing material. A B release may promote an existing A release or publish a new verified APK directly; either requires explicit user authorization. Never replace the binary or immutable metadata of an already published version.
- Channel catalogs live in each app's `updates/A.json` and `updates/B.json`. The publisher mirrors every B release into A using the same exact APK and metadata, then selects the highest global versionCode in each catalog. A newer B supersedes an older A for A users; mirroring an older B must preserve a newer A as A's latest.
- Clients read the selected channel's catalog, with same-channel GitHub contents recovery only. Do not use generic GitHub latest or let B fall back to A. B inclusion in A happens in the publisher, so existing clients receive it without an APK update.
- Do not publish a catalog before its exact APK is publicly downloadable with matching version, hash, size, package and signature.
- Current pre-channel releases are not automatically promoted to B. B starts empty until the user explicitly requests a B publication or promotion.

Read `README.md`, `docs/PROTOCOL.md` and `docs/PROVENANCE.md` before changing the updater.

- **B is the default update channel.** A requires an English development warning, explicit acknowledgement and slide confirmation. Legacy implicit A preferences migrate to B; choosing B clears A consent. This does not authorize publishing or promoting a B release.
