# Collaboration and multi-device sync: behavioural spec of v5.17.22

Status: descriptive specification of the **current** web app (`APP_VERSION = '5.17.22'`, `index.html:93`), written so that v6.0.0 (Rust/WASM) can be rebuilt with instant network sync while **staying interoperable with v5 devices** during the transition.

Scope: presence, revision polling, the three-way merge, the causal CRDT metadata (structure, fields, item stamps, tombstones), the PDF-embedded sync formats, the Drive ink journal, the Drive live-preview files, WebRTC signalling and data-channel protocol, live stroke and erase packets, BroadcastChannel tab sync and leader election, permissions and read-only viewers.

Out of scope: PDF rendering and export internals (only the metadata they carry is covered), the offline outbox (`offline-sync-v5.js`) beyond the leases it shares with the journal, and the Timeline UI.

Conventions:

- `file:line` cites the v5.17.22 checkout. `app` = `app-v5.js`, `live` = `live-collaboration-v5.js`, `core` = `collaboration-core-v5.js`, `journal` = `drive-ink-journal-v5.js`, `inkapp` = `drive-ink-app-v5.js`, `preview` = `drive-live-preview-v5.js`, `transport` = `peer-transport-v5.js`, `runtime` = `runtime-core-v5.js`.
- **MUST (interop)** marks behaviour a v6 device has to reproduce bit-for-bit or byte-for-byte to coexist with v5 devices. Anything else is v5 behaviour that v6 may change.
- All times are milliseconds unless stated otherwise. "Adaptive" values are listed with every state they can take.
- `rust-core/src` contains no collaboration code (no `collab*` file; no `syncStamp` or tombstone logic). All of this is JavaScript today.

---

## 1. Topology and authority

v5 has five channels. Only the PDF in Drive is authoritative and durable across accounts. The other four are accelerators that carry the **same causal data**, so any of them can be lost without losing content.

| # | Channel | Reach | Carries | Latency class | Durable |
|---|---|---|---|---|---|
| 1 | **Drive PDF** (Keywords metadata) | Every principal who can read the file | Full document: pages, strokes, images, tombstones, `IH_STRUCT`, `IH_FIELDS`, `IH_SYNC`, `IH_VERSIONS` | Seconds to tens of seconds | Yes (authoritative) |
| 2 | **WebRTC P2P** (Drive comments as rendezvous) | Editors (`driveCanEdit !== false`) of the same file, any account | Full or delta document snapshots, live stroke and erase packets, hints | ~RTT + 18 ms frame | No (anti-entropy only) |
| 3 | **Drive ink journal** (`ink-journal-v1` JSON files) | In practice the **same Google account** (see §8.10) | Causal edit operations (strokes, page shells, structure, fields) | ~0.3 to 2 s | Yes (until the PDF catches up) |
| 4 | **Drive live preview** (`ink-preview-v1` JSON files) | Same as the journal | In-progress stroke paths (transient) | ~100 to 400 ms per frame | No |
| 5 | **BroadcastChannel** (`inhousenotes-live-v5:<hash>`) | Tabs of the same browser profile | Snapshots, live packets, hints | Sub-millisecond | No |

Authority rules (invariants):

1. A peer-supplied document body is never trusted as a replacement. Live snapshots and journal operations are merged additively by item ID (`live:4318-4326`, `inkapp:116-118`). Only explicit tombstones delete.
2. A P2P or tab hint never acknowledges a local draft. `drive-committed` only triggers an authenticated Drive poll (`live:1625-1637`).
3. A journal receipt is **not** a PDF receipt: after applying remote journal ops the receiver marks its PDF dirty and schedules its own Drive save (`inkapp:120-132`).
4. A received final P2P stroke is durable content. The receiver enqueues it into its own stroke log, marks the page dirty and schedules a Drive save (`app:4075-4081`, `app:24807-24830`).

Consequence: every editor that receives an edit re-uploads the PDF. Section 15 covers why this matters for latency.

---

## 2. Identities and hash primitives

### 2.1 `simpleHash(str)` (MUST, interop) (`app:27687-27693`)

```
h = 0 (int32)
for each UTF-16 code unit c of str: h = ((h << 5) - h + c) | 0
return Math.abs(h).toString(36).slice(0, 8)
```

`Math.abs(-2147483648)` is `2147483648` (positive in JS doubles). The result is lowercase base 36, at most 8 characters.

It is used for the presence keys, peer IDs, the BroadcastChannel name, mailbox keys, the peer cache key, legacy page and item IDs, the background photo register hash, image fingerprints and failed-signal IDs.

### 2.2 `ihnHashString(value)` (MUST) (`core:19-31`)

The function runs two 32-bit lanes over the UTF-16 code units. The output is 16 lowercase hex characters (`hex8(first) + hex8(second)`).

```
first = 0x811c9dc5; second = 0x9e3779b9
for index, code in UTF-16 units:
  first ^= code;           first  = imul(first, 0x01000193)
  second ^= code + index;  second = imul(second, 0x85ebca6b)
```

### 2.3 `ihnStableStringify` (MUST) (`core:33-48`)

The value is first passed through `ihnStableValue` and then through `JSON.stringify`:

- Objects: the keys are sorted with the default JS sort (UTF-16 order), and keys whose value is `undefined` are dropped.
- Arrays: each element is mapped. An `undefined` element becomes `null`.
- Non-finite numbers (`NaN` and `±Infinity`) become `null`.

Numbers are serialized with JS shortest-round-trip formatting. A Rust implementation therefore needs an ECMAScript-compatible number formatter (for example ryu-js semantics) for the hashes to match.

### 2.4 Device, tab, peer and actor identities

| Name | Format | Lifetime | Source |
|---|---|---|---|
| Presence client ID | `c` + 8 base-36 random characters | Per browser profile (`localStorage` and `sessionStorage` key `presence-client-id-v1`) | `app:27710-27735` |
| Live tab ID | `t` + `Date.now().toString(36)` + first 14 hex of `randomUUID` without dashes | Per JS realm (page load) | `live:216-226` |
| Account hash | `simpleHash(email.trim().toLowerCase())` | Per account | `app:27745`, `live:229` |
| Peer ID | `peer_` + `simpleHash(accountHash + ':' + clientId.trim().toLowerCase())` | Per account and device | `live:228-231`; derived from presence `a`/`c` at `live:233-237` |
| Live actor ID | `${peerId}:${tabId}` | Per tab session | `live:1764`, `live:3629` |
| Collab stamp actor | same as the live actor ID; fallback `device_${simpleHash(email + ':' + clientId)}` | Per tab session | `app:1323-1330` |
| Journal actor | `crypto.randomUUID()` (the host passes no `actor`) | Per journal engine, that is per document session | `journal:49`, `inkapp:76` |

Quirk: `ihnGetLivePeerId` hashes `profile.email || 'unknown'`, while the presence identity hashes `email || ''` (`live:229`, `app:27745`). For a profile without an email, the peer ID computed locally differs from the one other devices derive from presence.

### 2.5 Generated IDs (`app:736-785`)

| Kind | Format |
|---|---|
| Stroke and image ID | `Date.now().toString(36) + '-' + random base36(9)` (`app:736-738`) |
| Page ID | `pg_` + `Date.now().toString(36)` + `_` + random base36(9) (`app:743-745`) |
| Legacy page ID (document without IDs) | `pg_legacy_${simpleHash(docId + ':' + pageNumber1Based)}` (`app:747-751`) |
| Legacy item ID | `st_legacy_` or `im_legacy_` + `simpleHash(identity)` + `simpleHash(reverse(identity))`, where `identity = docId:pageId:type:index:fingerprint-without-id` (`app:753-774`) |

---

## 3. Causal data model

### 3.1 Stamps and clocks (`core:50-69`, `app:835-869`)

A stamp is `{clock: non-negative integer, actor: string}`. Normalization: `clock = max(0, floor(Number(clock) || 0))` and `actor = String(actor || '')`.

**Total order** (`ihnCompareCollabStamps`): compare `clock` numerically, then `actor` by JS string `<` (UTF-16). Equal clocks with equal actors compare as equal.

**Allocation** is a hybrid logical clock: `clock = max(meta.clock + 1, Date.now())`, and the allocator's `meta.clock` is then set to that value.

There are two independent allocators:

- The **structure clock** `state.collabStructure.clock` is used for page structure and for every **item and deletion stamp** (`createCollabItemStamp`, `app:835-843`).
- The **field clock** `state.collabFields.clock` is used for field registers (`core:499-516`).

**Observation:** `observeCollaborativeCausalStamps` raises the structure clock to the maximum clock of every item stamp and deletion stamp it sees (`app:854-869`). It runs in `ensureStrokeIds` and in every page merge (`app:821-827`, `app:30150-30153`).

**Item stamp lookup** (`core:376-384`, also `app:29262-29269`) checks, in order:

1. `item.syncStamp`
2. `item._syncStamp`
3. `{clock: item.updatedClock, actor: item.updatedBy}`
4. `{0, ''}`

**Writers of item stamps:**

- A finished stroke (`app:4692`).
- An area erase (the new fragments) (`app:4631-4633`).
- Moved items (`app:12786`).
- Image add and edit, transforms, and undo or redo re-creation (`app:5291`, `app:31027-31036`, and others).

Each call stamps all of its items with **one** new stamp (`stampCollaborativeItems`, `app:845-852`).

### 3.2 Page shape on the wire (`sanitizePageForStorage`, `app:22413-22440`)

```
{ pageId, strokes:[Stroke], images:[Image], deletedStrokeIds:[id],
  deletedStrokeStamps:{id:Stamp}, backgroundImage:string|null (null when source 'pdf' or blob:),
  backgroundSource:'template'|'pdf'|'custom', templateKind, pdfPageIndex:int|null,
  legacyCoverStrokes:[...]|null, pageWidth, pageHeight, sidePanel:{title,dateKeys,html}|null }
Stroke = { id, tool, color, width, points:[{x,y,p}], syncStamp? }
Image  = { id, src(data URL), x, y, width, height, rotation, syncStamp? , ... }
```

### 3.3 Deletion stamps (tombstones per page) (`core:400-457`, `app:5265-5300`)

- `deletedStrokeStamps: {itemId: Stamp}` applies to **strokes and images alike**. `deletedStrokeIds` is kept as the sorted key list for legacy readers.
- Normalization accepts several input shapes:
  - a `Map`;
  - an object of `{id: stamp}` or `{id: {stamp}}`;
  - an array of `{id, stamp}`;
  - a legacy array of bare IDs, which get stamp `{0, ''}` so that any later causal re-creation wins.

  Each ID keeps the max stamp across shapes, and the output keys are sorted.
- Merge is a per-ID max. It is commutative, associative and idempotent (`core:440-449`).
- **A deletion wins** over an item if and only if `compare(deletionStamp, itemStamp) >= 0` (`core:451-457`). So undo, or re-creation with a newer stamp, beats an older delete.
- `trackDeletedStrokeIds(page, items)` allocates **one** new stamp for the batch and merges it in. `untrackDeletedStrokeIds` re-stamps the restored items (newer than the tombstone) and removes their tombstones (`app:5265-5300`).

### 3.4 Page structure CRDT `collabStructure` (`core:14-17`, `core:71-374`)

```
StructureMeta = { v:1, clock:int,
  entries:    { pageId: { rank:number, orderStamp:Stamp, createdStamp:Stamp } },
  tombstones: { pageId: Stamp } }
```

**Constants:** `IHN_STRUCTURE_SCHEMA_VERSION = 1`, `RANK_STEP = 1024`, `MIN_GAP = 1e-7`.

**Normalize** (`core:92-129`):

- Entries are visited in sorted key order. A missing or invalid rank falls back to `sortedIndex * 1024`.
- Every page ID in the supplied live list that lacks an entry gets `{rank: listIndex*1024, orderStamp:{0,''}, createdStamp:{0,''}}`.
- `clock` becomes the max over `meta.clock`, every `orderStamp.clock`, every `createdStamp.clock` and every tombstone clock.

**Operations** (each allocates one stamp `s`):

| Operation | Effect | Caller |
|---|---|---|
| `recordPageAdded(ids, id, index)` | Rank from neighbours (below). Entry `{rank, orderStamp:s, createdStamp:s}`; deletes `tombstones[id]` | `noteCollabPageAdded` (`app:1610-1621`), which also stamps all four page field registers |
| `recordPageMoved(ids, id, index)` | New rank; `orderStamp = s`; `createdStamp` kept | `app:1623-1632` |
| `recordPageDeleted(remaining, id)` | `tombstones[id] = s` (the entry is kept) | `app:1634-1642` |
| `recordStructureReplacement(prev, next)` (Timeline restore) | IDs in `prev` but not in `next` get tombstone `s`. Every ID in `next` gets rank `index*1024` and `orderStamp = s`. `createdStamp` becomes `s` when the page is newly present (not in `prev`, or tombstoned) or has a zero created clock, and is kept otherwise. Tombstones of `next` IDs are deleted. | `app:1644-1652` |

**Rank for index** (`core:146-165`):

- Between two neighbours: `prev + (next - prev) / 2` if the gap is greater than `1e-7`, otherwise NaN, which triggers a **rebalance**.
- After the last page: `prev + 1024`. Before the first page: `next - 1024`. Alone: `0`.
- A **rebalance** sets every page in the list to `rank = index*1024` and `orderStamp = s`. This is a write to every page's order register (`core:167-177`).

**Merge** (`ihnMergeStructureMeta`, `core:302-374`):

1. `merged.clock = max(local.clock, remote.clock)`. `allIds` is the union of entry keys, tombstone keys and both live page ID lists, sorted.
2. **Entry choice** for each ID (`core:252-271`):
   - Compare the `orderStamp`s; the greater stamp's `rank` wins.
   - On equal stamps, compare the strings `` `${rank}:${stableStringify(entry)}` `` lexicographically; the greater (or local, on equality) wins.
   - The result is `{rank: winner.rank, orderStamp: max, createdStamp: max}`.
   - An ID with no entry on either side receives a zero-stamp entry with fallback rank `sortedIndex*1024`. **Every** ID in `allIds` therefore gets an entry.
3. Tombstones merge with a per-ID max.
4. A page is **tombstoned** if and only if `tombstone exists && compare(tombstone, createdStamp) >= 0` (`core:273-279`). Re-adding with a newer `createdStamp` revives the page.
5. **Visible order:** non-tombstoned IDs sorted by `rank` ascending, then `orderStamp` ascending, then page ID ascending (JS `<`).
6. **Last-page fallback** (`core:342-359`): if no page is visible but `allIds` is non-empty, the smallest ID that has an entry is projected into `orderedPageIds` as `recoveredFallbackPageId`. Its tombstone is **kept** so the join stays associative. The application revives it later as a new causal write (§4.5 step P6).
7. The result includes `localContributed = mergedHash != remoteHash`, `remoteContributed = mergedHash != localHash`, `tombstonedPageIds` and the hashes.

**Structure hash** (`core:281-300`): `ihnHashString(stable({v:1, entries, tombstones}))` over sorted IDs. The `clock` is excluded.

Properties pinned by tests: the merge is commutative, idempotent and associative, including the all-deleted fallback (`tests/collaboration-core-v5.test.mjs:33-245`).

### 3.5 Field registers `collabFields` (`core:459-581`, `app:1463-1608`)

```
FieldMeta = { v:1, clock:int, fields: { key: { value:JSON, stamp:Stamp } } }
```

Keys (MUST):

| Key | Value | Notes |
|---|---|---|
| `doc:exportName` | string | Document title (`app:1463`) |
| `doc:calendarPageConfig` | `{mode, startDateKey, startPage, nextDateKey}` or `null` | Agenda/Diary configuration (`app:1464`) |
| `page:<pageId>:background` | `{backgroundSource, backgroundImageHash}` | `backgroundImageHash = simpleHash(dataURL)` for a custom portable photo, else `''` (`app:1473-1484`) |
| `page:<pageId>:template` | normalized `templateKind` string | `app:1486-1488` |
| `page:<pageId>:size` | `{pageWidth, pageHeight}` | `app:1490-1496` |
| `page:<pageId>:sidePanel` | `{title, dateKeys:[unique strings] or null}` or `null` | The HTML is **not** part of the register; it is a device-local render cache (`app:1498-1506`) |

- **Normalize:** records without an own `value` property are dropped, values are deep-cloned through stable JSON, and `clock` becomes the max of all field stamp clocks.
- **Seed:** any missing register is seeded at stamp `{0, ''}` with the current value (`app:1516-1538`, `core:488-497`). Two legacy snapshots therefore converge by stable value, while any real edit (clock > 0) wins.
- **Record:** `stamp = {max(meta.clock+1, now), actor}`.
- **Merge:** per key, the higher stamp wins. On equal stamps the greater `stableStringify(value)` wins (local on equality). The merged stamp is `max`.
- **Hash:** `ihnHashString(stable({v:1, fields}))`. The allocator `clock` is excluded (`core:573-581`).

### 3.6 Canonical document hash (MUST if ACK and dedupe semantics must match) (`core:583-649`)

