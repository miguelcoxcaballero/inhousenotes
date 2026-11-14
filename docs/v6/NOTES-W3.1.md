# W3.1 DOC-SESSION — notes for the other units

Branch `v6-w3_1` (from `v6-int` at `862ff20`). Owns `rust/crates/ihn-app/src/doc/{session,open,links,create,
save_state,stats}.rs`, `host/pdf.js`, `tests/v6/parity/states/open.mjs`; adds `tests/v6/e2e/internal-doc.spec.mjs`.

## STOP — two things the lead must decide before more units merge

1. **The wasm size budget is exceeded** (ARCHITECTURE §14: 480 KB raw / 160 KB gzip). `v6-int` is 243,702 bytes
   because nothing used the document model yet; this unit is the first to link `ihn-core::{doc, snapshot, stamp, …}`
   and the binary is now **≈ 529 KB raw / 218 KB gzip** (the build prints the exact numbers). Measured per function (name section of an unstripped
   build): `ihn_core` 15 → 79 KB, `core::slice::sort` 0 → 40 KB (five `sort_unstable*` instantiations of the model),
   `alloc` B-trees 0 → 29 KB, `hashbrown` 0 → 19 KB, `ihn-app::doc::*` (this unit, with what LLVM inlined into it:
   `DocRecord::from_value`, the loader) ≈ 78 KB, data +22 KB. `opt-level = "z"` gives 449 KB raw / 191 KB gzip.
   This unit replaced its own maps and sets by sorted vectors (−43 KB); the rest is not in files it owns.
   `npm run build` therefore **fails** on this branch, as the gate intends. To build for tests:
   `IHN_SIZE_REPORT_ONLY=1 npm run build` (new, in `scripts/build-wasm.mjs`: reports the two wasm budgets instead of
   throwing; default and CI behaviour unchanged; the limits are untouched). Options: wasm-opt, `opt-level="z"` plus a
   higher gzip limit, fewer generic collections in `ihn-core`, or a new budget.
2. **`v6-int` did not build before this unit either**: W1.4's maths gate rejected `self.log(io)` in W2.4's
   `store/selftest.rs` (a method named like a banned f64 function). Renamed to `note` here (4 lines, no behaviour).
   Also on `v6-int`: `internal-store.spec.mjs` fails with `Failed to load resource: 401` in most runs — the shell's
   connectivity probe (W1.3) reaches the real `www.googleapis.com` because that spec (W2.4) does not mock it, as
   `00-boot` does since W1.3. Not touched here.

## What exists now

- `doc::session` — one open document: `Doc`, the writer (`Author::resume` after every load), the in-memory journal,
  the persistence queue, the `ihn6-net:<account>:<docId>` lock, assets, compaction, and `Replica`.
  - `load(app, host, account, doc_id, notify)` → `notify` gets `LOADED_LOCAL` / `LOADED_EMPTY` (never inside the
    call). One read transaction: `kv` (install id — created here on first use — and the id mark), the `docs`
    record, all chunks, the journal; then the asset blobs the document refers to. Empty `account` = memory only.
  - `edit(app, host, |tx| …) -> Option<Edit{seq, inverse, changes}>` — **the only write path**: builds the batch,
    takes the inverse, applies, appends `ops` + `docs.seq` (+ the id mark, + the record on first write) in one
    transaction. `None` when the session is not writable (viewer, no actor slot) or the transaction was empty.
  - `drain_local(app)` → batches whose commit finished, in order (`SyncEngine::local_batch`), announced by
    `sig::COMMITTED`. In degraded storage they are handed over at once.
  - `with_replica(app, host, |r: &mut dyn Replica| …)` — how `ihn-sync` (W4.5) and importers reach the document.
    `receive` dedupes (also against the queue), applies everything at once without contact and ≤ 0.5 ms per call /
    frame while `app.sched.is_busy()`, then in 4 ms idle slices (`BatchDecoder::step` + `Doc::apply_part`, 256 ops
    between clock readings). Applied batches are journalled (`ops_put`). `sig::CHANGED` + `take_changes(app)`.
  - `set_sync_status(app, host, Some(status))` — W4.5 reports the engine's `SyncStatus`; until then
    `sync_status(app)` derives one from the stored record (own batches beyond `relay.sealed`, `flags.created`).
  - `persist(app, host, Record)`, `set_meta`, `put_asset`, `compact(app, host, force)`, `close`, `get`/`get_mut`,
    `Session::{writable, referenced_assets, asset_blob, net_holder, can_edit, record, export_size}`.
  - Compaction: only the `ihn6-net` holder (the session requests the lock at load and keeps it; **W4.5 reads
    `Session::net_holder`, it does not request the lock again**), 1200 ms after the last input, never with a remote
    batch half applied. Header chunk 0 + one chunk per page with items (chunk numbers are allocated per page and
    recovered from the chunk bytes at load). Journal entries leave `ops` when `Replica::retain` covers them or they
    are older than 24 h; over 8 MiB the oldest received batches go first, own ones last.
