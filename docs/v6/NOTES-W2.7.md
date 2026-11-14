# W2.7 SYNC-RELAY — notes for W4.5, W5.4 and integration

Files: `rust/crates/ihn-sync/src/{relay.rs,export.rs,relay_tests.rs}` (+ two additive lines in W2.6's `lib.rs`, see
deviation 1). Branch `v6-w2_7`. `cargo test -p ihn-sync`: 49 tests (15 of them W2.7's), < 1 s debug.
`IHN_SEED=<n>` shifts the seeds of the W2.7 scenarios (checked with 13 different seeds).

## What is there

- `relay.rs` (`Relay`, a `crate::Part`):
  - Codecs: `RelayFile` (§8.3 `Payload`), `archive_payload`/`decode_archive`, `wrap`/`unwrap` (`IHN6R`/`IHN6A`, ver,
    comp), `Record` (§8.4 value, ≤ 124 bytes with the key for the longest shape), `d16`, `ranges_of`, `legacy_keys`
    (5.x keys nulled by the first write), `perm_hash`, `relayed_by`, `valid_file_id` (ids from records are checked
    before they go into a request path), `stale_files` (housekeeping decision for §8.7's last row).
  - Writer (leader tab of an editor only): due rule of §8.5 (mode flush delay, 2 s cap, mailbox, `record_due`,
    60 s refresh in company, keepalive flush on hide/exit, `l` on suspend/leave); id pool (20, refill below 10) and
    sync folder (`x` flag); epoch rotation on first write of the UTC day, `permissionIds` change, archive rotation,
    404/invalid id; archive rotation when the deflated tail passes 48 KiB (old sealed tail → immutable archive; a new
    batch too big alone goes too); gen persisted (`RelayPlan`) before any upload, never reused (also above a gen
    found in our own record); file media PATCH/create ‖ record PATCH in parallel; seal (`RelaySealed`), retry of the
    same bytes with `min(30 s, 500 ms × 2^(f−1)) ± 20 %`, 409 ⇒ update, 404 ⇒ new epoch, 403
    `insufficientFilePermissions` on the record ⇒ no more writes until the poll says editor again, offline ⇒ waits for
    `online`; old epoch files, a file left behind by a crash during a rotation and export-covered archives deleted
    10 s after the seal; nulls foreign records (Drive copies) and GC'd records in the next write.
  - Reader (every leader, viewers too): per changed record ≤ 4 concurrent reads, digest check with 300 ms × 10
    retries, doc/inst check (a foreign doc id marks the record for nulling), archives for gaps (digest + doc check),
    actor check (only the installation's tabs and its migration actor), `receive(PeerRef::Relay(inst))`, then
    vouching (`presence.verified` only if the record is still the same value), profile, mail to us → `mail_in`,
    export descriptor → `KnownExport`, `RelayApplied`. A gap neither the tail nor the archives fill (archives GC'd, or
    batches only the writer's newest export holds) sets `need_export`. Own-record recovery: a record of ours whose gen
    is above `docs.relay.gen` (degraded storage, lost IDB) is read back: batches, gen, archives and seal adopted.
  - Record GC (§8.4): > 24 records ⇒ records whose relay file was written ≥ 30 days ago and whose ranges the newest
    export covers are nulled by the next write (once per session).
- `export.rs` (`Export`, a `crate::Part`, and `Desk` = `Core::export`):
  - Due rule (§14.1): scribe, VV not covered by the newest known export, 10 s without local input, ≥ 60 s since the
    last export (5 min while remote batches keep arriving), never during contact (contact aborts before the upload);
    leaving (Leave / hidden 60 s) while alone; forced (`desk.force`). A head nobody merged yet is downloaded and
    merged before any build (a 5.x save is never overwritten).
  - Loop (§14.2): Build (host) → CheckHead (`Meta`) → head = the one we built on, or a known export our VV covers ⇒
    `Put` with `If-Match`; otherwise download + merge (host) + build again (≤ 3 loops, then 30 s pause). 412 ⇒
    `Meta` again: same head ⇒ the same bytes at once (≤ 10 fast retries, no rebuild); new head ⇒ CheckHead path.
    Done ⇒ `KnownExport` + `LastExport` + `export_pending=false`, `ExportDone` to peers and tabs, a relay write soon
    (descriptor in the header), `desk.done = head`. Errors ⇒ backoff 2 s … 5 min, `export_pending`.
  - Known exports (§14.3): from own uploads, `ExportDone` frames and relay headers; a new head is downloaded only if
    nobody described it within 6 s, or the relay cannot fill a gap (`need_export`).
  - `classify(keywords)` (§15.1), `recovered_name`, `push_name` (§14.7 rule), `vv_tag` (`appProperties.ihn6_vv`).
  - Drive name (§14.7): a polled name different from `docs.drive_name` is recorded (`DriveName`) and, except on the
    very first observation, handed to the binding in `desk.rename`.

## How W4.5 / W5.4 plug in (the desk)

After every `tick`/`on_io`, the binding looks at `sync.core.export` and answers, then calls `tick`:

| Out (set by the part) | Host work | In (set by the binding) |
|---|---|---|
| `build = Some(job)` | export worker: PDF of the current state (ARCHITECTURE §8.2) | `built = Some((job, Some((bytes, vv))))` or `Some((job, None))` |
| `merge = Some((job, head, modified_ms, bytes))` | pdf.js Keywords → `classify`; V6 `load_snapshot`, V5 `v5::import::from_pdf(…, ActorId::import(head), modified_ms)` + `receive`, Plain on an existing doc: save the sibling `recovered_name(name)` | `merged = Some((job, Some(vv)))` (snapshot VV / the import actor's batches / empty), `None` on failure |
| `rename = Some(name)` (take it) | emit a `DocName` op stamped now if the register differs | — |
| `done = Some(head)` | export to the user downloads this head | — |
| — | user Export / headless `export_pending` | `force = true` |

The session stays in `Leaving` while `build`/`merge` are pending (≤ 30 s).

Executor contract the parts rely on (W4.5 `drive/relay.rs`, W5.4 `drive/revisions.rs`):
- `HttpReq::Put` answers 2xx with the new **`headRevisionId` as the body** (request `fields=headRevisionId`) and the
  new v2 etag; 412 when `If-Match` fails.
- `HttpReq::Create` puts the file in the account's sync folder (`parents`), answers 409 when the reserved id exists,
  400 for an id Drive refuses. `HttpReq::Update` answers 404 for a deleted file.
- Error answers carry Drive's error `reason` in the body (`rateLimitExceeded`, `userRateLimitExceeded`,
  `insufficientFilePermissions`); `status` 0 = no answer.
- `HttpReq::SyncFolder` answers `<id>` or `<id>\n1` (link sharing refused); `GenerateIds` one id per line.

## Assumptions still to verify on real accounts (W1.5 did not run)

Built against W2.6's Drive mock and SYNC §23; each of these is what the code relies on:

1. `properties` writes are refused (403 `insufficientFilePermissions`) to readers **and commenters**. §13.1 (owner /
   permissions-list fallback) is **not** implemented; if this fails, seals prove nothing and §13.1 must be added.
2. Files in a folder shared "anyone with the link: reader" are readable with OAuth by other signed-in accounts without
   a resource key, and `allowFileDiscovery:false` hides them.
3. A just-written file is served to other accounts within ~1–3 s (the digest retry covers 3 s; after that the reader
   waits for the next record change).
4. 30 public properties per app per file; key + value ≤ 124 bytes (records are ≤ 104 bytes with the key).
5. The v2 etag changes on any `properties` write and the `If-Match` media PUT answers 412 (the fast retry depends on
   it), while `headRevisionId` does not change on metadata writes.
6. Sustained `properties` PATCHes at 3 × 0.5 /s on one file are not throttled often; throttling answers 403 with
   `rateLimitExceeded`/`userRateLimitExceeded` in the body (or 429).
7. `permissionIds` is returned to writers by `files.get` and changes when a person or link permission is removed.
8. `generateIds` ids are accepted for documents and for files in the sync folder, are ≤ 44 characters, stay valid
   ≥ 7 days unused, and a re-create of an existing id answers 409 (lost-response recovery).
9. A media PUT can return `headRevisionId` (`fields=`), and the owner can delete link-shared files in its folder.
10. `fetch(keepalive)` relay flushes (file ≤ 48 KiB + record) fit the 64 KiB per-page keepalive budget.

## Deviations

1. `lib.rs` (W2.6's): one additive field `Core::export: export::Desk`, and `Leaving` also waits while
   `export.build`/`export.merge` are pending. Building a PDF and merging a revision are host work and `SyncEffect`
   has no family for them; this was the smallest way, like W2.8's `Core::live`.
2. Asset files (§8.3 row 3, §8.6 step 5, §8.7 row 3) are not uploaded or downloaded: `Replica` exposes assets only as
   host `BlobHandle`s (W2.8 deviation 3, same cause). The header carries `docs.relay.assets` (empty until a blob
   read/write effect exists); images then reach other devices through the export attachments. The version-asset
   budget (≤ 12 uploads/h) is therefore not exercised.
3. §13.1 not implemented (assumption 1 above).
4. Record GC's "unchanged for 30 days" uses the relay file's `written_at` (the writer's clock; skew is irrelevant at
   30 days) because the receiver-local first sighting is not persisted (`Stored` has no field for it).
5. The tail starts above the archives **and** above what is both sealed and in the newest known export (so a GC'd
   archive never grows back into the tail); readers detect batches that only that export holds through the export
   descriptor in the same header and download the export (§8.8 step 3).
6. A new head nobody described is downloaded after a 6 s grace (§14.3 leaves the timing open): the descriptor
   usually arrives first through `ExportDone` or the exporter's next relay write, so the scribe's exports cause no
   downloads (tested).
7. Every relay write rewrites file + record (mailbox, record refresh and `l` included); all count against the cap.
8. Pushing the Drive `name` with an export is W5.4's (the upload request has no name field); the rule is
   `export::push_name`. Housekeeping of stale relay files needs a `files.list`, which is Home's (W5.x):
   `relay::stale_files` is the decision.
9. Crash point "before the IDB commit" belongs to the session; the crash test covers "committed, not yet handed to
   sync" plus the three relay crash points.
10. No wasm size number: nothing in `ihn-app` instantiates `Sync` yet (W4.5), so the module is not linked.