```
contentHash = ihnHashString(stable({
  pages: pages.map(canonicalPage),
  structure: canonicalStructurePayload(structure)   // {v, entries, tombstones}, ALL ids incl. tombstone-only
  fields: normalizeFieldMeta(fieldMeta).fields,      // no clock
  calendarPageConfig: config || null,
  exportName: String(exportName || '') }))
canonicalPage = { pageId, strokes (sorted by id, minus preview/needsRedraw/unloaded/pendingUnload),
  images (same), deletedStrokeIds (unique, sorted), deletedStrokeStamps (normalized),
  backgroundSource (default 'template'), templateKind, pageWidth, pageHeight,
  sidePanel: {title, dateKeys} | null }
```

`backgroundImage`, `pdfPageIndex`, `legacyCoverStrokes` and the side-panel HTML are **excluded** (`tests/collaboration-core-v5.test.mjs:247`). Images keep their full `src`.

Hash domains are **not** the same:

- **P2P** snapshots hash raw in-memory points (`live:3650-3656`).
- **The PDF `IH_SYNC`** hash is computed by the worker over the **PDF metadata pages**, whose points are rounded to 0.01 and have `p` omitted when absent (`app:34152-34158`, `app:34356-34409`, `document-processing-worker-v5.js:179-245`).

Two converged replicas whose stroke data came through different paths (raw IndexedDB versus PDF-rounded) therefore report different hashes. The item merge compares fingerprints that round to 0.01 (`app:29232-29252`), so it sees no change. v6 should canonicalize (quantize) coordinates before hashing.

### 3.7 Item fingerprints (`app:29232-29299`)

| Fingerprint | Definition |
|---|---|
| `pointFingerprint` | `round2(x),round2(y),round2(p)` |
| `strokeSyncFingerprint` | `id \| tool \| color \| round2(width) \| points.length \| simpleHash(points.map(pointFingerprint).join(';'))` |
| `imageSyncFingerprint` | `id \| simpleHash(src) \| round2(x) \| round2(y) \| round2(w) \| round2(h) \| round4(rotation)` |
| `*CollabFingerprint` | `sync fingerprint \| clock \| actor` |

The location fingerprint for cross-page duplicates is `kind | itemFingerprint | pageId`.

**Concurrent same-ID choice** (`ihnChooseConcurrentItem`, `core:386-398`): the higher stamp wins. On equal stamps the greater fingerprint string wins (local on equality).

---

## 4. The merge (`applyRemotePages`) (`app:29466-30792`)

All transports converge through one function:

```
applyRemotePages(remotePages, {
  sessionToken, fileId,
  preserveLocalUnsynced,   // keep local-only content and baseline-inferred deletions
  additiveById,            // absence never deletes (true for editors on every live and Drive path)
  captureContentHash,      // P2P: load all pages, return exact merged hash for the ACK
  remoteStructure, remoteFields, remoteExportName?, remoteCalendarPageConfig? })
-> { changed, hasLocalMerges, pagesNeedingPdfBackground[], contentHash, invalidatedPageIds | null }
```

`preserveLocalContent = preserveLocalUnsynced || additiveById` (`app:29516`).

### 4.1 Caller option matrix

| Caller | preserveLocalUnsynced | additiveById | remoteStructure/Fields | captureContentHash |
|---|---|---|---|---|
| Drive pull (`app:28992-29001`) | pull option, or `driveContentVersion` changed during the download, or local changes pending at the apply boundary (`app:28953-28956`) | `driveCanEdit !== false` | decoded `IH_STRUCT` / `IH_FIELDS` (may be null) | no |
| P2P or tab snapshot (`live:4315-4326`) | `canEdit && hasUnsyncedLocalDriveChanges()` | `canEdit` | `envelope.structure` / `envelope.fields` | **yes** |
| Journal op (`inkapp:116-118`) | `true` | `true` | `op.structure` / `op.fields` | no |

### 4.2 Locking and preconditions

1. Acquire the **structure lock** `acquireRemotePageMerge` (`app:20308-20338`). It waits in 24 ms steps, up to **15 s**, until there is no local structure mutation, no other merge, no smooth interaction (pen, gesture, pan, active pointers, page drag, inertia, selection transform: `app:24926-24936`), no page-store writes and no stroke-op flush.
2. `assertRemoteMergeContext` runs after every await. It aborts if the session token, file ID or lock changed (`app:29475-29482`).
3. With `captureContentHash`, **every page** is loaded from IndexedDB (`app:29517-29522`). With `preserveLocalContent`, locally pending pages are hydrated (`app:29523-29529`).
4. Any pending page-store rewrite is finished first (`app:29530-29534`).
5. If neither side has causal structure activity and the page counts match, legacy page IDs are aligned to `pg_legacy_<hash>` canonical IDs on both sides. The field keys are remapped and both structures reset (`app:1375-1461`, `app:29537-29541`).

### 4.3 Steps

**F. Document fields (before any page work)** (`app:29547-29597`)

1. `remoteFieldMetadata` is the seeded remote fields. Seeds come from the remote export name and calendar fallbacks and from the remote pages' current values.
2. `merged = mergeFieldMeta(localSnapshot, remote)`, then `state.collabFields = merged`.
3. `hasLocalMerges |= mergedHash != remoteHash` and `changed |= mergedHash != localHash`.
4. Apply the resolved `exportName` and `calendarPageConfig`. Changing either invalidates all page caches.

**P. Page-ID aware structure pre-pass** (only if every remote page has a `pageId`) (`app:29599-29966`)

1. Local pages without an ID adopt the remote ID at the same index. Any remaining unassigned pages get legacy IDs (`app:29622-29631`).
2. If a remote structure is present: `resolution = ihnMergeStructureMeta(local, remote, localIds, remoteIds)` and `desired = resolution.orderedPageIds`. Without one: `desired = remote ID order`.
3. If the order or count changes:
   - Set `remotePageStructureApplyInProgress`, which blocks new local stroke queueing.
   - Wait up to **30 s** for collaborative idle (`app:20347-20354`).
   - Load all pages and flush the stroke-op queue and dirty pages under the lock. Failure aborts (`app:29650-29687`).
4. **Fallback materialization:** if the projected fallback page exists in neither the local nor the remote body, choose `ihnChooseMaterializableFallbackPageId` (smallest ID present in either list). If none exists, create a blank page with the fallback ID (`app:29696-29725`, `core:658-670`).
5. `state.collabStructure = resolution.meta`. Set `hasLocalMerges |= localContributed` and `changed |= remoteContributed`.
6. Build the new order:
   - For each desired ID: the local page if known, else a **new shell** built from the remote data.
   - Local pages not placed:
     - **with causal structure:** dropped, because they are tombstoned;
     - **without it:** dropped if previously synced (`lastSyncedPageIds`) and `!additiveById`, dropped if `!preserveLocalContent`, and kept otherwise (local-only, which sets `hasLocalMerges`).
   - Legacy pages with no ID are kept at the end (`app:29741-29811`).
7. If the order changed (`app:29813-29939`):
   - Release the assets of dropped pages.
   - Remap every index-keyed cache: dirty sets, dirty modes, pending stroke ops, access times, active pages.
   - Drop queued stroke ops that pointed at removed pages.
   - **Clear undo/redo history.**
   - Clear the PDF caches.
   - Set `collabPageStoreRewritePending`; the IndexedDB numeric page store is rewritten in one transaction.
8. Revive the fallback page causally with `noteCollabPageAdded` (a new `createdStamp`), and set `hasLocalMerges` (`app:29940-29959`).
9. Re-index `remotePages` positionally to the materialized order. Local-only slots become `null` (`app:29963-29965`).

**L. Legacy positional path** (remote without page IDs) (`app:29969-30074`)

- If the counts differ, wait and checkpoint as in step P3.
- Remote pages beyond the local count are appended, unless local pending deletions exist under `preserveLocalUnsynced`.
- Local pages beyond the remote count are removed, unless local pending additions exist under `preserveLocalUnsynced`.

**C. Per-page content merge** (for `i < min(local, remote)` with a non-null remote page) (`app:30076-30605`)

1. Load the page if it is unloaded, and wait for collaborative idle.
2. **Re-merge fields at this boundary**, so a local field edit made during earlier awaits is not lost (`app:30098-30114`).
3. Make sure every item has an ID.
4. Observe the stamps.
5. **Three-way deletion inference** (`app:30155-30177`):
   - `baseline = lastSyncedStrokeIds.get(i)`, the IDs confirmed on Drive at the last sync.
   - Any baseline ID now missing locally is a local deletion.
   - When `preserveLocalContent` holds and the ID has no stamp yet, it gets **one fresh local stamp**.
6. `structuralIdentityChanged = remote.pageId && local.pageId && they differ` (only possible on legacy positional paths). When set, remote content replaces local wholesale, and only the remote tombstones apply.
7. `mergedDeletionStamps = mergeDeletionStamps(localDeletions (incl. inferred), remoteDeletions)`.
8. **Strokes:**
   - For each remote stroke: if the local stroke with the same ID differs by collab fingerprint and `preserveLocalContent` holds, choose with `ihnChooseConcurrentItem` (a local win counts as a conflict). Keep the chosen stroke unless a deletion wins.
   - Append local-only strokes (absent remotely, deletion not winning) when `preserveLocalContent`.
   - When `preserveLocalContent`, **sort by ID**. This makes the order canonical, so merge results are order-independent.
9. **Images:** the same as strokes, with the image fingerprint (`app:30248-30284`).
10. **Effective tombstones:** keep a deletion stamp only if no surviving item has its ID, or the deletion beats the survivor. `deletedStrokeIds` becomes the sorted keys (`app:30286-30302`).
11. **Page-level `hasLocalMerges`** (`app:30304-30331`), when `preserveLocalContent` and any of these is true:
    - local-only items exist;
    - local tombstones add something to the remote set;
    - a local tombstone suppressed a remote item;
    - a local item won a conflict.
12. **Background register** (`app:30364-30432`):
    - The remote body (photo bytes, `pdfPageIndex`) is applied only if the merged `page:<id>:background` record **equals the remote record**, **and** the local body or record differs, **and** the remote body's register value matches. A photo stored in the PDF counts only with a verified PDF-page proof, which is available only on the Drive pull path (`app:29002-29021`).
    - If the register says the remote wins but no matching bytes exist locally or remotely, the merge **throws**. The checkpoint stays pending and is retried later.
    - Template and size: the resolved register value is applied (`app:30434-30462`).
13. If anything changed: write the page, IndexedDB-save it under the lock (or defer to the pending store rewrite), clear captured stroke ops, then redraw or re-preview (`app:30469-30531`).
14. **Side panel:** re-merge fields again, then apply the resolved `{title, dateKeys}`. The HTML is taken from the remote only when the remote register value matches (`app:30533-30590`).
15. **Baseline** for page `i`: the remote IDs if the page had local merges (still unconfirmed), else the merged IDs (`app:30592-30599`).

**X. Cross-page duplicates** (`app:29301-29430`, `app:30607-30618`)

- When the same item ID exists on more than one page (concurrent moves), the winner is chosen by `ihnChooseConcurrentItem` with the **location fingerprint**.
- Losers are removed. Losing pages receive a new local tombstone stamp.
- This sets `changed` and `hasLocalMerges`.

**K. Agenda reconciliation** (`app:19152`, `app:30627-30637`)

- When the user is not in Read mode, duplicate agenda date slots created by concurrent page additions are reallocated deterministically in the merged page order.
- This sets `hasLocalMerges`.

**D. Durable checkpoint** (`app:30646-30685`)

- If anything changed, flush legacy and dirty pages and write the metadata payload to IndexedDB (or a `localStorage` backup).
- If this fails, the merge **throws**. A torn merge is never published.

**B. Bookkeeping** (`app:30686-30742`)

- If there were no local merges, the confirmed baseline becomes `lastSyncedPageCount` and `lastSyncedPageIds` (current page IDs).
- The selection is remapped by IDs.
- If `changed` and this is not a P2P apply, `scheduleLiveDocumentBroadcast()` relays the change to peers.
- Return the merged `contentHash` (P2P only).
- `invalidatedPageIds` is `null` (all pages) for structural or field-wide changes, or the exact page IDs otherwise.

**Failure path** (`app:30743-30791`):

- Roll back any `page:*:background` register that was installed for a page whose body was not updated.
- Mark `remoteMergeCheckpointPending`, which makes the next merge a recovery merge.
- Finish or schedule the page-store rewrite.
- Release the lock.

### 4.4 Merge invariants

1. **Additivity:** with `additiveById`, absence of an item or page in a snapshot never deletes it. Only tombstones (item stamps, page tombstones) delete (`app:30203-30209`).
2. **Determinism:** the result is independent of arrival order. Items are sorted by ID, registers use LWW with a stable-value tie-break, and the structure uses LWW with a stable tie-break and a deterministic fallback.
3. **Undo safety:** re-creation re-stamps items, so a newer item stamp beats an older tombstone everywhere (`tests/collaboration-core-v5.test.mjs:355`).
4. **No torn state:** page bodies are durable before metadata. If a failure occurs, the previous checkpoint stays authoritative.
5. **Index safety:** history is cleared and queued stroke ops are remapped or dropped on reorder. A stroke finished during a reorder is routed by the `pageId` captured at pointerdown (`app:4653-4666`).
6. **Fields before bodies:** registers are merged first and re-merged per page, so a register never pairs with unrelated photo bytes.

### 4.5 Version history (Timeline) merge

`adoptVersionHistory` takes the union of in-memory, IndexedDB and remote histories through `mergeVersionHistories`, which concatenates, normalizes and prunes (`app:22380-22410`, `app:33454-33459`). It is applied on every Drive pull that carries `IH_VERSIONS` (`app:28918-28927`). It is not part of P2P or journal traffic.

---

## 5. Drive file formats

### 5.1 PDF Keywords (MUST, interop) (`app:34379-34438`, raster path `app:34969-34999`)

The PDF `Info/Keywords` field is a single string:

```
Keywords := ("STROKES_Z:" b64(deflate(utf8(json)))  |  "STROKES_DATA:" b64(utf8(json)))
            ";IH_NORM:1"
            [";IH_OVERLAY_REFS:" (pageIdx "=" (xobjectName|refNum)) ("," ...)*]     // pdf-lib path only
            [";IH_CAL:" b64(utf8(json calendarPageConfig))]                       // only when agenda allowed
            [";IH_STRUCT:" b64(utf8(json StructureMeta))]                          // full snapshot incl. clock
            [";IH_FIELDS:" b64(utf8(json FieldMeta))]
            [";IH_SYNC:" b64(utf8(json {v:1, writer:clientId, contentHash, contentVersion}))]
            [";IH_VERSIONS:" <timeline archive>]
json (STROKES_*) := "[" PageEntry ("," PageEntry)* "]"
PageEntry := { pageId, strokes:[{...stroke, points:[{x:round2, y:round2, p:round2|absent}]}], images,
               deletedStrokeIds, deletedStrokeStamps, hasCustomBackground,
               backgroundSource, templateKind, pdfPageIndex:(i+1 if pdf/custom bg else null),
               pageWidth, pageHeight, sidePanel|null }        // app:34178-34194
```

The writer sets Title `'Cuaderno Digital'`, Subject `'Notas con S-Pen'` and Creator `'Cuaderno Digital App'` (`app:34438-34441`). The reader takes `Title` as `remoteExportName` (`app:28831`). This only seeds `doc:exportName` when the register is missing, because the field register wins.

**Section parsing rules:**

- `STROKES_*` must be the **prefix**. Its payload ends at the first `;` (`app:7414-7418`).
- Each other section is located with `indexOf(';MARKER:')` and ends at the next `;`.
- Base64 never contains `;`.

**Size limits:**

| Limit | Value | Source |
|---|---|---|
| `IH_STRUCT` / `IH_FIELDS` encoded | ≤ 8 MiB characters | `app:1654` |
| `IH_CAL` | ≤ 64 KiB | `app:1655` |
| `IH_SYNC` raw | ≤ 16384 characters | `app:7457` |
| `STROKES_*` encoded | ≤ 48 MiB | `app:7476` |
| `STROKES_*` inflated | ≤ 96 MiB | `app:7477` |
| Metadata worker timeout | 30 s | `app:7475` |

Decoding of `STROKES_*` happens in an inline worker that uses `DecompressionStream('deflate')` (`app:7495-7566`).

**Strictness:** on the Drive pull path, a payload that is advertised (`STROKES_Z:` / `STROKES_DATA:` prefix) but undecodable **aborts** the pull, defers it and throws. It is never treated as "no strokes" (`app:28928-28933`). `IH_STRUCT` and `IH_FIELDS` decoding is strict there (`throwOnInvalid`, `app:28837-28847`).

**`IH_SYNC` uses:**

- `driveConfirmedContentHash` holds the hash of the last confirmed upload or applied revision (`app:1908`, `app:27045`, `app:29166-29168`, `app:32267`).
- A pull whose `IH_SYNC.contentHash` equals `driveConfirmedContentHash`, with no local pending work, is treated as an **echo or duplicate**. It updates the metadata only and skips the merge (`app:28961-28979`).

### 5.2 Drive revision tracking (`app:25693-26061`)

The tracked state is `state.driveHeadRevisionId`, `state.driveRevisionVersion` (the Drive `version` as a normalized decimal string) and `state.driveLastKnownModified` (ms).

**`isRemoteMetaNewerThanTracked(meta)`** (`app:25888-25923`):

