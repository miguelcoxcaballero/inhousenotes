# Inhouse Notes 6.0.0 — Build plan

Revision 2 (after review). 31 work units in 6 waves. Units are sized for one agent in a few hours (S ≈ 1–2 h,
M ≈ 2–4 h, L ≈ 4–6 h); none is larger than L. Each unit works in its own git worktree off the `v6` branch and merges
when its acceptance passes. **Inside a wave no two units edit the same file** (the lists below are exhaustive; "owns"
= creates or edits); across waves a later unit may extend a file an earlier unit created only where stated. A unit
starts when its dependencies have merged; a scheduler runs at most 4 units at once (4 cores) and prefers cargo-only
units in parallel with one Playwright-heavy unit.

Rules for every unit
- Read `REQUIREMENTS.md`, `ARCHITECTURE.md` §0 and the sections cited in your unit; port from **5.18.0** (`1d6e8a0`,
  files in the repo root), using `spec/*.md` as the map. Name the 5.18.0 function you reproduce in a `// v5:` comment.
- No new crates, no new npm packages, no edits to `vendor/`, `shared/inhouse-services-updater/`, `android app/`,
  `.github/android/`, `oauth-callback/` or anything frozen. The Android installer is never touched.
- Generated files (`wasm/ihn.wasm`, `offline-assets.js`, `native-assets.json`, native hashes in `index.html`) are never
  committed on `v6`; `npm run build` writes them to `build/` (ARCHITECTURE §14.2).
- `cargo test --workspace` stays green; `npm run test:parity -- --quick --states <your state files>` passes at
  tolerance 0 for your states (or the state is tagged with a `BEHAVIOUR-CHANGES.md` id); your smoke specs pass. You fix
  your own parity diffs before merging; W6.1 only handles cross-cutting ones.
- Interfaces named "provides" are frozen once merged; a change needs the lead's sign-off and a note in this file.
- Test commands go in `__IHN_TEST_API__.cmd('<module>.<name>', args)` handled in your module's `test_cmd`
  (pre-wired by W1.1).
- Host executors (`host/*.js`) are dumb and complete: the unit that owns one implements **every** op of its effect
  family (ARCHITECTURE §3.3) before dependents start, even ops it does not use itself.

---

## Wave 1 — walking skeleton, harness, platform, exact maths, Drive spike

### W1.1 FOUNDATION — workspace, ABI, host loader, DOM applier, build, service worker (L)
- Goal: an empty-but-real v6 that loads `index.html` (5.18.0 markup), instantiates the wasm, binds static nodes,
  writes `v6.0.0` labels, answers `__IHN_TEST_API__.ready()`, and builds reproducibly into `build/`.
- Owns: `rust/Cargo.toml`, `rust/Cargo.lock`, `rust/crates/*/Cargo.toml`, `rust/crates/ihn-wasm/src/lib.rs`,
  `ihn-app/src/{lib.rs,app.rs,abi.rs,dom.rs,host.rs,testapi.rs,ui_ids.rs}`, **stub files with an empty
  `pub fn test_cmd` for every module named in ARCHITECTURE §2.2–2.3 and SYNC §18.1** (incl. `drive/{home,manage,perms,
  revisions,relay}.rs`, `doc/{links,public,v5open,stats,versions}.rs`, `export/user.rs`, `jsfmt.rs`, `jsmath.rs`) and
  their `mod` declarations, `ihn-core/src/{lib.rs,codec.rs,testutil.rs}`, `ihn-sync/src/lib.rs` stub, `host/main.js`
  (incl. effect families 3.1 timers/rAF/idle and 3.7 worker messages), `host/dom.js`, `host/test-api.js`, stubs
  `host/{canvas,idb,fetch,pdf,peer,platform}.js`, `index.html` (5.18.0 copy with the v6 head/script section and the
  `native=__NATIVE__` placeholder), `boot.js` (from `boot-v5.js`), `service-worker.js` (v6, classic, shell cache,
  `v=5.18.0` cache exception, `ihn6-hello` client registry; ARCHITECTURE §12), `.gitignore` (`build/`, generated files),
  `scripts/{build-wasm.mjs,gen-ui-ids.mjs,build-offline-assets.mjs,build-native-assets.mjs}` (build calls
  `scripts/check-math.mjs`, a stub until W1.4), `package.json` scripts, `playwright.config.mjs`,
  `.github/workflows/production-checks.yml` (per-PR, nightly and release jobs of ARCHITECTURE §14.2),
  `tests/v6/e2e/{00-boot,14b-sw-upgrade}.spec.mjs`.
