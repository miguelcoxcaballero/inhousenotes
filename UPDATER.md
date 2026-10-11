# Inhouse services updater — mandatory shared updater policy

Canonical repository: https://github.com/miguelcoxcaballero/inhouse-services-updater

Every AI/developer working in this app must read this section even without opening the updater repository:

- **B is the default update channel.** A requires an English development warning, explicit acknowledgement and slide confirmation. Legacy implicit A preferences migrate to B; choosing B clears A consent. This does not authorize publishing or promoting a B release.
- All new changes and automatic releases go to **A** (rapid changes/testing).
- **B** (reviewed releases) requires the user's **explicit request**. Tests passing is not permission to promote.
- Every release needs a tiny changelog: preferably 2–10 words, maximum 120 characters.
- Never independently edit updater copies. Change the canonical repository first, test, commit, and sync the SAME revision into YouTube, Instagram, Notes, Photos and Read using its `tools/sync.py`.
- Commit `inhouse-updater.lock.json` and shared files together. Build checks must reject drift. App identity and host integration are the only permitted differences.
- Updates are optional. Later snoozes automatic prompts for 14 days; manual updates remain available in settings. The popup and settings both expose A/B selection.
- Keep APK package and signing key; increment Android versionCode globally across A/B. Switching channels never downgrades or deletes user data.
- Publish each exact signed APK before its matching channel catalog. Use `updates/A.json` and `updates/B.json`; never mix channels using generic GitHub latest. B initially has no releases until explicitly requested.
- See the central `AGENTS.md`, `docs/PROTOCOL.md` and `tools/catalog.py` before changing publication logic.

This section supersedes older instructions that mandate blocking update gates, independently copied updaters, or unconditional latest/stable publication. Existing signature and integrity requirements remain in force.
