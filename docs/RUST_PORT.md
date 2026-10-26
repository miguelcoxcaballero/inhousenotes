# Rust port: rules for every agent

Goal: move computation out of JavaScript into the Rust/WASM core so the app looks **identical** but runs
smoother. JavaScript stays only where the browser forces it (DOM, canvas calls, events, IndexedDB, fetch, WebRTC).
Everything else becomes Rust. A port is done only when the JS algorithm is **deleted** (no JS fallback copy).

## Architecture
- One WASM module (`rust-core/`, toolchain 1.90.0, std + `serde`/`serde_json`, no host imports, `panic=abort`).
- Each area owns `rust-core/src/<area>.rs` (already registered). Do **not** edit `lib.rs`, `bridge.rs`, `Cargo.toml`
  except to add a `mod` for a new submodule *inside your own area file*. Need a new crate? Record it in your report instead.
- Two ways to expose Rust:
  1. **Structured calls** through the bridge: implement `pub fn call(method, input) -> Option<Result<Vec<u8>,String>>`
     in your area file (see `bridge.rs`, helper `bridge::json`). JS: `InhouseRust.callJson('area.method', value)` / `callBytes`.
  2. **Hot kernels** (pixels, long arrays): `#[no_mangle] pub unsafe extern "C" fn name(ptr, len, ...)` with typed
     pointers, as in `ink.rs` and `editor.rs`. The JS wrapper lives in `rust-core-v5.js` **between your own marker lines**
     (`// >>> stream:<area>` ... `// <<< stream:<area>`), registering on the `api` object. Re-read `memory.buffer` after any call.
- Callers must `await InhouseRust.ready` before use. If WASM is unavailable throw a clear error; never silently degrade.
- Validate every length/finite value at the boundary and return `Err`/a sentinel instead of panicking.

## Protocol for each unit of work
1. Separate pure computation (data in, data out) from DOM/canvas/IO glue. Only the computation moves.
2. Write the Rust with `#[cfg(test)]` tests (`cargo test --manifest-path rust-core/Cargo.toml`).
3. **Differential test against the original JS**: `tests/helpers/legacy.mjs` loads the pre-port file from git
   (`loadLegacy('path.js')`). Compare outputs on fixtures plus a few thousand seeded random inputs. Byte-exact unless
   a documented 1-ULP/1-level rounding difference is proven harmless. Keep that test in `tests/`.
4. Replace the JS algorithm by a thin call into Rust and **delete the old code**. Update tests that grep source text
   (`tests/release-v5-smoke.mjs`) for what you removed.
5. Run the relevant tests: `cargo test`, `node --test <files>`, and browser specs through the lock script (below).
   Visual output must not change.
6. Commit on your branch: `rust(<area>): <what moved>`; no version bump, no generated files. End the message with:
   `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` and
   `Claude-Session: https://claude.ai/code/session_01GDSPhnzQidr8C2rqizVBK7`.
7. Update `docs/rust-port/<area>.md`: units done / remaining, JS bytes removed, Rust bytes added, speedup measured.

## Never do
- Touch `vendor/`, `shared/inhouse-services-updater/`, `.claude/`, or any installer-freeze file. Those are protected.
- Commit `wasm/inhouse_notes_core.wasm`, `native-assets.json`, `offline-assets.js`, `?native=` hashes or `APP_VERSION` bumps.
  The integrator rebuilds those. (The wasm is marked assume-unchanged in your worktree; rebuild it freely with
  `node scripts/build-rust-core.mjs`; do **not** run `npm run build:offline`.)
- Edit code outside your ownership, rename public globals other areas use, or change UI markup/CSS/behavior.
- Skip, loosen or delete a test to get green. Mark a parity doubt in the report instead.

## Tooling (4 CPU cores shared by all agents)
- Keep builds light: `cargo build -j2`, `nice` for long runs.
- Browser tests are serialised with a global lock and Chromium from `/opt/pw-browsers`:
  `/tmp/claude-0/-home-user-inhousenotes/6d17ca04-85d0-5c3e-86eb-8b9e5b6579d2/scratchpad/e2e.sh <your-worktree> tests/e2e/<spec>.mjs`
  Run only the specs for your area (they take minutes); the integrator runs the full suite.
- Unit tests: `node --test tests/<file>.test.mjs`, `node tests/release-v5-smoke.mjs`.
- GitHub language share (what the user tracks):
  `ruby /tmp/claude-0/-home-user-inhousenotes/6d17ca04-85d0-5c3e-86eb-8b9e5b6579d2/scratchpad/ling/lang.rb <worktree> line`.
  GitHub ignores files over ~1.5 MB (app-v5.js), so shrinking it does not change the percentage; adding Rust does.
  Do not split a ported file into new counted JS files to dodge it; delete the logic.

## Quality bar
Same pixels, same behavior, same persistence formats and wire formats. Faster. If a faithful port is impossible
for a piece (needs a browser API), leave it in JS and say why in your report.
