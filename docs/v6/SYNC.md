# Inhouse Notes 6.0.0 — Sync protocol (network documents that sync instantly)

Status: definitive design of the sync protocol, revision 2 (2026-10-30, after review). Binding inputs:
`REQUIREMENTS.md` (user decisions), `ARCHITECTURE.md`, `spec/collaboration.md`, `spec/persistence.md`,
`spec/documents.md` (5.17.22 behaviour; 5.18.0 wins where it differs). Scanner, agenda/diary/calendar sync are out of
scope.

**Ownership of text.** ARCHITECTURE.md §4 is the single normative text for the document model, stamps, actors,
version vectors, the `Replica`/`SyncEngine` interface and the storage schema (§7.1 there, including every field this
document persists). This document owns everything that moves bytes between replicas: encodings on the wire and in
Drive, transports, discovery, presence, security, acknowledgement, catch-up, and **when and by whom the PDF export is
written** (§14 here is the single source; ARCHITECTURE §8 owns only the container and upload mechanics). Behaviour
changes are recorded only in `BEHAVIOUR-CHANGES.md` (ids `B-Y*`); this document cites them.

---

## 0. The design in 15 lines

1. Every edit is a `Batch` (ARCHITECTURE §4.2): LWW registers stamped `(clock, actor, idx)`, idempotent and
   commutative. Sync needs only **at-least-once delivery with dedupe by `(actor, seq)`** — no causal order, no locks.
2. Durability before network: a local batch is committed to IndexedDB `ops` before any transport sees it (degraded
   mode without IndexedDB: §7.2).
3. Four transports carry the same encoded batches: **tabs** (BroadcastChannel), **P2P** (WebRTC data channel,
   ~1 RTT), **Drive relay** (one small single-writer file per installation and document), and the **export** (the
   PDF, a derived self-contained snapshot, never on the edit path).
4. Only the author uploads its own batches. Receivers never re-upload what they received (fixes 5.x amplification).
5. Relay files live in the writing account's **link-readable, non-discoverable "Inhouse Notes sync" folder**, so
   signed-in collaborators of any account and viewers read them without permission copying. A relay file's id
   **rotates** daily, on archive rotation and whenever the document's permissions change, so a removed person stops
   seeing new content (§8.1, B-Y2).
6. **Edit proof = the ability to write the document's Drive `properties`.** A relay file is applied only up to the
   generation *sealed* by its record property `ihn6_<inst>` (digest). Viewers can read but never seal.
7. One poll answers everything: `files.get(fields=properties,headRevisionId,…)` returns every device's record.
8. Cadence by mode: **Alone** 1 GET / 20 s while visible (0 hidden, 0 writes unless editing), **Company** 500 ms,
   **Linked** (P2P to everyone present) 10 s. Relay writes (file + record) are capped at **1 per 2 s per device**.
9. P2P signalling: plain SDP offer/answer entries inside the sealed relay files; the DTLS fingerprint in an
   authenticated SDP binds the channel. No extra encryption layer (B-Y4).
10. Live in-progress ink is streamed per animation frame over P2P (`InkPts`), then superseded by the batch.
11. Catch-up after days offline: export snapshot (only if the relay cannot fill the gap) + relay tails/archives.
12. The export is written by one elected device (the **scribe**) when idle, by a device leaving alone, and by any
    editor on demand (**forced export**: user Export). Bytes are built first, then a conditional PUT; a 412 whose
    head revision did not change is metadata churn and is retried at once with the same bytes (§14.2).
13. A 5.x revision is imported deterministically as batches of a per-revision import actor and **merged**.
14. `ihn-sync` is a sans-IO state machine (effects in, events out); `host/peer.js` is a ~200-line socket shim.
15. Proof: cargo property tests + a deterministic Drive/P2P simulator (clock skew, crashes, throttling) + the
    two-device e2e spec with sub-cases (TESTING §2, spec 11) + a Wave-1 spike on real Google accounts (§23).

Signed-out public-link readers keep 5.18.0's mechanism (whole-PDF poll every 2.5 s), because production has no Drive
API key (§12.4).

---

## 1. Goals, targets, budgets

| Target | Value | Where it is met |
|---|---|---|
| Local ink | the same frame as 5.18.0 (≤ 1 frame) | sync does nothing synchronous on the pen path except `live()` (≤ 50 µs, no allocation) |
| Remote ink, P2P | first point < 150 ms P95 after it is drawn | §10.6, §17 |
| Remote committed batch, Drive relay only | **< 4 s P95** (typ. 1–3 s; the first stroke after a pause ≈ 1.5 s) | §8, §17 |
| Signed-in viewer while editors write | < 4 s P95 | §12.1 |
| Signed-out public link | as 5.18.0: changes appear after the next export, polled every 2.5 s | §12.4 |
| Join (second device opens the document) | content at once from cache/export + relay; P2P ≤ 20 s + ~4 s | §9, §10.4 |
| Requests while alone, visible, idle | ≤ 3 GET/min, 0 writes | §9.2 |
| Requests while alone, visible, writing | ≤ 3 GET/min + ≤ 4 relay writes/min (file + record) + ≤ 12 version-asset uploads/h | §9.2, §14.6 |
| Requests per device in Company | ≤ 2 GET/s, ≤ 0.5 relay writes/s (record cap) | §8.5 |
| Requests while hidden > 60 s or on Home | 0 for this document | §9.3 |
| Data loss | none: every local batch is durable locally before sending and sealed in Drive by its author | §7, §8 |

Non-goals for 6.0.0: TURN relays, a signalling server, Drive push notifications (all need a server), live ink for
devices that cannot reach each other by P2P, interop with 5.x clients (REQUIREMENTS §4), relay reach for signed-out
readers.

---

## 2. Identities (normative definitions in ARCHITECTURE §4.1)

| Identity | Form | Scope and lifetime |
|---|---|---|
| Document | Drive file id (string), **always**: new documents take a reserved id from the id pool at creation (§8.2); there are no `local:` ids | forever |
| Installation `inst` | random u64, low 8 bits zero, `kv[account,'inst']` | per (browser profile, account) |
| Actor | `ActorId(u64) = inst \| slot`, slot 0..255 held by Web Lock `ihn6-slot:<account>:<slot>` | per (installation, account, tab slot) |
| Import actor | `IMPORT_BIT \| (fnv1a64(source) >> 1)`, `IMPORT_BIT = 1<<63`; source = head revision id, or `"migrate:" ‖ inst13 ‖ ":" ‖ docId` | per imported 5.x revision / migrated document |
| Page, item, version | `Id(u128) = (actor << 64) \| counter`; 5.x ids map to `fnv1a128(string)` | forever |
| Stamp | `{clock: u64 HLC ms, actor, idx: u32}`; all ops of a native batch share `batch.clock`, `idx` = op index | per write |
| Batch | `(actor, seq)`, seq = 1, 2, 3… contiguous per (document, actor) | forever |
| Signalling session `sid` | random u64 per offer | ≤ 60 s |
| Live stroke `live` | random u64 per in-progress stroke | until its batch arrives or 15 s |

`inst13` = `inst` in lowercase base 36, left-padded with `0` to 13 characters.

---

## 3. Merge rules sync relies on (summary of ARCHITECTURE §4.1–4.2, which is normative)

1. Every field is an LWW register compared by `(clock, actor, idx)`; native stamps are unique. Import stamps are
   explicit per op; stampless 5.x data is stamped `{revision modifiedTime ms, import actor, idx}` so a newer revision
   wins deterministically (§15.2).
2. `ItemPut` for an unknown item creates it; ops naming unknown pages/items are stored and appear later.
3. Deletions are registers kept forever; the inverse of a deletion carries the body.
4. HLC: next batch clock = `max(prev + 1, wall_ms, observed + 1)`; a received clock greater than `wall_ms + 24 h` is
   applied but not observed (a corrupt clock cannot drag every device forward).
5. `VersionVector[actor]` = highest contiguous seq; batches above a gap apply at once and sit in an extras set.
6. All pages deleted ⇒ `order()` shows the page with the greatest `deleted` stamp (projection only).
7. Snapshots merge register by register and VV pointwise; never replace.

---

## 4. Encodings (exact)

All binary data is little-endian. One codec module (`ihn-core::codec`) implements the primitives for the model,
the wire and the relay files.

### 4.1 Primitives

