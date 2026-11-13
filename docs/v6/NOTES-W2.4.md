# W2.4 STORE — notes for the other units

Branch `v6-w2_4` (from `v6-w1_1`). Owns `rust/crates/ihn-app/src/store/*`, `host/idb.js`; adds
`tests/v6/e2e/internal-store.spec.mjs`.

## What exists now

- `host/idb.js`: complete executor of effect families 3.2 (IndexedDB: open, close, transaction, delete database,
  read-only 5.x info/read) and 3.9 (Web Locks: request exclusive/`ifAvailable`, release, query). Op numbers and
  payloads: `store::fx` in `store/mod.rs`; operations of a transaction: `store/tx.rs`; keys and values:
  `store/sv.rs`.
- `store::tx(app, host, req, Tx)`: one IndexedDB transaction as one effect. `req` is **your** request id
  (`app.req(Mod::X)`); the completion is an `Event::Io` at `X::event`, read with `store::Reply`. Exactly one
  completion per call, never inside the call. `Tx` has the typed operations `kv_*`, `doc_*`, `chunk_*`,
  `ops_append/put/scan/scan_actor/trim`, `asset_*`, `catalog_*`, `persist` and the generic `get/put/del/scan/count/
  patch/add`.
- Schema `inhouse-notes-v6` v1 (frozen): stores `docs`, `chunks`, `ops`, `assets`, `catalog`, `kv`, all with
  out-of-line keys and no indexes. `schema::DocRecord` is the `docs` record (stored as a plain object, layout in the
  header of `schema.rs`), `schema::kv` the `kv` keys, `schema::Record` the `SyncRecord` mapping
  (`Tx::persist(account, doc, &mut record, Record)` writes the store fields **and** updates the in-memory record).
- The database opens with the first transaction (or `store::open`); a signed-out visitor never creates it.
- Degraded mode: `store::health(app)` / `store::degraded(app)`; a transaction that was not attempted completes with
  `status::DEGRADED`. Causes: open blocked 1.5 s, open timeout 5 s, open error, no IndexedDB, or a write that still
  fails with the quota after 3 retries (250/500/1000 ms). While degraded for the quota, reads still run.
  `store::reopen` tries again.
- **Order**: write transactions of a connection run one at a time, in the order they were issued (the next is
  sent when the previous completed), so a write that is retried (quota backoff, lost connection) is never
  overtaken and no batch becomes durable past a hole; when the first one degrades the store, the ones behind it
  complete with `DEGRADED`. A transaction issued while others wait runs after them (a read sees the writes
  issued before it); with nothing waiting a read is sent at once. A retry that comes due while the database is
  reopening waits for the open at the head of the queue (it is not reported as `DEGRADED`).
- Web Locks: `store::locks::{request, release, query, names}` with the names `slot_name/net_name/migrate_name`;
  `locks::acquire_slot(app, host, account, notify_req)` takes the first free `ihn6-slot:<account>:<n>` for the tab's
  lifetime and reports the slot as `Io::status` (`locks::EXHAUSTED`, `status::UNAVAILABLE` otherwise);
  `locks::slot(app, account)`, `locks::actor(inst, slot)`.
- 5.x dumper: `store::v5::{info, read}` → `Info` (per store: count, greatest numeric key, greatest stamp field),
  `Info::fingerprint()` and `Page` (records as `sv::Value`; a `Blob` is a JS handle + size + type). `store::delete_db`
  for the migration's last step.
- Cargo tests: `store::fake::MemDb` is an in-memory executor of both families with IndexedDB semantics (key order,
  atomic transactions, `add` conflicts, locks, 5.x databases, injected open/quota failures). Other units' cargo
  tests use it: `let (mut app, mut host, mut db) = store::fake::boot(); … db.pump(&mut app, &mut host);`.

## Things to know when using it

- **A transaction's operations run strictly in order** in the host (each is issued when the previous one
  finished), so `patch`/`add`/`put_if_absent` see everything before them in the same transaction.
- **64-bit ids** are 16 lower-case hex digits wherever they are keys (`ops` key `[docId, actor16, seq]`, object
  keys of `seq`/`sealed`/`applied`, `kv` `inst` and `ctr:<actor16>`). Hashes are 64 hex digits (`assets` key).
- `Tx::ops_append` = `add` of the batch + patch of `docs.seq[actor]`. Its two results are `undefined` and a
  boolean: **false means the `docs` record does not exist** (deleted by retention in another tab); the batch is
  stored, the seq is not — write the record again with `doc_put`. A `seq` already in the journal aborts the whole
  transaction with `status::CONSTRAINT`. The seq patch (`Tx::doc_seq`, also `Record::Seq`) is a maximum: the
  stored `docs.seq[actor]` never goes back.
