# W5.6 MIGRATION — notes for integration

Branch `v6-w5_6` (from `v6-w5_4`). Owns `rust/crates/ihn-app/src/migrate.rs` and `tests/v6/e2e/13-migrate.spec.mjs`.

## What exists now

- `migrate` runs at every foreground boot of a signed-in account (and on `sig::AUTH`/visibility while idle). It
  takes `ihn6-migrate:<account>` (`ifAvailable`; another tab migrating ⇒ nothing) and reads `kv.migration`
  (`{fp, docs: [[v5, v6]], folders: [[local, drive]], written, catalog, deleted}`). Once `deleted` is true the
  boot cost is that lock and one `kv` read. Without any 5.x database or key it only stores `deleted: true`.
- **Dump**: `store::v5::info` of the four databases (fingerprints), then `documents`, `outbox`, `entries`,
  `settings`, `recoveries` of `inhouse-offline-v1` and `snapshots`/`pages`/`stroke-ops` of both notebook databases;
  the 5.x localStorage keys (`LS_KEYS`). `sv::Value` → `v5::json::Value` with `migrate::to_json`.
- **Plan** (`migrate::plan`, pure, cargo-tested on every storage fixture): offline rows that are dirty / offline
  edited / in the outbox / `local-`; the working copy of `drive-session-v1` (`offlineLocalId`, `fileId`, else the
  pending-exit marker), working copy first then the offline snapshot; pending folders from outbox `folder` ops.
- **Write**, one document at a time through `doc::session` (only when the session is free and the view is not the
  editor; an open from Home takes the session over and the document is redone later): pool id for `local-`
  documents (recorded in `kv.migration` before anything else), Drive base (cached PDF when it is still the head,
  else `alt=media`; offline the cached PDF or nothing; a v6 container head is loaded as a snapshot), imported by
  `doc::v5open::import`; `originalBlob` as a `Saved` source asset; `from_local` per source → batches received into
  the session (journalled); the record written whole (`session::write_record`) with `seq[migration actor]`,
  `relay_pending`, `export_pending`, `flags.migrated`, the base head as a known export; Home catalogue entry for a
  new id. Pending folders are created in Drive first (their documents wait for them).
- **Seal**: `export::headless::run` (foreground runner) relays, creates and exports; the migration checks every
  4 s: done = `created && relay.sealed[mig] >= seq[mig]` (`migrate::doc_done`).
- **Delete**: fresh `info` equal to the stored fingerprints (else the dump is read again and the difference
  migrated), then the service worker census (`{windows, v6}`, every window must be v6), then `deleteDatabase` ×4,
  the `inhouse-media-*` caches and the localStorage keys. Any blocked deletion ⇒ phase `Blocked`, retried at the
  next boot (or visibility change).
- Test commands: `migrate.state`, `migrate.run`, `migrate.has {ids}` (5.x stroke ids missing from the open doc).

## Changes outside the owned files (seams)

1. `doc/v5open.rs`: `import(app, host, Foreign, notify)` (the base import outside an open; completion by
   `notify`), and **`supersede` now keeps this installation's migration actor** (it passed `&[]`: a later 5.x
   revision would have deleted migrated strokes, contrary to NOTES-W2.2 and SYNC §15.4).
2. `doc/session.rs`: `write_record`; **journal trimming no longer drops unsealed migration-actor batches by age**
   (their clocks are 5.x times, usually older than 24 h: they were trimmed by the first compaction and never reached
   the relay). In the 8 MiB cap they count as own batches.
3. `export/headless.rs`: `running`, `run`; the runner does not start while the migration holds the session.
4. `host/main.js`: effects `0x73` (service-worker census through a `MessageChannel`, 3 s timeout) and `0x74`
   (delete caches by prefix). `service-worker.js` already answered `{type:'ihn6-clients'}`.

## Deviations

1. **Foreground only**: the Android headless page does not migrate (it has no service worker for the census).
2. **Timeline rows are not migrated**: `doc/versions.rs` has no record codec to write `V5Version`s into
   (NOTES-W2.2). The versions are in the 5.x Drive revisions, which `from_pdf` imports when opened.
3. **Recoveries**: `kv.recoveries` lists `{id, fileId, generation, createdAt, exportName, asset}`; the asset key is
   `sha256("ihn6-recovery:" ‖ id)` (no digest round trip), not the content hash. The snapshot is not kept (its
   strokes are in the working copy / offline row that W2.2's tests cover).
4. `ihn-live-*` localStorage keys are not removed (the host cannot enumerate keys); they are inert in v6.
5. The catalogue is written after Home may already have rendered from cache on the first v6 boot (Home shows the
   Drive lists online; offline the migrated entries appear at the next render).
6. A Drive document whose head moved past the 5.x base is migrated over the **current head** when online (the
   export then needs no 5.x merge, which W5.4 does not answer during a session).
7. Commit trailer names the model given by the workflow.

## Verified

- `cargo test --workspace` green (new: plan over all 9 storage fixtures incl. account filtering; progress round
  trip; the done rule).
- e2e `13-migrate` 12 tests (~1.6 min): every dump (background-worker, local-doc, local-folder, offline-dirty,
  pending-exit, recoveries, recovery-conflict, stroke-ops-pending, xor-packed) → every expected stroke in the v6
  document and on a fresh device, `local-` doc created once with a reserved id, folder created once, 5.x databases
  and keys deleted, no errors; crash mid-migration resumes with no duplicates (exact live count); a 5.x window kept
  open blocks deletion and its later stroke is migrated after it closes; a held 5.x connection ⇒ `Blocked`, next
  boot deletes.
- Other suites with this branch: 00, 09, 10, 12, 14b pass; 08 passes except `412 … keeps the foreign copy`, which
  fails the same way on `v6-w5_4` (base build, same machine); `internal-store` fails on the known `401` console line
  of the signed-out probe (NOTES-W3.1/W4.5). Quick parity `home-shell`: 12 of 14 equal; `home-shell/reconnect`
  differs identically on `v6-w5_4`. The unit owns no parity state.