| Name | Encoding |
|---|---|
| `uv` | unsigned LEB128, at most 10 bytes; overlong or > u64 is an error |
| `u8`, `u16`, `u32`, `u64` | fixed width LE |
| `f64` / `f32` | IEEE-754 LE bit pattern, any value including NaN preserved bit-exactly |
| `bytes` | `uv len` + raw |
| `str` | `bytes` holding UTF-8; invalid UTF-8 is an error |
| `Id` | 16 bytes (u128 LE) |
| `Hash` | 32 bytes |
| `VV` | `uv n` + n × (`u64 actor`, `uv seq`), actors strictly ascending, seq ≥ 1 |
| `Range` | `u64 actor`, `uv first`, `uv last` (first ≤ last) |

Decoders never panic: truncation, overflow, trailing garbage or a limit violation rejects the whole frame/file.

### 4.2 Batch (`EncodedBatch`)

```
Batch    := u8 ver=1, u64 actor, uv seq, uv clock, uv n_ops, OpEntry × n_ops
OpEntry  := [Stamp]? Op            // Stamp present iff actor & IMPORT_BIT (import/migration batches)
Stamp    := uv clock, u64 actor, uv idx
```

Native batch: op *i* has stamp `{clock, actor, i}`. Limits: `n_ops ≤ 1,000,000`, encoded size ≤ 32 MiB (asset
bytes never travel inside batches).

### 4.3 Ops (tags match ARCHITECTURE `Op`)

```
0x01 PageNew     Id page, bytes pos, u8 template, uv w, uv h, Background
0x02 PageMove    Id page, bytes pos
0x03 PageDel     Id page, u8 deleted(0|1)
0x04 PageSet     Id page, u8 prop, value
                   prop 0 Template: u8      prop 1 Size: uv w, uv h
                   prop 2 Background        prop 3 SidePanel: u8 present, [bytes]
0x05 ItemPut     Id item, Id page, Z, Body
0x06 ItemDel     Id item, u8 deleted(0|1)
0x07 DocName     str name
0x08 VersionAdd  bytes meta            // VersionMeta codec owned by ihn-core::history
0x09 VersionDel  Id version

Z          := Stamp, uv sub                                     // ARCHITECTURE §4.1
Template   := 0 Default | 1 Agenda | 2 Diary
Background := u8 0 Template | 1 Pdf: Hash asset, uv page (1-based) | 2 Custom: Hash asset
Body       := u8 0 Stroke: u8 tool (0 Pen, 1 Highlighter, 2 LegacyEraseArea, 3 LegacyCover), Color, f64 width,
                           uv n, n × (f64 x, f64 y), u8 has_p, [n × f32 p]
            | u8 1 Image:  Hash asset, f64 cx, f64 cy, f64 w, f64 h, f64 rotation
Color      := u8 0 Hex: u32 0xRRGGBB | u8 1 Css: str
```

Unknown op tags are an error (all devices run the same release).

### 4.4 Snapshot

`EncodedSnapshot` is ARCHITECTURE's `snapshot.rs` format; sync transports it opaque. It always carries its `VV`.

### 4.5 JSON

JSON appears only in Drive API request/response bodies, built by `ihn-app::drive`. Nothing sync stores or sends is
JSON.

---

## 5. Undo and redo as inverse ops

Normative in ARCHITECTURE §4.3. Sync-relevant table:

| Forward op | Inverse emitted by undo |
|---|---|
| `ItemPut` creating an item | `ItemDel{item, true}` |
| `ItemPut` replacing page/z/body (transform, recolour, cross-page move) | `ItemPut{item, old page, old z, old body}` |
| `ItemDel{item, true}` (erase, delete, area-erase original) | `ItemPut{item, page, z, body}` **then** `ItemDel{item, false}` |
| `ItemDel{item, false}` | `ItemDel{item, true}` |
| `PageSet{prop}` / `DocName` | the old value |
| `PageNew`/`PageMove`/`PageDel` | not undoable (5.18.0 records no history for page operations) |

Undo and redo re-stamp, so they win over concurrent remote writes to the same registers, as 5.18.0 does.

---

## 6. Transport overview

```
 pointer ──► Tx ──► Batch ──► apply (Rust) ──► render (same frame)
                        │
                        └─► IDB ops append (txn commit) ──► SyncEngine::local_batch
                                                              ├─► tabs: BroadcastChannel   (µs)
                                                              ├─► P2P peers: Batches frame  (~RTT)
                                                              └─► relay writer (Drive)      (≤ 2 s cap + upload)
 live() per rAF ──► InkBegin/InkPts ──► peers + tabs (not durable)

 receive (any transport) ──► Replica::receive ──► dedupe (actor,seq) ──► apply (budgeted during pen contact,
                              ARCHITECTURE §4.4) ──► IDB append ──► repaint
                                                └─► forward to other peers/tabs lacking it (§10.7)
 export (scribe idle | leaving alone | forced) ──► PDF with snapshot + VV ──► Drive (If-Match)
```

Invariants:

1. A batch reaches any transport only after its IDB commit — except in degraded mode (§7.2). Live ink is never
   durable.
2. Every transport delivers the same `EncodedBatch` bytes; receivers dedupe by `(actor, seq)`.
3. **Durability in Drive** of a batch = it is inside its author's relay file (or archive) **and** that generation is
   sealed by the record. This clears "unsynced" everywhere: the save indicator, Home's pending badge and migration
   cleanup (ARCHITECTURE §6.6, §10.2). A P2P receipt is not durability.
4. The export is never needed for correctness; it is the human-readable PDF, a cache, and the point that lets relay
   archives be deleted.

---

## 7. Local durability and crash safety

### 7.1 Actors per tab