- `doc::open` — `open_file(app, host, File{id, name, resource_key}, notify) -> bool`; `busy`; `pdf::*` builders of
  effect family 3.4; `classify(keywords)`; `Meta`. Local copy first (shown without network, then revalidated: an
  unknown `headRevisionId` is downloaded and merged), else metadata → `alt=media` → revision re-check (5.18.0's
  stable-snapshot rule, 3 attempts) → pdf.js → Keywords → v6: inflate → `load_snapshot` → attachments
  `IHN6-<hash>` → `docs` record with the known export → `sig::OPENED`.
- `doc::links` — `parse(url)`, `pending(app)`; the signed-in open on `sig::AUTH` with 5.18.0's status texts.
- `doc::create` — `new_document(app, host, folder, base_name)`: id from `kv.idpool` (asks `generateIds` itself only
  when the pool is empty), first page A4 + name, `Created on this device`; `Failed to create PDF` without an id.
- `doc::save_state` — `derive(&Inputs) -> (Phase, badge)` (the §6.6 table; **W2.5 uses the badge texts**),
  `present`, `phase(app)`, `set_catching_up`, the indicator and the save-details popup. `SIG_SAVE_NOW`.
- `doc::stats` — `#doc-stats` on `sig::MENU`/`editor`.
- `host/pdf.js` — `pdf_load, pdf_open, pdf_pages, pdf_attachment, pdf_render, pdf_close` (0x50..0x55), pdf.js loaded
  on first use, security-core limits (blob ≤ 64 MiB and type, `isEvalSupported:false`, `maxImageSize`, 1..2000 pages).

## For W3.2 (editor view)

- Build the view on `session::sig::OPENED` (data = doc id); repaint on `sig::CHANGED` with `take_changes`.
- `open.rs` does **not** set the document title, read mode, the toolbar or its buttons (5.18.0 `updateDocTitle`,
  `setReadOnlyMode`, the `toolsToDisable` list of `updateViewerModeUI`): they are the editor's. It does the banner,
  `#mode-toggle-btn` display and the title's `contentEditable`/`pointer-events` for viewers.
  `Session::can_edit` is the Drive capability, `Session::writable()` adds the actor slot (B-Y7).
- Rasters: `open::pdf::render(out, req, doc, page, scale)` → `Io::obj` = the JPEG blob, data = `str blob URL, f64 w,
  f64 h` (revoke the URL and release the handle when the raster is dropped). Source assets:
  `Session::asset_blob(&hash)` → `pdf::open`.

## For W3.4 (5.x and plain PDFs)

`open.rs` announces a PDF that is not a v6 container with `sig::FOREIGN` (data `v5` | `plain`). Claim it by
returning `ret::PREVENT_DEFAULT` from your `event`, call `open::take_foreign(app)` → `Foreign{file, kind, blob, size,
pdf, opened, meta}` (you own `blob` and `pdf`: `pdf::close`, `obj_release`), load the session through
`session::with_replica(…|r| r.receive(batches, PeerRef::Import))` / `session::put_asset`, then
`open::finish_foreign(app, host, ok)`. Unclaimed, the open fails with `Could not open this document. Try again.`