- Provides: ABI v6 (ARCHITECTURE §3.2–3.5) with the DOM family complete; `trait Host` + `FakeHost`; generated `Ui`
  handle table; scheduler (timers, rAF, idle, input gating); test-api dispatcher.
- Acceptance: `npm run build` from two paths → identical `build/`; import allowlist and size gate active; `cargo test`
  (FakeHost DOM-op golden test); e2e 00 (CSP, no console errors, both `[data-app-version]` = `v6.0.0`, Welcome when
  signed out) and 14b (5.18.0 SW + cache installed → v6 deployed → page reloads into v6, `scriptURL` unchanged, old
  `v=5.18.0` assets still served); parity `welcome/*` (quick).
- Depends: —

### W1.2 HARNESS — mocks, e2e runner, parity harness, fixtures (L)
- Goal: the test infrastructure of `TESTING.md`, proven first on 5.18.0 against itself.
- Owns: `tests/v6/mock/{google.mjs,drive.mjs,github.mjs,fixtures.mjs}`, `tests/v6/server.mjs` (`--root`,
  `--overlay`), `tests/v6/parity/{run.mjs,compare.mjs,normalize.mjs,trace-init.js,determinism-init.js,matrix.mjs,
  report.mjs}`, `tests/v6/parity/states/{welcome,home-shell}.mjs`, `tests/v6/fixtures/v5/` + generator
  `tests/v6/fixtures/make-v5-fixtures.mjs`, `tests/v6/fixtures/traces/` + `tests/v6/fixtures/record-traces.mjs`,
  `tests/v6/perf/profile.mjs` (from `scratchpad/prof/profile.mjs`, parameterised for v5/v6).
- Provides: Drive/Google/GitHub mock with every endpoint 5.18.0 and v6 use (files list/get/create-with-id/update/
  copy (copies `properties`, honours nulls)/trash, multipart + resumable upload with offsets, revisions, v2 ETag that
  changes on **any** metadata write, `permissionIds`, permissions, properties merge with 124-byte/30-key limits,
  about, generateIds, thumbnails, token endpoint, GIS stub, updater catalogs; latency, 412, 403 rate-limit and
  failure injection); `parity(stateName, script)` API with the `?drive=<fixture>` scene loader; normalization of
  TESTING §3.2; v5 fixtures of TESTING §5 (incl. plain PDF, R0/R1); 5.18.0 canvas traces of every `canvas` scene.
- Acceptance: parity suite with **5.18.0 on both sides** gives 0 diffs over two runs; traces of a dense page
  byte-identical across runs; fixtures and traces regenerate byte-identically.
- Depends: —

### W1.3 SHELL-AUTH — routing, theme, profile menus, sign-in, platform and fetch executors (L)
- Goal: Welcome ⇄ Home routing, web GIS sign-in and Android PKCE, token lifecycle, sign-out, theme toggles, profile
  menus (both; editor doc-stats block left empty for W3.1), Soon buttons, loading overlay, status line, fullscreen
  buttons, profile-menu Refresh, connectivity probe and offline state, updater creation in app mode; the complete
  `host/fetch.js` (fetch, deadlines, resumable and keepalive uploads, blobs, digest, deflate) and `host/platform.js`
  (GIS, OAuth bridge, export bridge v2, background bridge, file chooser, clipboard, share, download, fullscreen,
  history) executors.
