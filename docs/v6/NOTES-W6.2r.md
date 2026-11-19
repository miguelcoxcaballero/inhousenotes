# W6.2r RELEASE — notes (release candidate 6.0.0; nothing published)

Branch `v6-w6_2r` from `v6-int` 1fc7e48. This is the release-preparation part of W6.2 (ARCHITECTURE §14.3 steps
1–3). Nothing was pushed to `main`, no PR, tag, GitHub release, APK or update catalog was created. The push waits for
the owner's approval of `BEHAVIOUR-CHANGES.md` and the parity report (§14.3 step 4).

## Deleted (proved unused first)

Each file was searched for real references (imports, `readFile`/`fetch`, script tags, `offline-assets.js`,
`native-assets.json`, workflow steps, `.github/scripts/*`, `android app/`, `rust/`, `tests/v6/`). The only hits
left were `// v5:` comments and the git history that `14b-sw-upgrade`, the parity harness and the fixture
generator read (`git archive 1d6e8a0`): none of them reads these files from the working tree.

- **v5 tests** (81 files): `tests/*.test.mjs` (19: `collab-cadence-v5`, `collaboration-core-v5`, `document-jobs`,
  `document-work-rust`, `drive-ink-journal`, `drive-live-preview`, `history-memory`, `live-collaboration-v5`,
  `local-save-status-v5`, `native-assets`, `offline-library-index`, `offline-sync-schedule`,
  `pdf-clean-original-cap`, `peer-transport`, `pen-writing-idle-v5`, `runtime-core-v5`, `rust-core`, `rust-folder`,
  `timeline-core-v5`), `tests/release-v5-smoke.mjs`, `tests/helpers/fake-ink-drive.mjs`, `tests/e2e/` (57 specs,
  `server.mjs`, `fixtures/collaboration-chaos.{html,js}`). No script or workflow ran them any more.
  `native-assets.test.mjs` asserted `app-v5.js` inside the Android bundle; the deploy workflow's new
  generated-files check covers what it checked for v6.
- **`rust-core/`** (5.x Rust sources: `Cargo.toml`, `Cargo.lock`, `src/{lib,checkpoint,document_work,editor,pdf_ink}.rs`)
  and **`scripts/build-rust-core.mjs`** (built `wasm/inhouse_notes_core.wasm` from it). `rust/` replaces both;
  only comments name them ("ported from …"), and the sources stay in history at 1d6e8a0.

## Kept on purpose

- **The 5.18.0 runtime files**: every root `*-v5.js` (incl. `rust-core-v5.js`), `wasm/inhouse_notes_core.wasm`,
  `boot-v5.js`, `bullet-template.js`, `ink-palette.js`, `vendor/jspdf/`. The lead asked to remove the v5 runtime if
  unused, but they are used: ARCHITECTURE §10.4 and §14.3 step 2 keep them **deployed but unreferenced in 6.0.0**,
  and the v6 `service-worker.js` answers every `?v=5.18.0` request of a still-open 5.18.0 tab from the
  `inhouse-shell-5.18.0` cache **and then the network** (`caches.match(…) || fetch(event.request)`). Deleting them would
  404 a 5.18.0 tab whose legacy cache misses an entry (evicted cache, a lazy worker or wasm fetch) before it reloads
  into 6.0.0. They are byte-identical to 5.18.0 (`git diff 1d6e8a0 -- '*-v5.js' …` is empty) and go in 6.0.1 with the
  `inhouse-shell-5.18.0` exception.
