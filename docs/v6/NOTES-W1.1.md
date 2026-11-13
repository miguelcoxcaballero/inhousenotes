# W1.1 FOUNDATION — notes for the other units

Branch `v6-w1_1` (git cannot create `v6/w1_1` while a branch named `v6` exists).

## What exists now

- `rust/` workspace: `ihn-core` (codec, test PRNG, stubs), `ihn-sync` (stubs), `ihn-app` (ABI, DOM builder, `Host`,
  `FakeHost`, `App`, scheduler, test API, `Ui` table, stubs), `ihn-wasm` (exports/imports). No external crates.
- `host/main.js` (loader, out-buffer dispatcher, events, effects 3.1 and 3.7, lifecycle, service-worker
  registration, WebView gate), `host/dom.js` (complete DOM family), `host/test-api.js`, executor stubs.
- `npm run build` → `build/{wasm/ihn.wasm,offline-assets.js,native-assets.json,index.html}`; serve with
  `node scripts/serve.mjs --root . --overlay build --port <n>`.
- `npm run test:cargo`, `npm run test:e2e` (`IHN_E2E_PORT`, `IHN_CHROMIUM` override port and browser).

## Contract every `ihn-app` module follows (`app.rs`)

Nobody but W1.1 edits `app.rs`, `lib.rs` or the shared `mod.rs` files, so each stub already has the four items the
module table calls: `State: Default` (stored in `app.st.<module>`), `init`, `event` and `test_cmd`. Replace the
bodies, keep the signatures.

- Ids given to the host (listeners, timers, idle slots, deferred jobs, effect requests) are `abi::tag(Mod::X, n)`;
  the matching event is delivered to `X::event`. `app.req(Mod::X)` makes a fresh request id.
- `Event::Life` and `Event::Msg` are broadcast to all modules in table order; `Event::Frame` goes to the modules
  that called `app.sched.frame(out, Mod::X)`.
- `init` runs in `Mod` order: infrastructure first, `shell` last.
- `app.hold_ready()` in `init` + `app.release_ready()` later delays `__IHN_TEST_API__.ready()`.
- Scheduler: `timer`, `interval`, `clear`, `frame`, `idle`, `defer(id, quiet_ms, bound)` → `Event::Work`,
  `set_busy(Mod, on)`, `activity(now)` (automatic for pointer events), `new_session()`.
- Test commands: `cmd('<module>.<name>', args)` → that module's `test_cmd`; `core.<module>.<name>` and
  `sync.<module>.<name>` reach the pure crates. Only delivered in e2e mode.
- Export worker: `ihn_worker` payload byte 0 is the role (`app::worker_role`): 0 → `doc::v5open::worker`,
  1 → `export::pdf::worker`.

## Contract of a host executor (`host/<name>.js`)

`export default function install(host)` returns `{ [family << 8 | op]: reader => … }`. Op ranges are in the header
of `abi.rs`. Results: `host.io(req, status, obj, bytes)`, `host.message(chan, data, obj)`; JS objects cross as
handles from `host.hold(obj)` / `host.obj(id)` (Rust frees them with `out.obj_release`). DOM nodes: `host.node(h)`.
`host/platform.js` sets the app-mode bit (`1`) in `host.config.flags` during `install`.

## Deviations from the plan

1. **Module contract is larger than "an empty `test_cmd`"** (adds `State`, `init`, `event`): without it later
   units would all have to edit `app.rs`.
2. **`trait Host` has `log`** (the `host_log` import needs a caller) and `ihn_core::testutil` is public and not
   `cfg(test)`, so `ihn-sync`/`ihn-app` tests can use the PRNG.
3. **ABI additions** to the §3.3 list: DOM `html` (innerHTML) and `select`; effects `obj_release`, `test_result`,
   `ready`, `raf_cancel`; `listen` flag `ACTIVE` (`passive:false`). Frames arrive through `ihn_frame`, so there is
   no `FRAME` event kind. Events report every known handle from the target upwards (`path`), which gives Rust both
   `closest` and `contains`.
4. **Host executors are loaded with dynamic `import()`** of `host/<name>.js?v=<APP_VERSION>&native=<hash>` in
   parallel with the wasm, instead of static imports: they get the same cache busting and APK asset hashes as
   every other file.
5. **`offline-v5.css?v=6.0.0`** (the plan's snippet kept 5.18.0): with `v=5.18.0` the v6 service worker would
   treat it as a legacy request and never serve it from the v6 shell.
6. **`scripts/serve.mjs`** is new and not in the owned list: W1.2's `tests/v6/server.mjs` does not exist yet.
   `playwright.config.mjs` uses W1.2's server as soon as the file exists (same flags).
7. **Parity `welcome/*`**: the W1.2 harness is not available, so the check ran as a one-off script with the
   TESTING §3.1 Chromium flags: Welcome light and system-dark at 1280×800@1 and 360×800@3 are pixel-identical to
   5.18.0 (tolerance 0). Not covered: the open welcome sheet (needs W1.3's click handling). The DOM differs where
   5.18.0's init writes invisible state (`data-app-ready`, `read-only` classes, avatar placeholders, title) — for
   W1.3/W3.1.
8. **Root `offline-assets.js` and `native-assets.json` are removed from git and ignored** on this branch (generated
   files, ARCHITECTURE §14.2). The release PR must `git add -f` them and `wasm/ihn.wasm`.
9. **`package.json`**: the 5.x `test:unit`/`build:*` scripts are gone (their targets no longer match the tree);
   the 5.x test files stay until the release deletes them. Version fields are `6.0.0`.
10. **WebView gate text** `Update Android System WebView to use Inhouse Notes.` is new wording (5.18.0 has none).
11. **Service-worker client registry** is stored in Cache Storage (`ihn6-clients`) so it survives worker restarts.
    Protocol: a v6 window posts `'ihn6-hello'`; `{type:'ihn6-clients'}` is answered with
    `{type:'ihn6-clients', windows, v6}` on the message port (or to the sender). 5.x data may be deleted only when
    `windows === v6`.
12. **Workflow**: the parity step is skipped with a warning while `tests/v6/parity/run.mjs` is missing, and passes
    the 5.18.0 tree as `IHN_V5_ROOT`; W1.2 should read that variable. `deploy-pages.yml` (not owned) still needs
    the §14.2 file-list change at release. `14b-sw-upgrade` needs commit `1d6e8a0` in the clone (`fetch-depth: 0`).
13. **Commit trailer** names the model that actually ran this session.

## Measured

`ihn.wasm` 72,703 bytes (30,262 gzip -9; limits 480 KB / 160 KB); first-party JS 45,371 bytes (limit 100 KB).
Two builds from different paths: identical SHA-256 for all four files in `build/`.
