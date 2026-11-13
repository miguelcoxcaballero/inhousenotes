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
- `shell::status(app, host, text, kind)` is 5.18.0 `showStatus(text, options)` in full: `kind` is a `Status`
  (`Info`/`Saving`/`Error`/`Phase(p)`) or a `StatusOpts { saving, error, preserve_state, saved_at, phase }`
  (`StatusOpts::from(Status::Saving).preserve()`, `.saved_at(ms)`, `.phase(p)`). With `preserve_state` the
  indicator's classes are left alone, as in 5.18.0 (47 call sites there). `sig::STATUS` carries the whole call, not
  only the text: read it with `shell::StatusMsg::parse(m.data)` → `{text, saving, error, preserve_state, saved_at,
  phase}` and `semantic_state()` (5.18.0 `requestedSemanticState`). Wire form `<flags> <phase> <savedAt>\n<text>`.
  The shell sets class, `title` and `aria-label` only; `setSaveDebugStatus`, `lastSavedAt`/timestamp, the debounced
  confirmation and `refreshSaveIndicatorState` (which may replace the title, e.g. "No pending changes" after
  "Signed out") are W3.1's, following the signal.
- `shell::{show_view, view, home_status, soon, loading, loading_home_save, hide_loading,
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

- `cargo test --workspace`: green (42 tests in `ihn-app`, with the FakeHost).
- e2e 01-signin (8 tests: web sign-in / reload / sign-out, refused consent and reconnect, late GIS, app-mode PKCE,
  theme + menus + fullscreen + loading overlay, and the two executor self-tests covering every op of
  `host/fetch.js` and `host/platform.js`), 14-updater (5), 15-soon (3), plus 00-boot and 14b.
- e2e 01-signin also covers going offline in the browser: the `offline` event, `shell.state.offline`, the NETWORK
  signal, the session flag, `#drive-status` (with and without a local library), probes after 30 s / 60 s / 120 s,
  the immediate probe on `online`, a reload that starts offline, recovery and no probing while connected.

## Parity — NOT signed off for the states that show Home

The harness is W1.2's and is not on this branch: check `v6-w1_2`'s `tests/v6/{parity,mock,fixtures,server.mjs}` out
beside it, unmodified (the first builder's local edit of `run.mjs` is gone).

**The gate** — `node tests/v6/parity/run.mjs --quick --states auth,profile-menu,updater`, unmodified, on this branch
alone: 52 comparisons, 0 pass, 20 pixel-equal (DOM/trace warnings), **32 fail**.
- Pixel-equal: `auth/app-pkce-started`, `auth/signed-out`, `auth/returning-skeleton`, `updater/offer`,
  `updater/installing`, `updater/channels` (and W1.2's `welcome/*`).
- Failing: `auth/signed-in-topbar`, `auth/returning`, `auth/reconnect`, `auth/reconnect-offline`, all of
  `profile-menu/*`, `updater/menu-entry`.
- So of the acceptance list, met today: welcome sheet, Home skeleton (boot skeleton with the top bar), updater
  screens. **Not met, and not to be counted as met**: Home empty, signed-in top bar, Home profile menu (Refresh,
  fullscreen), night mode, the offline state. W1.3's sign-off on those is the same unmodified command passing after
  W4.1 and the offline-library unit have merged.

**The diagnostic** — `IHN_SHELL_ONLY=1 node tests/v6/parity/run.mjs --quick --states auth,profile-menu,updater`
(committed: `tests/v6/parity/support/shell-only.mjs`, which the three state files wrap their scripts with; no
harness edit). Home's list sections (`#drive-home .drive-section`) are taken out of layout in both apps for each
screenshot, so top bar, search row, navigation, profile menu (it overlaps the list area), toast and updater are what
is compared. Result: 48 of 52 equal (12 of them inside the `VERSION-LABEL` rectangle); the 4 that differ:
- `auth/reconnect`: Search and the Add button are disabled in v6 only — 5.18.0's offline library is ready for the
  known account; v6 has none yet (`auth::set_local_library`).
- `auth/reconnect-offline`: the same, and 5.18.0 withdraws "Reconnect" when its library reloads after the
  connectivity change (`connectivity → reload → refreshed → renderHome → updateDriveAuthUI`).
This shows where the differences are; it does not replace the gate.

What the states capture (measured on 5.18.0):
- Skeleton cards exist in 5.18.0 only while its offline library is not ready, i.e. no account is known: the first
  sign-in (`auth/signed-in-topbar`: `showDriveSkeletons`, W4.1's on `sig::REFRESH`) and a remembered session without
  `drive-hint-email` (`auth/returning-skeleton`: the boot script's skeleton, equal today). A returning session of a
  known account (`auth/returning`) shows the cached library before the page is first idle — never a skeleton.
- `#drive-status` is `display: none` in the stylesheet, so the "offline status line" cannot be a pixel state: its
  text is asserted in e2e 01, `profile-menu/offline` is the menu over offline Home, and the visible effect of
  going offline on the shell is `auth/reconnect-offline`.
- The version label in the open profile menu reads `v6.0.0` instead of `v5.18.0` (tag `VERSION-LABEL`, the label's
  rectangle as the allowed region).

For the offline-library unit: call `auth::set_local_library` when the library is ready, and `auth::update_ui` where
5.18.0 calls `updateDriveAuthUI` from `renderHome` (after the reload that follows a connectivity change) — the shell
does not re-evaluate the auth UI on `sig::NETWORK`, because 5.18.0 does not either without a library.

## Deviations from the plan

1. **`BEHAVIOUR-CHANGES.md` has no id for the version label.** The parity tag `VERSION-LABEL` is used meanwhile; the
   lead should add an id (or normalise `[data-app-version]` in the harness) and rename the tag.
2. **Parity of Home states is not 0 on this branch alone** (see "Parity" above): those acceptance items are open
   until W4.1 and the offline library have merged.
3. **Two edits outside the owned files**, both in W1.1 test code:
   - `app.rs` (unit test only): the boot golden test now allows the modules' own boot records between the version
     labels and `ready`.
   - `tests/v6/e2e/00-boot.spec.mjs`: `www.googleapis.com` is answered by the test. The shell now probes Drive at
     boot like 5.18.0 (`probeConnection`), and the real, unauthenticated 401 was reported as a console error.
4. **`tests/v6/parity/support/shell-only.mjs`** is new and outside the owned list (the harness unit may turn it into
   a runner option). **Test commands** added: `auth.local_library`, and `shell.status` takes `preserve`/`savedAt`.
5. **New files in `shell/`**: `fx.rs` (effect builders), `json.rs`, `consts.rs` (icons and the default avatar copied
   from 5.18.0), `testkit.rs` (cargo-test helpers).
6. **Sub-families beyond the §3.3 list** that 5.18.0 needs: `BLOB_INFO`, `BOOT_SIGNED_IN`
   (`__INHOUSE_BOOT_SIGNED_IN__` for the boot script), `DISPATCH_RESIZE`, `UPDATER_SETTINGS`.
7. **Commit trailer** names the model that actually ran this session.

## Measured

`ihn.wasm` 151,624 bytes (63,503 gzip -9; limits 480 KB / 160 KB); first-party JS 66,226 bytes (limit 100 KB).