- **`offline-v5.css`**: the v6 stylesheet (index.html `<link>`, `offline-assets.js`); name kept by §14.3.
- **`stencil.html`**: referenced by neither 5.18.0 nor v6, but a public page (`/stencil.html`, the design template
  the Android builder's colours follow). Not sure it is unused by people: kept, question for the owner.
- **`tests/test_android_refresh_rate.py`**: not a v5 test; `build-android.yml` runs it.
- **`tests/idle-animations.test.mjs`** → moved to `tests/v6/shell/idle-animations.test.mjs`: it guards the
  shell CSS (index.html `<style>` + `offline-v5.css`), which v6 ships unchanged, and passes on v6 (6/6). New script
  `npm run test:shell`; added to `npm test` and to production-checks.
- **`android-update.json`, `updates/A.json`, `updates/B.json`**: catalogs for installed APKs; not changed (§14.3 step 6).
- `vendor/inhouse-services-updater/`, `inhouse-updater.lock.json`: untouched; `verify.py` passes.

## Workflows and scripts

- `deploy-pages.yml`: new step before the rsync that checks the committed generated files describe the committed
  bytes (every `native-assets.json` entry's size/SHA-256, the index.html native map, every `offline-assets.js` path
  exists). Excludes add `build/`, `rust/`, `rust-toolchain.toml`, `wasm/ihn-test.wasm` (drop `rust-core/`).
  Required files are now `index.html CNAME boot.js host/main.js wasm/ihn.wasm service-worker.js offline-assets.js
  native-assets.json` (§14.2), plus `app-v5.js` (§10.4), a `const APP_VERSION` line and no `native=__NATIVE__` left in
  `index.html`. Publishing and the final live check are unchanged.
- `verify_live_release.py`: hashes index.html, boot.js, service-worker.js, offline-assets.js, native-assets.json,
  wasm/ihn.wasm, offline-v5.css, vendor/fonts/ui.css, every `host/*.js`, `updates/A.json`, `updates/B.json` and the
  kept `*-v5.js` (39 files) against the live site; version check unchanged.
- `production-checks.yml`: `npm run test:shell`; on `main`, the committed `wasm/ihn.wasm`, `offline-assets.js`,
  `native-assets.json` and `index.html` must equal the fresh build (`cmp`); the second-path reproducibility build
  gets `node_modules` (a symlink: under `CI` the build fails without binaryen, so that step could not pass before);
  parity gets `--a-root` (the old `IHN_V5_ROOT` was read by nothing; the harness fell back to `git archive`); the
  full job's perf step runs `profile.mjs --short` for 5.18.0 and v6 into `test-results/perf/` (it used to profile
  5.18.0 only, `--app` defaulting to v5); report paths are fixed (`test-results/parity-report`).
- `build-android.yml`: unchanged. It needs `native-assets.json` in the tree (`install_native_assets.py`), which the
  release commit now provides; `verify.py` runs inside `build_android_apk.py`, unchanged.
- `package.json`: `test:shell`; `npm test` = cargo → shell → build → e2e → quick parity. Version stays `6.0.0`.
- `.gitignore`: the four generated files are no longer ignored (they are committed); `wasm/ihn-test.wasm` is.

## Versions

