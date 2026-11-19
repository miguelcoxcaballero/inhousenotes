# Handoff prompt for the next agent

Copy the block below into a new session on the repository `miguelcoxcaballero/inhousenotes`.

---

You are continuing the Rust rewrite of Inhouse Notes (version 6.0.0). The owner writes in Spanish: reply in Spanish
with short, plain updates.

**Where the work is**
- All of the 6.0.0 work is on branch `v6-int` (head after this handoff: see `git log -1 origin/v6-int`). `main` and
  https://inhousenotes.com/ still serve 5.18.0. Nothing has been published.
- Read first: `AGENTS.md`, `CLAUDE.md`, `docs/v6/CHANGELOG.md` (current state, sizes, pending decisions),
  `docs/v6/ARCHITECTURE.md`, `docs/v6/BEHAVIOUR-CHANGES.md`, `docs/v6/NOTES-W6.2r.md` (publish steps),
  `docs/v6/TESTING.md`.
- Ignore the old `claude/trusting-pasteur-qsmedx` branch content (an abandoned 5.x-era port from October,
  superseded by `v6-int`). The `v6-w*` branches are already merged into `v6-int`.

**First task: integrate 5.19.0.** `main` published 5.19.0 on 2026-11-15 (`35d0f22`, merge of
`codex/shared-google-oauth`): shared Google OAuth in `vendor/inhouse-services-updater/auth/` (canonical source:
`miguelcoxcaballero/inhouse-services-updater/auth`, locked by `inhouse-oauth.lock.json`), a new `AGENTS.md` section
"Shared Google OAuth", changes in `app-v5.js`/`index.html`/`service-worker.js`, and Android APK 1.0.29 / versionCode 30
published in channel A. `v6-int` is based on 5.18.0 (`1d6e8a0`) and has none of it.
1. Read `git diff 1d6e8a0 origin/main` and `vendor/inhouse-services-updater/auth/README.md` (on `main`).
2. Merge `origin/main` into `v6-int` (merge, not rebase). Keep `APP_VERSION = '6.0.0'`. Do not hand-edit the vendored
   auth or updater files: they come from `tools/sync.py` in the canonical repository.
3. v6 has its own sign-in port in Rust/host (`rust/crates/ihn-app` + `host/`). AGENTS.md now says not to fork the
   login: make v6 use `vendor/inhouse-services-updater/auth/google-oauth.js` (JS host executes it; Rust keeps the UI
   and state), including the Android `GoogleSessionBridge` path and the local sign-out opt-out semantics. Keep every
   5.19.0 behaviour; if v6 must differ, add an entry to `BEHAVIOUR-CHANGES.md` as "proposed".
4. Rebase the parity reference on 5.19.0 (`git worktree add ../r519 origin/main`), rerun e2e and the full parity
   matrix, update `RELEASE_NOTES_v6.0.0.md` (Android is 1.0.29 / 30, no longer 1.0.28 / 29) and `docs/v6/CHANGELOG.md`.

**Rules that must not be broken**
- `vendor/inhouse-services-updater/` contains a FROZEN Android installer. Do not edit, move, reformat or delete
  anything there, and do not name its installer files in shell commands (a hook blocks it). Only the owner, in
  their own words, can unfreeze it.
- Do not push to `main` until the owner explicitly approves `docs/v6/BEHAVIOUR-CHANGES.md`. When they approve,
  follow `NOTES-W6.2r.md` "Remaining steps to publish" and AGENTS.md: `APP_VERSION` stays `6.0.0` (already set),
  rebuild and commit the four generated files (`wasm/ihn.wasm`, `offline-assets.js`, `native-assets.json`,
  `index.html`), run all tests, push, check `git ls-remote origin main`, then fetch the live site without cache and
  confirm `v6.0.0` before saying it is published.
- Android updates: no APK for 6.0.0 unless the owner asks. Any APK release goes to channel A; channel B only on the
  owner's explicit request.
- Do not split the wasm into several modules, download prebuilt binaries/tools, or add post-link cutting or custom
  compression unless the owner authorises it explicitly.
- No pull requests unless asked. The owner's email is private.

**How to build and test** (Linux, Chromium preinstalled)
```
npm ci
export IHN_CHROMIUM=/opt/pw-browsers/chromium_headless_shell-1194/chrome-linux/headless_shell
export NODE_OPTIONS=--max-old-space-size=6000
cargo test --manifest-path rust/Cargo.toml --workspace --locked   # 173 + 138 (1 ignored) + 49
npm run build                                                     # budgets enforced in scripts/build-wasm.mjs
npm run test:shell                                                # 6/6
npx playwright test --workers=1 --trace off --reporter=line       # 98/98, about 9 min
```
- Parity needs a reference tree: 5.18.0 is `git worktree add ../r518 1d6e8a0`; after step 4 above use 5.19.0
  (`../r519`). One state file per run:
  `node tests/v6/parity/run.mjs --quick --a-root ../r519 --states tests/v6/parity/states/<file>.mjs --port-a 4680 --port-b 4681 --out /tmp/par-<file>`.
  Full matrix: drop `--quick`; merge with `node tests/v6/parity/report.mjs --summary <dirs…> --out tests/v6/parity/REPORT.md`.
- Perf: `tests/v6/perf/bench.mjs` (see `NOTES-W6.2z.md` for the method: alternating base/final rounds).
- The machine has 4 CPUs: run one heavy job at a time.

**What is open** (details in `docs/v6/CHANGELOG.md`)
1. Waiting on the owner (after 5.19.0 is integrated): approval of BEHAVIOUR-CHANGES; whether to split the wasm (598 KB gzip vs 456 KB in 5.18.0);
   confirm no APK for 6.0.0; keep or delete `stencil.html`; real Google test accounts.
2. Work that can be done without the owner:
   - Run the GitHub Actions workflows' steps end to end where possible (`production-checks.yml`,
     `deploy-pages.yml`); they have only been checked locally.
   - Re-run the full parity matrix on the current `v6-int` head (the last full run was at `6b4e85d`; later runs were
     the quick matrix on 22 files).
   - Further size work that changes neither behaviour nor performance (see the "tried and reverted" lists in
     `NOTES-W6.2z.md` and `NOTES-W6.2y.md`; do not repeat those).
   - Images and version snapshots over P2P/relay (currently they only arrive with the next PDF export): design it
     first in `SYNC.md` and ask the owner before changing the wire format.
3. After 6.0.0 is live: 6.0.1 removes the 5.18.0 runtime files (`*-v5.js`, `wasm/inhouse_notes_core.wasm`) and the
   `inhouse-shell-5.18.0` exception in the service worker.

Commit on `v6-int` (or a `v6-w*` unit branch merged into it), push, and tell the owner in Spanish what changed and
what still needs their decision.