## Verified

- `cargo test --locked --workspace`: 89 + 58 green. Session ordering (journal append before the engine sees the
  batch), reload from chunks + journal with the writer resumed, retention and `Compacted`, degraded mode (open
  blocked; quota after retries) with the error state until sealed, the remote-apply budget in contact / per frame /
  idle, snapshot merge, a second tab that must not compact, the journal cap, the §6.6 table row by row and its
  precedence, 5.18.0's texts, links, titles, stats.
- e2e `internal-doc.spec.mjs` (4 tests, the plan's `doc.selftest`): a v6 container (built in the spec: Keywords +
  one attachment) opened through `/?drive=`, state checked, **reload → identical state without a second download**,
  write path + third load (journal replay), save indicator and popup, doc stats; every `host/pdf.js` op against
  the real pdf.js and its limits; viewer banner; failing open; new document with a reserved id reopened before it
  exists in Drive. 00-boot, 01-signin, 14, 14b, 15 still pass.
- Parity `--quick --states open`: **0 of 14 equal — not met.** Measured: in `open/loading` the overlay, spinner and
  `Opening PDF...` are identical; the differences are the title (`template` vs the markup's `Untitled`) and the
  toolbar being visible (read mode) — W3.2. `open/loaded`, `save-details`, `editor-menu`, `viewer-banner` open 5.x
  fixtures, which v6 cannot import before W3.4 (the open fails and returns to Home). `open/failed` shows Home, whose
  lists are W4.1's. These states are the gate for W3.1 + W3.2 + W3.4 together; rerun them then.
- W1.3's states are unchanged by this unit: `IHN_SHELL_ONLY=1 … --quick --states auth,profile-menu,updater` gives
  the result of NOTES-W1.3 (48 of 52 equal; the 4 `auth/reconnect*` cells differ as before).

## Deviations from the plan

1. **Size budget and the report-only switch** (above): `scripts/build-wasm.mjs` and `store/selftest.rs` are not
   owned files.
2. **No v6 fixture exists** (the export is W5.4): the e2e builds its container by hand from `doc.session.fixture`.
   The plan's `doc.selftest` is the spec; the commands are `doc.session.{state,fixture,stroke,compact}`,
   `doc.open.{state,open,pdf_probe,pdf_probe_result}`, `doc.create.new`, `doc.save_state.state`, `doc.stats.line`,
   `doc.links.state` (a command name must start with a module name of `app::MODULES`).
3. **Drive requests of the open are issued in `open.rs`** with `drive::client` (the module table puts "open
   download" in `drive/home.rs`, which is another unit's and does not exist yet). No download progress text
   (`Downloading... N%`): the fetch executor reports no progress; the overlay keeps `Opening PDF...`.
4. **5.x/plain PDFs are a seam, not a stub** (`sig::FOREIGN`), see above.
5. **Two assertions of W1.3's shell tests changed** (`shell/mod.rs`, tests only): the editor menu now also writes the
   statistics line, and `shell::status` is followed by `save_state`'s correction, as 5.18.0's
   `refreshSaveIndicatorState` does at the end of `showStatus` (the class a status call paints lasts only until
   the presentation is re-derived — true in 5.18.0 too).
6. **Compaction quiet time is 1200 ms** (5.18.0's burst value) for every case; the 180 ms window is not used:
   the journal already makes edits durable, so compaction is never urgent.
7. **The session owns the `ihn6-net` lock** (requested at load, released at close), because compaction needs it
   before W4.5 exists.
8. **Asset reference counts**: `asset_ref(+1)` when a session first stores an asset; nothing decrements yet
   (retention, W5.5, deletes documents and should release their assets).
9. **`#save-timestamp`** is not written (hidden and `display: none` in 5.18.0); `lastSavedAt` is `session::local(app)
   .saved_at`.
10. **The id pool**: `create.rs` only takes ids and fetches 20 when the pool is empty; keeping it above 10 is
    `drive::relay`'s (W4.5), which must merge with what `kv.idpool` holds.
11. **Commit trailer** names the model that ran this session.