- Owns: `ihn-app/src/shell/*`, `ihn-app/src/auth.rs`, `ihn-app/src/drive/client.rs`, `ihn-app/src/updater.rs`,
  `host/platform.js`, `host/fetch.js`, `tests/v6/e2e/{01-signin,14-updater,15-soon}.spec.mjs`,
  `tests/v6/parity/states/{auth,profile-menu,updater}.mjs`.
- Provides: `auth::ensure_token(interactive)`, `drive::client::request(Req) -> ReqId` (5.18.0 `driveFetch`
  semantics), `shell::{show_view, status, soon, loading}`, app-mode flags, effect families 3.3, 3.6, 3.8, 3.10.
- Acceptance: smoke 01, 14, 15; executor self-test via test API for every platform/fetch op; parity: welcome sheet,
  Home empty/skeleton/signed-in topbar, Home profile menu (Refresh, fullscreen), night mode, offline status line,
  updater screens.
- Depends: W1.1 (Rust side can start on FakeHost at once)

### W1.4 JSMATH — exact JS number formatting and maths (M)
- Owns: `ihn-core/src/{jsfmt.rs,jsmath.rs}`, `tests/v6/fixtures/math/*.json` + `tests/v6/fixtures/record-math.mjs`,
  `scripts/check-math.mjs` (the grep gate of ARCHITECTURE §6.5).
- Provides: `jsfmt::num(f64) -> String` (ES `Number::toString`), `jsmath::{sin,cos,atan2,exp,pow,round,hypot}`.
- Acceptance (cargo): bit-identical to Node 22 on the recorded corpus; the gate rejects a planted `.sin()`.
- Depends: W1.1 (stubs)

### W1.5 DRIVE-SPIKE — verify the sync assumptions on real Google accounts (S, needs the user)
- Goal: SYNC §23 assumptions 1–8 measured on real accounts before W2.7 starts.
- Owns: `tests/v6/spike/drive-assumptions.mjs` (Node, uses tokens of three test accounts from env vars),
  `docs/v6/SPIKE-REPORT.md` (generated).
- Acceptance: the report states pass/fail per assumption with the observed numbers (properties 403 for reader and
  commenter; link-folder reads by another account; read-after-write lag; property quota; v2 etag on property write;
  rate limits at 3 × 0.5 writes/s for 10 min; `permissionIds` visibility; `generateIds` for documents). If assumption
  1 fails, W2.7 implements SYNC §13.1; if 6 shows frequent throttling, the lead raises the write cap before W2.7.
- Depends: — (the user provides the accounts, see open questions)

---

## Wave 2 — core subsystems (mostly cargo-only)

### W2.1 MODEL — ids, stamps, ops, document, snapshots, undo inverses, geometry (L)
- Owns: `ihn-core/src/{id.rs,stamp.rs,op.rs,doc.rs,pos.rs,snapshot.rs,geom.rs}`.
- Provides: ARCHITECTURE §4.1–4.4 types and API exactly (`Stamp{clock,actor,idx}`, `Z` register, `Batch` with
  import stamps, contiguous VV + extras, `Doc::{apply, order, draw_list, hit_strokes, lasso, inverse}`, `Tx`,
  `Replica`/`SyncEngine`/`SyncStatus` trait and type definitions, snapshot/batch codecs per SYNC §4).
- Acceptance (cargo): convergence property test; stamps test (same-ms batches reversed; far-future clock); undo
  inverses incl. revival of a dropped body; z rules (area-erase original z, cross-page move on top, draw order);
  snapshot round trip; geometry hit/split/swept-capsule tests ported from `rust-core/src/editor.rs`/`editor_input`.
- Depends: W1.1

