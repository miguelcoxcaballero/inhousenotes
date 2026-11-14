# W5.5 HOME-DIALOGS — notes for integration and later units

Branch `v6-w5_5` (from `v6-w5_4`). Owns `ihn-app/src/home/dialogs/*`, `ihn-app/src/drive/manage.rs`,
`tests/v6/e2e/{03-docs,03b-home-actions}.spec.mjs`, `tests/v6/parity/states/home-dialogs.mjs`.

## What exists now

- `drive::manage`: 5.18.0's requests for create folder, star, colour, trash, restore, move (`GET parents` then
  `PATCH addParents/removeParents`, URLSearchParams encoding), copy, Bin list, picker listing, resumable device upload
  (session POST + one PUT of the whole blob). Copy reads the source's `properties` first and sends every `ihn6_*` key
  as `null`. Picker and Bin listings add `appProperties` to 5.18.0's `fields`, so the sync folder and sync files are
  never offered (`Catalog::hidden`).
- `home::dialogs` (`mod.rs`): create menu (FAB, placement, focus, Escape/arrows — B-H2, outside `pointerdown`,
  resize), upload input (`files_take` → picker `upload`), `Make a Copy` (`#btn-make-copy`), document click/Escape
  closers. **It also answers `home::sig::OPEN_FILE` with `doc::open::open_file`** — nobody did, so Home card clicks
  opened nothing; move that line to `doc::open` if the lead prefers.
- `picker.rs`: folder picker in modes new/move/copy/upload (5.18.0 texts, meta path resolution with the folder
  metadata cache, breadcrumbs, listing from Drive or the library offline, Enter/overlay/cancel), confirm actions
  (new → `doc::create::new_document`, move, copy → open the copy, upload), New folder modal.
- `menu.rs`: card ⋮ menu (rendered on open, moved to `<body>`, flipped above the bottom bar next frame, returned
  on close), colour popup (24 swatches, clamped next frame), star, colour, offline pin (`docs.pinned`), move, bin.
  `home::nav::close_card_menu` now calls `menu::close_menu` (one-line seam in W2.5's file).
- `bin.rs`: move-to-bin modal with its own slider (90 % threshold, 120 ms), Bin list (5.18.0 row markup), item menu,
  Restore.
- `offline.rs`: Offline documents dialog (retention from `kv.preferences`, queue line from the catalogue's pending
  documents, recovery rows, Save → retention + 5.18.0's prune with `DocRecord::prunable` + Home reload),
  upload-error dialog (opened by `home::sig::SYNC_ERROR`; summary, redacted report, Copy error, Retry →
  `home::sig::RETRY_SYNC`), Download-app modal.
- Test command `home.dialogs.state`.

## Verified

- e2e 03-docs, 03b-home-actions (3 tests) pass; 00, 02, 08, 09, 15 still pass (20 tests). `cargo test --workspace`
  green.
- Parity `--quick --states home-dialogs`: 10 states, 34 comparisons, pixel-equal (offline-settings carries the
  `VERSION-LABEL` expectation of profile-menu.mjs: the open menu's `v6.0.0`).

## Deviations

1. **No "delete forever"** in the Bin and no "⋮ rename": 5.18.0 has neither (Bin offers Restore only; renaming is the
   editor title). Nothing is queued in `kv.relay-gc` from Home; trashed documents' relay files are found by
   housekeeping (SYNC §8.7, "trashed"). The 03 acceptance "rename survives export" is checked through the export
   name (`export.user.export {name}` → Drive name `Lecture one.pdf` → reopened title).
2. **BEHAVIOUR-CHANGES N-13** (added): no offline queue for device uploads and new folders; Save offline needs a copy
   on this device. Uploads are not resumed across reloads (no resume-key session cache).
3. Parity states not written: offline settings with `N pending` / recovery rows and the upload-error dialog need
   seeded 5.x storage (outbox rows, recoveries, an outbox error) on the 5.18.0 side and v6 flags on the other; left
   for W6.1's sweep. The e2e 03b covers both dialogs' content.
4. `kv.recoveries` format chosen here (W5.6 writes it): an array of 5.x recovery records
   `{name, createdAt, localBlob, remoteBlob}` (Blobs), shown newest first.
5. Retention pruning runs only on Save in the dialog; 5.18.0 also prunes 2 s after start (`init`), which no unit
   does yet.

## Found, not mine

- `editor/view.rs`: typing in `#doc-title` never commits — the `focus` event does not mark the title as editing
  (no `editing` class, blur on Enter not applied), so a rename from the title is lost. 03 renames through the export
  name instead.
- Opening from a stale Home card shows the card's old name as the title until something else updates it; the title
  does not follow the `DocName` register after load.
- `home::sig::RETRY_SYNC` has no receiver (the badge and the error dialog's Retry both send it).