- **Own-write echo window:** for **12 s** after our own upload, a `headRevisionId` equal to our own, or a `modifiedTime` at or before ours, is not newer.
- If `headRevisionId` is present:
  - equal to the tracked one: not newer;
  - otherwise, if both sides have a `version`, compare versions numerically;
  - if only the remote has a version: newer;
  - if only the local has one: not newer;
  - otherwise use `modifiedTime > local + 250 ms` (`DRIVE_REMOTE_META_SKEW_MS`).
- Without a head revision ID: `modifiedTime > local + 250 ms`.

**`canApplyDriveRemoteMeta`** (`app:25713-25756`):

- The same head revision can always be applied (properties-only version bumps).
- A different head is applied only if its `version` is greater.
- It never regresses `modifiedTime`, and refuses equal timestamps with different heads.

**`fetchStableDrivePdfSnapshot`** (`app:26013-26049`), up to 3 attempts:

1. Download the exact revision (`files/{id}/revisions/{rev}?alt=media`). This is permitted for owners only. On 403 or 404 the file is remembered as `driveExactRevisionUnavailableFiles` and the head download (`files/{id}?alt=media`) is used instead (`app:25966-26011`).
2. A head download is accepted only if its `md5Checksum` and `size` match metadata fetched around it.
3. Re-fetch the metadata afterwards. It must match (same `headRevisionId`, or the same `modifiedTime`).

**Conditional upload** (MUST for safe coexistence) (`app:26482-26536`):

- Read the write token with a **Drive v2** call: `GET drive/v2/files/{id}?fields=id,etag,headRevisionId,modifiedDate,version`.
- Upload with `If-Match: <etag>`:
  - ≤ 4 MiB: `PUT upload/drive/v2/files/{id}?uploadType=media`;
  - larger: resumable upload with 1 MiB chunks.
- On **412**, re-read the token. If `headRevisionId` is unchanged (only metadata or properties changed), retry the **same** blob, up to 2 attempts. Otherwise rethrow: the save is aborted, `deferRemotePull(null)` runs and a merge-then-save restart follows (`app:27102-27124`).

### 5.3 Drive `properties` and `appProperties` registry (MUST, interop)

Drive applies a 124-byte limit to key plus value (UTF-8) (`live:933-937`). `properties` are visible to every principal that can read the file. `appProperties` are private to the OAuth client.

| Key | Store | Value | Writer | Reader | Cleanup |
|---|---|---|---|---|---|
| `p_<simpleHash(email:clientId)>_q` | properties | Presence identity (§6.1) | `pingPresence` every 120 s | metadata polls | `clearPresence` on exit; stale records (> 10 min) are removed by any pinger (`app:28090-28098`) |
| `p_<…>_r` | properties | Presence runtime (§6.1) | heartbeat (2/6/30 s), save-phase pushes | metadata polls | same |
| `p_<simpleHash(email)>_<m\|t\|p>` (legacy single JSON) and `p_<…>_{n,e,p,t,d,v,l,c,h,s,m}` (legacy chunked) | properties | Legacy presence | not written any more (only deleted) | still parsed (`app:28326-28347`) | deleted by `pingPresence` when stale or corrupt |
| `ihn_live_key_v1` | properties | 32 random bytes, base64url without padding (AES-256-GCM key for signalling) | first editor, with compare-after-write (§10.1) | editors | never |
| `ihn_lm1_<h8(target)>_<h8(source)>` | **appProperties** | `${offerCommentId}\|${base36(expiresAt)}` | offer initiator (§10.3) | target's metadata poll, or mailbox burst | sender clears it once the channel opens; an expired value is cleared by the receiver |
| `ihn_ink_<device24>` | properties | ID of the writer's latest journal file | journal `announce` after every confirmed upload (`inkapp:136-147`) | `InhouseDriveInk.hints` on every metadata poll | never |
| `ihn_live_<device24>` | properties | ID of the writer's live-preview file | preview `announce` | same | never |

Here `device24 = clientId` stripped to `[A-Za-z0-9]`, keeping the last 24 characters (`inkapp:139`). `h8` is `simpleHash(peerId).slice(0,10)`, which is at most 8 characters because `simpleHash` already truncates.

Two hazards come from this registry:

- **Prefix collision:** `ihn_live_key_v1` starts with `ihn_live_`, so `hints()` registers the AES key string as a live-preview **file ID** (`inkapp:320-321`). The preview loop then GETs `files/<key>?alt=media`, which returns 404, and retries every 3 s (`preview:218`). This wastes requests and puts the key value into URL paths sent to Google.
- **Drive property quotas** (an external Drive API constraint, not enforced by the code): presence uses 2 public properties per device, plus 2 hint properties per device, plus the key. Documents with many devices can reach the per-file public property limit. The hint keys are never garbage-collected.

---

## 6. Presence

### 6.1 Record format (MUST, interop) (`app:27744-27807`)

```
q = accountHash "|" clientId(≤16) "|" email(≤38, '|' removed) "|" name(≤22, '|' removed) "|" deviceType[0] ("m"|"t"|"p") "|" deviceLabel(≤16, '|' removed)
r = base36(nowMs /* heartbeat timestamp */) "|" phaseCode ("p"=processing | "u"=uploading | "s"=saved | "") "|" base36(phaseAt)|"" ["|" base36(advertisedIntervalMs)]
```

The fourth `r` slot is written **only** when the interval is greater than 2000. A 2 s beat is therefore byte-identical to older builds (`app:27764-27766`, `tests/collab-cadence-v5.test.mjs:295`).

The parsed record is `{a, c, e, n, d, v, t=h=heartbeat, s, m, i}`. Here `i` is the advertised interval, clamped to the range 2000 to 120000 (`app:27769-27773`).

The clamping lengths keep `key + value ≤ 124` bytes. The `q` value is at most 106 characters and the key at most 12.

**Privacy:** the email and display name are stored in public `properties`, readable by anyone who can read the file.

### 6.2 Write cycle (`app:28040-28290`)

- **Ping** (`pingPresence`) on start (after 100 ms; waits for the profile) and every **120 s**:
  1. GET `fields=appProperties,properties`.
  2. Update presence from the response.
  3. PATCH `properties` with own `q` and `r`, nulling own legacy keys and the stale or corrupt records of others. In the same request, null any own private `appProperties` presence leftovers.
  4. Enable heartbeats (`_presenceCanHeartbeat`).
- **Heartbeat** (`pulsePresenceHeartbeat`): PATCH `properties` with only `r`.
  - Skipped while a Drive save is in progress, before the first successful ping, or while another heartbeat is in flight.
  - Its delay follows the presence mode (§6.4): **2000** `shared`, or whenever an active save phase is processing or uploading; **6000** `linked`; **30000** `solo`.
  - The next beat is due at `min(interval, advertised)`. A skipped or failed beat retries at `min(next, 4000)` (`app:28127-28153`).
  - When the mode needs a faster cadence than the advertised one, it beats immediately (`app:28155-28161`).
- **Save phase push** (`setPresenceSavePhase`): an extra `r` PATCH, sent only if the mode is not `solo` (`app:28259-28290`).
- **Clear** on exit or unload: PATCH nulls with `keepalive` fetch (`app:28724-28756`).

Presence writes are attempted regardless of `driveCanEdit`. For principals without write access they fail silently, because errors are swallowed.

### 6.3 Read and online window (`app:28292-28402`)

Presence is refreshed from every metadata response (`pollDriveRevision`, the post-save check, `pingPresence`).

| Rule | Value |
|---|---|
| Stale | `now - max(t, h) > 600000` drops the record |
| Online, fast records | `now - h ≤ 2000*6 + 1200 = 13200` |
| Online, otherwise | `now - h ≤ max(13200, 2*i + 1200)`: 13.2 s for a 6 s interval, 61.2 s for a 30 s interval |
| Sticky | an online user stays online for 3 extra polls that miss (`PRESENCE_ONLINE_STICKY_MISS_LIMIT`) |
| Stability state forgotten | after `600000 + 30000*(6+3)` |
| Dedupe | by email or name, plus client ID; the fresher record (or higher field score) wins |

Own records (same client ID) are excluded. The resulting `latestPresenceUsers` drive:

- `liveCollabUpdatePeers` (P2P discovery);
- `ihnConsumeSignalMailboxesFromProperties` (§10.3);
- `InhouseDriveInk.hints` (§8.7, §9.4);
- the avatars and the save-ring UI.

### 6.4 Presence mode (`getCollabPresenceMode`, `app:27306-27330`)

| Mode | Condition |
|---|---|
| `shared` | Another online device (or one with a recent save signal) has no open P2P channel, or any P2P peer is still connecting |
| `linked` | Every such device has an open P2P channel, or the P2P overview reports open peers |
| `solo` | Neither of the above |

A mode change re-tunes the poll and heartbeat timers (`app:27334-27389`).

---

## 7. Drive revision polling and pull

### 7.1 Cadence (`app:27231-27413`)

| Constant | Value |
|---|---|
| `COLLAB_POLL_INTERVAL` | **600** (`shared`, or for 60 s after any remote revision) |
| `COLLAB_LINKED_POLL_INTERVAL` | **3000** (`linked`; `drive-committed` hints drive the polls) |
| `COLLAB_IDLE_POLL_INTERVAL` | **20000** (`solo`) |
| `COLLAB_RECENT_REMOTE_FAST_MS` | 60000 |
| Post-save catch-up | solo `[1800]`; shared `[900, 1800, 3500]` |
| After a successful pull | another poll at **+120 ms** |
| Pull retry backoff | `round(min(15000, 700*2^(f-1)) * U(0.8, 1.2))`, `f ≤ 6` (`app:27269-27283`) |
| Commit hint throttle | 400 ms per file (`live:1629-1637`) |
| Poll request timeout | 7000 (`DRIVE_POLL_TIMEOUT`); post-save check 3500 |

A scheduled poll is never postponed past an earlier due time (`app:27350-27372`).

### 7.2 `pollDriveRevision` state machine (`app:27530-27612`)

The poll is skipped when:

- there is no file or token;
- a poll or pull is already in progress;
- the tab is hidden;
- the time is inside the retry window.

Otherwise it issues `GET files/{id}?fields=headRevisionId,modifiedTime,version,appProperties,properties` with `Cache-Control: no-cache`, then:

```
-> updatePresenceFromProps(app+props) ; InhouseDriveInk.hints(id, properties)
-> changed = !hydrated || isRemoteMetaNewerThanTracked(meta)
   changed:
     noteRemoteRevisionActivity()                        (fast for 60 s)
     driveSaveInProgress      -> restartDriveSaveForRemoteChange (abort upload, defer pull, re-save)
     hasUnsyncedLocalChanges  -> deferRemotePull(meta)    (merged by the next save, §7.4)
     else pullRemoteChanges(meta, {visualSync}) -> success ? poll(+120) : scheduleCollabPullRetry
   unchanged: refresh modifiedTime display
```

### 7.3 `pullRemoteChanges` (`app:28762-29185`)

1. Without `allowWhileLocalPending`, while local changes are unsynced: **defer**.
2. Download a stable snapshot (§5.2). Parse the PDF metadata: `STROKES_*`, `IH_STRUCT`, `IH_FIELDS`, `IH_CAL`, `IH_SYNC`, `IH_VERSIONS`.
3. **Apply boundary:** re-fetch the revision metadata. It must match, and `canApplyDriveRemoteMeta` must hold. Otherwise invalidate hydration and defer (`supersededRevision`).
4. Adopt the version history.
5. On an echo or duplicate `IH_SYNC` hash: mark the revision applied and return.
6. `applyRemotePages(...)` (§4).
7. Re-render backgrounds for pages that need PDF or photo pixels from the downloaded PDF.
8. **After apply:** re-fetch the metadata again. It must still match. Otherwise defer (`app:29094-29120`).
9. Then:
   - `hasLocalMerges`: bump the content version and `requestImmediateDriveSave({force:true})`;
   - otherwise mark the content uploaded and adopt `IH_SYNC.contentHash` as confirmed.

### 7.4 Save path interplay (`app:26802-27226`)

- **Debounce** (`scheduleDriveSync`, `app:26362-26421`): 40 ms idle (`DRIVE_SYNC_IDLE_DELAY`), with a maximum of 3000 ms.
  - Deferred while there is smooth interaction.
  - Deferred while there was user activity in the last 180 ms (`DRIVE_BUILD_IDLE_WINDOW_MS`).
  - Deferred during a pen burst: two strokes within 3000 ms start a 1200 ms quiet period (`PEN_BURST_*`, `app:1966-1967`, `app:25364-25369`).
- **Before building:** flush any deferred pull (merge first). If a remote device shows an active save phase, merge its revision too (`app:26874-26899`, `app:26965-26971`).
- **Final check loop:** read the etag. If the remote is newer, merge, rebuild the PDF and re-check, up to 3 times. After that the save aborts with "Remote changes still arriving" (`app:26975-27018`).
- **On success:**
  - set the presence phase to `saved`;
  - `publishDriveCommitHint(fileId)` over BroadcastChannel and every open peer (`app:27086-27087`, `live:1647-1653`);
  - schedule the catch-up polls;
  - if a remote change arrived during the save, pull it.

---

## 8. Drive ink journal (`journal`, adapter `inkapp`)

### 8.1 Purpose and lifecycle

The journal is a small, durable, causal edit log on Drive. It is independent of PDF compilation.

**Start and stop:**

- The adapter (`InhouseDriveInk`) starts when a signed-in offline-library account exists (`inkapp:71-158`).
- A 2 s `setInterval` starts or stops the engine as the account, document or session changes (`inkapp:334-337`).
- `online` events call `retry()` (`inkapp:338`).
- The engine is document-scoped when `usable()` holds: the session is activated, no open is in progress, an account exists, and the file ID is real (not `local-…`).
- Otherwise the engine runs **document-less**. It only drains the outbox, and is used on Home and in the background (`app:6035-6051`).

### 8.2 Local durability (IndexedDB `settings` store, per account) (`journal:12`, `journal:73-100`)

| Key | Value |
|---|---|
| `ink-journal:<docId>:<actor>:<part>` | Record `{kind:'ink-journal-v1', docId, actor, part, ops:[encodedOpString], bytes, rev, ack, version, previousKey, createdAt, fileId?, exists?, etag?, retryAt?, failures?}` |
| `ink-journal-outbox` | `{pending:{recordKey: rev}}`, a sparse index |
| `ink-journal-folder` | `{fileId}`, the cached Drive folder |

`enqueue(op, version)`:

1. Reject when stopped, when there is no docId, or with `INK_REQUIRES_PDF` when the op is too large (§8.5).
2. **Rotate parts** when the current record has 128 ops, or `bytes + len > 256 KiB`. A fully acknowledged previous record is dropped from memory.
3. Push the JSON string and increment `rev`. `version = driveContentVersion`.
4. Freeze the snapshot synchronously.
5. Commit the record and the outbox pointer **atomically** (`patchAndQueue`, `offline-store-v5.js:67-78`).
6. Then `wake()`.

### 8.3 Drive file (MUST for same-account v5 interop)

- Name: `ink-<docId>-<actor>-<part>.json`
- MIME type: `application/json`
- Parent: the writer's own folder `inhouse notes`
- `appProperties` (`journal:182-183`): `{ihnKind:'ink-journal-v1', ihnDoc:docId, ihnActor:actor, ihnPart:String(part)}`

Body (`journal:159-160`). The prefix object is serialized first; `ops` holds the raw op JSON values:

```
{"v":1,"docId":D,"actor":A,"part":N,"rev":R,"previous":"<predecessor fileId or ''>","previousRev":P,"ops":[op1,...,opR]}
```

Invariants checked by readers (`journal:329-330`):

- `v === 1`;
- `docId` matches;
- `actor` is a string;
- `rev` is a safe integer;
- `ops` is an array of at most 128 items;
- `rev === ops.length`.

The body may be at most 8 MiB (`MAX_BODY`). Each upload **rewrites the whole part** (cumulative ops).

**Folder:** found by the query `'me' in owners and mimeType=folder and name in ('inhouse notes','Inhouse Notes')`; the smallest ID wins. It is created if absent and cached. One lookup is shared per store and account by the journal and the preview (`journal:112-141`).

### 8.4 Write algorithm (`flush`, `upload`) (`journal:142-289`)

- **Run conditions:** at most once per 120 ms (`lastWrite`), and not before `nextOutboxCheck`.
- **Lease:** the cross-tab lease `withLease(account, 'ink-journal-write')`:
  - IndexedDB `leases` record with TTL 45 s, renewed every 10 s;
  - plus a Web Lock named for the owner (`offline-store-v5.js:205-224`).
- **Selection:** the outbox is repaired once per process (records with `rev > ack`). Then the **first** due record is uploaded, with records of the current document first. **One upload per pass.**
- **Identity:** the file ID is reserved before creation with `GET files/generateIds?count=1&space=drive&type=files` and persisted. The folder lookup and the reservation run in parallel.
- **Predecessor:** the predecessor's file ID is reserved, and `previous` and `previousRev` are written.
- **Create:** multipart `POST upload/drive/v3/files?uploadType=multipart&fields=id,modifiedTime` with the reserved ID. On **409** (a lost create response), the existing `appProperties` identity is verified and an update follows.
- **Update:** `PUT upload/drive/v2/files/{id}?uploadType=media&fields=id,etag` with `If-Match: etag`. Without a known etag:
  1. GET the v2 etag and the current content;
  2. verify the identity and that the remote ops are a **prefix** of the local ops; otherwise fail with "Ink history conflict; both copies retained";
  3. if the remote `rev` is at least the local `rev`, treat the record as already uploaded.