### W2.2 V5-READERS — 5.x document and storage readers, deterministic import (L)
- Owns: `ihn-core/src/v5/*`.
- Provides: `v5::keywords::parse`, `v5::import::{from_pdf(keywords, facts, import_actor, rev_time_ms),
  from_local(records, migration_actor, base)}` (ARCHITECTURE §9.3), `v5::idb::{meta_payload, page_row, stroke_ops,
  offline_document, outbox}`, `v5::checkpoint::unpack`, `v5::timeline::read`, `simpleHash`, legacy id rules, the
  security-core limits.
- Consumes: W2.1 types (code against ARCHITECTURE §4; rebase when W2.1 merges).
- Acceptance (cargo): every fixture imports with counts/ids equal to 5.18.0's record; deterministic; R0/R1 in both
  orders → same state, R1 wins; xor-v1 bit-exact; strict-mode and limit failures with 5.18.0's strings.
- Depends: W1.1, W1.2 (fixtures)

### W2.3 RENDER — canvas command stream (M–L)
- Owns: `ihn-core/src/render/*`, `host/canvas.js`, `icons/agenda.svg`, `icons/diary.svg`.
- Provides: `render::{page, live, damage, overlay::*, thumb, template, pdfink::content_stream}`; canvas ABI family.
- Acceptance (cargo): trace goldens — for every `canvas` scene, the Rust command stream serialised like `trace-init.js`
  equals W1.2's recorded 5.18.0 trace (pixel checks of these scenes run in W3.2/W3.5 through the editor); `pdfink`
  streams parse. Uses `jsmath` for every non-basic operation.
- Depends: W1.1, W1.2 (traces), W1.4

### W2.4 STORE — IndexedDB schema, locks, 5.x dumper (M)
- Owns: `ihn-app/src/store/*`, `host/idb.js` (families 3.2 and 3.9 complete).
- Provides: the **frozen** schema of ARCHITECTURE §7.1 (all `docs` fields incl. `relay`, `exports`, `drive_name`,
  `flags`; all `kv` keys; the `SyncRecord` mapping), `store::{open, tx, kv_*, asset_*, chunk_*, ops_append/scan/
  trim, catalog_*}`, Web Lock helpers (`ihn6-slot`, `ihn6-net`, `ihn6-migrate`), degraded-mode signal, the read-only
  5.x dumper with fingerprints.
- Acceptance: cargo (FakeHost transaction scripts, SyncRecord mapping); `store.selftest` e2e-internal: blocked/timeout
  → degraded signal, atomic batch+seq append, locks, 5.x dump of the W1.2 storage fixtures.
- Depends: W1.1

### W2.5 HOME — lists, cards, folders, navigation, badges (L)
- Owns: `ihn-app/src/home/{mod.rs,lists.rs,cards.rs,folders.rs,nav.rs,pull.rs,scroll.rs,badges.rs}`,
  `ihn-app/src/drive/home.rs`, `ihn-core/src/{catalog.rs,palette.rs,motion.rs,rows.rs}`,
  `tests/v6/e2e/02-home.spec.mjs`, `tests/v6/parity/states/home.mjs`.
- Provides: `home::{show, refresh}`, catalogue view models, card sync badges from `docs` flags (ARCHITECTURE §6.6),
  the sync-folder/`ihn6Kind` filter in lists, search and offline listings (ARCHITECTURE §13).
- Acceptance: smoke 02 (incl. sync folder never listed); parity: tabs, lists, folder cards (all palette colours),
  opened folder, breadcrumbs, folder zoom at 0/25/50/75/100 %, pull-to-refresh, search, empty/loading/offline states,
  fast-scroll thumb, badges (`Waiting to upload`, `Saved offline`, `Uploading…`, `Retry upload`) from seeded flags.
- Depends: W1.3, W2.4

### W2.6 SYNC-BASE — session machine, modes, wire codec, simulator (M–L)
- Owns: `ihn-sync/src/{lib.rs,mode.rs,wire.rs,sim.rs}`.
- Provides: `Sync` skeleton implementing `SyncEngine`, `SyncEffect`/`SyncIo`/`SyncRecord`, receiver-local presence
  and modes (SYNC §9), scribe election, frame codec; the simulator (fake Drive per SYNC §18.1, fake channels, skewed
  virtual clocks) used by W2.7/W2.8.