- `inst` is created once per (profile, account). At startup a tab takes the first free Web Lock
  `ihn6-slot:<account>:<n>`, n = 0..255 (`ifAvailable`, held for the tab's lifetime) and uses `actor = inst | n`.
  If all 256 are taken, the tab opens documents in 5.18.0's read-only viewer state (no new UI; B-Y7).
- Id counters: `kv[account,'ctr:'+actor]` holds a high-water mark reserved in blocks of 65,536.
- `seq` for (document, actor) = `docs[docId].seq[actor] + 1`, persisted with each `ops` append (same transaction).

### 7.2 Write path

1. Tool builds the batch and applies it (render in the same frame).
2. Session appends `{key:[docId, actor, seq], bytes}` to `ops` and bumps `docs.seq[actor]` in **one** transaction.
3. On commit: `SyncEngine::local_batch(bytes)`; the engine fans out (§6).
4. Commit failure (quota, transaction error): retry 3× with backoff 250 ms ×2. If IndexedDB is **unavailable** (open
   blocked 1.5 s / timeout 5 s, or quota exceeded after retries) the session enters **degraded mode**: batches are
   appended to an in-memory journal and sent at once; the relay seal becomes the only durability point; the UI shows
   5.18.0's local-save error state until every batch is sealed; `pagehide` issues the keepalive relay flush (§8.5).
   Leaving the editor in degraded mode waits for the seal (5.18.0's "keep this document open" rule).

### 7.3 Received batches and journal retention

`Replica::receive` applies and appends to `ops` in one transaction. Compaction (pen idle) folds journal entries into
page chunks but **keeps** them in `ops` until all of these cover them: the relay seal (own batches), the VV of every
known open peer and tab, and the latest known export; or until they are older than 24 h; with a cap of 8 MiB of
journal per document (oldest received batches go first). `batches_since` returns `Err(Compacted)` only when the
requested range has left the journal; the peer then gets a `Snapshot` (§10.7). A short reconnect or a new tab
therefore receives batches, not snapshots.

### 7.4 Restart

On open: load all chunks + `ops` (replay, idempotent) → VV; the relay writer re-reads every own batch with
`seq > sealed[actor]` and resumes (§8.5). `docs.flags.relay_pending/export_pending` resume work in the foreground or
the Android headless page (ARCHITECTURE §12).

---

## 8. Durable path: the Drive relay

### 8.1 Why this layout (and how it fixes 5.x)

5.x journal files lived in the writer's private folder (same-account only), were rewritten whole (O(n²)), were
discovered by `files.list` every 5 s, created a new actor per session and were never deleted
(`spec/collaboration.md` §8.10, §15.1 rows 13–18). v6:

- **Reach:** one folder per account, shared *anyone with the link: reader* (`allowFileDiscovery:false`); every file in
  it inherits that. File ids are published only in the document's own `properties`, which only the document's current
  readers see. So the document's readers, of any account, find the relay files with no permission copying. If link
  sharing is refused by policy, the record carries flag `x` and readers who cannot read the file rely on P2P and the
  export (5.x reach).
- **Revocation:** knowing an id is enough to read a link-shared file, so a person removed from the document could
  keep reading a stable file. Therefore the mutable relay file **rotates to a new id** (an *epoch*) on every archive
  rotation, at the first write of each UTC day, and at the first write after the poll shows that the document's
  `permissionIds` changed. The new id is published only through the record; the old file is deleted 10 s after the
  new record is written. Residual exposure (B-Y2): a removed person can read (a) what was written before the removal
  was observed by each writer — at most one write per writer — and (b) archives and assets whose ids they already
  knew, which hold only content they could read before removal, until export GC deletes them. Group-membership
  changes are not visible in `permissionIds`; the daily rotation bounds them to one day.
- **Single writer per file:** one relay file per (installation, document, epoch), written only by the tab holding
  `ihn6-net:<account>:<docId>`. No `If-Match`, no prefix conflicts; ids are reserved in advance, so a retry uploads
  the same bytes to the same id.
- **Bounded rewrite:** the mutable file holds a tail ≤ 48 KiB (an exit flush fits a `keepalive` request); older
  batches move to immutable archive files listed, with digests, in the sealed header.
- **No discovery listing:** the record points at the file; one metadata GET per poll covers every device.

### 8.2 The sync folder and the id pool

| Item | Value |
|---|---|
| Name | `Inhouse Notes sync` in My Drive root (visible in Google Drive's own UI, B-Y3; hidden in the app, ARCHITECTURE §13) |
| Lookup | `files.list q="'me' in owners and mimeType='application/vnd.google-apps.folder' and appProperties has {key='ihn6Kind' and value='sync-folder'} and trashed=false"`, smallest id wins; cached in `kv[account,'relay-folder']` |
| Create | `POST drive/v3/files {name, mimeType: folder, parents:['root'], appProperties:{ihn6Kind:'sync-folder'}}`, then `POST files/{id}/permissions {type:'anyone', role:'reader', allowFileDiscovery:false}`; a refused permission sets `kv.relay-nolink=true` (flag `x`), retried once a day |
| Id pool | `GET drive/v3/files/generateIds?count=20&space=drive&type=files`, persisted in `kv[account,'idpool']`, refilled below 10 whenever online. Used for **documents** (a new document gets its final Drive id at creation, ARCHITECTURE §7.1), relay files, archives and assets. Ids expire after a while unused; a create answered 400 "invalid id" discards the id and takes the next |

### 8.3 Files

| File | Name | appProperties | Mutability |
|---|---|---|---|
| Relay file | `ihn6-<docId>-<inst13>.bin` | `{ihn6Kind:'relay', ihn6Doc:docId, ihn6Inst:inst13}` | rewritten by its single writer within an epoch |
| Archive | `ihn6-<docId>-<inst13>-a<k>.bin` | `{ihn6Kind:'archive', ihn6Doc, ihn6Inst}` | immutable |
| Asset | `ihn6-<docId>-<inst13>-<hex16 of hash>.bin` | `{ihn6Kind:'asset', ihn6Doc, ihn6Hash:<hex64>}` | immutable |

All `application/octet-stream`, parent = sync folder. Create = multipart `POST upload/drive/v3/files?
uploadType=multipart` with a reserved id (409 ⇒ created by a lost response: continue with an update); relay update =
`PATCH upload/drive/v3/files/{id}?uploadType=media`; assets > 5 MiB use the resumable upload of ARCHITECTURE §8.3.

```
RelayFile  := "IHN6R" u8 ver=1, u8 comp (0 none | 1 deflate-raw), Payload (compressed iff comp=1)
Payload    := str doc_id, u64 inst, uv gen, uv written_at_ms,
              Profile,
              uv n_mail, Mail × n_mail,                           // §10.2
              u8 has_export, [str head_revision_id, VV vv],       // last export known to this device (§14.4)
              uv n_assets, (Hash, str file_id, uv size) × n,      // asset files this writer still keeps
              uv n_arch, Archive × n_arch,
              uv n_ranges, Range × n,                             // what the tail holds, per actor
              uv n_batches, (bytes Batch) × n
Profile    := str email(≤38), str name(≤22), str photo_url(≤512), u8 device_type ('m'|'t'|'p'), str label(≤16),
              u8 save_phase                                       // ARCHITECTURE §6.6 phase, for remote save rings
Archive    := str file_id, bytes digest(12), uv n_ranges, Range × n
ArchiveFile:= "IHN6A" u8 ver=1, u8 comp, str doc_id, u64 inst, uv n_batches, (bytes Batch) × n
```

`digest` = first 12 bytes of SHA-256 over the exact uploaded bytes (crypto effect 3.8). A relay file carries the
batches of every actor of its installation plus that installation's migration actors (§15.4). Import-actor batches
of Drive revisions are **not** relayed (every device can re-derive them; the first export embeds them).

### 8.4 Record property (the seal)

Stored in the document's `properties` (public, 124-byte key+value limit, ≤ 30 public properties per app):

```
key   = "ihn6_" + inst13                                   (18 bytes)
value = "1|" gen36 "|" n36 "|" fileId "|" d16 "|" flags      (≤ 90 bytes)
  gen36  relay file generation, base 36
  n36    write nonce (sender's ms time, base 36); only makes every write a change; never compared with a clock
  fileId relay file id of the current epoch (≤ 44 chars; a longer id is re-reserved)
  d16    base64url (no padding) of the 12-byte digest of relay file generation `gen`
  flags  subset of: c (in a live session), l (left), x (relay not link-readable)
```

Only an editor can write a record (writing `properties` requires write access — verified by the Wave-1 spike, §23;
fallback §13.1). `PATCH drive/v3/files/{docId}?fields=id` with `{properties:{key: value}}` merges keys.

The **first** record write of an installation on a document also, in the same request: nulls every 5.x sync key
(`p_*`, `ihn_live_key_v1`, `ihn_ink_*`, `ihn_live_*` properties, `ihn_lm1_*` appProperties), and nulls every
`ihn6_*` record whose relay file names a different `doc_id` (records carried over by a Drive copy, §8.6 step 3).

Record GC (any editor, at most once per session): when more than 24 `ihn6_*` records exist, delete records unchanged
for 30 days (receiver-local first sighting, §9.1) whose relay ranges are covered by the latest export VV.

### 8.5 Writer (network-leader tab only)

| Step | Action |
|---|---|
| Due | own batches with `seq > sealed[actor]` exist and the mode's flush delay (§9.2) has passed, or the mailbox changed, or hide/exit, or a record refresh is due — **and at least 2 s have passed since the previous relay write** (hide/exit/keepalive flushes ignore the cap) |
| 1 Epoch | rotate to a new relay file id (pool) if: archive rotation in step 3, first write of the UTC day, or `permissionIds` differ from `docs.relay.perm` |
| 2 Assets | upload asset files referenced by pending batches and missing from the header list (serial) |
| 3 Rotate | if `tail + new > 48 KiB` (deflated): write the current tail as a new archive; a batch > 48 KiB alone goes straight into its own archive |
| 4 Plan | `gen = docs.relay.gen + 1`; **persist `docs.relay.gen = gen` (and the epoch's file id) before uploading**; a gen is never reused, even after a crash |
| 5 Build | header + tail, `written_at`; deflate-raw (effect 3.10); digest |
| 6 Upload | **in parallel**: relay file media PATCH (or create) **and** record PATCH with the new gen/digest |
| 7 Seal | both succeeded ⇒ `sealed[actor] = max seq in the file` per actor; persist `docs.relay{…}`; `relay_durable_through` advances; delete the previous epoch's file 10 s later |
| Fail | retry the **same bytes and gen** with backoff `min(30 s, 500 ms × 2^(f−1)) × U(0.8,1.2)`; 403 `rateLimitExceeded`/`userRateLimitExceeded` uses the same backoff; 404 on the file ⇒ new epoch; 403 `insufficientFilePermissions` on the record ⇒ capability check (§12.3); offline ⇒ wait for `online` |

One write in flight per document; new batches coalesce into the next write. Readers handle a seal that lands before
the file (§8.6 step 2).

Exit/hide flush: the same write with `fetch(keepalive:true)` when the deflated file is ≤ 48 KiB and the record PATCH
fits the remaining keepalive budget (64 KiB per page); otherwise `docs.flags.relay_pending` and the Android headless
page or the next session completes it.

### 8.6 Reader

Every poll (§9) issues **one** request:

```
GET drive/v3/files/{docId}?fields=properties,headRevisionId,version,modifiedTime,name,trashed,capabilities(canEdit),
                                 permissionIds
    Cache-Control: no-cache
```

For each record `ihn6_<inst>` other than our own:

1. Parse; reject malformed values. Record first-sighting time of every new value (presence, §9.1).
2. If `(gen, d16)` differs from `applied[inst]`: `GET files/{fileId}?alt=media` (≤ 4 concurrent reads). Compute
   the digest; if it differs from `d16`, the upload has not landed yet: retry after 300 ms, up to 10 times, then wait
   for the next record change.
3. Decode. Reject the file if its `doc_id` ≠ the polled docId or `inst` ≠ the record's inst (a copied document carries
   the original's records; §8.4 removes them). For each tail `Range`: if `vv[actor] + 1 < first`, the gap is in
   archives: fetch the covering archives (verify digests, check `doc_id`). If an archive is gone (GC'd), the export
   covers it (§8.7): fetch the export (§14.3) and continue.
4. `Replica::receive(batches, PeerRef::Relay(inst))` in seq order per actor.
5. Assets: for every `missing_assets()` hash listed in the header, `GET` the asset file (check `ihn6Doc`,
   `ihn6Hash`, and the SHA-256 of the bytes) and `put_asset`.
6. Mail entries addressed to us → signalling (§10.4). Export descriptor → known exports (§14.4). Profile → peer table.
7. `applied[inst] = (gen, d16)`.

The same poll yields `headRevisionId` (export check, §14.3), `name` (Drive-side rename, §14.7), `trashed` and
`capabilities.canEdit` (revocation, §12.3) and `permissionIds` (epoch rotation, §8.5 step 1).

### 8.7 Garbage collection

| What | Deleted by | When |
|---|---|---|
| Previous epoch's relay file | its writer | 10 s after the new epoch's record is written |
| Own archive | its writer | an export whose VV covers all its ranges is known (§14.4) |
| Own asset file | its writer | a known export **attached** that asset (version-snapshot assets over the export budget and assets of deleted items are kept, ARCHITECTURE §8.2) |
| Own relay file + record + archives + assets of a document | its writer | the user deletes the document forever (Home Bin): the doc id joins `kv['relay-gc']`, processed by housekeeping |
| Stale relay files | housekeeping, once a day on Home idle, and the `relay-gc` queue | files in the sync folder whose `ihn6Doc` is in `relay-gc` or no longer exists / is trashed (`files.get` 404/trashed) |

Archives are never deleted before an export covers them, so a reader always finds every batch in a relay
file/archive or in the export.

### 8.8 Catch-up after days offline

1. Open renders from IndexedDB at once (no network wait).
2. First poll: read every record's relay file; fetch archives for gaps above the VV.
3. Only if a gap remains (archives GC'd) and `headRevisionId` is not a known export covered by our VV: download the
   PDF and `load_snapshot` (or import, §15), then re-read relay tails.
4. Flush own pending batches (durable locally all along); the relay seals them.
5. If alone, the export follows when idle (§14.1).

Cost is proportional to what changed: usually a few ≤ 48 KiB tails; one PDF download only after archive GC.

---

## 9. Discovery, presence, wake-ups and cadence

### 9.1 Presence (receiver-local time only)

Clocks of other devices are never compared with ours. For each record the reader keeps `seen_at[inst]` = local time
at which it first saw the record's current value. A device is **present** if its record has no `l` flag and
`now − seen_at ≤ 120 s`, or if it is a connected P2P peer. Present editors in Company/Linked refresh their record
every 60 s, so the window holds with any clock skew.

Mapping to 5.18.0's avatar markup (`renderPresenceAvatars`, app-v5.js:24790; owner W4.5):

| v6 state of a present remote device | wrapper classes | `title` suffix | device badge | peer badge | save ring |
|---|---|---|---|---|---|
| peer `Open` | `p2p-connected` | ` · Peer-to-peer connected` | `presence-device-badge live` | yes | `Profile.save_phase` / `Presence` frame |
| peer `Offering`/`Answering`/`Connecting`/`Backoff` | `p2p-connecting` | ` · Connecting peer-to-peer` | `live` | no | same |
| peer `Idle`/`Unreachable`, record present (relay only) | — | — | `live` | no | same |
| own device | 5.18.0 own-device rule | — | — | — | own ARCHITECTURE §6.6 phase |
| viewer (no record, P2P never) | not shown (5.18.0 shows viewers only through their own presence keys, which v6 does not write) | | | | |

The popup (`Actividad del documento`) transport line uses 5.18.0's texts: `This device`, `Peer-to-peer` for Open,
`Drive fallback` otherwise. 5.18.0 dedupe (email, then name) and own-device exclusion apply.

### 9.2 Modes of the network-leader tab (visible)

| Mode | Condition | Poll | Relay flush delay | Record refresh |
|---|---|---|---|---|
| **Alone** | no other present editor, no peers | 20 s; 60 s after 5 min without local input | 5 s after the last batch, ≤ 30 s while writing | none (written with flushes) |
| **Company** | ≥ 1 present editor without an open channel, or a handshake in progress | 500 ms | 150 ms | every 60 s |
| **Linked** | every present editor has an open channel | 10 s | 2 s idle, ≤ 10 s while writing | every 60 s |
| **Relay-only pair** | a present editor is P2P-unreachable (§10.4) and no other reason for Company | 1 s | 150 ms | every 60 s |

All writes obey the 2 s cap (§8.5). Entering Company from Alone writes the own record at once (flag `c`) so the
newcomer sees us, then the P2P handshake starts. A newcomer polls immediately on open.

**Why the lone device still polls.** Without a server, Drive offers no push to a browser (`changes.watch` needs an
HTTPS webhook; Web Push needs a sender). A lone device therefore keeps one small GET per 20 s (60 s idle) while
visible and **zero** requests while hidden, with no writes unless the user edits. A newcomer has the whole document
at once from the cache/export and relay files; its own edits reach the lone device on that next poll.

### 9.3 Background and lifecycle

| Event | Action |
|---|---|
| Hidden | stop polling; flush relay now (keepalive rules §8.5); keep P2P |
| Hidden 60 s | close peers (`Bye`), record with `l` (keepalive), release `ihn6-net`; ARCHITECTURE §12 suspends canvases |
| Visible | poll now; re-enter the mode; reconnect peers |
| `online` / connection change | poll now; probe open channels (§10.8) |
| `offline` | no requests; batches accumulate in `ops`; status "paused" |
| Home / close | `Bye`; record with `l`; relay flush; export if alone (§14.1); Home waits for local durability only |

Viewers never write records; their cadence is in §12.1.

---

## 10. P2P

### 10.1 Vouching

A remote installation is **vouched** for a document while (a) its record is present-or-recent: `now − seen_at ≤
10 min` (receiver-local), and (b) the relay file of that record's current generation passed the digest check. Only
editors can write records (§8.4), so a vouched installation is an editor's (or one demoted less than 10 minutes ago,
§12.3). Only vouched installations are offered to, answered, or kept connected.

### 10.2 Signalling mail (inside the relay file)

```
Mail := u64 to_inst, u64 sid, u8 kind (1 offer | 2 answer), uv ttl_ms (≤ 60,000), str sdp
```

The SDP comes from a sealed relay file of a vouched editor, so it is authentic; it contains the DTLS fingerprint
WebRTC enforces, so the channel is bound to that editor's browser. A mail entry is accepted iff its sender is vouched,
`(from_inst, sid, kind)` was not processed before, and less than `ttl_ms` has passed since this receiver **first
saw** it (receiver-local, no clock comparison). Writers remove entries when the channel opens or after `ttl_ms`.
Viewers of the document can read the SDP (ICE candidates, i.e. the editors' IP addresses) for at most 60 s; they
cannot use it to join (no fingerprinted certificate, no vouching). B-Y4 records this.

### 10.3 Channels

`RTCPeerConnection({iceServers: the four Google STUN servers of 5.x, bundlePolicy:'max-bundle',
rtcpMuxPolicy:'require'})` with two **negotiated** data channels:

| id | label | options | carries |
|---|---|---|---|
| 0 | `ihn6` | ordered, reliable | Hello, Have, small Batches, ink, presence, control |
| 1 | `ihn6-bulk` | ordered, reliable | `Part`s of large frames (Batches > 60 KiB, Snapshot, assets) |

Messages are binary `ArrayBuffer`s ≤ 64 KiB. Bulk sends wait for `bufferedAmount < 256 KiB`
(`bufferedAmountLowThreshold = 128 KiB`), so ink on channel 0 never queues behind a photo.

### 10.4 Connection state machine (one per remote installation)

```
          discover (present, vouched, editor; we are smaller inst)          offer for us (valid mail)
 Idle ───────────────────────────────────────────► Offering      Idle ─────────────────────────────► Answering
                                                     │ answer applied                                  │ answer in our relay file
                                                     ▼                                                 ▼
                                                 Connecting ◄──────────────────────────────────────────┘
                                                     │ both channels open + Hello verified
                                                     ▼
                                                   Open ──(no traffic 8 s | channel/ICE failed | not vouched |
                                                     │      own canEdit lost | Bye)──► Closed
 Offering: 15 s without answer ─┐   Connecting: 12 s ─┐                                               │
                                ▼                     ▼                                               ▼
                             Backoff (1, 2, 4 … 60 s × U(0.8,1.2)) ◄──────────────────────────────────┘
 3 consecutive failures with a peer ⇒ Unreachable for 5 min (that pair uses the relay; mode "Relay-only pair")
```

- **Initiator:** the smaller `inst`. The larger side offers only if no offer arrived 8 s after it first saw the
  other's present record. Glare: the smaller side ignores the larger side's offer while its own is pending; the
  larger side drops its own offer when it receives the smaller side's.
- **Offer:** create the connection and channels, `createOffer`, `setLocalDescription`, wait for ICE gathering
  complete, or 70 ms after the first server-reflexive candidate, at most 650 ms (5.x values); put the SDP in the
  mail; relay write immediately (mailbox trigger, still subject to the 2 s cap).
- **Answer:** `setRemoteDescription`, `createAnswer`, same gathering rule, mail, write.
- **Open:** each side sends `Hello`; the receiver checks `doc_id` and that `inst` is the expected peer.
- **Fast reconnect:** on `Closed` from a transient cause (no traffic, network change, ICE failed) the first retry is
  immediate.

Typical setup once both are present: 2 × (relay write ~0.4 s + poll wait ~0.25 s + read ~0.3 s) + ICE gathering
≤ 2 × 0.65 s + DTLS ~0.3 s ≈ 3–5 s (5.x: 1–5 s through comments).

### 10.5 Wire frames (channel messages and BroadcastChannel messages)

```
Frame := u8 type, payload
0x01 Hello       u16 proto=1, str doc_id, u64 inst, u8 role (1 editor), VV vv, Profile, Presence
0x02 Have        VV vv                                       // ack / anti-entropy
0x03 Batches     uv n, (bytes Batch) × n
0x04 Part        u32 msg_id, u16 index, u16 count, bytes     // bulk channel; count ≤ 8192, reassembled ≤ 64 MiB
0x05 AssetWant   Hash
0x06 AssetPart   Hash, uv offset, uv total, bytes            // bulk
0x07 SnapshotWant
0x08 Snapshot    bytes EncodedSnapshot                       // sent as Parts
0x10 InkBegin    u64 live, Id page, u8 tool, Color, f64 width, u8 has_p
0x11 InkPts      u64 live, uv from, uv n, n × (f64 x, f64 y), [n × f32 p]
0x12 InkEnd      u64 live, Id item                           // the batch putting `item` follows
0x13 InkCancel   u64 live
0x14 Erase       u64 gesture, Id page, uv n, Id × n (hidden), uv m, (u64 tmp, Body) × m (fragments)
0x15 EraseEnd    u64 gesture
0x18 Presence    u8 mode (editing|viewing|reading), Id page, Color tool_colour, u8 save_phase
0x19 ExportDone  str head_revision_id, VV vv
0x20 Ping        u32 t
0x21 Pong        u32 t
0x22 Bye         u8 reason
```

Reassembly limits: ≤ 4 assemblies per peer, ≤ 64 MiB reserved per peer, 60 s TTL, duplicates with different bytes
drop the assembly. A frame that fails to decode closes the channel.

### 10.6 Live ink

- The editor calls `SyncEngine::live(LivePreview)` once per animation frame during a stroke with the samples added
  since the previous frame: `InkBegin` on the first call, one `InkPts` per frame. ≤ 24,000 points per stroke (5.x).
- Pen up: `InkEnd{live, item}` then the `Batches` frame with the stroke's `ItemPut`. Cancel: `InkCancel`.
- Erasers: `Erase` (complete current hidden set and fragments) at most every 50 ms; `EraseEnd` before the batch.
- Receivers draw previews in the remote-ink overlay with 5.18.0's `applyRemoteLiveStrokePreview` rendering and drop
  a preview when its item's batch is applied, on cancel, or after 15 s. Previews go to direct peers and tabs only.
- Viewers never receive live ink (they are never peers, as in 5.x).

### 10.7 Anti-entropy, acknowledgement, retransmission, forwarding

- After `Hello`, each side calls `Replica::batches_since(peer.vv, 1 MiB)` repeatedly and sends `Batches` until the
  peer has everything. Only on `Err(Compacted)` (§7.3) it sends `Snapshot`, then the batches above its VV.
- **Ack:** `Have{vv}` 200 ms after applying a `Batches` frame (debounced); `peer.vv` = max of everything heard. Known
  peer/tab VVs feed journal retention (§7.3).
- **Retransmission:** 5 s after a send, if `peer.vv` does not cover it, resend from `batches_since(peer.vv)`. On
  reconnect, the `Hello` VV exchange resends everything missing.
- **Forwarding:** a batch received from one source goes to every other open peer and the tab channel whose known VV
  lacks it. Live ink is not forwarded (except to tabs).
- **Assets:** a receiver missing a `Hash` sends `AssetWant` to one peer (round-robin, 10 s timeout) and falls back to
  the relay asset file or the export. Pull-only.
- **Export notice:** after a successful export the writer sends `ExportDone` to peers and tabs (§14.4).

### 10.8 Health

| Check | Value |
|---|---|
| Ping when nothing was sent for | 2 s |
| Dead after nothing received for | 8 s ⇒ `Closed`, immediate retry |
| Network change or resume with an open channel | `Ping`; no `Pong` within `clamp(3 × RTT + 200, 300, 2000)` ms ⇒ `Closed`, immediate retry |
| Peer authorization recheck | every poll: the peer must stay vouched (§10.1) |

---

## 11. Tabs of the same browser

- `BroadcastChannel('ihn6:' + fnv1a64hex(account) + ':' + docId)`, owned by `host/peer.js`; messages are §10.5 frames.
- Each tab is its own replica and actor (§7.1). On open a tab sends `Hello`; others answer with `Hello` and
  anti-entropy (no `Have` timers: delivery is reliable).
- Exactly one tab per document holds Web Lock `ihn6-net:<account>:<docId>` (released on hide-60 s or close). It
  polls, writes the relay (including other tabs' batches, read from `ops`), holds the P2P peers and forwards between
  peers and tabs. Others do no network work; when the holder goes away another tab acquires the lock and resumes from
  `ops`/`docs.relay`. Web Locks are executed by `host/idb.js` (ARCHITECTURE §3.3, family 3.9).
- The Android headless page takes the same lock, so it never writes a relay file while a visible tab owns it.

---

## 12. Viewers, Read mode and public links

### 12.1 Signed-in viewers (`canEdit == false`)

- Read the same poll and relay files; `receive` remote batches; never write records, relay files, exports or P2P.
- Cadence (visible): 1 s while any editor record changed within 120 s (receiver-local); 5 s within 30 min; 20 s
  otherwise; nothing while hidden. Latency while editors write ≈ relay target (< 4 s P95).
- Initial state: local cache, else the export PDF (snapshot), then relay tails.

### 12.2 Read mode (editor toggled the UI to read)

Stays an editor replica: keeps P2P, keeps relaying its unsynced batches, exports when scribe.

### 12.3 Revocation

- Own capability comes with every poll. `canEdit` false or `insufficientFilePermissions` on a record write ⇒ switch
  to viewer: close peers, stop relay writes, keep own unsealed batches in `ops` (uploaded if rights return; 5.18.0's
  view-only UI). `trashed` ⇒ 5.18.0's removed-document handling.
- A demoted editor cannot write records: peers stop vouching for it 10 min after its record last changed (others
  refresh every 60 s while in company), and readers never apply relay content beyond the last sealed generation.
- A removed person (no access at all) loses the relay files at the next epoch rotation (§8.1).

### 12.4 Public links (signed out)

Production has no Drive API key (5.18.0 `PUBLIC_DRIVE_API_KEY` is empty: no meta tag, no global), and Drive metadata
and properties cannot be read anonymously without one. Signed-out readers therefore keep **5.18.0's mechanism**
exactly (`spec/home.md` §8.3–8.4): download through `drive.usercontent.google.com` then `drive.google.com/uc`
(`credentials:'omit'`, `%PDF-` check), open read-only from the export snapshot, poll every 2.5 s while visible by
re-downloading the PDF and applying it when its fingerprint changed (`load_snapshot`), iframe preview as the fallback.
They see edits after the next export (scribe idle 10 s, at most every 60 s). They never read relay files, so nothing
in v6 depends on an API key. (If the user later provisions a referrer-restricted key — open question in BUILD-PLAN —
the same code adds `key=` to the §8.6 poll and relay reads; no other change.)

---

## 13. Security model

| Threat | Defence |
|---|---|
| A viewer or commenter injects ops | it cannot write `properties`, so it has no sealed relay file; readers apply only sealed generations |
| A viewer joins P2P | not vouched: its offers are ignored; a vouched editor never offers to it |
| A viewer reads signalling | it can read the SDP in mail entries for ≤ 60 s (editors' ICE candidates/IP addresses), never a key; using it does not give a channel (DTLS fingerprint) — B-Y4 |
| Man in the middle on the data channel | the SDP with the DTLS fingerprint arrives in an editor-sealed relay file; WebRTC enforces the fingerprint |
| Someone replays an old offer | receiver-local TTL from first sighting, `(from, sid, kind)` dedupe |
| A demoted editor keeps writing | relay content beyond the last seal is ignored at once; P2P drops it within 10 min; own client self-demotes |
| **A removed collaborator or former viewer keeps reading via saved file ids** | relay file epoch rotation on any `permissionIds` change and daily (§8.1); residual: archives/assets already known until export GC, ≤ 1 write per writer before the change is observed — B-Y2 |
| A copied document imports the original's later edits | readers reject relay/archive/asset files whose `doc_id`/`ihn6Doc` differs; the copy's first editor write nulls foreign records (§8.4, §8.6) |
| Tampered relay file or archive | only the owning account can write its files; 96-bit digest in the sealed record / sealed header; mismatch ⇒ ignored |
| Malformed bytes from any source | codecs never panic; limits on sizes, counts and reassembly; frame rejected whole |
| Relay files found by strangers | not discoverable (`allowFileDiscovery:false`); ids published only to the document's current readers |
| Profile exposure | email/name/photo/device in the relay header are readable by the document's readers — the same audience as 5.x presence properties |
| PDFs and assets from peers or relay | the same security-core limits as a downloaded PDF (64 MiB, 2,000 pages, 40 MP; ARCHITECTURE §9.1) |

### 13.1 Fallback if assumption 1 (§23) fails

If the Wave-1 spike shows that readers or commenters **can** write `properties`, seals no longer prove edit rights.
Then a record is accepted only if the relay file's owner (`files.get(fields=owners(emailAddress))` on the relay file,
cached per epoch) appears with role `writer`/`owner` (or a domain/anyone `writer` permission matches) in the
document's `permissions.list(fields=permissions(emailAddress,role,type,domain))`, fetched by editors at most every
10 min and on every `permissionIds` change. Readers that cannot list permissions (viewers) apply records whose relay
owner equals the document owner or `lastModifyingUser` of the head revision, and otherwise relay content only from
installations that an editor has vouched for in its own relay header (`u8 vouched, uv n, u64 inst × n` appended to
`Payload`). Owner: W2.7 (relay), sim tests "reader with properties write access is rejected" and "viewer applies only
editor-vouched relays". Cost: one `permissions.list` per editor per 10 min.

---

## 14. The PDF export: who writes it and when (single source)

ARCHITECTURE §8 owns the container and the upload request shapes; this section owns triggers, authorship and the
retry loop.

### 14.1 Who and when

- **Scribe** = the smallest `inst` among present editor installations (self, present records, open peers), excluding
  records with `l`. Alone ⇒ you are the scribe. While others are present only the scribe exports automatically.
- The scribe exports when its VV is not covered by the last known export VV **and** this device has had no local input
  for 10 s; at most once per 60 s; during continuous remote activity at most every 5 min; never during pen contact
  (accepted canvas contact aborts the job, as 5.18.0 aborts saves).
- **Leaving** (Home, close, hidden 60 s): a device that is **alone** exports if needed; a device leaving company sends
  `Bye` and leaves the export to the new scribe (its batches are already sealed in its relay file).
- **Forced export**: any editor, ignoring the scribe rule and the 60 s spacing, when (a) the user runs Export to the
  user (ARCHITECTURE §8.5) or (b) the Android headless page finds `export_pending`. Same loop (§14.2).
- Drive "modified by" therefore shows the scribe's account for automatic exports (B-Y6).
- The headless page exports documents with `export_pending` or with own sealed batches not covered by a known export.

### 14.2 Upload loop

```
Build:     export worker (ARCHITECTURE §8.2): snapshot + VV of the current state → bytes
CheckHead: GET metadata (headRevisionId; v2 etag; appProperties.ihn6_vv)
  head is a known export covered by the built VV, or the head we built on ─► Upload
  otherwise ─► Download head, classify (§15.1), merge, ─► Build (counts as a loop)
Upload:    PUT with If-Match: <etag just read>, immediately after CheckHead
  2xx  ─► Done: record export {head, vv}; ExportDone to peers/tabs; header field in the next relay write;
              delete covered archives/attached assets (§8.7); clear export_pending
  412  ─► GET headRevisionId+etag: unchanged head ⇒ metadata churn (another device's record write changed the etag)
              ⇒ PUT the same bytes again at once (≤ 10 fast retries, not counted as loops);
          changed head ⇒ CheckHead (≤ 3 loops, then retry in 30 s)
  error─► backoff 2 s … 5 min; export_pending stays set
```

Record writes are capped (§8.5), so the window between the etag read and the PUT (one round trip) rarely contains a
record write; churn costs one extra GET+PUT, never a rebuild.

### 14.3 Seeing someone else's export (avoids downloads)

A device stores known exports `{headRevisionId → VV}` (ARCHITECTURE §7.1 `exports`) from its own uploads,
`ExportDone` frames and relay headers. When a poll shows a new `headRevisionId`:

- known and `vv ⊆ ours` ⇒ nothing to download;
- known and not covered ⇒ wait: the missing batches arrive through P2P/relay; download only when the relay cannot
  fill a gap (§8.6 step 3);
- unknown ⇒ download and classify (§15.1).

### 14.4 Export descriptor

`(headRevisionId, VV)`; carried in `ExportDone`, in the writer's relay header and in the `exports` table. It makes
archive GC safe and lets every device decide coverage without downloading the PDF.

### 14.5 Exit

Home waits only for local durability (ARCHITECTURE §12). Then, detached: relay flush, `Bye`, record `l`, export if
alone. On `pagehide` only the keepalive relay flush is attempted; a due export stays `export_pending` and is done by
the Android headless page or the next session of any editor. Nothing is lost if it never runs: the batches are
sealed in the relay.

### 14.6 Timeline versions and the export

Each editor captures its own versions (ARCHITECTURE §4.6). A `VersionAdd` references a version-snapshot asset, which
travels like any asset (P2P pull, relay asset file). Budget: versions are coalesced per author over 5 min, so ≤ 12
asset uploads per hour of writing per editor (counted in §1 and in the sim's request-budget test). The export attaches
the newest version assets within 5.18.0's 3.5 MB budget; older ones stay as relay assets (not GC'd, §8.7).

### 14.7 Document name

- An in-editor rename emits a `DocName` op and PATCHes the Drive `name` at once (5.18.0 behaviour).
- The Drive `name` from every poll/open is an input: `docs.drive_name` holds the last name this replica pushed or
  observed. When the polled name differs from it (a rename from a Home card, from another app, or from Google
  Drive), the replica records it and, if it differs from the `DocName` register, emits a `DocName` op stamped now.
  Several devices doing this concurrently write the same value (harmless).
- An export pushes `name` only when the local `DocName` register is newer than the stamp at which `docs.drive_name`
  was last observed; otherwise it leaves `name` untouched. A rename made outside the editor is never reverted.

---

## 15. Documents written by 5.x

### 15.1 Classifying a downloaded revision

| Keywords | Meaning | Action |
|---|---|---|
| contains `;IHN6:` | v6 export | `load_snapshot` (merge) |
| starts with `STROKES_` | 5.x document (first adoption, or a 5.x device saved later) | `v5::import` → import batches (§15.2) → `receive` |
| neither | plain PDF: new document, or a foreign app replaced the content | new document: import as PDF pages (ARCHITECTURE §9.1). Existing v6 document: keep the v6 state; before the next export, save the foreign revision as a sibling copy `"<name> (recovered Drive version).pdf"` (5.18.0 recovery rule, `spec/persistence.md` §4 #19), then overwrite |

### 15.2 Deterministic import batches

- Import actor = `IMPORT_BIT | (fnv1a64(headRevisionId) >> 1)`. Every device that downloads the same revision
  derives the same actor and the same batches (`v5::import::from_pdf(keywords, facts, import_actor, rev_time_ms)` is
  pure, ARCHITECTURE §9.3): one batch per page in page order then one for document fields, seq 1, 2, …
- Stamps are explicit per op: 5.x stamps map to `{clock, fnv64(actor string), 0}`; **stampless** registers (items
  without `syncStamp`, `deletedStrokeIds` without stamps, page properties without `IH_FIELDS`/`IH_STRUCT` stamps) get
  `{rev_time_ms, import actor, idx}` where `rev_time_ms` = the revision's Drive `modifiedTime` from the same
  `files.get` that returned `headRevisionId`. A newer revision therefore wins over an older one regardless of import
  order.
- Same `(actor, seq)` everywhere ⇒ dedupe works across devices and the VV records the import.
- Import batches of Drive revisions are not relayed (§8.3).

### 15.3 A 5.x device saves after v6 last saw it

The head changes to an unknown revision ⇒ download ⇒ `STROKES_` ⇒ new import actor ⇒ merge by ids and stamps. The
scribe then exports (its `If-Match` protects the 5.x revision until merged). The `STROKES_Z:!` fence keeps a still-open
5.x tab from opening a v6 export (ARCHITECTURE §8.2); its own release watcher reloads it.

### 15.4 Unsynced 5.x local data (migration)

ARCHITECTURE §10 builds the migrated state. Migrated content that exists only on this device is emitted as batches of
the migration actor `IMPORT_BIT | (fnv1a64("migrate:" ‖ inst13 ‖ ":" ‖ docId) >> 1)` with explicit stamps (5.x
stamps; stampless → the record's `savedAt`), appended to `ops`, and **relayed** like own batches. Migration counts a
document as done when `relay_durable_through` covers its migration actor's batches (ARCHITECTURE §10.2). 5.x journal
and preview files in the user's `inhouse notes` folder are neither read nor deleted.

---

## 16. State machines

### 16.1 Document sync session (`ihn-sync::Sync`, one per open document)

```
Closed ─open─► Loading (replica from IDB/export/import) ─first poll─► Live{Alone|Company|Linked|RelayOnly|Viewer}
Live ─hidden─► Background (no poll, relay flushed, peers kept) ─60 s─► Suspended (peers closed, record l, lock released)
Background/Suspended ─visible─► Live (poll now)
Live ─offline─► Offline (no requests; ops accumulate) ─online─► CatchingUp (§8.8) ─► Live
Live ─Home/close─► Leaving (Bye, record l, relay flush, export if alone) ─► Closed
any ─canEdit false / 403─► Live{Viewer}        any ─trashed / 404─► Closed (5.18.0 UI)
```

A document whose Drive file does not exist yet (created offline, ARCHITECTURE §7.1) stays in `Loading{NotCreated}`:
no polls, no relay; batches accumulate in `ops`; after the create succeeds (W5.4) it moves to `Live`.

### 16.2 Relay writer

`Idle ─due─► Epoch? ─► Assets ─► Rotate? ─► Plan(persist gen) ─► Build ─► Upload(file ‖ record) ─both ok─► Sealed
─► Idle`; any failure ⇒ `Retry(same bytes, same gen, backoff)`; 404 ⇒ new epoch; 403 permissions ⇒ Viewer.

### 16.3 Peer

§10.4.

### 16.4 Export

`Idle ─due/forced (§14.1)─► Build ─► CheckHead ─► (Download ─► Merge ─► Build)? ─► Upload ─2xx─► Done ─► Idle`;
`412 ─► same-head fast retry | CheckHead (≤ 3)`; contact/abort ⇒ `Idle (still due)`; error ⇒ `Backoff`.

---

## 17. Latency budgets

**P2P, first point of a remote stroke**

| Stage | Budget |
|---|---|
| sample → next rAF on the author (where 5.18.0 also draws) | ≤ 16.7 ms |
| encode `InkPts`, `send` on channel 0 | < 1 ms |
| network one way (same LAN / same region) | 2–60 ms |
| receiver event, decode, overlay geometry, next rAF draw | ≤ 18 ms |
| **total** | **typ. 25–95 ms, P95 < 150 ms** |

**Drive relay (no P2P)**

| Stage | Budget |
|---|---|
| flush delay after pen-up | 150 ms |
| wait for the 2 s write cap while writing continuously | 0–2 s (0 for the first stroke after a pause) |
| relay media PATCH ‖ record PATCH (≤ 48 KiB) | 300–700 ms |
| reader poll wait (500 ms period) | 0–500 ms |
| metadata GET | 100–300 ms |
| relay file GET + digest + decode | 150–400 ms |
| receive + draw | ≤ 20 ms |
| **total** | **≈ 0.8–3.9 s, P95 < 4 s** (the e2e gate, TESTING §4) |

---

## 18. Rust layout and interfaces

### 18.1 Crate `ihn-sync` (pure, no I/O, depends on `ihn-core`)

| Module | Contents | Unit |
|---|---|---|
| `lib.rs` | `Sync` (implements `SyncEngine`), `SyncEffect`, `SyncIo`, `PeerRef`; session machine (§16.1) | W2.6 |
| `mode.rs` | presence table (receiver-local), modes and cadence (§9), scribe election (§14.1), timers | W2.6 |
| `wire.rs` | §10.5 frame codec, `Part` fragmentation/reassembly with limits | W2.6 |
| `sim.rs` (`cfg(test)`) | deterministic simulator: fake Drive (properties with 124-byte/30-key limits, files, etags that change on any metadata write, revisions, `permissionIds`, 412, 403 rate limits, latency and failure injection), fake data channels, virtual clocks with per-device skew | W2.6 |
| `relay.rs` | relay/archive/record codecs, writer plan (epochs, assets, rotation, gen persistence, seal), reader plan (digest, doc check, gaps), GC plan, §13.1 fallback | W2.7 |
| `export.rs` | export due/forced/abort decisions, upload loop (§14.2), known-export table, head classification, name rule (§14.7) | W2.7 |
| `peer.rs` | per-peer machine (§10.4), glare, backoff, mail, vouching (§10.1) | W2.8 |
| `entropy.rs` | Hello/Have/Batches/Snapshot exchange, ack bookkeeping, resend, forwarding, asset pulls (§10.7) | W2.8 |
| `live.rs` | live ink/erase sender and receiver preview table with TTL (§10.6) | W2.8 |

Size budget: ≤ 60 KB of the wasm module (raw).

### 18.2 Interface with the core

`Replica`, `SyncEngine`, `SyncStatus` and the persistence hooks are defined **once**, in ARCHITECTURE §4.4. Effects
and inputs of the engine:

```rust
pub enum SyncEffect {
    Http { id: u32, req: HttpReq },                   // executed by ihn-app::drive (auth, deadlines, 401 retry)
    Rtc(RtcReq),                                      // Create{peer, ice}, SetRemote{peer, kind, sdp}, Answer{peer}, Close{peer}
    Send { to: Dest, ch: Channel, bytes: Vec<u8> },   // Dest = Peer(inst) | Tabs
    Digest { id: u32, bytes: Vec<u8> },               // SHA-256 (effect family 3.8)
    Deflate { id: u32, inflate: bool, bytes: Vec<u8> }, // deflate-raw (effect family 3.10)
    Persist(SyncRecord),                              // mapping to stores: ARCHITECTURE §7.1 table "SyncRecord"
    Timer { at: f64 },
}
pub enum SyncIo { Http{id, status, etag, body}, Rtc(RtcEvent), Tab(Vec<u8>), Digest{id, out},
                  Deflate{id, out}, Lifecycle(Lifecycle), Locks(LockEvent) }
```

### 18.3 `host/peer.js` (≈ 200 lines)

Creates/closes `RTCPeerConnection`s with the given ICE servers and the two negotiated channels
(`binaryType='arraybuffer'`), implements the gathering rule (§10.4), reports `LocalDescription`, channel open/close,
messages, `bufferedamountlow` and connection-state events; sends frames; owns the `BroadcastChannel`. Web Locks are
in `host/idb.js`. It makes no decisions.

---

## 19. Tests

### 19.1 Cargo property tests (`ihn-core` + `ihn-sync`, in-repo seeded PRNG, ~2 s in CI)

1. **Convergence:** 2–5 replicas run random `Tx` sequences (every op kind, undo/redo, area-erase splits, cross-page
   moves, page add/move/delete, renames, imports of two revisions R0/R1 of a 5.x fixture on several replicas in both
   orders); random delay, reordering, duplication, loss with redelivery, partitions; random compaction +
   `load_snapshot`. Assert equal canonical state digest, `order()` and `page_items()`; apply-twice = apply-once.
2. **Stamps:** one actor emits several batches within the same millisecond, delivered in reverse order → identical
   state on all replicas; a far-future clock (wall + 10 days) is applied but not observed.
3. **Undo:** undo then redo restores the visible state; undo of an erase revives the body on a replica that compacted
   it.
4. **Codecs:** round trip of random batches, frames, relay files, archives, mail and records; random, truncated and
   bit-flipped inputs never panic and never apply partially; record values ≤ 124 bytes for the longest id.

### 19.2 Cargo simulation (`ihn-sync::sim`, deterministic, virtual time)

| Scenario | Assertion |
|---|---|
| Two editors, P2P allowed | Alone → Company → Linked; first ink point at the peer ≤ 1 link delay + 1 frame; states equal |
| P2P blocked | Relay-only; committed batch visible at the peer < 4 s P95 with the default latency model |
| Clock skew ±10 min between devices | presence, vouching, mail TTL and P2P setup behave as with no skew |
| Device offline 3 days, others edit and export, archives GC'd | catch-up uses export + tails; no batch missing; no lost own batch |
| Viewer | receives everything; emits no writes, no records, no P2P |
| Demoted editor | dropped by peers ≤ 10 min; relay writes after demotion never applied |
| Removed reader | after a `permissionIds` change every writer rotates its epoch at its next write; the old file id stops receiving content |
| Copied document | the copy's replica never applies the original's relay files; foreign records are nulled by the first write |
| Scribe | in steady company exactly one device exports; record writes from 3 editors cause 412 churn that resolves without rebuilds; descriptor prevents downloads |
| Forced export | a non-scribe user Export completes within one loop while the scribe is writing |
| Rename outside the editor | Drive name changed by a Home PATCH while two replicas are open → both converge to it and no export reverts it |
| 5.x save after v6 export | imported with a new import actor, merged, then exported; nothing overwritten |
| Throttling | 403 rate-limit responses on properties PATCH → backoff, no lost batch, latency recovers |
| Request budget | Alone visible idle ≤ 3 GET/min; hidden 0; ≤ 1 relay write / 2 s / device; version assets ≤ 12/h; properties never exceed 30 keys / 124 bytes |
| Crash | kill a replica at random points (before/after IDB commit, after gen persisted but before upload, after upload before seal persisted, during epoch rotation); restart; no loss, no duplicate items, no gen reuse, readers never skip content |
| Degraded storage | IDB unavailable: batches sent from memory, sealed, status error until sealed; reload after seal loses nothing |

### 19.3 Browser end-to-end

The sync cases are sub-tests of TESTING §2 spec `11-two-devices` (owner W4.5): (a) instant sync with P2P; (b)
relay-only + signed-in viewer that cannot edit or connect; (c) offline then reconnect, then Home ⇒ export, then a fresh
context opens the PDF and shows the same state; (d) two tabs of one context: draw in tab 1, see it in tab 2, close the
`ihn6-net` holder, tab 2 takes over the relay; (e) rename from Home while the document is open in the other context.
5.x documents on two contexts are covered by spec 12.

### 19.4 Real accounts

Done before W2.7 starts, by the Wave-1 spike W1.5 (BUILD-PLAN), not as a pre-release check. It verifies §23.

---

## 20. Constants

| Constant | Value |
|---|---|
| Presence window (receiver-local since last record change) | 120 s |
| Vouching window (receiver-local) | 10 min |
| Poll: Alone / Alone idle 5 min / Company / Relay-only pair / Linked | 20 s / 60 s / 500 ms / 1 s / 10 s |
| Viewer poll | 1 s / 5 s / 20 s (editors changed ≤ 120 s / ≤ 30 min / otherwise) |
| Relay flush delay: Alone / Company / Linked | 5 s (max 30 s) / 150 ms / 2 s (max 10 s) |
| Relay write cap (file + record) | 1 per 2 s per device (exit flushes exempt) |
| Record refresh in Company/Linked | 60 s |
| Epoch rotation | archive rotation, first write per UTC day, `permissionIds` change; old file deleted after 10 s |
| Relay tail cap (deflated) | 48 KiB |
| Digest retry after seal-ahead | 300 ms × 10 |
| Relay write backoff | 500 ms × 2^n, max 30 s, ±20 % |
| Concurrent relay reads | 4 |
| Id pool | 20, refill below 10 |
| Mail TTL | ≤ 60 s, receiver-local |
| ICE gathering | complete, or 70 ms after first srflx, max 650 ms |
| Offer timeout / connect timeout / responder offers after | 15 s / 12 s / 8 s |
| Reconnect backoff | 1 s → 60 s ×2, ±20 %; 3 failures ⇒ unreachable 5 min |
| Ping idle / dead | 2 s / 8 s |
| Have debounce / resend check | 200 ms / 5 s |
| Journal retention | until covered by seal + known peer/tab VVs + export, or 24 h; ≤ 8 MiB per document |
| Live ink | per animation frame; ≤ 24,000 points; preview TTL 15 s; erase preview ≤ every 50 ms |
| Frame size / bulk high-water / low threshold | 64 KiB / 256 KiB / 128 KiB |
| Part reassembly | ≤ 4 per peer, ≤ 64 MiB, TTL 60 s |
| Hidden → suspend | 60 s |
| Export: idle / min interval / max interval under remote activity | 10 s / 60 s / 5 min |
| Export loops / same-head fast retries | 3 / 10 |
| HLC observation clamp | wall + 24 h |
| Record GC threshold / age | > 24 records / 30 days |
| Max tab slots | 256 |

---

## 21. 5.x weaknesses and how v6 removes them

| 5.x weakness (`spec/collaboration.md`) | v6 |
|---|---|
| Whole-document snapshots on P2P after every edit; full-page clones, locks, 1.2 s activity deferral (§15.1 rows 3–6) | per-edit batches; no locks; remote ops applied at once (budgeted during contact) |
| Every receiver re-uploads the PDF; 412/merge loops (rows 17, 28) | only authors upload their batches; one scribe exports; etag churn retried without rebuilds |
| Journal same-account only, O(n²) rewrites, 5 s `files.list`, new actor per session, never deleted (§8.10, rows 13–18) | link-readable sync folder, bounded tail + immutable archives, record-pointed reads, stable actors, GC after export |
| Drive pull downloads the whole PDF on each change (row 25) | relay tails; PDF downloads only for unknown exports or gaps |
| Hash-domain mismatch churn (row 12, §3.6) | version vectors only |
| Signalling key public, peers not checked for edit rights (§12, §16.3) | no key exists; only sealed (editor) relay files carry signalling; peers must stay vouched |
| `ihn_live_key_v1` fetched as a preview file every 3 s (§16.1) | key gone; 5.x keys deleted on first v6 record write |
| Journal op for an unknown page blocks a source (§16.7); prefix conflict retries forever (§16.8) | ops on unknown pages are stored and appear later; single-writer files |
| Presence: 2–3 property writes per device per 2–30 s (§6) | one record per installation, ≤ 1 write / 2 s, refreshed every 60 s only in company |
| Peer-id mismatch without email (§16.6) | identity is `inst` |

## 22. Behaviour changes

Recorded in `BEHAVIOUR-CHANGES.md` §3.1 as B-Y1 … B-Y8 (live preview files removed, sync folder and epoch rotation,
visible sync folder, SDP readable by viewers, presence window, scribe authorship, tab-slot limit, relay latency).

## 23. Assumptions — verified by the Wave-1 spike W1.5 on real accounts before W2.7 starts

1. `properties` writes are refused (403) to readers and commenters. If false: §13.1.
2. Files in a folder shared "anyone with the link" are readable by other signed-in accounts with OAuth, without a
   resource key (new files have none); `allowFileDiscovery:false` hides them from search.
3. Drive serves a just-written file's new content to other accounts within ~1 s (the digest retry tolerates lags).
4. The 30 public properties per app per file quota holds as documented (records capped at 24 + GC).
5. The v2 file etag changes on any metadata write (properties) — measured, and the §14.2 fast retry is exercised.
6. Sustained properties PATCHes from 3 accounts at 1 per 2 s each on one file for 10 min produce no (or rare,
   recoverable) `rateLimitExceeded`; the spike records the observed limit. If throttling is frequent, raise the cap
   in §8.5 to 3 s and the relay target in §1 accordingly.
7. `permissionIds` is returned to writers on `files.get` and changes when a person or link permission is removed.
8. `generateIds` ids are accepted for documents created in My Drive and in shared folders, and stay valid for at
   least 7 days unused.
