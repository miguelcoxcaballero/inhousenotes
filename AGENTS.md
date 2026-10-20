# Inhouse Notes release policy

This file is mandatory guidance for any AI or developer editing this repository.

## Version bump required for every upload

Every push or upload to GitHub `main` must increment the public app version,
including documentation-only or small UI changes. Do not publish two different
commits with the same visible version.

The single source of truth is:

```js
const APP_VERSION = 'x.y.z';
```

in the root `index.html`. The home and editor profile menus use
`[data-app-version]` and must display that value as `vx.y.z`. Do not hardcode
the release number separately in either profile menu.

Use semantic versioning:

- Patch (`x.y.Z`) by default for fixes, polish, documentation, and small features.
- Minor (`x.Y.0`) for a substantial group of new user-facing capabilities.
- Major (`X.0.0`) only for an intentionally incompatible release.

## Required release checks

Before pushing to `main`:

1. Increment `APP_VERSION` in the root `index.html`.
2. Confirm every `[data-app-version]` label resolves to the new number.
3. Regenerate any affected standalone or Android artifacts.
4. Run the relevant tests, type checks, and builds.
5. Commit the version bump together with the change it identifies.

After pushing:

1. Verify GitHub `main` points to the new commit.
2. Fetch `https://inhousenotes.com/` without cache and confirm it serves the new
   visible version.
3. Do not report the release as published until both checks pass.

# Inhouse services updater — mandatory shared updater policy

Canonical repository: https://github.com/miguelcoxcaballero/inhouse-services-updater

Every AI/developer working in this app must read this section even without opening the updater repository:

- **The Android installer is FROZEN** at tag `installer-known-good` (verified on a real Samsung phone, 2026-10-20). Never edit, refactor, reformat or delete `vendor/inhouse-services-updater/android/PhotosUpdateInstaller.kt`, `vendor/inhouse-services-updater/android/UpdateFileProvider.java` or `vendor/inhouse-services-updater/register_android.py`, and never register `.update-provider` with `androidx.core.content.FileProvider` or replace `ACTION_VIEW`. `verify.py` fails the build if they change. Read `vendor/inhouse-services-updater/FROZEN_INSTALLER.md`; only the user, in their own words, can unfreeze it.
- **B is the default update channel.** A requires an English development warning, explicit acknowledgement and slide confirmation. Legacy implicit A preferences migrate to B; choosing B clears A consent. This does not authorize publishing or promoting a B release.
- New changes and automatic releases go to **A** unless the user explicitly requests B.
- **A receives all A and B releases; B receives only B releases.** The highest global Android versionCode wins: a newer B supersedes an older A for A users, while an older B never displaces a newer A.
- **B** (reviewed releases) requires the user's **explicit request**. Tests passing is not permission to publish or promote. B may promote an existing A release or publish a new verified APK directly; a previous A publication is not required.
- Every release needs a tiny changelog: preferably 2–10 words, maximum 120 characters.
- Never independently edit updater copies. Change the canonical repository first, test, commit, and sync the SAME revision into YouTube, Instagram, Notes, Photos and Read using its `tools/sync.py`.
- Commit `inhouse-updater.lock.json` and shared files together. Build checks must reject drift. App identity and host integration are the only permitted differences.
- Updates are optional. Later snoozes automatic prompts for 14 days; manual updates remain available in settings. The popup and settings both expose A/B selection.
- Keep APK package and signing key; increment Android versionCode globally across A/B. Switching channels never downgrades or deletes user data.
- Publish each exact verified signed APK before its matching channel catalog. Never replace a published binary or its immutable metadata. Use `updates/A.json` and `updates/B.json`.
- The publisher mirrors every B release and its exact metadata into A, preserving a newer A as latest. Existing clients therefore receive B releases through A without an APK update. B never falls back to A; neither channel uses generic GitHub latest. B initially has no releases until explicitly requested.
- See the central `AGENTS.md`, `docs/PROTOCOL.md` and `tools/catalog.py` before changing publication logic.

This section supersedes older instructions that mandate blocking update gates, independently copied updaters, unconditional latest/stable publication, disjoint A/B catalogs, or publication in A before an explicitly requested B release. Existing signature and integrity requirements remain in force.