- **Receipt:**
  - `ack = rev`, store the etag, clear the outbox entry;
  - mark the own source as local and retired;
  - `host.receipt({...receipt, docId, version, actor, part, operations: ops.slice(oldAck)})`, which retires the confirmed live-preview paths and advances `ackVersion`;
  - `notifyPeers` (§8.7).
- **Failure:**
  - `retryAt = now + min(30000, 600 * 2^min(6, failures))`;
  - 412 clears the etag;
  - 404 clears the folder and file ID;
  - the next outbox check comes after at most 2 s.
- Request timeout: 8000 per request (`journal:63`). `driveFetch` adds no 429 or 5xx retries.

### 8.5 Operations (`inkapp:16-19`, `inkapp:165-259`)

```
StrokeOp  = { v:1, kind:'stroke', pageId, stroke:{id,tool,color,width,points,syncStamp?} }
ContextOp = { v:1, structure:StructureMeta, fields:FieldMeta(filtered), name:exportName,
              calendar:calendarPageConfig|null, pages:[PagePatch] }      // pages may be []
PagePatch = shell(page) + { strokes:[≤32 changed], images:[≤4 changed] }
shell     = buildTimelineSnapshotFromPages([{...page, strokes:[], images:[], legacyCoverStrokes:null}])[0]
```

**Fields filtering** (`contextForPage`): all `doc:*` fields, plus only **this** page's four `page:<id>:*` fields, in the original property order (`inkapp:33-59`). A context with `pages:[]` carries only the document fields.

**Size limits:**

- An op may be at most 4 MiB of JSON (`MAX_BODY/2`).
- A preflight sums the photo `backgroundImage` and image `src` lengths before serializing.
- An op that is too large is rejected with `INK_REQUIRES_PDF`: the edit travels only in the PDF (`journal:12-44`, `journal:73-77`).

**Capture paths:**

1. **Immediate stroke path.** `enqueueStrokeOp` calls `InhouseDriveInk.stroke(index, clonedStroke)` at pen-up, so every finished stroke becomes one `StrokeOp`, with no page scan. It is skipped while journal ops are being applied (`inkapp:260-272`, `app:24591-24593`).
2. **Page capture.** `InhouseDriveInk.page(i)` and `.change()` mark pages or metadata dirty, and a capture runs 60 ms later (`inkapp:159-164`, `inkapp:273-274`):
   - Wait for hot-interaction idle (§15).
   - Wait for `persistChain`.
   - On the first run, establish **baselines**: per page, the item ID to stamp string; a metadata snapshot (shell JSON plus the photo string compared by identity); the field key; the context key. A dirty session marks all pages for capture.
   - For each changed page, diff the items against the baseline by `clock:actor` stamp.
     - **Stroke-only fast path:** used when the shell, the fields and the context are unchanged and only existing IDs changed. Each changed stroke becomes a `StrokeOp`; photos are not retransmitted.
     - **Otherwise:** context ops are sliced into `count = max(1, ceil(strokes/32), ceil(images/4))` parts.
   - If metadata changed and the context key differs from the last one sent, send a context op with `pages:[]`.
   - **Abort and requeue** if `driveContentVersion` moved or an apply is in progress (`deferChangedCapture`).
   - Baselines advance only after a successful enqueue.

### 8.6 Discovery and read (`journal:290-397`)

| Aspect | Behaviour |
|---|---|
| Discovery | Every **5 s**: `files.list q="trashed=false and appProperties has {ihnKind='ink-journal-v1'} and appProperties has {ihnDoc=<docId>}"`, all pages (pageSize 100). Results are added as passive hints. |
| Sources | `Map(fileId -> {next, retryAt, rev, hotUntil, idleReads, actor, part, requiredRev, retired, local, raw, readEpoch})`, at most **4096**. IDs must match `^[\w-]{1,200}$`. |
| Lanes | At most **4** concurrent reads, one per source. Hot sources first, then by `next`. |
| Read | `GET files/{id}?alt=media` with `Cache-Control: no-cache`, buffered. The first empty poll waits for the initial discovery, unless a direct hint arrives. |
| Cadence | Before a read: `next = now + (hot ? 120 : 1800)`. Identical raw text: `idleReads++` and `next = hot ? 120 : min(30000, 1800 * 2^floor(idleReads/3))`. New data: `hotUntil = now + 6000` and `next = now + 120`. Error: `retryAt = now + 5000`. |
| Apply | Serialized through `applyChain`. Waits for `host.idle()` (hot-interaction settle, 180 ms window). Validates (§8.3). `seen[id]` holds the last applied `rev`. A read with `rev ≤ seen` is ignored (no rollback). Otherwise `host.apply(ops.slice(seen), …)` runs unless the actor is self, and only after success are `seen`, `raw` and `rev` advanced. |
| Chain | Reading a successor hints its `previous` (passive), and sets `predecessor.requiredRev = max(previousRev)`. A predecessor behind its required `rev` is un-retired and read immediately. Sources of the same actor with `part < latestPart` that reached their `requiredRev` are **retired**. |
| Own files | Own uploads set `local = true` and retire the source. Another tab's actor stays readable (`journal:205-214`). |

**Scheduler** (`journal:399-441`):

- One timer. `tick` runs `flush()` and `poll()` independently.
- The next delay is the earliest useful deadline, capped at **2000**.
- While any I/O is outstanding the delay is **120**.
- `wake()` after enqueue honours the 120 ms write spacing.
- Hints may only bring the timer earlier.

### 8.7 Hints and announcements

**`notifyPeers(docId, fileId, preview)`** runs after each confirmed upload. It is coalesced per `docId:preview`, and calls `host.announce` (`journal:218-235`, `inkapp:136-147`):

1. `publishDriveInkHint` posts `{type:'drive-ink-hint', fileId, id, preview}` on the BroadcastChannel and to every open peer (`live:1639-1645`).
2. Unless a Drive save is in progress, it PATCHes `properties[ihn_ink_<device> or ihn_live_<device>] = id`. This happens once per new ID.

**Receivers:**

- A BroadcastChannel or P2P `drive-ink-hint` calls `InhouseDriveInk.hint(docId, id, preview)`. That makes the journal source **active**, so the read is due in ≤ 120 ms (`live:326-327`, `live:3948-3949`, `inkapp:303`).
- **Metadata properties:** every `ihn_ink_*` value calls `journal.hint(id)`, which is **active**. While metadata polls arrive faster than every 6 s, every announced current journal part therefore stays hot and is fetched about every 120 ms plus the RTT (`inkapp:304-319`, `journal:101-111`).

### 8.8 Apply adapter (`inkapp:83-135`)

1. Sort the ops so that non-stroke ops come first, which materializes page shells before strokes.
2. A `StrokeOp`:
   - is skipped if its page is tombstoned;
   - **throws** ("Waiting for the journal page metadata") if the page is unknown, which keeps `seen` unadvanced so the op is retried in 5 s;
   - consecutive stroke ops for the same page are folded into one patch (at most 32 distinct strokes and 8192 points);
   - the patch is wrapped as `{v:1, ...context(), pages:[shell + strokes]}`. **The receiver's own current structure and fields are used as the context.**
3. Per op: wait for hot-interaction settle, then `applyRemotePages(op.pages, {additiveById:true, preserveLocalUnsynced:true, remoteStructure, remoteFields, remoteExportName, remoteCalendarPageConfig})`.
4. Clear the matching live previews. If anything changed, invalidate the page caches, bump the content version, set `driveDirty` and `scheduleDriveSync()`. This means **the receiver re-uploads the PDF**.

### 8.9 Status (`journal:69-72`, `inkapp:330`)

`{writing, error, pending: Σ(rev - ack), ackVersion, pendingCapture}` feeds the save indicator.

### 8.10 Reach limitation (important for v6)

The journal and preview files are created in the **writer's own folder**, and no permissions are ever granted (there is no `permissions` call in `journal` or `preview`). Discovery only returns files the reader can access, and a `properties` hint to an unreadable ID yields 404s and 5 s backoffs.

In practice the journal and the preview are therefore **same-account, cross-device** transports. Cross-account collaboration relies on P2P plus the PDF. Files are never deleted: each session creates a new actor, so the parts accumulate.

---

## 9. Drive live preview (`preview`)

A subclass of `Journal` that shares its request, folder, hint and announce machinery (`preview:6-17`).

### 9.1 Frame file (MUST for same-account interop)

- Name: `live-<docId>-<actor>.json`
- Parent: the same `inhouse notes` folder
- `appProperties`: `{ihnKind:'ink-preview-v1', ihnDoc, ihnActor}`
- **One file per actor** (tab session), reserved through `generateIds`

Body (`preview:89-95`):

```
{ "v":1, "docId":D, "actor":A, "frame":int, "at":ms, "packets":[Path ≤24] }
Path = live-stroke packet (§10.8) with offset:0, points = complete prefix up to the first gap (≤24000), updatedAt
```

The raw body may be at most **2 MiB**. A larger frame throws, and the stroke stays in the durable journal.

Create uses multipart POST, with an identity check on 409. Update uses `PATCH upload/drive/v3/files/{id}?uploadType=media` **without a precondition** (single writer per actor).

### 9.2 Writer (`preview:19-126`)

1. `push(packet)` accepts only own, local-origin `live-stroke` packets for this document. It is called from `ihnSendRealtimeStrokePacket` when the packet is not a relay (`live:1697`).
2. Point slots are merged by `offset`. `finalBatch` with offset 0 restarts the geometry, `totalPoints` truncates, and `cancel` empties the path.
3. Paths older than 15 s are dropped, keeping at most 24 (oldest first). Each push increments `frame`.
4. `scheduleFrame`: at most one write in flight, at least **100 ms** between writes (`FRAME_MS`). After a failure, retry in 1000 (5000 on 429).
5. `start()` publishes an initial frame immediately when the user can edit. This is the warm-up that creates the file.
6. `retireConfirmed(ops)` runs after the own journal receipt. It removes **final** paths whose ID and stamp were confirmed, then writes a new frame (`preview:39-59`).

### 9.3 Reader (`preview:127-260`)

| Aspect | Value |
|---|---|
| Discovery | Every 5 s: `files.list` with the same `appProperties` query for `ink-preview-v1`, `orderBy=modifiedTime desc`, `pageSize=30` |
| Sources | At most 128. A new source gets an immediate probe; an `active` hint gives 5 s of activity. |
| Lanes | 4 concurrent reads, the most active first |
| Cadence after a read | **100** if active (5 s after a new frame or active hint), **300** if the source wrote a frame within the last 120 s (`WARM_MS`/`WARM_FOR_MS`), else **2000**. Stale frame (`at` older than 20 s or more than 30 s in the future): 15000. Response received: `retryAt = +2000` unless valid. Error: `+3000`. |
| Validation | `v===1`, same `docId`, `actor != self`, integer `frame`, at most 24 packets, finite `at` |
| Delivery | Only when `frame > source.frame`. Each packet goes to `host.preview` (`inkapp:276-302`). |
| Decode cache | Identical bytes skip parsing. At most 4 Mi characters in total; LRU. Never extends freshness. |

The adapter `queuePreview` (`inkapp:276-302`):

- drops packets from its own actor;
- dedupes by `actor:stroke` sequence;
- batches per `requestAnimationFrame`;
- clears the preview if the stroke is already committed, deleted or cancelled;
- otherwise renders the tail with a one-point overlap through `applyRemoteLiveStrokePreview`.

### 9.4 Heartbeat-gated wake (`inkapp:304-329`)

- An `ihn_live_<device>` hint is **active** only when the same device's presence heartbeat `h` is fresh (age ≤ 13200 and not more than 30 s ahead) **and** newer than the last heartbeat used for that source. Otherwise it is passive.
- The age limit is fixed at 13.2 s, so devices that advertise a 30 s solo heartbeat rarely wake previews.

---

## 10. WebRTC P2P (`live`)

### 10.1 Signalling key and envelope (MUST for P2P interop with v5)

**The key** (`live:427-668`):

- `properties.ihn_live_key_v1` holds 32 random bytes encoded as base64url, imported as raw AES-GCM.
- **Creation is compare-after-write.** It runs only if a successful GET shows the key is absent; a failed GET never creates a key:
  1. PATCH the candidate.
  2. GET again and import whatever Drive now holds.
  3. Mark the key `needsConfirmation`, so the next encode re-reads it once.
- Concurrent requests share one load or refresh promise.
- **On a decrypt failure,** reload the key from Drive once and retry. Undecryptable ciphertexts are remembered for 5 s, keyed by `simpleHash(content):length` and the key fingerprint, with at most 256 entries (`live:696-813`).
- **Opening prefetch** (`prewarmLiveCollaboration`, `live:445-475`): the properties and recent comments are fetched in parallel with the PDF download. Each result is used once, within 15 s, for the same session.

**The comment content:**

```
"IHN_LIVE_V1:" + base64url( iv[12 random bytes] || AES-GCM(key, iv, utf8(JSON(payload))) )   // no AAD, 128-bit tag
```

### 10.2 Signal payloads (all carry `v:1`)

| type | Fields | Carried in | TTL and acceptance |
|---|---|---|---|
| `rendezvous` | `fileId, from, to:'*', epoch:'r'+base36(now)+12hex, announcedAt, expiresAt` | Top-level comment (one per leader, PATCHed in place) | Accepted if `now ≤ expiresAt ≤ now + 180000` (`live:1152-1163`) |
| `offer` | `fileId, from, to, sessionId:'s'+base36(now)+6 random, expiresAt:now+90000, fastRecovery, description:{type,sdp}` | New top-level comment | Expired offers to or from self are deleted on scan |
| `answer` | `fileId, from, to, sessionId, expiresAt, description` | Reply to the offer comment | `peer.initiator`, same `sessionId`, no remote description yet |
| `candidates` | `fileId, from, to, sessionId, expiresAt, candidates:[{candidate, sdpMid, sdpMLineIndex, usernameFragment?}]` | Reply to the offer comment (both sides) | Same `sessionId`, peer not closing |

### 10.3 Discovery

There are four discovery inputs, and all are encrypted or private:

1. **Presence** (`liveCollabUpdatePeers`, `live:2772-2812`):
   - Each online presence record becomes a known peer.
   - A **newly** expected peer schedules an immediate retry with `allowReverse` (either side may offer), a mailbox burst and a fast signalling window.
   - `mainPeerId` is the lexicographically smallest ID.
2. **Rendezvous beacon** (`live:1032-1216`):
   - Published by the leader when it is visible and online.
   - **New epoch or announcement:** the peer becomes expected (`lastPeerSeenAt`, non-speculative), and is cached in `localStorage` at most once a minute.
   - **First sight or new epoch:** fast window, immediate offer (`allowReverse`) and a mailbox burst.
   - **New epoch with an open channel:** route probe. **New epoch while connecting:** close and retry.
   - An unchanged, re-read beacon does **not** keep a peer expected.
3. **Mailbox** (`live:916-1017`, `live:2520-2567`):
   - After posting an offer, the initiator PATCHes `appProperties[ihn_lm1_<h(target)>_<h(self)>] = commentId|base36(now+90000)`. It must fit in 124 bytes.
   - The target consumes mailboxes from **every metadata poll** at no extra cost. It also runs a **mailbox burst** (GET `fields=appProperties`) at `[0, 70, 150, 260, 420, 650, 950, 1400, 2100, 3200, 5000]`.
   - For each entry it GETs **that exact comment** and processes it. The value is remembered until expiry, and the receiver never deletes it.
4. **Comment scan** (`ihnPollSignals`, `live:2687-2770`):
   - Lists comments modified within the last 180 s, following every page.
   - Processes comments in batches of 4. One failing handshake does not hide the others.

Also:

- **Cached peers:** `localStorage['ihn-live-peers-v1:' + simpleHash(fileId)] = {v:1, peers:[{id, at}]}`, at most 12 peers within 7 days. Each gets **one** speculative warm-start offer per session (`live:349-403`, `live:2977-2981`).
- **Direct reply poll:** the initiator and the answerer GET their exact offer comment at `[0, 80, 170, 280, 280, 300, 300, 350, 450, 600, 900, 1400, 2100, 3000, 4500, 6500]` until the channel opens (`live:2569-2606`).

**Signalling mode and scan cadence** (`live:851-903`, `live:2648-2671`):

| Mode | Condition | Scan delay |
|---|---|---|
| `hot` | `now < fastUntil` (start +3000, discovery or network +10000) or a non-speculative handshake in progress | **300** |
| `present` | Peer company or an expected peer | **3000**, or **10000** if the metadata feed is fresh (≤ 45000 since the last properties delivery) |
| `solo` | Neither | **2500**, or **30000** if fed |

- Hidden tab: 30000 (and `ihnPollSignals` returns immediately while hidden).
- Offline: 2500.
- Failures: `max(delay, min(15000, 1000 * 2^min(4, f-1)))`.
- Network or route bursts: supervise at `[40, 120, 260, 480, 800]` (`live:3101-3117`).

**Beacon cadence** (`live:869-880`, `live:1046-1059`):