- Acceptance (cargo): codec round trips and fuzz; mode transitions incl. ±10 min skew; simulator self-tests.
- Depends: W2.1

### W2.7 SYNC-RELAY — relay writer/reader, records, epochs, export decisions (L)
- Owns: `ihn-sync/src/{relay.rs,export.rs}`.
- Provides: SYNC §8 (writer with gen persistence, 2 s cap, epochs; reader with digest/doc checks; GC), §13.1 if the
  spike requires it, §14 (export due/forced, upload loop with same-head fast retry, name rule), §15 classification.
- Acceptance (cargo): SYNC §19.2 rows: P2P blocked, offline 3 days, viewer, demoted editor, removed reader, copied
  document, scribe/etag churn, forced export, rename outside the editor, 5.x save after export, throttling, request
  budget, crash points (relay), degraded storage.
- Depends: W2.6, W1.5

### W2.8 SYNC-PEER — peers, signalling mail, anti-entropy, live ink, peer executor (L)
- Owns: `ihn-sync/src/{peer.rs,entropy.rs,live.rs}`, `host/peer.js` (family 3.5 complete: RTC + BroadcastChannel).
- Provides: SYNC §10–§11 machines; `host/peer.js` executor.
- Acceptance (cargo): two editors Alone → Company → Linked; glare; backoff/unreachable; journal-retention-driven
  `Compacted` only beyond the journal; resend; forwarding over a partial mesh; live ink preview TTL; crash points
  (peer); `peer.selftest` e2e-internal: loopback RTC channel + BroadcastChannel echo.
- Depends: W2.6

### W2.9 HISTORY — timeline rules, version snapshots, restore, diff (M)
- Owns: `ihn-core/src/history.rs`.
- Provides: capture/coalesce/prune rules, per-actor delta chains with checkpoints every 8, `VersionMeta` codec,
  `plan_restore`, diff highlights (ARCHITECTURE §4.6).
- Acceptance (cargo): 5.18.0 rules (5 min, same author/device, milestones, 80/30 caps), chain survives pruning,
  restore reaches the target, diff equals 5.18.0's for recorded page pairs.
- Depends: W2.1

---

## Wave 3 — documents and the editor surface

### W3.1 DOC-SESSION — session, persistence, open via link/cache, status (L) — delivers first in wave 3
- Owns: `ihn-app/src/doc/{session.rs,open.rs,links.rs,create.rs,save_state.rs,stats.rs}`, `host/pdf.js` (family 3.4
  complete: lazy load, open with security-core params, metadata/Keywords, attachments, render page → blob URL),
  `tests/v6/parity/states/open.mjs` (loading overlay states, viewer banner, editor menu with doc stats, save
  indicator classes).
- Provides: `doc::session` (implements `Replica`: load all chunks, journal append, compaction by the `ihn6-net`
  holder, retention, degraded mode, remote-apply budget), open from Drive/cache/signed-in shared link `?drive=` (the
  scene loader path) for v6 containers and template/photo pages, `create.rs` (new document with a reserved id, not yet
  created in Drive), `save_state` (§6.6 table), doc stats.
- Acceptance: cargo (session ordering, degraded mode, apply budget, §6.6 mapping); parity states above; e2e-internal
  `doc.selftest` (open fixture via link, reload, state equal).
- Depends: W1.3, W2.1, W2.4