`APP_VERSION = '6.0.0'`; both `[data-app-version]` read `v6.0.0` (e2e `00-boot`, and the smoke run below);
`service-worker.js` `VERSION='6.0.0'` (cache `inhouse-shell-6.0.0`); `package.json` 6.0.0; every app `?v=` is
6.0.0 (the updater's own `?v=1.2.3` is the updater version). Android `versionName 1.0.28` / `versionCode 29`
(`build_android_apk.py`, `android-update.json`) are **not** bumped: §14.3 step 6 says no APK build and no catalog
change for 6.0.0, and the APK loads the live site.

## Generated files committed

`npm run build` output copied over the tree: `wasm/ihn.wasm` (SHA-256 `89532cbb…f9685`), `offline-assets.js`
(212 entries), `native-assets.json` (211 resources), `index.html` (native map and `native=` hashes filled). A second
`npm run build` from the committed tree gives byte-identical files (what production-checks' release job asserts).
Any later change to `host/*.js`, `boot.js`, the CSS, the updater files or `rust/` needs `npm run build` and these four
files copied again before the push.

## Verification (all on this branch, after the deletions)

- `cargo test --workspace --locked`: ihn-app 170, ihn-core 137 (+1 ignored), ihn-sync 49 — all pass.
- `npm run build` (CI=1, budgets enforced): `ihn.wasm` 1,589,269 bytes raw (limit 1,656,832), 697,048 gzip -9
  (limit 727,040); first-party JS 124,718 bytes in 12 files (limit 128,000); `ihn-test.wasm` 1,670,277 / 734,020.
- `npm run test:shell`: 6/6. Updater `verify.py`: PASS (revision d0d2afd).
- e2e, full suite, one worker: **98 passed** (8.9 min).
- Parity `--quick` against 5.18.0 (`--a-root` r518): `home-shell` 16 comparisons, 0 fail, 16 warnings (pixels
  equal); `open` 14, 0 fail, 2 expected (`VERSION-LABEL`, the `v6.0.0` label in the editor menu), 12 warnings;
  `canvas` 18, 0 fail, 18 warnings.
- Deploy emulation: the rsync excludes applied to the tree (280 files, 11 MB; no `rust/`, `build/`, `tests/`,
  `node_modules`, `ihn-test.wasm`), the generated-files check passed (211 native assets, 212 shell entries), and that
  directory was served at `http://127.0.0.2:<port>` (a loopback secure context that is not one of the service
  worker's localhost names, so the shell is the production one) to headless Chromium with the Google mock:
  signed out → welcome; signed in → Home, profile menu shows `v6.0.0`; worker active and controlling, cache
  `inhouse-shell-6.0.0` with all 212 assets (no `ihn-test.wasm`), no test API, controlled after reload; 0 HTTP
  errors, 0 failed requests, 0 page errors in 258 own-origin requests.
- `verify_live_release.py` run locally: it builds its 39-file list and waits, because the live site still serves
  5.18.0 (expected).

## Remaining steps to publish (after the owner approves)

1. Owner reviews `docs/v6/BEHAVIOUR-CHANGES.md` (every entry is still "proposed") and the parity report, and approves.
2. Merge this branch into the release line and push to `main` in one commit that carries the version bump and the
   generated files (the version is already 6.0.0 everywhere; if anything changes before the push, rebuild and copy
   the four generated files again, and keep `RELEASE_NOTES_v6.0.0.md` in sync).
3. Before pushing: `npm ci && npm run test:cargo && npm run test:shell && npm run build && npm run test:e2e`, the
   full parity matrix, and `git diff --exit-code` on the four generated files after the build.
4. Push; check `git ls-remote origin main` equals the local commit.
5. Wait for `deploy-pages.yml` (its last step runs `verify_live_release.py`), then fetch `https://inhousenotes.com/`
   without cache (`curl -sH 'Cache-Control: no-cache' 'https://inhousenotes.com/?v=<sha>'`) and confirm
   `const APP_VERSION = '6.0.0'`, `service-worker.js` with `VERSION='6.0.0'` and `wasm/ihn.wasm` with the hash above.
   Only then report the release as published (AGENTS.md).
6. Channel A/B: nothing to publish for 6.0.0 — the APK, the frozen installer and `updates/*.json` are unchanged and
   the installed APK loads the new site. If the owner wants an Android release too, it is a separate A release
   (`build-android.yml` with a new versionName/versionCode: 1.0.28/29 is already published) with the changelog line
   below.

Proposed updater changelog (if an APK is ever cut for it): `Rust rewrite: faster start, less memory` (6 words, 38
characters).

## Not done / open

- The second-path reproducibility build, the full parity matrix and the perf check were not run here (CI runs
  them; disk is tight). GitHub Actions themselves could not be run: YAML parsed, step commands run locally where
  possible (deploy check and site preparation emulated without `rsync`, which is not installed here).
- `stencil.html`: delete or keep? Android versionCode: confirm that no APK is cut for 6.0.0.