| Situation | Refresh | Beacon TTL |
|---|---|---|
| An open peer exists | 15000 | 45000 |
| Company within the fast window | 1500 | 45000 |
| Company | 15000 | 45000 |
| Alone | 60000 | 100000 |

The beacon is renewed whenever it is within 30000 of expiry. It is **rotated** (new comment and new epoch; the old one is deleted) only on a network route change with company present.

**Supervisor period:** 250 (2000 when `solo`) (`live:23-26`, `live:882-893`).

**Capability check:** every 30000 (300000 when `solo`) (`live:3365-3400`).

### 10.4 Negotiation state machine

**Who may offer** (`live:1503-1525`):

- **Leader tab only**, and only with edit rights and a token, to an **expected** peer.
- Normally only when `ownPeerId < peerId` lexicographically. `allowReverse` (discovery, closing an established link, cached peers) permits either side.

**Initiator** (`ihnCreateOffer`, `live:2280-2363`):

1. Create `RTCPeerConnection({iceServers: 4 Google STUN servers, iceCandidatePoolSize:0, bundlePolicy:'max-bundle', rtcpMuxPolicy:'require'})` (`live:102-107`, `live:203-214`).
2. Create data channel `'inhousenotes-live-v5'` with `{ordered:true}`.
3. Create the offer. **Subscribe to ICE before** `setLocalDescription`.
4. Wait for ICE: complete, or 70 ms after the first candidate, or a timeout of 650 (280 for fast recovery) (`live:1218-1243`).
5. Encrypt and POST the offer comment. Then publish the mailbox, start the direct reply poll and flush local ICE.
6. If the peer was replaced or the document changed meanwhile, DELETE the comment.

**Answerer** (`ihnAcceptOffer`, `live:2365-2451`):

1. Dedupe by `commentId:sessionId`, with in-flight and processed maps (expiring at `expiresAt`).
2. Ignore the offer if a channel is open and the offer is not `fastRecovery`.
3. **Glare:** if a peer for the sender exists and is younger than 18000:
   - the local offer wins if this side is the initiator, the channel is not open and `ownId < from`;
   - a duplicate or older answer session is ignored.
4. Otherwise close the old peer, then `setRemoteDescription`, drain the queued remote ICE, create the answer, wait for ICE, and POST the answer as a reply.

**Answer application** (`live:2453-2478`): reserved synchronously (`applyingRemoteAnswer`), so concurrent deliveries of the same answer apply the SDP only once.

**Local ICE trickle** (`live:1290-1393`):

- Candidates are deduped by `candidate|sdpMid|mLine|ufrag`. Candidates already present in the signalled SDP are skipped.
- Batches are flushed after 45 ms as an encrypted `candidates` reply.
- Retry delay is `min(2400, 250 * 2^(n-1))`, with a limit of 4 attempts. The supervisor re-arms the flush after 5 s.

**Remote ICE** (`live:1395-1443`): queued until a remote description exists. A bad candidate is skipped without failing the route.

**Timeouts** (`live:1270-1288`):

- Connect deadline: `createdAt + 18000` (12000 fast).
- Each signalling activity extends the deadline to `now + 8000` (6000 fast), capped at `createdAt + 30000` (22000 fast).
- Disconnected grace: 5000.

**Retry backoff:** `ihnComputeReconnectDelay(failures-1, peerId) = round(min(30000, 1000 * 2^min(6, a)) * (0.8 + 0.4 * jitter))`. The jitter is `hex8(ihnHashString(peerId:attempt)) / 0xffffffff`, so it is deterministic (`core:651-656`). Failures are capped at 7.

**Fast recovery** applies to reasons matching network, route, watchdog, visible, resumed, focused, online or offline (`live:911-914`).

**Peer states:** `connecting`, then `open` (data channel `onopen`), then `closing` and deleted. `failed` or `closed` from the ICE or connection state closes the peer and retries (`live:2132-2166`).

**On open** (`live:2213-2252`):

1. Mark the peer known and cached.
2. Clear the mailbox and delete the offer comment.
3. Send `hello{interactiveChannel:1}` and `state-request`.
4. Flush the realtime backlog.
5. Force a targeted snapshot.

### 10.5 Data channels and framing (MUST for P2P interop)

| Channel label | Created by | Use |
|---|---|---|
| `inhousenotes-live-v5` | Initiator at offer time; `{ordered:true}`, reliable | Bulk: chunked snapshots, plus the control fallback |
| `inhousenotes-interactive-v1` | **Initiator only**, after receiving `hello` with `interactiveChannel === 1` (capability gated, because older builds treat any second channel as primary) | Control and realtime packets |

Any other label is closed (`live:2157-2165`, `live:3886-3892`). `ihnSendControl` prefers the interactive channel, and requires the bulk channel to be open (`live:1612-1623`).

**Framing v1** (`transport`) for snapshot envelopes:

```
{t:'start', id:uuid, total:ceil(chars/16000), chars:raw.length}
{t:'chunk', id, index:0..total-1, data:raw.slice(i*16000, (i+1)*16000)}   // sent in order, each JSON-encoded
```

**Sender:**

- Rejects payloads whose `JSON.stringify` exceeds 50,000,000 characters.
- Before **every** frame, waits until `bufferedAmount < 64000` (`bufferedAmountLowThreshold = 32000`). Gives up after 15000 ("congested") or when the channel closes.
- `assertCurrent` runs before each frame (`transport:7-52`).

**Receiver** (`transport:54-115`), key `JSON.stringify([peerKey, id])` with `peerKey = JSON.stringify([peerId, sessionId])`:

- Rejects `id` longer than 128.
- `start` requires: integer `total` in 1 to 5000, integer `chars ≥ total`, `chars ≤ total*16000` and `chars ≤ 50M`.
- Limits: at most 16 assemblies, at most 4 per peer, reserved characters ≤ 50M.
- A repeated `start` never resets an assembly.
- Chunks: bad index or data drops the assembly. A duplicate index with different data drops it; identical duplicates are ignored. Overrun drops it. Completion requires `size === chars`, then `JSON.parse`.
- TTL 60000. Partial assemblies are released on peer close.

**Wire guard:** each raw message must be a string of at most 2 MiB characters (`live:3831`).

### 10.6 Wire message catalogue (all JSON over the channels)

| Message | Fields | Handling |
|---|---|---|
| `{t:'ping'}` | `at, documentHash, [reason, networkProbe]` | Marks `protocolV2`, replies `pong`, reconciles the document hash (`live:3844-3850`) |
| `{t:'pong'}` | `at` (echo), `receivedAt, documentHash` | Valid if `at` is a pending ping time at or after `openedAt` and not in the future. Then sets `lastPongAt`, RTT EWMA 0.7/0.3, marks healthy, reconciles the hash (`live:3851-3881`). |
| `{t:'hello'}` | `at, interactiveChannel:1` | Marks `protocolV2`; the initiator opens the interactive channel; forced targeted snapshot |
| `{t:'state-request'}` | `at` | Forced targeted snapshot |
| `{t:'snapshot-received'}` | `hash, snapshotId, at` | Assembly complete. Records `pendingAckReceivedHash` and the time. |
| `{t:'snapshot-ack'}` | `hash, currentHash, snapshotId, at` | Merge complete. `lastAckedHash = hash` if `currentHash === hash`, else `''` and send a `state-request`. Clears the pending state, marks healthy. Legacy acks without `currentHash` imply `currentHash = hash` (`live:3905-3928`). |
| `{t:'snapshot-nack'}` | `hash, currentHash, [baseHash], [retryAfterMs], at` | Clears the pending state; forced targeted resend after `clamp(retryAfterMs \|\| 100, 100, 2000)` |
| `{type:'drive-committed'}` | `fileId` | Triggers `pollDriveRevision` (throttled 400 ms) |
| `{type:'drive-ink-hint'}` | `fileId, id, preview` | Journal or preview hint (§8.7) |
| `{type:'live-stroke'}` | §10.8 | Realtime preview, commit and relay |
| `{type:'live-erase'}` | §10.8 | Realtime erase preview and relay |
| framing `start`/`chunk` | §10.5 | The assembled envelope is a `document-snapshot` |

### 10.7 Snapshot protocol

**Envelope** (MUST) (`live:3628-3656`):

```
{ v:1, type:'document-snapshot', fileId, actorId:`${peerId}:${tabId}`, deviceId:clientId,
  sequence:++ihnLiveSequence (initialized to Date.now() at load), sentAt, baseRevision:driveHeadRevisionId|null,
  exportName, calendarPageConfig, structure:StructureMeta, fields:FieldMeta,
  pages:[sanitized page for EVERY page], snapshotId:`${actorId}:${sequence}`, contentHash,
  [partial:true, baseHash] }
```

**Build** (`live:3592-3665`):

1. Flush pending remote stroke commits.
2. Take the structure lock (15 s).
3. Flush stroke ops.
4. Load **all pages**.
5. Wait for collaborative idle (up to 15 s).
6. Fail if a remote stroke commit is pending.
7. Clone each page cooperatively, in 4 ms slices, each waiting for a 180 ms interaction quiet window.
8. **Abort** if `driveContentVersion`, `pageMutationGeneration`, a smooth interaction or a pending commit changed meanwhile.
9. Hash with `ihnCanonicalDocumentHash`.

**Delta** (`live:3534-3590`):

- A cache keeps the per-page serialized payloads of the last 3 snapshots (≤ 16M characters).
- The base is the peer's `lastAckedHash` or `remoteCurrentHash`, provided it is cached and differs from the current hash.
- A partial snapshot carries only the changed pages. It is used only if not every page changed **and** `changedChars + 2048 < 0.8 * fullChars`.
- `force` always sends the full snapshot.

**Scheduling** (`live:3667-3778`):

- `scheduleLiveDocumentBroadcast` debounces 40 ms (60 ms while applying or interacting, 0 when immediate).
- **Every local content bump** (`bumpDriveContentVersion`, `app:25621-25629`) schedules one, and clears `ihnLiveCurrentHash`.
- `ihnBroadcastDocument` defers by 180 while it is busy, while an apply runs, while there is smooth interaction, or while there was **user activity in the last 1200 ms**.
- Targets: all peers, or the targeted set minus exclusions.
- A peer is skipped unless forced if `lastAckedHash` or `pendingAckHash` already equals the hash.
- The tab channel always receives the full snapshot when the hash changed or the send is forced.
- Retry after a build or send failure: `min(2000, 120 * 2^(n-1))` for `n ≤ 4`, then `min(30000, 3000 * 2^min(4, n-5))`, with `n` capped at 16.

**Per-peer send queue** (`live:3780-3822`):

- At most 4 pending sends; serialized.
- `pendingAckHash` is registered **before** the first chunk, because the ACK can arrive on the interactive stream first.
- After transmission, the apply deadline restarts unless a receipt already arrived.

**Receive and apply** (`live:3963-4114`, `live:4220-4394`):

1. On assembly: send `snapshot-received`.
2. **Envelope validation:**
   - `v1`, `document-snapshot`, same file, `actorId` present;
   - `pages` non-empty unless the snapshot is partial;
   - every page has a string `pageId` and `strokes`/`images` arrays;
   - otherwise the envelope is rejected with no ACK.
3. **Apply coalescer:** one pending apply per `generation\0fileId\0actorId`.
   - A newer sequence replaces a pending older one, which resolves `false`.
   - An identical sequence and hash shares the same promise.
   - 36 ms gap between applies.