### W3.2 EDITOR-VIEW — editor skeleton, layout, viewport, page activation (L)
- Owns: `ihn-app/src/editor/{view.rs,layout.rs,viewport.rs,pages.rs}`, `tests/v6/parity/states/editor-view.mjs`,
  `tests/v6/parity/states/canvas.mjs` (the pixel side of W2.3's scenes).
- Provides: `EditorView` (page rows, ≤ 3 active canvases, resolution sync, previews, client→page mapping, read/edit
  mode, title, add-page indicator, pull-to-add), request-repaint API.
- Acceptance: parity editor-view states (zoom 0.5/1/2/3, pan clamps, pull-to-add, read/edit, toolbar absent) and
  canvas scenes (dense page sync/staged repaint, thumbnails, templates at all scales); gesture budget test.
- Depends: W3.1, W2.3, W1.4

### W3.3 PDF-BACKGROUNDS — pdf.js page rasters (M)
- Owns: `ihn-app/src/doc/backgrounds.rs`.
- Provides: 5.18.0 scale policy, LRU 6, 240 ms interaction deferral, one page per frame; source-asset rebuild from an
  export (ARCHITECTURE §4.5) requested through the worker (stubbed until W5.4).
- Acceptance: parity: PDF-background doc pages at zoom 1/2, during and after a pan; agenda/diary pages.
- Depends: W3.2

### W3.4 V5-OPEN — 5.x and plain PDF open, foreign revisions, worker import (M)
- Owns: `ihn-app/src/doc/v5open.rs`, `host/export-worker.js` (creates the worker with the **import role**; W5.4 adds
  the export role later), `tests/v6/e2e/12-open-v5.spec.mjs`.
- Provides: SYNC §15.1 classification on open/poll, import actor + `rev_time_ms` from the same `files.get`, large
  imports in the worker (terminate after > 64 MiB), plain-PDF adoption, foreign-overwrite recovered copy (request
  shape in `drive/revisions.rs` is W5.4's; stub until then).
- Acceptance: smoke 12 (every fixture equals 5.18.0; plain PDF; size limit message; later 5.x revision merged;
  recovered copy once W5.4 merged); memory: main-instance wasm memory unchanged after the largest fixture.
- Depends: W3.1, W2.2

### W3.5 INK — input pipeline, pen and highlighter, undo (L)
- Owns: `ihn-app/src/editor/{input.rs,pen.rs,history.rs}`, `tests/v6/e2e/04-ink.spec.mjs`,
  `tests/v6/parity/states/editor-tools.mjs` (pen/highlighter/undo states; later units add their own state files).
- Provides: pointer pipeline (ARCHITECTURE §11.1), tool dispatch trait used by eraser/selection/image tools, pen and
  highlighter, undo/redo stack, `SyncEngine::live` samples.
- Acceptance: smoke 04 (pen, highlighter, undo/redo, reopen); parity: live pen frames 1/5/20, lift, highlighter live
  and committed, single-tap dot (B-E3); perf: 0 allocations per pointer event in steady state.
- Depends: W3.2

---

## Wave 4 — tools and live collaboration

### W4.1 ERASERS (M)
- Owns: `ihn-app/src/editor/eraser.rs`, `tests/v6/e2e/04b-erase.spec.mjs`, `tests/v6/parity/states/erasers.mjs`.
- Acceptance: stroke/area eraser mid-gesture with cursor, damage repaint before lift, swept-segment test (B-E6),
  area-erase undo at original z (B-E4).
- Depends: W3.5

### W4.2 SELECTION — lasso, selection box, transforms, cross-page move (L)
- Owns: `ihn-app/src/editor/selection.rs`, `tests/v6/e2e/04c-select.spec.mjs`, `tests/v6/parity/states/selection.mjs`.
- Acceptance: lasso drawing, handles, move/scale/rotate (jsmath), duplicate +18/+18, recolour, delete, cross-page drag
  (B-E2) incl. an old stroke dropped over a newer highlighter (draws on top), non-A4 pages (B-E1).
- Depends: W3.5

### W4.3 IMAGES (M)
- Owns: `ihn-app/src/editor/images.rs`, `tests/v6/e2e/06-images.spec.mjs`, `tests/v6/parity/states/images.mjs`.
- Acceptance: smoke 06; paste/drop/insert, place, transform, rotated image; asset stored once by hash.
- Depends: W3.5

### W4.4 TOOLBAR — toolbar drag/snap, tool and colour menus, eraser menu, Draw with finger (M)
- Owns: `ihn-app/src/editor/toolbar.rs`, `tests/v6/parity/states/toolbar.mjs`.
- Acceptance: toolbar at every snap position, pen palette, highlighter menu, eraser menu, `Draw with finger` toggle
  (and `touchDrawEnabled` persistence), undo/redo button states.
- Depends: W3.5

### W4.5 COLLAB — session ↔ sync binding, relay executor, presence UI, remote ink (L)
- Owns: `ihn-app/src/collab/*`, `ihn-app/src/drive/relay.rs`, `tests/v6/e2e/11-two-devices.spec.mjs`,
  `tests/v6/parity/states/presence.mjs`.
- Provides: binding of `ihn-sync` to the session, Drive request execution for relay/records/id pool/sync folder,
  presence avatars and popup per SYNC §9.1, remote live overlay.
- Acceptance: smoke 11 sub-tests (a)–(e) (latency gates TESTING §4); parity: avatars connecting/connected/relay-only,
  remote save rings, popup.
- Depends: W2.7, W2.8, W3.5

---

## Wave 5 — features

### W5.1 PAGES — Manage-Pages panel, photo page, Import PDF, slider, Scan Soon (M–L)
- Owns: `ihn-app/src/panels/{pages_panel.rs,photo_page.rs,slider.rs}`, `ihn-app/src/doc/import.rs`,
  `tests/v6/e2e/{05-pages,15b-soon-scan}.spec.mjs`, `tests/v6/parity/states/pages.mjs`.
- Acceptance: smoke 05 (incl. Import PDF); parity: panel, thumbnails, drag reorder mid-drag, delete slider 0/50/100 %,
  photo modal states, Scan Soon.
- Depends: W3.2, W3.3

### W5.2 SHARE-PUBLIC — share modal, permissions, viewer mode, signed-out public links (M)
- Owns: `ihn-app/src/panels/share.rs`, `ihn-app/src/drive/perms.rs`, `ihn-app/src/doc/public.rs`,
  `tests/v6/e2e/07-share.spec.mjs`, `tests/v6/parity/states/share.mjs`.
- Acceptance: smoke 07 (incl. signed-out 2.5 s poll, no `key=`); parity: every share modal state, viewer banner,
  sign-in gate, public-link viewer.
- Depends: W3.1

### W5.3 TIMELINE — capture trigger, panel, preview, restore (M)
- Owns: `ihn-app/src/panels/timeline.rs`, `ihn-app/src/doc/versions.rs`, `tests/v6/e2e/16-timeline.spec.mjs`,
  `tests/v6/parity/states/timeline.mjs`.
- Acceptance: smoke 16; parity: grouped list, preview with highlights, restore slider, `Original` entry.
- Depends: W2.9, W3.2, W4.5 (versions travel through sync)

### W5.4 EXPORT-LIFECYCLE — PDF export, Drive create/upload, export to the user, Home exit, headless (L)
- Owns: `ihn-app/src/export/*`, `ihn-app/src/drive/revisions.rs`, `host/export-worker.js` (adds the export role;
  W3.4 created the file), `tests/v6/e2e/{08-save-reopen,09-offline,10-exit-upload}.spec.mjs`,
  `tests/v6/parity/states/export.mjs`.
- Provides: first create with the reserved id (sets `flags.created`), conditional upload (SYNC §14.2), container
  (ARCHITECTURE §8.2, `/IHN6Ink`), source rebuild from exports, export to the user (forced export + bridge v2 /
  share / download), lifecycle (Home exit, pagehide keepalive, hidden suspension), `?offline-worker` headless run.
- Acceptance: smoke 08, 09, 10; exported PDF re-imports to the identical snapshot; parity export states.
- Depends: W3.4, W4.5

### W5.5 HOME-DIALOGS — create/upload/picker/new folder/card menu/bin/offline dialogs/download-app (L)
- Owns: `ihn-app/src/home/dialogs/*`, `ihn-app/src/drive/manage.rs`, `tests/v6/e2e/{03-docs,03b-home-actions}.spec.mjs`,
  `tests/v6/parity/states/home-dialogs.mjs`.
- Acceptance: smoke 03/03b (incl. copy nulls `ihn6_*`, rename survives export, delete forever queues relay GC);
  parity: every dialog/menu state incl. download-app modal, offline settings queue line, offline sync-error report.
- Depends: W2.5, W3.1, W5.4

### W5.6 MIGRATION — one-time 5.x local data migration (L)
- Owns: `ihn-app/src/migrate.rs`, `tests/v6/e2e/13-migrate.spec.mjs`.
- Acceptance: smoke 13 with every storage fixture (dirty offline doc, `local-` doc, pending folder, newer working
  cache, uncompacted stroke-ops, pending-exit marker, recoveries, blocked DB, 5.x tab writing after migration, crash
  resume); done = migration batches sealed (ARCHITECTURE §10.2); deletion conditions enforced.
- Depends: W2.2, W2.4, W3.1, W4.5, W5.4

---

## Wave 6 — parity sweep and release

### W6.1 PARITY-SWEEP (L, may repeat)
- Goal: run the full matrix (TESTING §3.3), triage cross-cutting diffs (shared DOM helpers, number formatting, event
  ordering), add the `sweep` states, and produce `tests/v6/parity/REPORT.md` (generated). Module-specific diffs go
  back to their owner as fix branches.
- Owns: `tests/v6/parity/states/sweep.mjs`; fixes by agreement with the owner (one fix branch per module, serialized).
- Acceptance: 0 unexplained diffs; every remaining diff references a BEHAVIOUR-CHANGES id.
- Depends: all of waves 1–5

### W6.2 PERF-RELEASE (M)
- Goal: performance check vs 5.18.0, budgets, release preparation (ARCHITECTURE §14.3 steps 1–3).
- Owns: `tests/v6/perf/*` (except W1.2's `profile.mjs` core), `RELEASE_NOTES_v6.0.0.md`,
  `.github/workflows/deploy-pages.yml`, `.github/scripts/verify_live_release.py`, deletion of v5 tests and
  `rust-core/`, final version strings, the commit of `build/` outputs on the release PR.
- Acceptance: perf report meets ARCHITECTURE §14; all suites green; release candidate ready. **The push to `main`
  happens only after the user has reviewed BEHAVIOUR-CHANGES and the parity report and approved it.**
- Depends: W6.1

---

## Dependency summary

| Wave | Units | Starts when |
|---|---|---|
| 1 | W1.1, W1.2, W1.5; then W1.3, W1.4 | at once; W1.3/W1.4 after W1.1 (W1.5 whenever the user provides accounts) |
| 2 | W2.1, W2.4, W2.2, W2.3, W2.5; W2.6 → W2.7, W2.8; W2.9 | per unit (W2.7 also needs W1.5's report) |
| 3 | W3.1 → W3.2, W3.4 → W3.3, W3.5 | W3.1 first: it delivers the `?drive=` open path every editor parity state uses |
| 4 | W4.1, W4.2, W4.3, W4.4, W4.5 | after W3.5 (W4.5 also W2.7, W2.8) |
| 5 | W5.1, W5.2, W5.3, W5.4 → W5.5, W5.6 | per unit |
| 6 | W6.1 → W6.2 | waves 1–5 merged |

Critical path: W1.1 → W2.1 → W2.6 → W2.7 (+W1.5) → W4.5 → W5.4 → W5.6 → W6.1 → W6.2, in parallel with
W1.1 → W1.3 → W3.1 → W3.2 → W3.5 → W4.x.
