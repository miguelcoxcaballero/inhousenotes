# Handoff prompt for the next agent

Copy the block below into a new session on the repository `miguelcoxcaballero/inhousenotes`.

---

You are continuing the Rust rewrite of Inhouse Notes (version 6.0.0). The owner writes in Spanish: reply in Spanish
with short, plain updates.

**Where the work is**
- All of the 6.0.0 work is on branch `v6-int` (head after this handoff: see `git log -1 origin/v6-int`). `main` and
  https://inhousenotes.com/ serve 5.19.0. The v6 candidate is unpublished.
- Read first: `AGENTS.md`, `CLAUDE.md`, `docs/v6/CHANGELOG.md` (current state, sizes, pending decisions),
  `docs/v6/ARCHITECTURE.md`, `docs/v6/BEHAVIOUR-CHANGES.md`, `docs/v6/NOTES-W6.2r.md` (publish steps),
  `docs/v6/TESTING.md`.
- Ignore the old `claude/trusting-pasteur-qsmedx` branch content (an abandoned 5.x-era port from October,
  superseded by `v6-int`). The `v6-w*` branches are already merged into `v6-int`.

**5.19.0 integration**: merged into `v6-int` at `657f3d0`. The host uses the shared OAuth client and
the parity baseline is `35d0f22`. Check `CHANGELOG.md` for verification results and open release gates.

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
npx playwright test --workers=1 --trace off --reporter=line       # includes the shared Android grant regression
```
- Parity needs the reference tree: `git worktree add --detach ../r519 35d0f22`. One state file per run:
  `node tests/v6/parity/run.mjs --quick --a-root ../r519 --states tests/v6/parity/states/<file>.mjs --port-a 4680 --port-b 4681 --out /tmp/par-<file>`.
  Full matrix: drop `--quick`; merge with `node tests/v6/parity/report.mjs --summary <dirs…> --out tests/v6/parity/REPORT.md`.
- Perf: `tests/v6/perf/bench.mjs` (see `NOTES-W6.2z.md` for the method: alternating base/final rounds).
- The machine has 4 CPUs: run one heavy job at a time.

**What is open** (details in `docs/v6/CHANGELOG.md`)
1. Waiting on the owner (before publication): approval of BEHAVIOUR-CHANGES; keep or delete `stencil.html`; real Google test accounts and devices. Keep the single wasm and the existing APK unless the owner asks otherwise.
2. Work that can be done without the owner:
   - Investigate the intermittent raster difference documented in the full 5.19.0 matrix without widening tolerances.
   - Further size work that changes neither behaviour nor performance (see the "tried and reverted" lists in
     `NOTES-W6.2z.md` and `NOTES-W6.2y.md`; do not repeat those).
   - Images and version snapshots over P2P/relay (currently they only arrive with the next PDF export): design it
     first in `SYNC.md` and ask the owner before changing the wire format.
3. After 6.0.0 is live: 6.0.1 removes the 5.x runtime files (`*-v5.js`, `wasm/inhouse_notes_core.wasm`) and the
   5.18.0 and 5.19.0 legacy cache exceptions in the service worker.

Commit on `v6-int` (or a `v6-w*` unit branch merged into it), push, and tell the owner in Spanish what changed and
what still needs their decision.