4. **Partial:** requires `baseHash === current hash`. Otherwise reply `snapshot-nack{hash, baseHash, currentHash}` and return `false` (no ACK).
5. `sequence ≤ seen[actorId]`: treated as already applied (ACK). Clear that actor's previews.
6. A hash applied within the last 120 s **and** equal to the current document hash: treated as a duplicate. Fan out only. A historical hash is merged again (handles C to D to C).
7. Wait while a local smooth interaction is active, polling every 32 ms.
8. `applyRemotePages(..., captureContentHash)`. Then `seen[actor] = sequence` (**only after success**), record the applied hash, and set `ihnLiveCurrentHash`.
9. **Convergence:** if there were local merges, or the merged hash differs from the advertised one:
   - when `ihnShouldQueueMergeUpload` allows it (one request per current hash, suppressed if the hash equals Drive's confirmed hash), bump the version once, set `driveDirty` and request an immediate Drive save;
   - force a full broadcast to everyone.

   Otherwise, **fan out** the exact envelope to the tab channel (when it came from WebRTC) and to peers that are neither the source, nor the origin peer, nor already holding the hash. A partial envelope is never relayed; a fresh full snapshot is broadcast instead (`live:4162-4214`).
10. Reply `snapshot-ack{hash, currentHash}`. On an apply exception, reply `snapshot-nack{…, retryAfterMs:500}`.

**ACK supervision** (`live:2882-2937`), for `protocolV2` peers:

| Condition | Action |
|---|---|
| No receipt after **4000** (and not still draining, with a recent pong) | Resend on the same route |
| Pending more than **20000** without a matching receipt and no recent pong | Close and retry |
| Pending more than **120000** with a healthy pong | Drop the pending state and resend (no reconnect) |
| Pending more than **120000** otherwise | Close and retry |

**Hash reconciliation** (`live:2168-2182`):

- A ping or pong `documentHash` different from ours triggers `state-request` plus a forced targeted snapshot.
- This is rate-limited to once per **3000** per peer, and skipped while busy.

### 10.8 Live stroke and erase packets (MUST for P2P interop)

**`live-stroke`** (`live:1739-1832`):

```
{ v:1, type:'live-stroke', fileId, actorId:`${peerId}:${tabId}`, strokeId, pageId, tool, color,
  width(≥0.1), syncStamp:{clock, actor≤240}, sentAt, sequence:int (per stroke, from 1),
  offset:int, points:[{x,y,p (default 0.5)}] (≤700 per packet), finalBatch:bool, totalPoints:int,
  final:bool, cancel:bool }
```

**Sender:**

- One record per stroke. The first call re-asserts `startLiveCollaboration()`, once per stroke.
- Frames every **18 ms** carry the unsent tail with a **one-point overlap** (`offset = sentPoints - 1`).
- **Final:** the complete stroke from offset 0, chunked into 700-point packets. Each carries `finalBatch:true` and `totalPoints`; the last has `final:true`.
- **Cancel:** an empty packet with `cancel:true`.
- At most 24000 points in total.
- Sent to the tab channel only once another tab has been seen. Serialized **once** and sent to every open peer. Also stored in the realtime backlog (192 packets, 12000 TTL), which is flushed to newly opened peers.
- The local own-origin packet also feeds the Drive live preview (§9.2).

**Receiver** (`live:1978-2110`):

- Drop packets whose `actorId` is self.
- Dedupe per `actorId:strokeId` by the set of sequence numbers (at most 512; the entry expires 60 s after its last update).
- Sanitize the lengths (actor, stroke and page IDs ≤ 240; tool ≤ 32; color ≤ 64; width ≤ 1000).
- Render with `applyRemoteLiveStrokePreview`. The preview TTL is 15000 (`app:3746`, `app:3809-3881`).
- Reassemble the final slots. When all `totalPoints` slots and the `final` flag are present, or for a legacy single packet with `final` and offset 0, call **`commitRemoteLiveStroke`** exactly once (`app:3933-4112`):
  - queue the commit until there is no smooth interaction and no structure lock, then load the page;
  - drop the stroke if a deletion wins;
  - otherwise resolve same-ID conflicts by stamp or fingerprint, sort the page's strokes by ID, and for editors `enqueueStrokeOp` plus `markPageDirty('stroke')`, which schedules a local Drive save; viewers persist to IndexedDB only;
  - remove a stale tombstone for the winning ID.
- **Relay** to the tab channel (if it came from WebRTC), and to every peer except the source and the origin peer. This builds a mesh even when the peer graph is not fully connected.

**`live-erase`** (`live:1834-1976`):

```
{ v:1, type:'live-erase', fileId, actorId, gestureId, pageId, sequence,
  removedStrokeIds:[id≤240] (≤500 per packet), addedStrokes:[{id,tool,color,width,points≤1200 (resampled)}] (≤200 per packet),
  final:bool, sentAt }
```

- The sender batches changes every 18 ms. The final flush happens at gesture end.
- The receiver keeps the highest sequence per `actor:gesture` (forgotten after 30 s), renders a non-durable overlay (TTL 15 s) and relays like strokes.
- Erasures become durable only through snapshots, the journal and the PDF (tombstones).

### 10.9 Health and route recovery (`live:2822-2994`, `live:3119-3189`)

**Supervisor tick:**

- Expire unexpected peers (presence expired and the channel unhealthy).
- Close peers from a stale document or generation.
- Connection timeout; re-arm the ICE flush; 5 s disconnected grace.
- ACK supervision (§10.7).
- Health timeout **30000** without receiving anything (legacy peers, which never sent a ping: 90000).
- Route watchdog: when visible and nothing has been received for more than **2500**, probe.
- Ping every **500**. Refresh the path stats every 15 s.
- Publish the beacon when due.
- Start due offers.

**Probe:**

- Send a ping with `networkProbe`.
- Wait `probeDelay = clamp(round(2.5 * RTT + 160), 220, 5000)`, or 320 if the RTT is unknown, using the best of the measured and Network Information RTTs.
- A pong or any received data cancels the probe. Then, except for the watchdog, resend the current state.
- Two misses (with a 120 ms yield between them) close the peer and retry immediately.
- Resume grace is `max(650, probeDelay + 180)`.

**Wake** (`live:2996-3099`), on online, offline, `pageshow`, focus, `visibilitychange` (visible), `resume` and connection `change`:

- Offline closes every peer with retry.
- A confirmed transport change (Wi-Fi to cellular), or `browser online`, replaces the route immediately.
- Other network events with company present probe open routes and rotate the beacon.
- A resume probes only routes stale for more than 2500, and otherwise grants a 10000 resume grace.
- Alone, at most one scan per 5000.
- The wrapper does nothing unless a session is running (`live:4400-4404`).

**Path stats:** `getStats` reads the selected candidate pair. The RTT EWMA is 0.7/0.3. `networkPath = 'local-network'` if both candidates are host candidates (`live:3268-3298`).

### 10.10 Leader election (one P2P owner per device per document) (`live:239-302`, `live:3402-3412`; pinned by `tests/live-collaboration-v5.test.mjs:550`, `:3589`)

- `localStorage['ihn-live-leader-v1:' + fileId] = {tabId, expiresAt: now+15000, heartbeatAt, hb:5000, visible}`.
- **Claim** succeeds unless another tab holds a lease that is fresh (`expiresAt > now`), with a heartbeat younger than `max(2500, min(15000, hb) + 2500)` = 7500, **and** that tab is visible or the claimer is hidden.
- Renewal runs every 5000. The lease is released on `pagehide`. A `storage` event triggers immediate supervision. A hidden leader republishes `visible:false`, so a visible tab takes over at once.
- **Non-leaders close all their peers.** They stay current through the BroadcastChannel.
- Leader-only operations: comment scans, mailbox consumption, offers, rendezvous.

### 10.11 Lifecycle

**`startLiveCollaboration`** (`live:3402-3429`, wrapped at `live:4410-4414`):

- Does nothing while `isDocumentLiveWorkPaused()` (closed to Home, or suspended in the background, `app:6030-6033`).
- Ensures the tab channel.
- Requires a token, edit rights and `RTCPeerConnection`.
- Installs the lifecycle listeners.
- Starts the leader timer, the 30 s capability timer and the supervisor.
- Opens a 3000 fast window, then runs: an immediate supervise and scan, a forced beacon, and a mailbox burst if a peer is expected.

**`stopLiveCollaboration`** (`live:3454-3522`):

- Delete the beacon and release the leader lease.
- Bump the generation and every run ID.
- Close all peers.
- Clear every map, cache and backlog.
- Close the BroadcastChannel and invalidate the key.

**Exit:** `flushLiveCollaborationBeforeExit(650)` tries for up to 650 ms to send the current snapshot when Drive work is pending (`live:3431-3452`, `app:16024-16027`).

---

## 11. BroadcastChannel tab sync (`live:304-347`)

- Name: `inhousenotes-live-v5:` + `simpleHash(fileId)`. Reopened when the document changes. BroadcastChannel never delivers a tab's own messages.
- On open, the tab posts `{type:'tab-state-request', fileId, tabId}`. Every other tab replies `{type:'tab-present', fileId, tabId}` and force-broadcasts its snapshot (if it can edit).

Message types:

| Message | Handling |
|---|---|
| `tab-present` | Sets `otherTabSeen` |
| `tab-state-request` | Reply plus snapshot |
| `drive-ink-hint` | Journal or preview hint |
| `drive-committed` | Throttled Drive poll |
| `live-stroke` / `live-erase` | As over WebRTC, transport `'tab'` (not relayed back to tabs) |
| `document-snapshot` (raw object, structured clone, no chunking) | `ihnHandleLiveEnvelope(…, 'tab')`. No ACK. Fanned out to WebRTC peers after apply. |

Any message from another tab sets `ihnLiveOtherTabSeen`. Pen frames are posted to the channel **only** once another tab exists (`live:1700-1702`).

The tab channel runs even for viewers. Snapshot **sending** requires edit rights (`live:3667-3668`).

---

## 12. Permissions and read-only viewers

| State | Source | Effect |
|---|---|---|
| `driveCanEdit` | `capabilities.canEdit` at open (`app:9097-9111`, `app:11172`, `app:11414-11499`) | `false` means a viewer: **no P2P at all** (`live:2823`, `live:3406`); no snapshot sending; no live-preview publishing (`inkapp:155`); remote snapshots, journals and the PDF are merged with `additiveById:false` and `preserveLocal:false`, so remote content is authoritative; remote final strokes go to IndexedDB only (`app:4082-4084`) |
| Revocation | Capability probe every 30 s (5 min when solo): `GET fields=capabilities(canEdit,canModifyContent),trashed` | `trashed`, `canEdit:false` or `canModifyContent:false` switches to view only (`driveCanEdit = false`, P2P stops, `setReadOnlyMode(true)`). A failed probe never downgrades (`live:3378-3400`). |
| UI Read mode `state.isReadOnly` | Toggle (`app:2117-2170`) | Editing UI disabled. Drive saves are suppressed unless forced. **The P2P session continues**, and local unsynced edits are preserved (`tests/live-collaboration-v5.test.mjs:2004`). |
| Public link without sign-in | `startPublicSharedPolling` (`app:8536-8678`) | Polling every 1800 (with an API key: metadata-only check, then fetch the PDF only if the head or version advanced) or 2500 (without a key: full PDF download plus a head and tail FNV fingerprint). The whole document is **re-imported** on change, with `strictCollabMetadata`. Read-only, no presence, no P2P. |
| Sharing UI | `fetchDrivePermissions`, `addDrivePermission`, `setDriveLinkPermission` (`anyone` with `allowFileDiscovery:false`), `updateDrivePermissionRole`, `revokeDrivePermission` (`app:9045-9095`) | Plain Drive permissions. There is no app-level ACL. |

**Trust model** (as implemented): P2P authenticity is "can decrypt and post Drive comments with the shared key". The key lives in public `properties`, readable by every principal who can read the file metadata.

A peer is never checked for edit rights. Received snapshots are merged additively and then uploaded by the receiving editor. v6 should bind P2P sessions to verified Drive identities and capabilities.

---

## 13. Background and visibility lifecycle (`app:6020-6100`, `app:16555-16585`)

| Event | Action |
|---|---|
| Tab hidden | Pause Drive polling and presence immediately (`pauseCollabPollingForBackground`). The P2P peers and the journal stay warm. `ihnPollSignals` and the beacon skip while hidden. |
| Hidden for **60000** | `suspendDocumentForBackground`: stop P2P (and delete the beacon), and rebuild the ink adapter without a document. Only the outbox keeps uploading. |
| Visible | Resume the ink session, `startCollabPolling` (presence plus an immediate poll), the catch-up burst, and the live wake (`app visible`). |
| Home or exit | `clearPresence` (keepalive). Attempt the P2P snapshot flush when a Drive upload is pending. `stopLiveCollaboration` via `stopCollabPolling`. |
| `beforeunload` | `clearPresence` plus `flushOnClose` (`app:16531-16536`) |

---

## 14. Interoperability requirements for v6 (summary)

To coexist with v5 devices, v6 MUST:

1. **Drive PDF:** read and write the Keywords grammar of §5.1 exactly. That covers the `STROKES_Z` deflate format, the page entry schema with `deletedStrokeStamps`, `IH_STRUCT` and `IH_FIELDS` as base64 of UTF-8 JSON, and `IH_SYNC`. It must upload conditionally with a v2 `If-Match` etag and merge the latest revision before every upload (§5.2, §7.4).
2. **Causal metadata:** emit stamps `{clock, actor}` with `clock = max(local+1, wallMs)`, compare stamps by `(clock, actor)`, and implement §3.3 to §3.5 and §4 with the exact tie-breaks. These are the stable-JSON comparisons, so they need JS-compatible number serialization. Keep legacy readers working: stampless items, `deletedStrokeIds`-only pages, pages without IDs.
3. **Field keys and values:** use exactly §3.5. The background register hash is `simpleHash(dataURL)`.
4. **Presence:** write `p_<hash>_q` and `p_<hash>_r` (§6.1) so v5 devices see v6 devices, switch to the 600 ms cadence and attempt P2P. Advertise the real heartbeat interval in slot 4.
5. **Optional P2P interop:** if v6 speaks v5's P2P, it must reproduce §10.1 to §10.8, including the data-channel labels, the framing, the snapshot envelope and its ACK messages, `ihnCanonicalDocumentHash` for acks and deltas, and the `live-stroke` packets. If it does not, v5 peers will time out (18 s or 30 s) and fall back to Drive. v6 must then **not** post `IHN_LIVE_V1:` comments it cannot answer.
6. **Same-account journal interop (optional):** read and write `ink-journal-v1` and `ink-preview-v1` files and the `ihn_ink_*` and `ihn_live_*` hints (§8, §9).
7. **Never** remove or rotate `ihn_live_key_v1` while v5 devices may be signalling. Never delete other devices' presence records younger than 10 min.

Safe divergences: a different `IH_SYNC.contentHash` domain only costs extra merges or uploads on v5. A missing `IH_FIELDS` or `IH_STRUCT` is seeded at Lamport zero by v5.

---

## 15. What limits instant sync today

### 15.1 Latency sources by path

Each row is a deliberate delay or cost.

**P2P (editors, any account)**

| # | Source | Value (current, adaptive) | Where | Why it exists |
|---|---|---|---|---|
| 1 | Live-stroke frame batching | 18 ms timer per stroke | `live:88`, `live:1828-1830` | Bounds packets per second and DataChannel churn during pointermove |
| 2 | The final stroke exists only at pen-up; the receiver commits it only when **its own** pen is idle and no structure lock is held | 24 ms polling until idle; queued during the remote user's interaction | `app:3979-4019` | Never mutate page arrays under an active local gesture |
| 3 | Full-snapshot anti-entropy after **every** content bump | 40/60 ms debounce, then deferred while there was user activity within **1200 ms** (re-checked every 180 ms) | `live:3667-3709` | Cloning, hashing and serializing the whole notebook is too heavy during writing; live packets cover the gap |
| 4 | Snapshot build loads **all pages** from IndexedDB, takes the structure lock (≤ 15 s) and clones cooperatively, yielding each 4 ms and waiting for 180 ms quiet windows | O(document) per edit | `live:3592-3665`, `app:22529-22552` | Correctness: the hash and ACK must describe the whole document; avoid UI jank |
| 5 | Snapshot transfer: 16000-character chunks with 64 KB backpressure, one bulk SCTP stream, at most 4 queued sends per peer | Proportional to document size (photos are data URLs) | `transport:4-52`, `live:3780-3822` | Mobile DataChannel message limits; fairness with the interactive stream |
| 6 | Apply: per-actor coalescing with a 36 ms gap; waits for the local gesture (32 ms polls); loads **all pages** (`captureContentHash`); full merge; IndexedDB checkpoint of every changed page and of the metadata **before** ACK | Tens to hundreds of ms on large documents | `live:4012-4055`, `live:4299-4326`, `app:29517-29522`, `app:30646-30685` | Durable ACK semantics; never a torn merge |
| 7 | Connection setup through Drive comments: discovery (presence poll 0.6 to 20 s, beacon refresh 1.5 to 60 s, comment scan 0.3 to 30 s), ICE gathering (≤ 650 ms), offer and answer comment round trips plus polls (`[0, 80, 170, 280, …]`), mailbox PATCH | Typically 1 to 5 s after both devices are present; up to tens of seconds alone or idle | §10.3, §10.4 | Static hosting with no signalling server. Drive comments are the only shared writable channel that is encrypted. |
| 8 | Only the **leader tab** holds peers; other tabs relay through BroadcastChannel | +1 hop | `live:2827-2834` | One set of sockets and Drive signalling per device |
| 9 | Route failure detection: watchdog at 2.5 s, RTT-sized probes, 2 misses; health timeout 30 s; ACK repair 4 s, 20 s and 120 s; reconnect backoff 1 to 30 s | Seconds | §10.9 | Avoid reconnect storms on busy WebViews |
| 10 | STUN only, no TURN | Some NAT pairs never connect | `live:102-107` | No server infrastructure |
| 11 | Background: P2P stops after 60 s hidden | Reconnect on return | `app:6070-6091` | Battery and network cost on mobile |
| 12 | Hash-domain mismatch (raw versus 0.01-rounded points) can make converged replicas advertise different hashes, which triggers 3 s rate-limited `state-request` repair exchanges and convergence uploads | Bandwidth and CPU churn | §3.6, `live:2168-2182`, `live:3922-3926` | Unintended: the PDF quantizes while live state does not |

**Drive journal (same account)**

| # | Source | Value (current, adaptive) | Where | Why it exists |
|---|---|---|---|---|
| 13 | The write side is per op at pen-up, but: IndexedDB commit first; write spacing ≥ 120 ms; cross-tab lease (IndexedDB plus Web Lock); **one upload at a time across all records**; first write needs `generateIds` plus a folder lookup and a multipart create; update needs a v2 PUT with `If-Match` (extra GET metadata and GET content when the etag is unknown); 8 s timeouts; failure backoff 0.6 to 30 s | ~1 to 3 Drive round trips per batch | `journal:236-289` | Durability before the network, single writer per part, no lost or duplicated files |
| 14 | Whole-part rewrite (cumulative ops, up to 256 KiB or 128 ops per part) | O(n²) bytes per part | `journal:78-85`, `journal:159-160` | Simple atomic file semantics without append |
| 15 | Read side: discovery via `files.list` every 5 s (subject to Drive **search index lag**); hot reads every 120 ms only for 6 s after activity; cold 1.8 s backing off to 30 s; 5 s after errors; 4 lanes | 0.12 to 30 s | `journal:290-397` | Bound Drive read quotas |
| 16 | Remote ops apply only after the local pen has been idle for **180 ms**, serialized | Visible lag while the reader writes | `inkapp:82`, `inkapp:112`, `app:31963-31967` | Never merge under an active local stroke |
| 17 | Every receiver re-uploads the PDF after applying journal ops | N × PDF uploads per edit | `inkapp:120-132` | A journal receipt is not a PDF receipt |
| 18 | Same-account only | Cross-account users get nothing from the journal | §8.10 | Files live in the writer's private folder |

**Drive live preview (same account)**

| # | Source | Value (current, adaptive) | Where | Why it exists |
|---|---|---|---|---|
| 19 | Writer: frames ≥ 100 ms apart, one in flight, whole-frame rewrite (≤ 24 paths, ≤ 2 MiB) | ≥ 100 ms plus PATCH RTT | `preview:10`, `preview:60-126` | Drive write quotas |
| 20 | Reader: 100 ms while active, 300 ms warm (120 s after a frame), 2 s cold; discovery every 5 s (pageSize 30); heartbeat wake only with a fresh (≤ 13.2 s) presence heartbeat | 100 ms to 2 s, plus RTT | `preview:178-260`, `inkapp:304-329` | Bound reads; wake a sleeping reader without P2P |

**Drive PDF (authoritative, all accounts)**

| # | Source | Value (current, adaptive) | Where | Why it exists |
|---|---|---|---|---|
| 21 | Save debounce: 40 ms idle; 180 ms after activity; **1200 ms pen-burst quiet**; max 3 s; aborted by new strokes | 0.04 to 3 s, and longer while writing continuously | `app:363-376`, `app:25364-25369`, `app:26362-26421` | A whole-PDF build per stroke is too expensive and would be aborted anyway |
| 22 | PDF build: pdf-lib or raster, metadata deflate, timeline archive encoding | Hundreds of ms to seconds | `app:34300-34460` | The PDF is both the export and the sync container |
| 23 | Pre-upload revision check (v2 GET etag), up to 3 merge-rebuild loops, conditional upload (media ≤ 4 MiB, else resumable 1 MiB chunks), 412 retries | 1 to 3 round trips plus upload | `app:26740-27039` | No lost updates without a server lock |
| 24 | Receivers: metadata poll **600 ms** (`shared`) / **3 s** (`linked`) / **20 s** (`solo`; 600 ms for 60 s after a remote revision); commit hints only over P2P or tabs | 0.6 to 20 s | `app:27239-27244` | Drive request quotas; battery |
| 25 | Pull = download the **entire PDF** (exact revision for owners, md5-verified head otherwise), parse and inflate metadata in a worker, check revision metadata three times (before, at apply, after), full merge, background re-render | Seconds for large documents | `app:28762-29185`, `app:26013-26049` | Revision-consistent snapshot; no torn reads |
| 26 | A pull is **deferred** while local changes are unsynced; it is merged only at the next own save | Remote Drive changes invisible until the local upload starts | `app:27568-27571`, `app:26874-26899` | Avoid overwriting pending local work |
| 27 | Hidden tabs skip polls | Catch-up on return | `app:27533` | Battery |
| 28 | Every merge with local contributions triggers another PDF upload (pull with `hasLocalMerges`, P2P convergence, journal receipt, received live stroke) | Upload amplification and conflict loops under concurrency (412, then merge, then rebuild) | `app:29158-29161`, `live:4362-4377`, `inkapp:130-131`, `app:4080-4081` | The PDF is the only cross-account durable channel |

**Presence and discovery**

| # | Source | Value (current, adaptive) | Where | Why it exists |
|---|---|---|---|---|
| 29 | Heartbeat 2 s / 6 s / 30 s; online window 13.2 to 61.2 s; sticky +3 polls; stale after 10 min | A newcomer may wait up to 20 s (solo metadata poll) to be seen; a departed device lingers | §6 | Drive write quotas; avoid flicker |

**Drive API in general**

| # | Source | Value (current, adaptive) | Where | Why it exists |
|---|---|---|---|---|
| 30 | `driveFetch` retries only on 401 (token refresh); no 429 or 5xx backoff except in preview (5 s on 429) and the journal record backoff | Rate-limit bursts surface as failures | `app:7799-7890` | Simplicity; callers own retry policy |
| 31 | Per-file property limits and the never-cleaned `ihn_ink_*` / `ihn_live_*` keys; key-prefix collision with `ihn_live_key_v1` (a failing GET every 3 s) | Wasted requests; scalability ceiling | §5.3 | Unintended |

### 15.2 Structural reasons (summary for v6 design)

- **No server:** Drive (comments, properties, files) is used as a mailbox, a database and a pub/sub system. Every hop is a polled HTTPS request with quota limits, so push latency becomes poll latency.
- **The document is the unit of sync.** P2P anti-entropy, Drive pulls and uploads all move whole documents or whole pages. Only live stroke packets and journal ops are deltas.
- **No operation log is shared across accounts.** The only cross-account durable channel is a PDF that must be rebuilt, uploaded, downloaded and parsed in full.
- **Interaction gating:** every merge waits for the local pen to be idle (180 ms or 1200 ms windows). This protects index-based editor state (page arrays, undo history) that is not itself CRDT-safe.
- **Multiple writers re-upload** the same converged content, which turns concurrency into 412 and merge-rebuild loops.

---

## 16. Observed hazards (verify in v6 parity tests)

1. The `ihn_live_key_v1` key matches the `ihn_live_` hint prefix, so the AES key is treated as a preview file ID (`inkapp:320-321`).
2. The journal and preview files are not shared, so they cannot reach other accounts (§8.10). They are never deleted, and a new actor (UUID) appears per session.
3. The signalling key is stored in public `properties`, readable by viewers. P2P peers are not checked for edit capability (§12).
4. Hash domains differ between P2P and `IH_SYNC` (§3.6). The live merge-upload guard compares the live hash with the Drive-confirmed hash (`live:4116-4153`), so the suppression rarely triggers after PDF round trips.
5. The PDF Title is always `Cuaderno Digital`. The reader seeds the export name from Title only when no register exists (§5.1).
6. Peer-ID mismatch when the profile has no email (§2.4).
7. A journal op for an unknown, non-tombstoned page **throws** and blocks that source, retrying every 5 s until the page shell arrives (`inkapp:91-92`).
8. A history prefix conflict ("Ink history conflict") makes a record retry forever with backoff (`journal:171-172`, `journal:266-271`).

---

## 17. Constants index (all values in v5.17.22)

| Constant | Value | Location |
|---|---|---|
| `IHN_LIVE_SIGNAL_TTL` (offer/answer/candidates/mailbox) | 90000 | `live:7` |
| Comment scan window | 2 × TTL = 180000 | `live:469` |
| `IHN_LIVE_RENDEZVOUS_TTL` / `_SOLO_TTL` | 45000 / 100000 | `live:8`, `live:50` |
| `IHN_LIVE_RENDEZVOUS_REFRESH_MS` / `_SEARCH_MS` / `_SOLO_REFRESH_MS` / `_RENEW_MARGIN_MS` | 15000 / 1500 / 60000 / 30000 | `live:9-10`, `live:49-52` |
| `IHN_LIVE_PEER_CACHE_TTL` / `_LIMIT` | 7 days / 12 | `live:11-12` |
| `IHN_LIVE_BULK_HIGH_WATER` | 64000 | `live:13` |
| Leader heartbeat / TTL / stale grace | 5000 / 15000 / 2500 | `live:20-22` |
| Supervisor (active / idle) | 250 / 2000 | `live:23`, `live:26` |
| Signal poll (base, join, present, present fed, solo fed) | 2500 / 300 / 3000 / 10000 / 30000 | `live:35-39` |
| Join and start fast windows | 10000 / 3000 | `live:40-41` |
| Wake scan minimum | 5000 | `live:44` |
| Property feed TTL | 45000 | `live:48` |
| Capability check (company / solo) | 30000 / 300000 | `live:53-54` |
| Connect timeout (normal / fast) | 18000 / 12000; absolute caps 30000 / 22000; extension per activity 8000 / 6000 | `live:55-56`, `live:1275-1287` |
| Disconnected grace | 5000 | `live:57` |
| Ping interval / route stale / probe misses | 500 / 2500 / 2 | `live:58-60` |
| Health timeout (v2 / legacy) | 30000 / 90000 | `live:61`, `live:65` |
| ACK / receipt / apply-ACK timeouts | 20000 / 4000 / 120000 | `live:62-64` |
| Peer expiry (normal / speculative) | 90000 / 20000 | `live:66`, `live:72` |
| Healthy reset | 15000 | `live:73` |
| Failed-signal retry / max | 5000 / 256 | `live:74-75` |
| Broadcast fast-retry limit | 4 | `live:76` |
| Apply coalesce gap | 36 | `live:77` |
| Resume grace | 10000 | `live:78` |
| Network probe minimum / recovery grace | 220 / 650 | `live:79-80` |
| ICE gather (normal / fast) / settle / batch / batch retries | 650 / 280 / 70 / 45 / 4 | `live:81-85` |
| Snapshot cache (entries / characters) | 3 / 16,000,000 | `live:86-87` |
| Stroke frame / max points per packet / total | 18 / 700 / 24000 | `live:88-90` |
| Erase frame / ids / strokes / points | 18 / 500 / 200 / 1200 | `live:91-94` |
| Realtime backlog TTL / limit | 12000 / 192 | `live:95-96` |
| Mailbox burst delays | [0, 70, 150, 260, 420, 650, 950, 1400, 2100, 3200, 5000] | `live:97` |
| Direct reply delays | [0, 80, 170, 280, 280, 300, 300, 350, 450, 600, 900, 1400, 2100, 3000, 4500, 6500] | `live:101` |
| Rapid supervise delays | [40, 120, 260, 480, 800] | `live:3104` |
| Wire message maximum | 2 MiB characters | `live:3831` |
| Framing chunk / max characters / max chunks / assemblies / per peer / TTL | 16000 / 50,000,000 / 5000 / 16 / 4 / 60000 | `transport:4-5`, `transport:55`, `transport:85` |
| Capacity wait timeout | 15000 | `transport:7` |
| Remote live preview TTL | 15000 | `app:3746` |
| Journal: part ops / part bytes / op maximum / body maximum | 128 / 256 KiB / 4 MiB / 8 MiB | `journal:12-13`, `journal:78` |
| Journal: write spacing / outbox recheck / request timeout | 120 / ≤ 2000 / 8000 | `journal:237`, `journal:279`, `journal:63` |
| Journal: hot window / hot read / cold read / idle backoff maximum / error retry / discovery | 6000 / 120 / 1800 / 30000 / 5000 / 5000 | `journal:105`, `journal:310`, `journal:324`, `journal:361`, `journal:368` |
| Journal: lanes / sources | 4 / 4096 | `journal:103`, `journal:387` |
| Journal: write backoff | min(30000, 600 × 2^min(6, f)) | `journal:267` |
| Lease TTL / renew | 45000 / 10000 | `offline-store-v5.js:206-218` |
| Capture timer / stroke chunk / image chunk / apply fold | 60 / 32 strokes / 4 images / 32 strokes or 8192 points | `inkapp:159`, `inkapp:234`, `inkapp:101-105` |
| Preview: frame / warm / warm-for / maximum bytes / read cache | 100 / 300 / 120000 / 2 MiB / 4 Mi characters | `preview:10` |
| Preview: paths / points / sources / lanes / discovery / stale | 24 / 24000 / 128 / 4 / 5000 (pageSize 30) / 20000 old or 30000 future | `preview:25-36`, `preview:137`, `preview:206` |
| Presence ping / stale | 120000 / 600000 | `app:27626-27627` |
| Heartbeat (shared / linked / solo) / misses (fast / relaxed) / maximum advertised / retry / sticky | 2000 / 6000 / 30000 / 6 / 2 / 120000 / 4000 / 3 | `app:27632-27639` |
| Save ring (active / done / minimum processing) | 6500 / 2200 / 900 | `app:27698-27700` |
| Collab poll (shared / linked / solo) / recent remote fast | 600 / 3000 / 20000 / 60000 | `app:27239-27242` |
| Save catch-up (solo / shared) | [1800] / [900, 1800, 3500] | `app:27243-27244` |
| Pull retry | min(15000, 700 × 2^(f-1)) × U(0.8, 1.2), f ≤ 6 | `app:27269-27283` |
| Drive timeouts (poll / meta / post-save) | 7000 / 7000 / 3500 | `app:371-373` |
| Remote meta skew / own-write echo window | 250 / 12000 | `app:374-375` |
| Save debounce (idle / max) / build idle window / doc save idle / interaction idle | 40 / 3000 / 180 / 180 / 100 | `app:352`, `app:363-370` |
| Pen burst window / quiet | 3000 / 1200 | `app:1966-1967` |
| Background suspend | 60000 | `app:6070` |
| Structure lock wait / collaborative idle wait | 15000 / 30000 | `app:20308`, `app:20347` |
| Public link poll (API key / no key) | 1800 / 2500 | `app:8678` |
| Commit hint throttle | 400 | `live:1633` |

---

## 18. Tests pinning this behaviour (one line each)

Unit tests run via `npm run test:unit` (`package.json:6`). End-to-end tests are listed where they pin collaboration behaviour.

### `tests/collaboration-core-v5.test.mjs` (deterministic CRDT core)

- `:33` Structure merge is commutative and idempotent for independent page moves.
- `:56` Same-page concurrent moves have a deterministic Lamport/actor winner.
- `:70` Concurrent page additions both survive with canonical ordering.
- `:86` Page deletion is durable and does not erase an unrelated concurrent add.
- `:102` Timeline structure replacement causally restores a previously deleted page.
- `:171` Concurrent last-page deletes recover one deterministic page.
- `:186` An unavailable historical fallback chooses a deterministic materializable page body.
- `:203` Structure merge stays associative when an all-deleted fallback meets a concurrent add.
- `:247` Canonical document hash ignores arrival order and derived panel HTML.
- `:298` Same-ID content conflicts use stamps, then a stable fingerprint tie-break.
- `:311` Legacy deletion IDs normalize to Lamport zero and keep newer causal stamps.
- `:333` Deletion-stamp merge is commutative and idempotent.
- `:355` A newer causal undo survives a remote deletion while an old item stays deleted.
- `:388` Reconnect backoff is deterministic, jittered and bounded.
- `:396` Normalized metadata keeps every referenced page exactly once.
- `:401` Field metadata merge is commutative and idempotent.
- `:419` Field and document hashes track semantic records but ignore the allocator clock.
- `:472` Independent document fields survive concurrent edits.
- `:511` Production page fields preserve independent edits and atomic compound values.
- `:550` Side-panel removal and update obey the same LWW rule.

### `tests/peer-transport.test.mjs` (v1 framing)

- `:6` Wire transfer round-trips unicode and reverse-order duplicate chunks.
- `:13` Receiver bounds aggregate reservations and expires idle streams.
- `:18` Fractional lengths and overflowing chunks cannot allocate or complete a transfer.
- `:22` Inconsistent duplicates and peer closure discard partial snapshots.
- `:27` Backpressure applies before the first frame and closure rejects immediately.
- `:32` A replaced document cannot send the next frame after draining.

### `tests/collab-cadence-v5.test.mjs` (adaptive Drive polling and presence)

- `:151` Alone and idle: metadata polls every 20 s and heartbeats every 30 s.
- `:167` Another online device switches to the fast cadence immediately.
- `:204` A P2P peer that connects before its presence is polled counts as present.
- `:215` A remote revision keeps polling fast for a minute, then relaxes.
- `:229` A save confirms once when alone and keeps the burst when shared.
- `:250` A heartbeat skipped during a save retries within 4 s instead of a full interval.
- `:265` Alone, save phase rings are not pushed to Drive; shared, they are.
- `:278` Stopping or pausing collaboration stops every poll and heartbeat.
- `:295` The online window follows the advertised interval and stays compatible (13.2 s, 61.2 s, cap 120 s).

### `tests/drive-ink-journal.test.mjs` (durable Drive journal)

- `:11` Direct IDs carry consecutive strokes during search-index lag and search failure.
- `:21` A late receipt cannot acknowledge a newer stroke, and restart drains durable edits.
- `:31` A lost create response retries the reserved identity instead of duplicating a journal.
- `:37` Rotation preserves a predecessor chain, and two writers never overwrite one another.
- `:48` Wrong document and failed merges cannot advance the applied revision.
- `:56` A stale writer cannot roll back an acknowledged newer file revision.
- `:68` An unavailable discovery request never delays an already known live file.
- `:77` A successor cannot seal a stale partial read of its predecessor.
- `:90` Slow discovery hints cannot hold the write lease or delay the next durable revision.
- `:101` An idle journal avoids storage churn but wakes immediately for new durable work.
- `:114` Failed advisory hints never turn a confirmed save into an upload failure.
- `:122` A stalled writer cannot block consecutive edits from a responsive writer.
- `:142` Journal reads have four global lanes and one request per source.
- `:161` Parallel responses serialize merges, and a stopped journal never applies a queued response.
- `:173` Cold uploads reserve their identity while folder lookup is still pending.
- `:190` Failed folder setup retains the write lease until identity reservation has settled.
- `:204` Durable and preview transports share a cold folder lookup only within the same account.
- `:217` A cancelled shared folder lookup can be retried by the surviving transport.
- `:232` A direct hint arriving during first discovery bypasses the stalled search index.
- `:246` Updating an existing journal does not require folder metadata.
- `:253` Preview creation invalidates a deleted folder cache and recovers on retry.
- `:268` A backing-off outbox avoids lease and status churn until its next useful check.
- `:283` A fresh persisted edit arriving during an empty outbox scan cannot be put back to sleep.
- `:296` Ordinary presence hints cannot bypass a failed source read backoff, but explicit retry can.
- `:314` An expired source backoff clears on successful read without clearing an unrelated write error.
- `:323` New samples wake the scan without bypassing a failed file write backoff.
- `:361` A sealed predecessor ignores an old refresh but reactivates for a genuinely newer required revision.
- `:389` Recovering another tab's journal does not suppress its current or future edits.
- `:401` Outbox recovery of another document does not add a wrong-document read source.
- `:408` A queued self-read retired before merge cannot reactivate the source.
- `:430` A successful slow upload immediately drains queued edits after releasing its lease.
- `:455` Success follow-up enforces the write rate limit, and failures never schedule a retry burst.
- `:476` Oversized photo payloads are rejected before serialization without mutating the durable queue.
- `:488` Photo preflight sums all page and embedded image payloads conservatively.
- `:498` Fitting photos retain identical wire bytes, and the exact serialization check catches escaped overflow.
- `:509` Size preflight does not reject omitted fields or custom JSON serialization.
- `:517` Size classification never masks a real local persistence failure.
- `:584` An idle journal recovers another tab's outbox within two seconds without a local wake.
- `:599` Receive hints preempt idle deadlines without resetting the outbox or postponing an earlier read for a write.
- `:622` An occupied write lease keeps the old 120 ms probe and recovers as soon as it releases.
- `:638` Active hints preserve failed read backoff, and passive retired hints do not wake an idle journal.
- `:662` A direct receive hint bypasses stalled bootstrap discovery, and stop prevents late rearming.
- `:679` Four blocked receive lanes keep bounded probes, and a queued source uses the next freed lane.
- `:704` An earlier receive wake preserves a queued upload deadline while its response is stalled.
- `:721` The scheduler rejects non-finite delays and bounds huge hints without replacing an earlier timer.

### `tests/drive-live-preview.test.mjs` (transient Drive frames)

- `:11` Unfinished strokes arrive as complete previews even after missed frames, without save receipts.
- `:20` One in-flight preview coalesces new samples and cannot acknowledge or lose the latest frame.
- `:29` Stale frames and another document cannot overwrite the live preview.
- `:37` A pen-gated receiver cannot block the next durable stroke upload.
- `:50` A prewarmed preview delivers before stroke completion with bounded scheduling delay.
- `:66` A slow discovery hint cannot freeze unfinished stroke updates.
- `:76` Final geometry replaces the preview prefix and does not retain a stale tail.
- `:85` Only the exact confirmed final stroke is retired from live payloads.
- `:97` A stalled preview source does not freeze successive frames from another device.
- `:119` Preview polling bounds concurrency and ignores late responses after leaving.
- `:136` Cold preview folder discovery and ID reservation begin concurrently.
- `:152` Existing preview updates need neither folder discovery nor another reserved ID.
- `:192` Stopping during cold preview setup awaits both branches without publishing or accepting a late ID.
- `:207` The first cold preview upload includes points that arrived during setup.
- `:241` Latest-frame serialization retains the byte bound and can recover with the reserved identity.
- `:257` Identical large live frames decode once across fifty reads without changing the path.
- `:272` Cached frames retain the original activity and freshness deadlines.
- `:291` Changed bytes at the same frame number still validate before final and cancelled paths arrive.
- `:314` The preview decode cache is bounded, evicts least-recently-read frames and cannot cross source identities.
- `:339` Failed preview callbacks and failed HTTP reads never become successful cache proofs.
- `:358` Stopping during a cached-source read releases memory and ignores its late response.
- `:373` Fresh preview activity preempts a sleeping timer, and later polling cannot postpone it.
- `:391` Passive discovery leaves existing deadlines alone and gives new sources a timely first probe.
- `:414` Active preview hints cannot bypass failed-read backoff, and success restores normal cadence.
- `:472` Stopping removes an activity wake, and later hints cannot restart it.
- `:514` New activity interrupts an idle deadline immediately, and successive frames keep their 100 ms cadence.
- `:534` Quiet failure retries and discovery retain their absolute deadlines despite hints.
- `:553` No finite deadline waits for a real hint instead of clamping an infinite timer into a busy loop.
- `:569` Four stalled read lanes retain bounded 100 ms scheduling, and a queued source starts when a lane returns.
- `:589` An in-flight discovery retains bounded polling while a direct hint delivers new ink immediately.
- `:607` A device that wrote recently stays warm: a new stroke after a pause is read within 300 ms.

### `tests/live-collaboration-v5.test.mjs` (P2P, signalling, snapshots, tabs)

- `:396` Opening prefetches signalling read-only and reuses each result once.
- `:413` Opening prefetch cannot reuse data across document sessions or create a key from an old empty result.
- `:427` Cold peer discovery reads comments while the signal key request is still pending.
- `:446` Interactive packets bypass a congested bulk channel, with a compatible fallback.
- `:460` The second stream is capability gated and cannot replace the document channel.
- `:480` Bulk backpressure times out without treating congestion as permission to send more.
- `:490` Reasserting an active collaboration session does not start another Drive poll per stroke.
- `:499` Incoming data during a route probe preserves the peer even without its exact pong.
- `:530` Connection info exposes a direct link only for an open remote peer channel.
- `:550` A visible tab immediately takes signalling leadership from a hidden or suspended tab.
- `:574` First contact gets the tolerant negotiation window, while broken established routes recover fast.
- `:584` An in-progress stroke is streamed in frames and finalized with a complete recovery packet.
- `:633` Live stroke preview frames serialize only the unsent point tail.
- `:662` Live stroke packets render once and relay without echoing to their source peer.
- `:693` Final stroke chunks commit once even when mesh paths deliver them out of order.
- `:742` An active eraser streams each removal frame and flushes the last change immediately.
- `:779` Live erase packets render once and relay without echoing to their source peer.
- `:809` A peer that opens during an edit receives the recent realtime stroke and erase backlog.
- `:831` Live snapshots hold, and always release, the page-structure lock.
- `:879` A stroke arriving during hydration aborts the stale snapshot and retries after its commit.
- `:902` Async page copies send real strokes, not Promise objects, to peers and local tabs.
- `:919` An edit during cooperative page cloning invalidates the snapshot before transmission.
- `:935` An immediate interactive ACK is not lost when the bulk-send promise completes.
- `:953` A receipt for a different merged state requests it and cannot become a delta base.
- `:965` Incomplete page packets are rejected without advancing the seen sequence.
- `:974` The merged content hash never assumes the advertised hash is the current document.
- `:985` A join request cannot narrow a queued edit to only the joining peer (both orders).
- `:1000` A healthy heartbeat repairs divergent documents without a new edit or reconnect.
- `:1017` A failed durable merge sends a prompt retry instead of leaving a two-minute receipt.
- `:1032` Delivery bookkeeping is per peer, so a new peer gets the current state without a new edit.
- `:1054` An acknowledged peer receives only changed pages and keeps a full snapshot fallback.
- `:1092` A page delta with the wrong base is rejected and requests a full snapshot.
- `:1127` A page delta with the confirmed base applies and advances to the advertised full hash.
- `:1164` Resuming verifies an apparently open peer with a sub-second probe.
- `:1184` A dead WebView route is replaced after two bounded checks on app resume.
- `:1208` A delayed first probe response cancels recovery without replacing a healthy peer.
- `:1232` Resuming a recently responsive peer keeps the route without a destructive network probe.
- `:1245` A network change keeps a responsive route and immediately resends the current state.
- `:1266` A dead route is replaced after two short network probes instead of one false-positive check.
- `:1291` A confirmed Wi-Fi to cellular switch replaces the route without a probe delay.
- `:1315` A network change event probes the route even when Android reports the same signature.
- `:1329` A slow route gets an RTT-sized recovery window instead of the old 700 ms cutoff.
- `:1355` WebRTC candidate-pair stats feed the adaptive RTT estimate.
- `:1377` An encrypted rendezvous discovers a peer before Drive presence arrives.
- `:1420` Discovery polls rapidly on join, remains attentive for a third device, and backs off on errors.
- `:1443` Scheduled discovery does not queue a duplicate behind a slow poll.
- `:1467` Discovery errors also rate-limit externally requested polls.
- `:1478` Concurrent duplicate answers apply the remote SDP only once.
- `:1500` The crossed-offer winner remains the initiator after receiving its answer until the channel opens.
- `:1513` The directed mailbox reuses file metadata and answers without listing comment history.
- `:1595` An initiator polls its exact offer comment and applies the answer directly.
- `:1653` Offer creation listens for the first ICE candidate before gathering starts.
- `:1727` A new remote rendezvous epoch wakes a stale route without reopening the document.
- `:1759` A previously verified peer is attempted immediately on the next document open.
- `:1771` Rendezvous refreshes one Drive comment and rotates it only for a new route.
- `:1815` An old rendezvous completion cannot clear the next document owner.
- `:1856` A newly discovered peer may offer immediately regardless of lexical peer order.
- `:1876` Simultaneous fast-recovery offers resolve glare deterministically.
- `:1905` The heartbeat watchdog verifies twice before replacing a silent route.
- `:1933` C to D to C sends C again instead of treating an old ACK as the current state.
- `:1966` A local merge is queued and sent on the first pass, not on the next edit.
- `:2004` UI Read mode preserves editable local changes, while a true viewer remains authoritative.
- `:2054` A failed apply does not consume the remote sequence.
- `:2071` Exact snapshots fan out between local tabs and WebRTC peers without echoing to their source.
- `:2122` An already-applied hash can still bridge a new actor's snapshot to a newly connected peer.
- `:2151` A historical applied hash is merged again after the document moves C to D to C.
- `:2204` The apply coalescer discards superseded snapshots while preserving other actors.
- `:2245` Stopping a document invalidates an in-flight apply before any post-merge side effect.
- `:2279` An in-flight snapshot build cannot publish into a later document session.
- `:2314` An old broadcast `finally` cannot clear the busy owner of a new document build.
- `:2361` A transient snapshot-lock failure preserves force and targets for a bounded retry.
- `:2391` Snapshot recovery continues at a low rate after the fast retries are exhausted.
- `:2416` The supervisor evicts a stuck connecting peer and schedules an autonomous retry.
- `:2441` The supervisor replaces an apparently open but silent zombie channel.
- `:2464` Missing receipts retry on the healthy route, but not while bytes are still draining.
- `:2487` Healthy pongs extend a slow snapshot apply, and hard expiry resends without reconnecting.
- `:2523` A healthy peer survives stale Drive presence and stays eligible for autonomous retry.
- `:2555` Either endpoint may initiate recovery after an established route closes.
- `:2579` Late ICE candidates wait for the answer, then apply once without duplicates.
- `:2626` A failed local ICE batch is retained, encrypted and retried.
- `:2684` Backoff resets only after a valid ACK, not on open or on a mismatched ACK.
- `:2720` Signal-key creation imports the Drive-authoritative value after a concurrent write.
- `:2751` A losing initiator adopts the final Drive key on its second offer.
- `:2803` A verified existing signalling key is reused without a Drive GET per message.
- `:2824` Parallel local key requests share one compare-after-write creation.
- `:2856` A failed initial key read never creates a replacement key.
- `:2872` A key GET completed after a document switch cannot create or cache a key for the stale document.
- `:2906` A key PATCH completed after a document switch cannot cache, or continue publishing, in the new document.
- `:2950` Signal polling follows every Drive page and accepts an offer after more than 300 recent comments.
- `:3032` A signal request arriving during a slow poll is drained immediately afterwards.
- `:3071` A transient answer failure leaves the same offer retryable.
- `:3124` A stale signal poll cannot clear the busy owner of the next document.
- `:3180` A decrypt mismatch invalidates the cached key, reloads it from Drive, and succeeds on one retry.
- `:3220` An undecryptable signal stops after the single refresh retry.
- `:3258` Merge-upload dedupe is scoped to the confirmed Drive baseline and the content generation.
- `:3284` Equivalent live convergence bumps the Drive PDF generation exactly once before saving.
- `:3311` One slow or broken handshake does not hide other devices or later comment pages.
- `:3342` Full snapshots wait across short pen lifts without delaying live stroke packets.
- `:3412` A lone editor scans comments rarely and rewrites its beacon only once a minute.
- `:3447` A short-lived search beacon is renewed before it expires once the cadence slows down.
- `:3468` A lone beacon stays valid across its slow refresh and inside the window every build accepts.
- `:3492` A newly present device switches signalling to fast immediately, and back to slow after it leaves.
- `:3539` A lone editor without a metadata feed still finds an offer within a few seconds.
- `:3552` Only a first sight or a new route of a rendezvous peer opens a fast window.
- `:3569` The capability check runs every 30 s with company and every 5 min alone.
- `:3589` The leader lease renews on a slow heartbeat and hands over on events.
- `:3616` Presence arriving after the document was closed does not restart its signalling.
- `:3635` Pen frames are not cloned into a tab channel until another tab of the document exists.
- `:3657` A newcomer tab learns of an existing tab from its presence reply.
- `:3667` A remembered device gets one warm-start offer, without fast scans or repeated offers while it is absent.
- `:3707` Evidence from a remembered device promotes it before its warm-start window closes.
- `:3723` An unchanged beacon left behind by a closed device does not keep it expected.
- `:3743` A network estimate change while alone neither rotates the beacon nor bursts reads.
- `:3780` Focus and visibility events alone cost at most one scan every few seconds.
- `:3807` Pen frames are serialized once for all peers, and not at all without an open peer.
- `:3838` The beacon is rewritten every 1.5 s only in the short window after a discovery.
- `:3852` Confirmed Drive writes notify peers and tabs; hints only request a throttled authoritative poll.
- `:3875` App resume and network events do not revive a stopped live session.
- `:3891` A document closed to Home or suspended in the background is not revived by starts, edits or tab requests.

### `tests/runtime-core-v5.test.mjs` (request primitives used by Drive transports)

- `:8` `fetchWithDeadline` aborts a stalled request and identifies a timeout.
- `:24` `fetchWithDeadline` forwards caller cancellation without relabeling it.
- `:43` `settleWithTimeout` returns the fallback on timeout and on rejection.
- `:48` `runWithRetries` recovers from transient failures without duplicating success.

### End-to-end (Playwright, `tests/e2e/`)

- `collaboration-chaos.spec.mjs:24` Three concurrent replicas converge for strokes, fields, page moves and deletes (within 5 s).
- `collaboration-chaos.spec.mjs:52` Network changes reconnect immediately, and a slow stale fallback cannot revert state.
- `live-handshake.spec.mjs:61` Real WebRTC connects new devices without edits, including a third account.
- `live-handshake.spec.mjs:86` A device that has been alone for a while still connects a newcomer within seconds.
- `live-handshake.spec.mjs:114` A remembered device reconnects promptly when the document is reopened.
- `live-handshake.spec.mjs:134` A second tab joins an idle document and reconciles without a new edit.
- `live-handshake.spec.mjs:146` A received final stroke invalidates the prepared document generation even without a safety snapshot.
- `live-handshake.spec.mjs:166` A missing snapshot is repaired on the existing WebRTC link without another edit.
- `live-handshake.spec.mjs:188` Real WebRTC reconciles different starting versions and deletions without a subsequent edit.
- `drive-ink-sync.spec.mjs:36` Two isolated devices exchange durable pen edits and erasures without compiling a PDF.
- `drive-ink-sync.spec.mjs:54` A stored ink outbox recovers after reload from Home without opening the PDF.
- `drive-ink-sync.spec.mjs:76` Finished strokes reach the small Drive journal while heavy document work is paused for the next contact.
- `drive-ink-sync.spec.mjs:91` Drive previews show a growing stroke before pen-up without P2P or durable merge waits.
- `drive-live-heartbeat.spec.mjs:61` A new matching presence heartbeat wakes an existing preview without P2P or a 15 second wait.
- `drive-live-heartbeat.spec.mjs:87` Repeated, stale, future and mismatched presence cannot repeatedly wake a sleeping preview.
- `drive-live-heartbeat.spec.mjs:119` A heartbeat for another document cannot consume activity or leak across a restarted session.
- `drive-ink-batching.spec.mjs:29` A backlog of 65 strokes uses three durable merges without changing its contents.
- `drive-ink-batching.spec.mjs:46` Repeated stroke revisions retain their ordering, and deletion tombstones still win.
- `drive-ink-batching.spec.mjs:61` An in-flight page capture cannot be presented as saved by an earlier stroke receipt.
- `drive-ink-batching.spec.mjs:82` Long paths split by point budget without simplifying or dropping coordinates.
- `drive-ink-cache-invalidation.spec.mjs:103` One incoming stroke preserves unrelated photo and PDF caches, and duplicate sources do no extra work.
- `drive-ink-cache-invalidation.spec.mjs:119` Moving one item invalidates the losing page even when that page is absent from the packet.
- `drive-ink-cache-invalidation.spec.mjs:167` An ordinary stroke in an unchanged calendar keeps unrelated page caches.
- `drive-ink-cache-invalidation.spec.mjs:172` Recovery after failed metadata invalidates every cache before acknowledging the source.
- `drive-ink-cache-invalidation.spec.mjs:187` A direct P2P merge returns its targeted cache proof and exact content hash without adapter invalidation.
- `drive-ink-cache-invalidation.spec.mjs:201` The adapter falls back safely for older or malformed proofs without repeating completed invalidation.
- `drive-ink-context-projection.spec.mjs:88` Many dirty pages and stroke chunks enumerate the full field map only once without changing serialized edits.
- `drive-ink-context-projection.spec.mjs:114` Document-only updates retain numeric and unknown field order without leaking any page fields.
- `drive-ink-context-projection.spec.mjs:129` A newer photo and document revision during persistence uses a fresh context and leaves the first encoded packet intact.
- `drive-ink-photo-delta.spec.mjs:33` Writing on an unchanged one-megabyte photo sends only the new stroke.
- `drive-ink-photo-delta.spec.mjs:53` Changed photos, deletion stamps and document metadata are never skipped.
- `drive-ink-photo-delta.spec.mjs:76` New pages and dirty-session recovery retain complete photo payloads.
- `drive-ink-photo-delta.spec.mjs:91` A failed photo capture does not install a baseline that would skip its retry.
- `drive-ink-photo-delta.spec.mjs:126` A same-value, newer photo register carries bytes that replace a divergent replica.
- `drive-ink-photo-delta.spec.mjs:144` Two photo updates deliver each register with its own page payload.
- `drive-ink-photo-delta.spec.mjs:169` A photo edited during an asynchronous capture is retried with a matching register.
- `drive-ink-photo-delta.spec.mjs:192` Two newly added photo pages are fully materialized on a replica that has only the original page.
- `drive-ink-photo-delta.spec.mjs:245` Edited stroke deltas update another replica and persist without replacing its photo.
- `drive-ink-photo-delta.spec.mjs:298` Capture-only new strokes retain the page shell needed by a receiver without that page.
- `drive-ink-photo-delta.spec.mjs:309` An edited stroke is immutable during persistence, and a newer revision is captured again.
- `drive-ink-replay-durability.spec.mjs:144` Identical ink replay cannot acknowledge a page whose IndexedDB writes still fail.
- `drive-ink-replay-durability.spec.mjs:157` Identical replay retries its failed metadata checkpoint before marking the source received.
- `drive-ink-replay-durability.spec.mjs:184` A failed remote checkpoint cannot leak into the next document session.
- `drive-ink-replay-durability.spec.mjs:225` A complete local checkpoint repairs failed remote metadata without another peer replay.
- `drive-ink-replay-durability.spec.mjs:356` A newer photo register with mismatched payload bytes cannot replace the matching local photo.
- `drive-ink-replay-durability.spec.mjs:367` A winning photo register without matching local or incoming bytes stays unacknowledged and consistent.
- `drive-ink-replay-durability.spec.mjs:391` Manual saving defers unregistered photo bytes after a partial merge until a stamped edit resolves them.
- `drive-ink-replay-durability.spec.mjs:424` Home keeps an unresolved photo checkpoint open, and a stamped repair can exit and reopen offline.
- `drive-ink-snapshot.spec.mjs:4` The immediate ink journal freezes the original stroke without another point-array clone.
- `collab-photo-metadata.spec.mjs:3` Repeated collaborative snapshots do not rehash unchanged photo bytes.
- `collab-photo-metadata.spec.mjs:21` Missing photo registers are seeded while existing remote revisions stay authoritative.
