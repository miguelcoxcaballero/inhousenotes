# Inhouse services updater

Mandatory guidance for every AI and developer. This repository is the canonical updater for Inhouse YouTube, Instagram, Notes, Photos and Read.

- Every new app change and automatic publication goes to **A**. A is the testing/rapid-change channel.
- Publish or promote to **B** only after an **explicit request from the user**. Passing tests, elapsed time, a scheduled job or an AI's judgment is not authorization. B is the reviewed release channel; never describe it as a guarantee of zero bugs.
- Add a tiny user-facing changelog to every release: preferably 2–10 words, at most 120 characters. The updater shows at most three entries and a compact skipped-version count.
- Never duplicate or independently fix updater code in a consumer app. Edit this repository first, test it, commit it, then run `tools/sync.py` for ALL five apps. Commit the lock and identical shared files in each app. App-specific configuration and host wiring are the only allowed differences.
- Every app must include these rules in its own `AGENTS.md`, `UPDATER.md` and updater integration comments so an AI working only in that repo knows where to look.
- Updates are always optional. Later defers automatic prompts for 14 days per channel; settings and manual checks remain usable. Changing channels never uninstalls, clears data or downgrades the APK.
- Use monotonically increasing Android versionCode across both channels, the existing package ID and permanent signing key. Never commit signing material. Promotion to B references an already verified exact APK, not a new silently replaced binary.
- Channel catalogs live in each app's `updates/A.json` and `updates/B.json`. Never fall back to the other channel or generic GitHub latest. A may be marked prerelease; B is promoted explicitly.
- Do not publish a catalog before its exact APK is publicly downloadable with matching version, hash, size, package and signature.
- Current pre-channel releases are not automatically promoted to B. B starts empty until the user requests promotion.

Read `README.md`, `docs/PROTOCOL.md` and `docs/PROVENANCE.md` before changing the updater.

- **B is the default update channel.** A requires an English development warning, explicit acknowledgement and slide confirmation. Legacy implicit A preferences migrate to B; choosing B clears A consent. This does not authorize publishing or promoting a B release.
