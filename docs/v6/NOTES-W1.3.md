# W1.3 SHELL-AUTH — notes for the other units

Branch `v6-w1_3`, on top of `v6-w1_1`.

## What exists now

- `shell/` — boot routing (Welcome / Home / editor), theme, welcome sheet, both profile menus, Soon toast, loading
  overlay, status lines, fullscreen buttons, connectivity probe and offline state, global click/keyboard handlers.
- `auth.rs` — web GIS and Android PKCE sign-in, stored session, token lifecycle (silent refresh, reconnect state,
  cancellation by generation), profile, sign-out.
- `drive/client.rs` — 5.18.0 `driveFetch`: deadline including the token, `Authorization`, one retry after 401,
  5.18.0's error texts.
- `updater.rs` — creates the shared updater in app mode and reveals `[data-inhouse-update-settings]`.
- `host/fetch.js`, `host/platform.js` — every op of families 3.3, 3.8, 3.10 and 3.6.

## Interfaces (frozen once merged)

- `auth::ensure_token(app, host, interactive, waiter) -> Token` (`Ready(token)` | `Pending` | `Failed(message)`).
  With `Pending` the caller's module gets `Event::Io { req: waiter, status, data }` later (`status == 0`: `data` is
  the token; `< 0`: the error text). `waiter` is `app.req(Mod::X)`.
  Also `auth::{token, signed_out, reconnecting, profile, known_account, has_stored_session, sign_in, sign_out,
  mark_needs_reconnect, update_ui, drop_token}`.
- `auth::set_local_library(app, host, ready)` — **the offline library unit must call this** when its library for the
  known account is ready (5.18.0 `InhouseOfflineLibrary.ready()`). Until then v6 treats the library as absent, which
  is the one visible difference left in the reconnect state (below).
- `drive::client::request(app, host, owner, Req) -> u32`: the owner gets one `Event::Io` with that id; `status >= 0`
  is the HTTP status (2xx or one of `Req::accept`) and `data` an `fx::Response`; `status < 0` is `client::err::*` or
  `-(HTTP status)` with 5.18.0's error message in `data`. `client::abort(id)`, `client::busy()`.
- `shell::{show_view, view, status, home_status, soon, loading, loading_home_save, hide_loading,
  close_profile_menus, offline, network_available, dark, pending_share, set_pending_share}`.
- `shell::signal` / `shell::sig::*` — broadcasts (`Event::Msg`) that stand where 5.18.0 calls straight into another
  subsystem: `AUTH`, `REFRESH` (5.18.0 `refreshDriveFiles()` — **W4.1 lists Drive on this**), `SIGNING_OUT` (return
  `ret::PREVENT_DEFAULT` to veto), `SIGNED_OUT`, `VIEW`, `ACCOUNT`, `NETWORK`, `THEME`, `RECONNECT`, `MENU` (the
  editor menu's `#doc-stats` is filled by W3.1 on `MENU`/`editor`), `STATUS`.
- `shell::fx` — effect builders for the four families, `fx::Response`, URL/form encoding helpers; `shell::json` — a
  small JSON reader.
- Test commands: `shell.state`, `shell.fx` / `shell.fx_results` (the executor self-test: runs any effect of the four
  families against the real host and returns what came back), `auth.state`, `drive_client.*`, `updater.state`.

## Verified

- `cargo test --workspace`: green (41 tests in `ihn-app`, with the FakeHost).
- e2e 01-signin (7 tests: web sign-in / reload / sign-out, refused consent and reconnect, late GIS, app-mode PKCE,
  theme + menus + fullscreen + loading overlay, and the two executor self-tests covering every op of
  `host/fetch.js` and `host/platform.js`), 14-updater (5), 15-soon (3), plus 00-boot and 14b.
- Parity, with W1.2's harness checked out beside this branch (it is not merged here):
  - Pixel-identical at tolerance 0 as they are: `welcome/*`, `home-shell/sign-in-failed`, `auth/app-pkce-started`,
    `auth/signed-out`, `updater/offer`, `updater/installing`, `updater/channels`.
  - Every state that shows Home differs **only inside Home's list sections** (`#drive-home .drive-section`: skeleton
    cards, lists, empty messages — W4.1). Proof: with those sections taken out of layout in both apps for the
    screenshot (a local, uncommitted switch in the harness), all of `auth/*`, `profile-menu/*`, `updater/*` and
    W1.2's `home-shell/*` are pixel-identical at tolerance 0, except the two noted below. They pass unmodified once
    W4.1 has merged.
  - The version label in the open profile menu reads `v6.0.0` instead of `v5.18.0`. The states that show it carry
    the tag `VERSION-LABEL` with the label's rectangle as the allowed region.
  - `auth/reconnect` and `home-shell/reconnect`: 5.18.0 keeps Search and the Add button enabled because its offline
    library is ready; v6 has no offline library yet (`auth::set_local_library`).

## Deviations from the plan

1. **`BEHAVIOUR-CHANGES.md` has no id for the version label.** The parity tag `VERSION-LABEL` is used meanwhile; the
   lead should add an id (or normalise `[data-app-version]` in the harness) and rename the tag.
2. **Parity of Home states is not 0 on this branch alone** (see above): Home's content is W4.1's and the offline
   library is another unit's. Nothing in the shell's own pixels differs.
3. **Two edits outside the owned files**, both in W1.1 test code:
   - `app.rs` (unit test only): the boot golden test now allows the modules' own boot records between the version
     labels and `ready`.
   - `tests/v6/e2e/00-boot.spec.mjs`: `www.googleapis.com` is answered by the test. The shell now probes Drive at
     boot like 5.18.0 (`probeConnection`), and the real, unauthenticated 401 was reported as a console error.
4. **New files in `shell/`**: `fx.rs` (effect builders), `json.rs`, `consts.rs` (icons and the default avatar copied
   from 5.18.0), `testkit.rs` (cargo-test helpers).
5. **Sub-families beyond the §3.3 list** that 5.18.0 needs: `BLOB_INFO`, `BOOT_SIGNED_IN`
   (`__INHOUSE_BOOT_SIGNED_IN__` for the boot script), `DISPATCH_RESIZE`, `UPDATER_SETTINGS`.
6. **Commit trailer** names the model that actually ran this session.

## Measured

`ihn.wasm` 149,474 bytes (62,571 gzip -9; limits 480 KB / 160 KB); first-party JS 66,226 bytes (limit 100 KB).