- Document patches never resurrect a deleted record (`doc_patch` is "only when it exists").
- `asset_get` and scans of `assets` return a **new JS handle per `Blob`**; release them (`Value::handles`,
  `OutBuf::obj_release`). `asset_keys` lists hashes without creating handles. `asset_put` takes the handle of a
  `Blob` the caller already holds and keeps the caller's ownership of that handle.
- **References may arrive before the bytes**: `asset_ref` on an asset that is not stored counts in a record
  without `blob` (`{refs}`), and `asset_put` then adds `blob/mime/size` to it and keeps the count. So an asset
  record **without `blob` means "missing"** (`asset_get`), and `asset_put`'s result is the record as it was
  before: `undefined` or an object without `blob` = this call stored the bytes; with `blob` = already there
  (release that handle). `asset_keys` lists such records too.
- Retention: `DocRecord::prunable(open, now, days)` + `schema::retention_days(preferences)` + `Tx::doc_delete`.
  `prunable` also requires `seq[actor] <= relay.sealed[actor]` for every actor in `seq` (the flags are written
  later than the append, and only by the lock holder), so `RelaySealed.sealed` must cover every actor it seals.
  "Open" must include other tabs: query the `ihn6-net:<account>:<docId>` lock (`locks::query`) before deleting.
- Journal retention (24 h, 8 MiB) is the session's rule: the store keeps a batch as its bytes; age comes from the
  batch clock, size from the bytes. `ops_trim(doc, actor, through)` deletes a prefix of one actor's journal.
- `docs` is keyed by `docId` only (ARCHITECTURE §7.1); `docs_scan` returns every account's records — filter on
  `DocRecord::account`.
- Durability is the browser's default (`relaxed`: survives a tab or browser crash). Set `tx.strict = true` for a
  transaction that must survive power loss before its completion is acted on.
- 5.x dumps: `v5::read` pages by count and bytes (`after` = last key of the previous page). The host never creates
  or upgrades a 5.x database and closes its connection after every request. `Info::fingerprint()` leaves out
  `v5::VOLATILE_STORES` (`leases`, `drive-open-cache`), which 5.x rewrites without user edits.

## Deviations from the plan

1. **`SyncRecord` is mirrored as `store::schema::Record`** with the fields of the ARCHITECTURE §7.1 table:
   `ihn-sync` (W2.6) does not define the type yet and `ihn-app` owns the mapping. `RelaySealed` carries the new
   `relay_pending` value (the table maps it to `docs.relay.*` + `flags.relay_pending`). W2.6/W3.1 convert or re-export.
   `schema::Stamp` likewise mirrors §4.1 until `ihn-core::stamp` (W2.1) exists; it is stored as 20 bytes.
2. **Values are plain objects, not opaque blobs**, crossing the boundary in a small tagged binary form
   (`store/sv.rs`). Needed so that two tabs can patch single fields of one `docs` record atomically, and it is the
   same form the 5.x dumper returns.
3. **The 5.x dump is a structured-value tree (`sv::Value`), not bytes for `ihn-core::v5::idb`**: `ihn-core` cannot
   depend on `ihn-app`. W2.2 decodes from its own input type; W5.6 adapts `sv::Value` to it (or `sv` moves to
   `ihn-core` at integration — it has no dependencies besides `ihn_core::codec`).
4. **The fingerprint is stronger than "record count + greatest `savedAt`/outbox id"**: per store it takes the
   count, the greatest numeric key and the greatest of `savedAt, updatedAt, createdAt, cachedAt, generation,
   contentVersion`, hashed per store, because 5.x offline documents carry `updatedAt`/`generation`, not `savedAt`.
5. **Lazy open** instead of opening at boot (no database for signed-out visitors; boot does no storage work).
6. **`internal-store.spec.mjs` is a new file** outside the owned list (the plan names the self-test but no spec).
   Its dump tests need the W1.2 fixtures and `restoreStorage`; they skip when those are absent and run against
   another checkout with `IHN_W12_ROOT=<path>`.
7. **Quota retries are done by the store for every write**, not by each caller (SYNC §7.2 describes them for the
   session's append): one place, and because writes are serialised the order holds through a retry.
8. **`DELETE_DB` reports `BLOCKED` after 1.5 s but the deletion stays requested** (IndexedDB cannot cancel it): it
   completes when the blocking connection closes. Deleting the `inhouse-media-*` Cache Storage entries is not an
   IndexedDB operation and is not in this executor.
9. **Commit trailer** names the model that ran this session.

## Measured

`ihn.wasm` 141 KB (55 KB gzip) from 80 KB: the store is ~47 KB of code, of which roughly 20 KB are the e2e
self-test scenarios (`store/selftest.rs`) and the test commands; the rest of the growth is `alloc`/`core` generics.
`host/idb.js` 340 lines / 17 KB (first-party JS 66 KB of 100 KB).
