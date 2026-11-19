# Inhouse Notes v6 spec — Local persistence, offline, save pipeline, service worker, boot, Android host

Status: exact behavioural description of **v5.17.22** (`index.html:93`, `service-worker.js:2`), written so that
v6.0.0 (Rust/WASM) can reproduce it and read every byte that v5 users already have on disk.

Conventions
- Citations are `file:line` (or `file:a-b`) in this checkout. `app` = `app-v5.js`, `store` = `offline-store-v5.js`,
  `sync` = `offline-sync-v5.js`, `lib` = `offline-library-v5.js`, `oapp` = `offline-app-v5.js`, `sw` = `service-worker.js`,
  `boot` = `boot-v5.js`, `rt` = `runtime-core-v5.js`, `builder` = `android app/html_to_apk_builder.py`.
- **MUST** marks an invariant that a historical bug proved necessary (see §4). "Quirk" marks behaviour that is
  odd but observable; v6 must either keep it or migrate around it explicitly.
- All `app-v5.js` top-level `let/const/function` are one shared global lexical scope; the offline modules
  (`oapp`, `drive-ink-*`) read editor globals such as `state`, `driveDirty`, `driveContentVersion` directly.
- `account` everywhere in the offline layer = `localStorage['drive-hint-email'].trim().toLowerCase()`
  (`lib:106-108`, `lib:610-611`).
- `\0` below means U+0000.

Cross-references (sibling files in `docs/v6/spec/` own these): PDF/metadata format (`STROKES_Z`, `IH_VERSIONS`,
`IH_CAL`, clean-original attachment, overlays, timeline archive) — `documents.md`; three-way/causal merge
(`pullRemoteChanges`, `collabStructure`, `collabFields`, stamps, tombstones), ink journal / live preview transport,
presence and revision polling — `collaboration.md` (only their *persisted records* are listed here); OAuth flows,
updater UI, release rules, build/deploy — `shell.md`; Home cards, offline badges UI, file open UX — `home.md`;
page records/structure operations — `pages.md`; ink capture — `editor.md`.

---------------------------------------------------------------------------------------------------

## 1. Storage inventory (migration contract)

v6 MUST open and read all of the following with the exact names/keys below. Nothing here is ever
schema-destructively migrated by v5; every upgrade is additive. v6 must not delete any of it on first run.

### 1.1 IndexedDB `notebook-data-db-v1` — the single-editor working cache

- Name: `notebook-data-db-v1`; **the hidden background processor uses `background-notebook-data-db-v1`**
  (`app:432`, prefix applied when URL has `?offline-worker`, `app:71`). Version **4** (`app:433`).
- Holds **only the document currently/last open in this tab's editor** (cleared and refilled on every open:
  `clearPageStore()` at `app:11800`, `app:24123`, `app:14136`, `app:21286`). Per-document durable copies live in §1.3.
- Open semantics (`app:22164-22255`): `onupgradeneeded` creates any missing store (additive):

| store | key | created as |
|---|---|---|
| `snapshots` | in-line `keyPath:'key'` | `app:22189-22191` |
| `pages` | **out-of-line numeric key = 0-based page index** | `app:22192-22194` |
| `stroke-ops` | `keyPath:'id', autoIncrement:true`, index `pageIndex` (non-unique; created on an existing store if missing) | `app:22195-22206` |
| `timeline-history` | in-line `keyPath:'key'` | `app:22207-22209` |

  `onblocked` → wait 1500 ms then resolve `null` (`INDEXED_DB_BLOCKED_TIMEOUT_MS`, `app:439`, `app:22237-22242`);
  open timeout 5000 ms → `null` (`app:440`, `app:22243-22246`); `null` means "continue with the localStorage backup".
  `onversionchange`/`onclose` drop the cached connection (`app:22220-22232`). A failed open is retried on next call
  (`app:22249-22253`).

#### 1.1.1 `snapshots` record (metadata checkpoint)
Key `'latest'` (`DB_KEY`, `app:435`). Written by `saveToIndexedDb` (`app:22257-22291`) and atomically by the
collaborative page-order rewrite (`app:23245-23250`):
```
{ key:'latest', data: MetaPayload, savedAt: <ms epoch>, version: 3 }
```
Readers accept `data` as an object **or a JSON string** (legacy) (`app:23997-24004`).

#### 1.1.2 `MetaPayload` (also the localStorage backup, §1.4 `notebook-data-v3`)
`buildMetaPayload` (`app:23747-23757`):

| field | type | notes |
|---|---|---|
| `sessionId` | integer | document session that owns the `pages`/`stroke-ops` rows (§2.2). Absent in older payloads. |
| `pages` | `PageMeta[]` | one per page, in document order |
| `version` | `3` | |
| `savedAt` | ms epoch | used to pick the newest of several candidate payloads at boot |
| `calendarPageConfig` | `{mode:string,startDateKey:string\|null,startPage:number,nextDateKey:string\|null}` \| null | only kept when the doc may use agenda calendar (`app:23753`, read back `app:24060-24067`) |
| `collabStructure` | object | collaboration page-structure register (collab spec) |
| `collabFields` | object | collaboration field registers (collab spec) |

Legacy payloads (localStorage `notebook-data-v2`, `app:431`) have `pages[i].strokes` arrays and optionally
`versionHistory`; any payload whose pages contain a `strokes` array is migrated via `migrateLegacyPayload`
(`app:24034-24036`). `versionHistory` from any payload is merged into the timeline store (`app:24080`).

`PageMeta` (`buildPageMeta`, `app:22563-22591`):

| field | value |
|---|---|
| `pageId` | string; generated `pg_<base36 ms>_<9 random base36>` (`app:743-745`); legacy deterministic `pg_legacy_<simpleHash(docId:index+1)>` when a Drive id is known (`app:747-751`, `app:22564-22568`) |
| `backgroundImage` | string data URL, or `null` when `backgroundSource==='pdf'` or the value is a `blob:` URL |
| `backgroundSource` | `'pdf'\|'custom'\|'template'` (derived: `pdfPageIndex` → pdf, `backgroundImage` → custom, else template) |
| `templateKind` | normalized template kind |
| `pdfPageIndex` | finite number or `null` (1-based PDF page locator; 0/absent = not a PDF page) |
| `pageWidth`,`pageHeight` | integers ≥ 64 (defaults by template kind, `app:2392-2395`) |
| `strokeCount` | visible stroke count (used to detect "page reports strokes but body missing") |
| `preview` | `image/jpeg` data URL quality 0.6 (`app:23673`) or `null` |
| `sidePanel` | calendar side-panel object or `null` (only when calendar panels render) |

`simpleHash(str)` (`app:27687-27693`): `h=0; for each UTF-16 unit h=((h<<5)-h+code)|0; return abs(h).toString(36).slice(0,8)`.
v6 must reproduce it bit-exactly (used in storage keys §1.1.5, §1.4).

#### 1.1.3 `pages` record (page body)
Key = page index (number). Value (`savePageToIndexedDb`, `app:22627-22734`):
`{ ...StoredPage, __sessionId: <documentSessionId at write> }` where `StoredPage` = `sanitizePageForStorage`
(`app:22413-22438`):

| field | notes |
|---|---|
| `pageId`, `backgroundImage`, `backgroundSource`, `templateKind`, `pdfPageIndex`, `pageWidth`, `pageHeight`, `sidePanel` | as PageMeta |
| `strokes` | stroke array (document-model objects) — points possibly packed, see below |
| `images` | image array |
| `deletedStrokeIds` | string[] |
| `deletedStrokeStamps` | normalized deletion stamps |
| `legacyCoverStrokes` | array or `null` |
| `__inkCheckpoint` | optional binary ink block (below) |

Read (`loadPageFromIndexedDb`, `app:22736-22764`): a row whose finite `__sessionId` ≠ current `documentSessionId`
**is treated as absent** (returns `null`); `__sessionId` is stripped (`app:22556-22561`); a decode error resolves `null`
and leaves the stored row untouched ("preserving stored checkpoint").

**Packed ink (`__inkCheckpoint`)** — written since 5.16.45 by `packInkPageForStorage` (`app:22443-22494`); also used
for offline snapshots (§1.3). Readers MUST accept both packed and plain pages (`app:22496-22527`):
- Only `strokes` and `legacyCoverStrokes` are packed. A stroke is "portable" iff every point is an object whose keys are
  a subset of `{x,y,p}`, `x`,`y` are numbers and `p` (if present) is a number (`app:22466-22469`). Non-portable strokes
  keep `points` as a plain structured-clone array.
- For a portable stroke, `points` is replaced by `{inkRange:[offset, count]}`; `offset` is an index into a
  `Float64Array` of 4 words per point: `x, y, p??0, ('p' in point ? 1 : 0)` (`app:22470-22474`). Strokes with zero
  points and no buffer at all get `points: []` (`app:22489-22492`).
- If `pointCount*32 > 64 MiB` the whole page uses the plain (unpacked) representation (`app:22450`).
- `__inkCheckpoint = { inkCheckpoint: 1, codec: 'xor-v1'|'raw-v1', bytes: Uint8Array, rawLength: <bytes of the
  Float64 buffer> }` (`document-processing-worker-v5.js:169-175`; main-thread fallback writes `raw-v1`,
  `app:22486-22488`).
  - `raw-v1`: `bytes` = the little-endian Float64 buffer verbatim; `bytes.length === rawLength`.
  - `xor-v1` (`rust-core/src/checkpoint.rs:3-18`): for word *i* (u64 LE), `delta = bits XOR previous[i%4]`,
    `previous[i%4]=bits`; emit one byte `L = ceil((64-lzcnt(delta))/8)` (0..8) followed by the low `L` bytes of
    `delta` (LE). Chosen only when shorter than raw.
  - Decoding validation (`rust-core-v5.js:341-366`, `app:22501`, `app:22514-22519`): `rawLength` safe integer,
    `0 ≤ rawLength ≤ 64 MiB`, `rawLength % 32 === 0`; `L ≤ 8`; no truncation and **no trailing bytes**;
    `inkRange` offset/count safe non-negative integers, `offset % 4 === 0`, `offset+count*4 ≤ words`; flag word must be
    exactly 0 or 1. Any violation = error (page treated as unreadable, row retained). Pure-JS decoder is the required
    fallback when WASM is unavailable (`rust-core-v5.js:353-366`).
  - Decoding reconstructs `{x,y}` and adds `p` only when the flag word is 1 (signed zero, NaN payloads and ±Inf are
    preserved bit-exactly: `checkpoint.rs:54-63`).

#### 1.1.4 `stroke-ops` record (operation journal)
`appendStrokeOpsToIndexedDb` (`app:22766-22813`), built by `enqueueStrokeOp` (`app:24576-24615`):
```
{ id:<autoinc>, pageIndex:<int>, type:'add-stroke', stroke:<cloned stroke + __pendingOpId>, savedAt:<ms>, sessionId:<int> }
```
`__pendingOpId = [session.toString(36), Date.now().toString(36), seq.toString(36), 5 random base36].join('-')`
(`app:24584-24589`). Persistence identity of a pending stroke: `op:<__pendingOpId>` else `id:<stroke.id>` else
`legacy:<strokeSyncFingerprint>` (`app:22835-22840`).
Load (`app:22815-22833`): rows whose finite `sessionId` ≠ current session are ignored; rows without a numeric
sessionId are accepted. Only `type==='add-stroke'` rows with finite `pageIndex` and a `stroke` are replayed
(`app:24759-24796`); if the replay references page indices beyond `state.pages`, blank A4 template pages are appended.

#### 1.1.5 `timeline-history` record
`saveTimelineToIndexedDb` (`app:22319-22346`):
```
{ key, historyArchiveJson: <string, timeline archive JSON>, savedAt:<ms>, schemaVersion: 3 }   // ARCHIVE_SCHEMA_VERSION, timeline-core-v5.js:4
```
`key` = `drive:<driveFileId>` if any Drive id is known (`state.driveFileId`, then `driveOpenFileId`, then
`window.pendingSharedDriveId`), else `local:<simpleHash(lowercase(trim(exportName||'untitled')) + ':' + firstPageId||'untitled')>`
(`app:22309-22317`). Readers also accept legacy rows with `historyArchive` (object) or `history` (array)
(`app:22391-22393`). Local history is merged with in-memory and incoming histories, never replaced
(`app:22380-22411`). Never pruned by key (rows accumulate per document).

### 1.2 IndexedDB `inhouse-original-pdfs-v1` (version 2) — clean originals + fast-open cache

`openOriginalPdfDb` (`app:451-518`) with the same blocked (1.5 s) / timeout (5 s) → "unavailable" semantics.

| store | key | value |
|---|---|---|
| `originals` | out-of-line, **Drive file id** (string) | `ArrayBuffer` of the clean original PDF bytes (`app:520-535`, keys chosen at `app:7201`, `app:32296`) |
| `drive-open-cache` | `keyPath:'fileId'`, index `cachedAt` | record below (`app:656-705`) |

`drive-open-cache` record:
```
{ fileId, account:<offline account at write>, blob: Blob(application/pdf),
  meta: { headRevisionId:string, version:string|null(normalized digits), modifiedTime:<ms number>, md5Checksum:string, size:number },
  analysis: { keywords:string, embeddedStrokes:null } | null,     // stroke arrays deliberately NOT stored (5.11.34)
  details: { canEdit, ownedByMe, ownerName, ownerEmail, ownerPhoto, resourceKey, parentIds[], folderKind, fileName },
  cachedAt:<ms> }
```
- `normalizeDriveOpenCacheMeta` accepts both `{headRevisionId,version,modifiedTime}` and
  `{driveHeadRevisionId,driveRevisionVersion,driveLastKnownModified}` shapes (`app:563-575`).
- Read returns the row only if `row.account === InhouseOfflineLibrary.account()` (`app:585`).
- Used only if `cache.meta.modifiedTime ≥ Date.parse(card.modifiedTime) > 0` (`app:593-603`).
- Trim (`app:640-654`, after every successful save): delete rows (except the open file and any id that is
  pinned or dirty in the offline library) whose `cachedAt < now − retentionDays·86400000` (retention from §1.3
  preferences, default 14).
- Every successful write also calls `InhouseOfflineLibrary.cache()` (`app:700-703`), i.e. mirrors into §1.3.
- `originals` is never trimmed (`deleteOriginalPdfBytes` exists, `app:553-561`, but has no caller).

### 1.3 IndexedDB `inhouse-offline-v1` (version 2) — per-account offline library, outbox, leases

`offline-store-v5.js`. Shared by the visible app, the hidden processor iframe, the service worker
(`importScripts`, `sw:4`) and the Android headless WebView.

- 7 stores, **all `keyPath:'key'` with a non-unique index `account` on field `account`** (`store:5`, `store:10-15`).
- **Record key = `${account}\0${id}`** (`store:33`). Every record written through `put/patch` gets
  `account`, `id`, `key` fields merged in (`store:52`).
- Upgrade (`store:9-21`): create any missing store; then backfill `document-index` from every `documents` row.
  Version 1 had the first six stores (inferred from the additive upgrade); v2 added `document-index`.
- `onblocked` rejects ("Offline storage is blocked by another window"); `onversionchange` closes (`store:22-24`).
- Every transaction touching `documents` automatically also includes `document-index` and keeps it in sync
  (`store:37`, `store:52`, `store:60`, `store:99`, `store:126`, `store:181`).

#### 1.3.1 `documents` (one per document per account; `id` = Drive file id or `local-<uuid>`)

| field | written by | meaning |
|---|---|---|
| `generation` | `checkpoint` (`store:92`) | +1 on every checkpoint; the outbox op carries the generation it must upload |
| `dirty` | checkpoint/acknowledge | `true` ⇒ local content not yet confirmed in Drive. Sticky: `dirty || old.dirty` (`store:97`); cleared only by a settled `acknowledge` |
| `snapshot` | capture (`oapp:82-84`) | editable native snapshot, or `null` once uploaded |
| `blob` | cache/acknowledge/upload-file | last known complete PDF (Drive-confirmed, downloaded, or a queued device PDF) |
| `originalBlob` | capture (`oapp:90`) | clean original PDF bytes (`state.cleanOriginalPdfBytes`) |
| `preparedBlob`, `preparedGeneration` | `prepared()` (`lib:282-289`) | PDF built from *exactly* `generation`; reset to `null` on every checkpoint (`store:95`) |
| `baseMeta` | capture/cache/acknowledge | Drive revision the local edit is based on: `{id, headRevisionId, version, modifiedTime(ISO\|null)}`. **Quirk:** `cache()` after a foreground save stores the `{driveHeadRevisionId, driveRevisionVersion, driveLastKnownModified, size}` shape (`app:27048-27069` → `lib:246`); readers must normalise both |
| `analysis` | cache | `{keywords, embeddedStrokes:null}` or null |
| `details` | capture/cache | `{fileName, parentIds, canEdit, ownedByMe, resourceKey, folderKind, ownerName, ownerEmail, ownerPhoto}` |
| `entry` | capture/cache | Drive-like entry copy (also merged into `entries`) |
| `contentVersion` | capture | editor `driveContentVersion` at capture |
| `session` | capture | `${offlineCheckpointLaunchId}:${documentSessionId}` (`app:24951-24953`; launch id = `crypto.randomUUID()` per JS realm, `app:1977`) |
| `offlineEdited` | capture/upload-file | edited while offline (forces foreground outbox path, `oapp:422`) |
| `lastOpenedAt` | checkpoint/openRecord/pin | retention clock |
| `pinned` | pin | exempt from retention |
| `updatedAt` | checkpoint | ms |
| `remoteId` | `reserve` (`sync:56-64`), acknowledge | Drive id reserved/confirmed for a `local-` document |
| `lastUploadMeta`, `lastUploadedAt` | acknowledge (`store:113`) | |

#### 1.3.2 `document-index`
`documentInfo(doc)` = the document **without** `blob, snapshot, originalBlob, preparedBlob, analysis`, plus
`available: !!(blob?.size || snapshot?.pages?.length)` (`store:29-32`). The Home UI reads only this store (`lib:112`).

#### 1.3.3 `outbox`
- Document op: `{account,id,key,generation,kind:'document',createdAt}` (`store:100`). **A new checkpoint replaces the
  row** (fresh `generation`, no `attemptedAt`/`attempts` ⇒ immediately due). Quirk: `createdAt: old.createdAt||Date.now()`
  reads the *documents* row, which never has `createdAt`, so it is always "now".
- Folder op: `{account,id,key,kind:'folder',generation:1,createdAt}` (`store:140`).
- Attempt bookkeeping (patched by the runner, `sync:34-40`, `sync:255`): `attemptedAt`, `attempts`,
  `lastError` (HTTP status number or code string, default `'network'`), `lastErrorMessage`, `lastErrorPhase`.
- Deleted by a settled `acknowledge` (`store:124`) or `finishFolder` (`store:167`).

#### 1.3.4 `entries` (Drive catalogue cache, one per file/folder id incl. `local-` ids)
Whatever Drive fields were observed (`id,name,mimeType,parents,modifiedTime,createdTime,thumbnailLink,iconLink,
starred,ownedByMe,resourceKey,folderColorRgb`, see `lib:333`, `app:10750-10814`) plus:
`pending` (local or not yet uploaded), `remoteId` (Drive id of a `local-` entry), `trashed` (set to `true` when a
**complete** enumeration no longer lists it, `lib:342-344`), `localPreview` (first-page JPEG data URL from capture,
`oapp:88`; cleared when Drive reports a newer `modifiedTime` for a non-pending entry, `store:152-153`).
Parents are stored with the Drive root id replaced by `'root'` (`lib:338`). `cacheEntries` merges field-by-field
(`store:144-157`). Local folder entry: `{id:'local-<uuid>',name,mimeType:'application/vnd.google-apps.folder',
parents:[parentId||'root'],pending:true,ownedByMe:true}` (`store:136-137`).

#### 1.3.5 `settings` (key/value per account; `account` may be the pseudo-account `__session__`)

| account / id | value | writer → reader |
|---|---|---|
| `__session__` / `active` | `{accountId, enabled:true}` on sign-in/startup; `{enabled:false}` on sign-out | `lib:595`, `oapp:218`, `app:6898` → SW (`sw:21`, `sw:57`, `sw:61`), worker guard (`oapp:356`) |
| `<acct>` / `preferences` | `{retentionDays:int 1..3650}` (default 14) | `store:80-86` |
| `<acct>` / `drive-root` | `{remoteId:<Drive root id>}` | `lib:329` |
| `<acct>` / `home-views` | `{recents?:id[], starred?:id[], shared?:id[]}` (last Drive order per Home row) | `lib:239` |
| `<acct>` / `background-auth` | `{token, expiry(ms), refreshToken|null, clientId|null}`; `{}` after sign-out | `oapp:219-220`, `app:6900` → SW refresh (`sw:63-71`) |
| `<acct>` / `sync-priority` | `{documentId, token:uuid, expires:ms}` (manual "Save now" priority, 60 s, renewed every 30 s) | `oapp:459-475` → `sync:23-29`, `lib:446-448` |
| `<acct>` / `ink-journal-folder` | `{fileId}` (deleted via patch→null on 404) | `drive-ink-journal-v5.js:128-140,269` |
| `<acct>` / `ink-journal-outbox` | `{pending:{[recordKey]: rev}}` | `drive-ink-journal-v5.js:92-95,204,250-259` |
| `<acct>` / `ink-journal:<docId>:<actor>:<part>` | `{kind:'ink-journal-v1',docId,actor,part,ops:[json strings],bytes,rev,ack,version,previousKey,createdAt,fileId,exists,etag,retryAt,failures}` | `drive-ink-journal-v5.js:81-96,146,200,267-269` |

`patchAndQueue` (`store:67-79`) commits an ink record and its `ink-journal-outbox` pointer in one transaction
(`pending[id] = max(old, revision)`).

#### 1.3.6 `leases`

| id | value | semantics |
|---|---|---|
| `sync` | `{owner:uuid, expires:ms, at:ms, lock:bool}` | upload ownership, TTL 45 s, renewed every 10 s; plus a Web Lock named `inhouse-lease\0<acct>\0sync\0<owner>` held for the lease lifetime; a lease whose lock is no longer held (owner page/worker died) is treated as expired immediately (`store:186-225`) |
| `editor` | `{documentId, expires}` (`expires = 0` when the editor is hidden) | the visible editor's open document, 5 min, renewed at half-life only while visible (`oapp:228-241`) |
| `ink-journal-write` | same shape as `sync` (no Web Lock; only `sync` is a "live" lease, `store:191`) | ink journal writer |

`withLease` returns `{busy:true}` when another live owner holds it; on finish it deletes the row if still owned
(`store:205-225`).

#### 1.3.7 `recoveries` (never pruned by v5)

| id | value |
|---|---|
| `last-upload-draft:<docId>` | `{fileId, generation, snapshot, originalBlob, previousBlob, uploadedMeta, createdAt}` — the last editable draft before a settled acknowledge replaced it with the PDF (`store:117-122`); one per document, overwritten |
| `<opId>:<generation>:<remote version or headRevisionId>` | `{fileId, name, localBlob, remoteBlob, snapshot, createdAt, remoteCopyId?, remoteCopySaved?}` — both full PDFs captured before any concurrent merge (`sync:202-204`, `sync:93-115`) |

The settings dialog lists recoveries newest first and offers "This device"/"Other device" downloads
(`lib:559-567`).

#### 1.3.8 Retention (`prune`, `store:171-185`)
Deletes a `documents` row (and its index row) iff `!pinned && !dirty && id ∉ protectedIds && lastOpenedAt < now −
retentionDays·86400000` **and** no outbox row exists for it. Runs 2 s after library init and after saving retention
settings, protecting the open document (`lib:569`, `lib:595`). Never touches `entries`, `recoveries`, `settings`.

### 1.4 localStorage keys

| key | value format | written | read | removed |
|---|---|---|---|---|
| `inhouseNotesAppMode` | `'1'` | `?inhouse_app=1\|true` or UA `InhouseNotesApp/<n>` (`app:100-121`) | app-mode detection | `?inhouse_app=0\|false` |
| `inhouse-theme` | `'dark'\|'light'` (absent = follow system) | theme toggle (`app:5869-5881`) | `boot:15`, `app:5893-5920` | — |
| `notebook-view-v1` (`background-notebook-view-v1` in worker) | JSON `{panX:number, panY:number, zoom:number}` (all numbers required) | 250 ms after gesture idle and on close (`app:21675-21693`, `app:26277`) | `app:21695-21708`; zoom clamped to [0.5,3] (`app:22105`) | — |
| `notebook-view-mode-v1` | `'drive'\|'editor'` | `app:6136`, `app:6253` | `app:2310` (value read but unused: cold boot always opens Home) | — |
| `notebook-data-v3` (`background-…` in worker) | JSON `MetaPayload` (§1.1.2) | throttled backup (`app:23685-23745`) | boot (`app:23974`) | — |
| `notebook-data-v2` | JSON legacy payload with strokes | never (legacy) | boot fallback when v3 absent (`app:23982-23994`) | never |
| `drive-token-v1` | JSON `{token, expiry(ms), scopeVersion:'drive-calendar-v2', grantedScopes:string}` | `app:21727-21737` | `app:21758-21786`, `boot:49-62`, `app:2236` | invalid scope/expired (≤60 s left) → removed; sign-out |
| `drive-remember-v1` | `'1'` | any token save / sign-in start | boot destination, initial listing (`app:6329`) | sign-out |
| `drive-session-v1` (`background-…`) | JSON DriveSession (below) | `saveDriveSession` (`app:21788-21814`) | `loadDriveSession` (`app:22049-22101`) | sign-out |
| `drive-pending-exit-upload-v1` (`background-…`) | JSON `{fileId, fileName, contentVersion:int≥1, savedAt:ms}` | `app:21833-21847` | `app:21824-21831` | `app:21849-21855` (only if fileId matches or no fileId given) |
| `drive-refresh-token-v1` | raw refresh token (Android PKCE) | `app:6490` | `app:6440`, boot, worker | refresh 400/401 (`app:6460`), sign-out |
| `drive-pkce-verifier-v1` | PKCE verifier | `app:6372` | `app:6409` | after exchange / sign-out |
| `google-scope-version-v1` | `'drive-calendar-v2'` | token save | token validity gates | sign-out; calendar 403 (`app:19348`) |
| `google-granted-scopes-v1` | space-separated scopes | token save | token validity (needs `calendar.events`) | sign-out |
| `drive-consent-granted` | `'1'` | after consent | prompt selection | sign-out |
| `drive-hint-email` | email as returned by Drive (original case) | `app:7056` | **offline account id** (lowercased), login hint, boot destination | sign-out |
| `drive-profile-v1` | JSON `{name,email,photo}` | `app:7057` | `app:6993-6998` (used only if email matches hint) | sign-out |
| `presence-client-id-v1` | `c<8 base36>` (mirrored in sessionStorage) | `app:27713-27735` | presence / live peer id | — |
| `touchDrawEnabled` | `'true'\|'false'` | `app:15972-15980` | same | — |
| `ihn-live-leader-v1:<driveFileId>` | JSON `{tabId, expiresAt, heartbeatAt, hb:5000, visible}` | `live-collaboration-v5.js:268-286` | leader election | released on close |
| `ihn-live-peers-v1:<simpleHash(fileId)>` | JSON `{v:1, peers:[{id, at}]}` ≤12, TTL 7 d | `live-collaboration-v5.js:372-380` | warm start | — |
| `inhouse-services-updater:notes` (+`:installation`, `:installation-seen`) | updater state `{channel:'A'\|'B', developmentConsent:0\|1, snooze:{}}` etc. (native state authoritative in APK) | `shared/inhouse-services-updater/updater.js:51-77,407-424` | updater | — |
| `theme` (scanner page only) | `'dark'\|'light'` | `scanner/script.js:997` | `scanner/script.js:1509` | — |

DriveSession JSON (`app:21790-21809`): `{fileId, offlineLocalId|null, fileName, folderId|null, parentIds[], folderKind|null,
exportName, autosave:bool, canEdit:bool, ownedByMe:bool, ownerName, ownerEmail, ownerPhoto, resourceKey,
lastDriveSaveAt|null, headRevisionId|null, revisionVersion|null, lastKnownModified|null(ms)}`.
Load defaults (`app:22049-22101`): `autosave` missing → `!!fileId`; `canEdit` missing → `autosave` if boolean else
`true`; `ownedByMe` missing → `canEdit`.

### 1.5 sessionStorage keys

| key | value | use |
|---|---|---|
| `ihn-release-reload` | published version string | one reload per release per tab (`boot:137-152`) |
| `ihn-network-offline` | `'1'\|'0'` | sticky "probe said offline" across reloads in the tab (`lib:58`, `lib:63`) |
| `ihn-home-tab` | Home tab name | restore selected Home tab (`app:10736`, `app:16235-16236`) |
| `presence-client-id-v1` | as localStorage | fallback/mirror (`app:27721-27733`) |

The live-collaboration tab id is deliberately **not** stored (per-realm, `live-collaboration-v5.js:216-226`).

### 1.6 Cache Storage

| cache name | keys | values | owner |
|---|---|---|---|
| `inhouse-shell-<VERSION>` (e.g. `inhouse-shell-5.17.22`) | origin-relative paths from `INHOUSE_OFFLINE_ASSETS` (`/index.html`, `/app-v5.js`, …) — **no query string** | network responses | SW install/fetch (`sw:3-16`, `sw:34-54`); boot deletes `/index.html` and `/scanner/index.html` from older shells on release reload (`boot:157-164`) |
| `inhouse-media-<encodeURIComponent(account)>` | `${origin}/__offline-media__/<encodeURIComponent(id)>` and the same URL + `?revision` | image response (may be opaque) / `Response(<revision string>)` | `offline-media-v5.js:5-35`; ids: `preview:<fileId>` with revision = entry `modifiedTime` (`app:9288`), `avatar` with revision = photo URL (`app:7006`) |

Other platform storage: `navigator.storage.persist()` is requested when the user pins a document (`lib:312`).
Web Locks names: `inhouse-lease\0<acct>\0sync\0<owner>` (`store:193`).

### 1.7 v6 migration checklist (all MUST)
1. Read `inhouse-offline-v1` exactly (key format, `account` index, all 7 stores, both `baseMeta` shapes, packed and
   plain snapshots, both outbox kinds, ink-journal settings rows). Keep dirty documents, outbox rows and recoveries.
2. Read `notebook-data-db-v1` + `notebook-data-v3`/`-v2` with the "newest `savedAt` wins" rule and **adopt
   `MetaPayload.sessionId` (or, if missing, page 0's `__sessionId` when its `pageId` matches) before reading
   `pages`/`stroke-ops`** (`app:24017-24033`); otherwise persisted ink is ignored as foreign (5.16.42).
3. Replay `stroke-ops` on top of page bodies (dedupe by stroke id with concurrent-item choice, else fingerprint;
   `app:22963-22990`).
4. Read `inhouse-original-pdfs-v1` and timeline rows (`drive:`/`local:` keys, three historical shapes).
5. Read every localStorage key above (tokens, session, pending-exit marker) — a pending-exit marker found at boot
   is outstanding upload work.
6. Keep Cache Storage names/keys if the v6 SW wants to reuse media; the shell cache can be replaced by version.
7. Never delete `background-*` keys/DB: the hidden processor may hold an in-progress copy.

---------------------------------------------------------------------------------------------------

## 2. Save pipeline

### 2.1 Layers (innermost first)

```
 pen up / edit
   │  state.pages[i] (memory)  ── markPageDirty / enqueueStrokeOp ──► driveContentVersion++  (app:24807-24831, 25621-25629)
   ▼
 L1 stroke-ops row (per finished stroke, after input idle)          app:24576-24757
 L2 pages row (full page body, "compaction")                         app:23759-23840, 22627-22734
 L3 snapshots/latest metadata (only after L1+L2 are durable)          app:23891-23965
 L3b localStorage notebook-data-v3 backup (throttled ≥3 s, idle)      app:23713-23745
 L4 inhouse-offline-v1 documents+outbox (whole-document snapshot)     lib:250-281, store:87-105
 L5 Drive PDF (conditional upload, receipt)                           app:26802-27226 / sync:20-261
 (side) ink journal: per-stroke causal ops to Drive JSON files        drive-ink-*.js — never marks the PDF saved
```

### 2.2 Document session & persistence context
- `documentSessionId` (int, starts at 1 per JS realm, `app:1976`) is incremented by `beginDocumentSession`
  (`app:24976-25106`) on every open/new/restore. It aborts every in-flight page/op/meta IndexedDB transaction of the
  previous session (`app:25023-25031`), aborts the Drive save controller, clears all dirty/queue state, and resets
  local-save error/retry state.
- At boot `documentSessionId` is set to the persisted `MetaPayload.sessionId` (§1.7 item 2).
- `captureDocumentPersistenceContext()` = `{sessionToken, fileId:String(state.driveFileId||'')}`; every async step
  rechecks `isDocumentPersistenceContextCurrent` (`app:24963-24974`). **Invariant: a completed operation of another
  document/session can never mark the current one saved, retried, or dirty** (DOCUMENT_PIPELINE.md:28,
  5.16.6 "late exit uploads cannot mark the next document dirty").
- Offline session token = `${launchId}:${documentSessionId}` (`app:24951-24953`) so prepared PDFs and
  acknowledgements of a previous launch never match a new editor session (5.16.42).
- `documentSessionActivated` (`app:1978`) is false after boot until a document is explicitly opened.

### 2.3 Dirty tracking (state variables)

| variable | decl | meaning |
|---|---|---|
| `pageDirty:Set<int>`, `pageDirtyMode:Map<int,'stroke'\|'full'>` | `app:721-722` | page body needs L2 write; mode `full` dominates |
| `pageDirtyGeneration:Map<int,int>`, `pageMutationGeneration:int` | `app:723-724` | bumped by `invalidatePageMutationCaches` (`app:24798-24805`); a snapshot whose generation changed is stale |
| `pendingStrokeOps:Map<int,stroke[]>` | `app:726` | strokes appended since the last L2 write of that page (replayed on load) |
| `strokeOpsQueue:op[]` | `app:2014` | ops not yet in L1 |
| `saveQueued`, `saveInProgress`, `localSaveError`, `localSaveErrorSource`, `lastLocalSaveAt`, retry timer/attempts | `app:24853-24861` | local checkpoint lane |
| `driveContentVersion` / `driveUploadedContentVersion` | `app:1906-1907` | **the authoritative "Drive is behind" signal: `driveContentVersion !== driveUploadedContentVersion` ⇒ pending** (5.5.0) |
| `driveDirty`, `deferredDriveSave`, `driveSaveQueued`, `driveSaveInProgress`, `driveSavePhase`, `driveLastSaveError`, `driveSaveBlockedReason`, `driveUploadProgress` | `app:1883-1892`, `app:2005` | Drive lane |
| `preparedDrivePdf {blob,version,sessionToken,contentHash}`, `preparedDrivePdfBuild` | `app:1918-1919` | shared PDF build |
| `remoteMergeCheckpointPending` | `app:2023` | a remote merge whose local checkpoint has not completed (blocks "saved") |

Mutation rules:
- `markPageDirty(i, mode)` (`app:24807-24831`): `mode!=='stroke'` ⇒ `bumpDriveContentVersion()`; sets dirty+mode,
  invalidates caches, notifies ink adapter; schedules Drive sync unless interacting (then sets `driveDirty`,
  `deferredDriveSave` and arms a deferred retry).
- `enqueueStrokeOp` bumps the content version itself (unless `skipDriveVersion`) and schedules a microtask flush
  unless the pen is writing (`app:24576-24615`). A finished local stroke calls `enqueueStrokeOp` then
  `scheduleSave(false, page, 'stroke')` (`app:4701-4708`); a received remote final stroke calls `enqueueStrokeOp` +
  `markPageDirty(page,'stroke')` (`app:4080-4081`).
- `bumpDriveContentVersion` (`app:25621-25629`): notify ink, schedule offline capture after the pen burst, clear a
  Drive block, `driveContentVersion++`, invalidate the prepared PDF and schedule a new preparation, live broadcast.
- `markDriveContentUploaded(v)` sets `driveUploadedContentVersion = min(driveContentVersion, v)` (`app:25631-25636`).
- Pending predicates: `hasPendingLocalSave` (`app:25454-25466`), `hasPendingDriveSave` (`app:25468-25476`),
  `hasUnsyncedLocalDriveChanges` (`app:25478-25492`), `isDriveUploadFullySettled` (`app:21857-21863`).

### 2.4 Input gating (when background work may run)
- `DOCUMENT_SAVE_IDLE_MS = 180` (`app:363`); `DRIVE_BUILD_IDLE_WINDOW_MS = 180` (`app:369`);
  `INTERACTION_IDLE_MS = 100` (`app:352`); pen burst: strokes started < 3000 ms apart form a burst that needs
  1200 ms of quiet before whole-document work (`app:1966-1967`, `app:25364-25369`); 0 when leaving
  (`forceFlushingForExit`) or hidden.
- `waitForHotInteractionToSettle` = `DocumentWorkQueue.wait` (`app:31963-31967`, `rt:206-299`): resolves `true`
  once `!busy && now−lastActivity ≥ quietMs` (or immediately if `bypass` = exiting or hidden page), `false` if the
  session changed or the signal aborted; Rust planner with identical JS fallback.
- The hidden processor iframe consults its parent's interaction state (`app:31903-31905`, `app:31949-31956`).
- Accepted pen contact on a canvas: `interruptDocumentBackgroundWork()` aborts PDF workers, offline fetches,
  prepared build and **the active Drive save controller** (keeping `driveDirty`/`deferredDriveSave`), and pauses
  Android WorkManager once per burst (`app:4442-4445`, `app:25152-25162`, `lib:373-383`). Incidental pointer
  activity elsewhere never aborts an in-flight upload (`app:25141-25150`; smoke `release-v5-smoke.mjs:1259-1260`).
- While `forceFlushingForExit` no interaction can pause/abort anything (`app:25133-25142`).

### 2.5 L1 — stroke-op journal
- `flushStrokeOpsQueue` (`app:24715-24757`): wait input idle (180 ms); refuse (deferred) while another structure owner
  holds the numeric page mapping; coalesce callers onto one in-flight flush.
- `runStrokeOpsQueueFlush` (`app:24635-24713`): batches of 40 (`app:362`), one transaction each; success ⇒
  `confirmLocalSave(savedAt,'operations')`; failure ⇒ note error source `operations`, mark those pages `full` dirty,
  put the batch back at the queue head, retry after 250 ms.
- Pen writing: the flush runs once at pen idle (`finishPenWritingIdle`, `app:25317-25339`), which also forces a
  `full` page rewrite for any page with ≥200 pending ops (`STROKE_OP_COMPACT_THRESHOLD`, `app:361`).

### 2.6 L2/L3 — local checkpoint (`queueSave` → `runLocalCheckpoint` → `saveToStorage`)
- `queueSave` uses a `CoalescingQueue` keyed by session token (`app:26190-26195`; `rt:93-127`): a request made while a
  checkpoint runs joins the *next* batch; waiters never resolve with the older snapshot.
- `saveToStorage(controller)` (`app:23891-23965`), strictly in this order:
  1. flush L1; abort ⇒ `cancelled`; refusal ⇒ `deferred`.
  2. if legacy migration pages are pending, checkpoint them (`app:23004-23053`).
  3. `flushDirtyPages` (`app:23759-23840`): for each dirty loaded page, skip `stroke`-mode pages whose ops are already in
     L1 (unless `force`); write the body (§1.1.3) with snapshot-generation checks before and after commit; on success
     delete exactly the L1 rows captured before the write (`capturedKeys`), keep newer ones; a page that changed
     during its write stays dirty (`deferred`).
  4. refuse if any page is still dirty or a remote photo checkpoint is inconsistent (`app:21865-21878`) ⇒ `deferred`.
  5. write `snapshots/latest` (L3); then schedule the localStorage backup (immediate if L3 failed). `ok = L3 || backup`.
  6. if `pageMutationGeneration` changed during the commit ⇒ `{ok:false, deferred:true}` (never "saved").
  7. success ⇒ `confirmLocalSave`, `state.lastSavedAt`; if a remote merge checkpoint was pending and is now repaired,
     bump the content version and schedule Drive + offline capture (`app:23945-23958`).
- `runLocalCheckpoint` result handling (`app:26197-26266`): `deferred` ⇒ retry in 400 ms, no error shown;
  `cancelled` ⇒ `saveQueued`; failure ⇒ "Save failed" + bounded retry `min(30 s, 800·2^n)`, n capped at 6
  (`app:24897-24914`), never giving up (5.16.50). Read-only mode still checkpoints edits made before
  (`app:26199-26200`).
- `confirmLocalSave(_, 'operations')` cannot clear a `checkpoint` failure (`app:24880-24895`).
- Local backup (`app:23713-23745`): only one pending payload, ≥3000 ms apart (`LOCAL_BACKUP_MIN_INTERVAL`,
  `app:347`), run in `requestIdleCallback` (5 s timeout) and postponed while interacting; context-checked.
- `scheduleSave(immediate, pageIndex, mode)` (`app:26152-26188`) is the public entry for edits.

### 2.7 L4 — offline library snapshot (capture)
- `InhouseOfflineLibrary.scheduleCapture()` debounces 180 ms, re-arms while `host.busy()`, then `capture()` and
  `kick({reason:'checkpoint'})` (`lib:274-281`). Called after the pen burst pauses (`app:25388-25411`).
- Also captured synchronously-before: every `queueDriveSave` (`oapp:417`), Home, page hide (`oapp:269`), sign-out
  (`app:6896`), opening another document (`app:11273-11276`), new document (`app:11783`).
- `captureQueue` is a `CoalescingQueue` keyed by `[account, session, id]` (`lib:250-267`). It skips when the account,
  identity or session changed, when the library already has this `contentVersion` for this session, or when the
  stored record is from the same session with `contentVersion ≥` the capture (**never roll back**, 5.16.44).
- `oapp.capture()` (`oapp:34-91`) returns `null` (nothing written) if: no id, no pages, an open is in progress,
  `driveCanEdit===false`, or **(boot guard, 5.16.42)** the session was not activated in this launch and the
  library already holds an available record from another session. Pages are packed incrementally with yields,
  aborting if the session/content version changes; an LRU of ≤64 packed pages / ≤16 MiB is reused while
  `pageDirtyGeneration` and the listed fields are unchanged (`oapp:10-33`).
- Snapshot value: `{pages:[packed StoredPage-like pages (timeline snapshot shape + packed ink)], collabStructure,
  collabFields, calendarPageConfig, exportName, versionHistory, baseline:[[pageIndex,[ids]]], baselinePages:[pageId]}`.
- `dirty = contentVersion≠uploadedVersion || driveDirty || id startsWith 'local-' || pending-exit marker for this file`
  (`oapp:86`). `S.checkpoint(..., dirty)` writes documents + index + (if dirty) a fresh outbox op + merged entry, in
  **one transaction** (`store:87-105`). Then `scheduleBackground()` (§2.12).
- `prepared(id, blob, version, session)` stores a built PDF as `preparedBlob` only if snapshot/version/session match
  (`lib:282-289`, called from `app:25606`).

### 2.8 L5 — foreground Drive save

#### 2.8.1 Scheduling
- `scheduleDriveSync` (`app:26362-26421`): no-op in the worker; sets `driveDirty`, schedules PDF preparation;
  read-only ⇒ watchdog; interacting/pen burst/recent activity ⇒ `deferredDriveSave` + retry
  (`max(180, remaining+90)`); no token ⇒ retry 900 ms; save running ⇒ `driveSaveQueued`; else debounce
  `DRIVE_SYNC_IDLE_DELAY=40 ms` (`app:365`) → `queueDriveSave(false)` and a max-latency timer
  `DRIVE_SYNC_MAX_DELAY=3000 ms` (`app:370`) → `queueDriveSave(true)` (postponed while a pen burst is active).
- `scheduleDeferredDriveSaveRetry(delay=280)` (`app:26312-26360`) re-evaluates the same gates, refreshes the token
  when missing.
- `queueDriveSave(force, options)` (`app:26783-26800`): a non-forced call during a pen burst is deferred *before*
  the offline copy; otherwise enqueued on `driveCheckpointQueue` (CoalescingQueue keyed by session; merge = OR of
  `force`, `keepalive`, `forceUpload`, `app:26773-26782`) whose executor is
  `InhouseOfflineApp.queueSave(force, options, queueDriveSaveOnline)`.

#### 2.8.2 `InhouseOfflineApp.queueSave` (`oapp:413-454`)
1. worker ⇒ no-op. Library not ready ⇒ run `queueDriveSaveOnline` directly.
2. `await L.capture()` (L4). Context changed ⇒ `{cancelled:true}`.
3. Offline ⇒ refresh indicator, return (the outbox owns it).
4. Document is `local-…` or `offlineEdited` ⇒ run the **outbox runner in the foreground** for this document only
   (`skip: other ≠ id`, `targetId` when priority, `build` = current editor PDF build, `merge` = editor merge,
   `saved` callback updates `state.drive*`, marks uploaded version, `driveDirty=false` and baseline if unchanged).
   Busy lease ⇒ retry in 1 s; afterwards `kick({reason:'after-save'})`.
5. Otherwise `S.withLease(account,'sync', queueDriveSaveOnline)`; busy ⇒ retry in 1 s
   (`oapp:404-412`); finally `leaseReleased()`.

`saveNow()` (manual "Save now", `oapp:455-478`): writes `sync-priority` (60 s, renewed every 30 s), loops
`queueDriveSave(true,{priority:true})` every 250 ms while the lease is busy for ≤120 s, then deletes priority and
kicks. Background runners yield before starting another document while a priority is live (`sync:23-29`); an
in-flight upload finishes (5.16.37).

#### 2.8.3 `queueDriveSaveOnline` state machine (`app:26802-27226`)
States = `driveSavePhase ∈ {idle, verifying, preparing, restoring, building, uploading, retrying, error}`.
1. Gates: `!driveAutosave` ⇒ return; read-only and not forced ⇒ return; blocked and not forced ⇒ show block;
   forced clears the block. `driveDirty |= (version mismatch)`; nothing dirty and not forced ⇒ return.
   Not forced and interacting ⇒ deferred retry.
2. Wait for clean-original recovery (`app:26828-26834`) — never export from a baked temporary source.
3. **Hydration**: an existing file must be hydrated (`driveHydratedFileId === driveFileId`) or the save is
   deferred 1.5 s with "Sync paused until remote state is verified" (`app:26835-26849`, `app:26076-26132`).
4. New `AbortController`; `driveSaveInProgress=true`, phase `preparing`; record `hadPendingDriveContent`
   (dirty intent captured **before** clearing `driveDirty`, 5.16.56).
5. If a remote pull was deferred, merge it first (phase `verifying`); failure aborts the save (`app:26874-26899`).
6. PDF backgrounds needed but not restored ⇒ restore; still missing ⇒ "Waiting for PDF…", reschedule, return.
7. **No-upload shortcut** only if the file exists, there was no pending content intent, versions match and not
   `forceUpload` ⇒ "Saved" (`app:26915-26921`).
8. New file: build the PDF (`prepareDrivePdfBlob`), `ensureDriveFile` creates it with a resumable POST to
   `upload/drive/v3/files?uploadType=resumable&fields=id,name,headRevisionId,modifiedTime,version` into
   `driveFolderId` or `ensureDriveFolder()` (`app:26423-26464`).
9. Existing file: if another device's presence shows an active save, pre-merge (`app:26965-26971`). Then loop ≤4 times:
   build the PDF if none; `mergeLatestRemoteBeforeDriveUpload` (`app:26740-26771`) reads the **v2 write revision**
   `GET drive/v2/files/<id>?fields=id,etag,headRevisionId,modifiedDate,version` (`app:26482-26492`; missing ETag or
   foreign id ⇒ error). If the remote is not newer than tracked ⇒ keep this ETag as `writeRevision`, break. Else
   `pullRemoteChanges(meta,{allowWhileLocalPending,preserveLocalUnsynced,visualSync})`, discard the PDF, rebuild,
   recheck. After 4 merges ⇒ abort "Remote changes still arriving" (dirty kept).
10. Upload `updateDriveFile(fileId, pdf, {writeRevision})` (`app:26494-26536`):
    - ≤4 MiB: single `PUT upload/drive/v2/files/<id>?uploadType=media&fields=id,headRevisionId,modifiedDate,version`
      with `If-Match: <etag>`, `Content-Type: application/pdf`; response must be 200/201 with `id === fileId`.
    - >4 MiB: resumable session on the same v2 URL (`uploadType=resumable`, `PUT`), `If-Match` on initiation and on
      every chunk (§2.8.4).
    - **412**: if the PDF head revision is unchanged (metadata-only change), fetch a fresh v2 ETag and retry the same
      bytes, max 2 retries; otherwise rethrow (`app:26524-26531`).
    - **Never use v2 PATCH for media** (files.patch is metadata-only) (5.16.39).
11. Success: update `driveHeadRevisionId/RevisionVersion/LastKnownModified`, `rememberOwnDriveWrite` (12 s echo window,
    `app:375`, `app:25797-25805`), `saveDriveSession`; `InhouseOfflineApp.confirmed()` acknowledges the outbox
    record if its `contentVersion` and session match (`oapp:394-402`); `markDriveContentUploaded(uploadedVersion)`;
    store the uploaded blob in the fast-open cache; only if no edit arrived during the save:
    `driveDirty=deferredDriveSave=false`, `updateSyncBaseline()` (`app:27077-27083`). Publish commit hint.
12. Failure (`app:27091-27158`): session changed ⇒ `{cancelled}`; signed out ⇒ drop flags; **412 ⇒ treated as
    abort + `deferRemotePull(null)`** (merge then retry, never "Saved"); aborts ⇒ keep dirty, reschedule;
    "missing page data for export" ⇒ checkpoint legacy migration then retry, else **block** with
    "Drive save blocked: page data could not be recovered"; any other error ⇒ "Drive save failed", keep dirty,
    retry in 1800 ms.
13. After: pending remote change during save ⇒ pull; flush deferred pull or post-save check
    (`app:27416-27469`); `driveSaveQueued` ⇒ run again; version mismatch ⇒ keep dirty and reschedule; fully settled ⇒
    clear the pending-exit marker and show Saved.

#### 2.8.4 Resumable upload (`uploadDriveBlobResumable`, `app:26541-26730`)
- Session reuse key = `resumeKey + ':blob-<per-Blob sequence>:<conditionalETag>'` — only the **exact same Blob
  object** resumes; equal size/hash is not enough (5.17.x `drive-save-truth`). Sessions live in memory
  (`driveResumableSessions`, `app:1889`), cleared on new session.
- Initiation: `X-Upload-Content-Type`, `X-Upload-Content-Length`, JSON metadata if any, `If-Match` if conditional;
  must return `Location`.
- Existing session ⇒ first query `PUT Content-Range: bytes */<size>`; 308 ⇒ resume at `Range: bytes=0-N` + 1;
  200/201 ⇒ done; 404/410 ⇒ new session; 412 ⇒ discard session and fail.
- Chunks of 1 MiB (multiple of 256 KiB, `app:26541`), 60 s timeout each; each waits for a background turn
  (pauses between chunks without resending accepted bytes, 5.16.59).
- 308 without progress >3 times ⇒ fail "Drive did not acknowledge uploaded bytes" (no loop, 5.11.37).
- Network error ⇒ ≤3 retries, each first asks the server what it has (`bytes */size`), backoff
  `min(1500, 250·retry)` ms.
- Completion requires status 200/201 and a JSON body with a string `id`.
- `keepalive` is only used for blobs ≤48 KiB; full PDFs never use keepalive (5.4.0).

#### 2.8.5 PDF preparation (`prepareDrivePdfBlob`, `app:25500-25619`)
- One shared build (`preparedDrivePdfBuild`) for idle warm-up and foreground saves; warm-up after 40 ms
  (`schedulePreparedDrivePdf(delay=40)`), postponed while interacting, **not** postponed by repeated requests
  for the same version (5.16.8).
- Build holds the remote-page-merge structure lock (≤15 s) and calls `buildPdfBlob({signal, sessionToken,
  highQuality:false, optimizeForUpload:true})` (`app:377-380`). Result reusable **only** for the same session and
  `driveContentVersion`; edits made while waiting for the lock belong to the build (`app:25594`).
- `buildPdfBlob` (`app:34521`) prefers the non-destructive pdf-lib path, retries it once, and only then falls back
  to the raster path (marking the doc honestly) — see `documents.md`; blobs carry `inhouseContentHash` and
  `inhouseOpenCacheAnalysis`.

#### 2.8.6 Drive HTTP client (`driveFetch`, `app:7749-7890`)
- Deadline (default 60 s) **includes token acquisition** (5.16.65). `Authorization: Bearer` injected.
- Any successful response notifies `InhouseOfflineLibrary.networkAvailable()` (authenticated reachability, 5.16.39).
- JSON, error, or `bufferResponse` bodies are read inside the deadline; streaming media bodies keep the deadline and
  an idle watchdog (`bodyIdleTimeout`, default 15 s) while consumers read (5.17.1).
- 401 ⇒ drop token, retry once; second failure ⇒ "needs reconnect". Non-OK and not in `acceptStatuses` ⇒
  `Error("Drive API error <status>: <body>")` with `code=status=<status>` (status preserved for the outbox, 5.16.35).
- Abort by caller ⇒ `AbortError("Drive request aborted")`; timeout ⇒ `Error("Drive request timed out")`.

#### 2.8.7 Remote revision ordering (`app:25693-25923`)
`normalizeDriveVersion` strips leading zeros; versions compare as big decimal strings. `isRemoteMetaNewerThanTracked`
ignores echoes of our own write within 12 s, prefers headRevisionId+version ordering, falls back to
`modifiedTime > tracked + 250 ms` (`DRIVE_REMOTE_META_SKEW_MS`). `canApplyDriveRemoteMeta` never lets two different
heads with equal timestamps replace each other; identical head with advanced version (properties change) is accepted
(5.16.x "metadata-only updates do not reject an already verified PDF head").
`fetchStableDrivePdfSnapshot` (`app:26013-26049`): download exact revision (owners) or head media guarded by
md5+size; meta before/after must match; ≤3 attempts.

### 2.9 Exit and lifecycle

#### 2.9.1 Home button (`goHome`, `app:15981-16061`)
1. `forceFlushingForExit=true` (no interaction can pause work; `getPenBurstQuietRemaining()=0`).
2. If anything is pending locally or for Drive: show "Saving on this device…", interrupt background work and await
   an in-flight PDF preparation (it owns the page-store lock, 5.16.61), `await queueSave()`,
   `await persistExitLocalCheckpoint(fileId)` (`app:21880-21924`): wait ≤5 s for an L1 flush, flush legacy pages,
   L1, all dirty pages (`force`), refuse inconsistent photo checkpoints, write L3 + **immediate** localStorage backup,
   persist the timeline, **`await InhouseOfflineLibrary.capture()`**, confirm.
3. If Drive is pending, require the library record to be `available`, from this session and at the current
   `contentVersion`, else throw.
4. Any failure ⇒ stay in the editor: "Could not finish saving. The document is still open; please try Home again."
   **Home never waits for Drive, auth, or peers** (5.16.6; smoke forbids `Promise.race` here,
   `release-v5-smoke.mjs:510`).
5. Success ⇒ show Home ("Saved on this device · waiting to sync" or "All changes saved."), `kick()` the outbox, and
   (online) start a detached `startLifecycleExitSave(fileId, Promise.resolve(true))`.

#### 2.9.2 Hide/close (`flushOnClose`, `app:26268-26299`; listeners `app:16531-16598`)
- Triggers: `visibilitychange→hidden`, `pagehide`, `beforeunload` (the latter shows the native "leave site?"
  prompt while an exit upload or marker is pending, `app:16534-16537`).
- If Drive content is pending: write the **pending-exit marker** first. Save view state. Only when nothing local is
  pending, write an immediate metadata backup (**never publish metadata before bodies**, smoke `:504-508`).
- `getLifecycleLocalCheckpoint` reuses one checkpoint per (file, content version) (`app:21926-21948`).
- `startLifecycleExitSave` (`app:21959-22007`): after the checkpoint, wait ≤20 s for any running save, then force
  `queueDriveSave(true)`; clear the marker only when fully settled.
- On hidden, page memory is released only after the checkpoint (`app:16555-16560`); the offline app also captures
  and requests platform background work (`oapp:266-270`).

#### 2.9.3 Pending-exit marker resume (`resumePendingExitUploadIfNeeded`, `app:22009-22047`)
Runs on visible/online/after an open of the same file (`app:11236-11254`, `app:16547-16549`, `app:16580`):
only if the marker's `fileId` equals the open file and autosave is on; raises `driveContentVersion` to
`max(marker.contentVersion, current)` and forces a save; clears the marker on success. Opening a document never
awaits a previous exit save (smoke `:478-479`).
Also consulted by the offline capture (`oapp:85-86`) and startup reconciliation (`oapp:184-186`).

### 2.10 Offline outbox runner (`InhouseOfflineSync.run`, `sync:20-261`)
Shared by the foreground (`oapp:426`), the hidden processor (`oapp:361`), the SW (`sw:76`) and Android headless.
Runs entirely under `withLease(account,'sync')`; every request first asserts the lease (`sync:46-51`).

Adapter contract: `{fetch(url,opts)→Response, background?:bool, build?(doc)→Blob, merge?(doc,meta,remoteBlob)→Blob,
skip?(id)→false|true|'owner', progress?({id,phase}), saved?(id,meta,blob), targetId?}`.
Result: `{saved, pending, errors:[{id?,phase,code,message}]}` or `{busy:true}` if the lease is held.

Folders first (`sync:116-153`): (with `targetId`, only the target's local ancestors) multi-pass until no progress;
parent must resolve (`resolveId`: non-local → itself; `local-` → its `remoteId` or null); reserve an id via
`GET drive/v3/files/generateIds?count=1&space=drive&type=files` (stored on entry/document **before** creating);
`GET` metadata; 404 ⇒ `POST drive/v3/files` `{id, name, mimeType: folder, parents:[parent], appProperties:{ihnOfflineId}}`;
`finishFolder`. Unresolvable parents ⇒ pending.

Documents (`sync:154-258`), per outbox op:
1. Priority live (background only) ⇒ stop, report pending. Not dirty ⇒ skip. `skip(id)` ⇒ pending (background marks
   an attempt unless `'owner'`).
2. No `build` adapter and an unprepared snapshot ⇒ pending, no request (SW never touches drafts).
3. Parent unresolved ⇒ pending. Id = `reserve()` for `local-`, else `remoteId||op.id`.
4. `metadata(id)` with fields `id,name,mimeType,parents,modifiedTime,version,headRevisionId,trashed,capabilities(canEdit),appProperties`
   (`sync:7`); 404 tolerated only for `local-` (create). Trashed ⇒ `remote-deleted`; `canEdit===false` ⇒ 403.
5. **Lost-response recovery**: `appProperties.ihnOfflineReceipt === "<opId>:<generation>"` ⇒ download the committed
   media, recheck metadata unchanged, acknowledge with the server bytes (no second upload).
6. Blob = `preparedBlob` (same generation) else `build(doc)` if a snapshot exists else `doc.blob`; must be a non-empty Blob.
7. Remote changed vs `baseMeta` (`changed()`: headRevisionId, else version, else modifiedTime, `sync:8-9`) ⇒
   download remote, **write both full PDFs to `recoveries` first**, then if bytes differ from `doc.blob`
   (SHA-256): `merge()`; `merge-unavailable` (flattened remote without editable metadata) ⇒ preserve the remote as a
   sibling `"<name> (recovered Drive version).pdf"` with a reserved id and `appProperties:{ihnRecoveryFor, ihnRecoveryKey}`
   (idempotent across retries) before overwriting; recheck remote unchanged after merge else 412.
8. Generation changed meanwhile ⇒ pending (do not upload an old generation).
9. No ETag from v3 ⇒ obtain v2 `etag,version,headRevisionId` and require the same revision, else 412.
10. Multipart upload with `appProperties.ihnOfflineReceipt` (create: v3 POST with `{id,name,mimeType,parents}`;
    update: `If-Match`, v2 token ⇒ **v2 PUT** with properties mapped to PRIVATE `properties`, v3 token ⇒ v3 PATCH);
    412 with unchanged head ⇒ refresh v2 ETag, ≤2 retries.
11. `S.acknowledge(account, opId, generation, savedMeta, blob)`: settles only if `doc.generation === generation`
    (edits during upload stay pending, 5.12.0 "outbox acknowledgment cannot delete an edit made during upload");
    settled ⇒ save `last-upload-draft` recovery, `baseMeta=meta`, `dirty=false`, drop snapshot/analysis/original/
    prepared, `blob=uploaded`, delete the outbox op; always update `remoteId`, `lastUploadMeta`,
    `lastUploadedAt` and the entry (`pending:!settled`).
12. Errors: `needs-processor`/`needs-merge` with no capable adapter only record `lastError*` (no backoff);
    everything else records an attempt; `AbortError` is a pause (attempts not incremented).
Backoff per op: `attemptedAt + min(15 min, 15 s·2^(attempts−1))` (`sync:14-19`); a new checkpoint resets it.

### 2.11 Outbox scheduling (`lib:349-548`)
- **No polling.** `kick(reason)` runs only when initialized, resumed, account known, online. Reasons:
  `checkpoint, after-save, after-run, lease, again` (own work — never force-save the open document), `timer`,
  `online`, `probe`, `signed-in`, `launch`, `retry`, `account` (`lib:417`).
- Flow (`lib:430-486`): busy input ⇒ recheck in 3 s; cannot sync (signed out / reconnect needed) ⇒ wait for sign-in;
  priority live ⇒ wait until it expires; the open document is pushed through its own editor (`host.syncActive` =
  `queueDriveSave(true)`) only for outside events or its own failed attempt's timer; background ops = folders +
  documents other than the open one; none due ⇒ arm timer at the next `retryAt`; run-level backoff
  `min(15 min, 15 s·2^(failures−1))` when a run could not start (busy lease or an error without document id);
  ≥5 s between runs — both waived by a forced kick (launch, reconnection, sign-in, retry, account); a live `sync`
  lease ⇒ wait for `expires+50` or `leaseReleased()`.
- Processor = hidden same-origin iframe `/?offline-worker=1&v=<APP_VERSION>` (`lib:535-536`); reused for
  later runs via `postMessage({type:'ihn-offline-run'})`; retired when the outbox is empty, on timeout (8 min), or
  after 90 s idle (`lib:409`, `lib:490-548`). Messages from the iframe: `ihn-offline-progress {progress}`,
  `ihn-offline-complete {result, reusable}`.
- Launch: `resume()` captures the restored editor (waiting ≤3 s), then `kick({force:true,reason:'launch'})` and a
  catalogue refresh (`lib:598-609`) — a document left mid-upload uploads ≈2 s after reopening (5.17.22).
- Connectivity (`lib:57-105`): offline = `sessionStorage` sticky flag or (`navigator.onLine===false` and no
  reachability proof). Probe = `GET https://www.googleapis.com/drive/v3/about?fields=kind` (`credentials:'omit'`,
  5 s); any HTTP status proves reachability (401/403 count). Probes only while offline and visible: 30 s doubling to
  5 min; `online`/visible probe at once. An authenticated `driveFetch` response wins over a late failed probe
  (`networkEpoch`).
- Platform background work (`lib:355-399`): requested (`InhouseBackground.schedule()` + Background Sync
  `registration.sync.register('inhouse-outbox')`) **only while the app is hidden with outbox rows**; cancelled when
  visible, when the outbox is empty, and once per writing burst (resumed after the idle window).
- Hidden processor (`oapp:310-384`): account from hint; auth from the parent's `backgroundAccess` (same session,
  invalidating only the rejected token, account must not change, 5.16.38) or, headless, from localStorage tokens /
  PKCE refresh; verifies `drive/v3/about user.emailAddress` equals the account once; every fetch rechecks
  `__session__.enabled` and the hint email; skips the parent's active document (`'owner'`) and any document named
  by a live `editor` lease; builds/merges by restoring the snapshot into its own (background-prefixed) editor.

### 2.12 Open paths and caches (persistence side)
`openDriveFile` (`app:11261-11408`):
1. Library ready ⇒ checkpoint a visible editor, `openRecord(file)` (bumps `lastOpenedAt`); an available record ⇒
   `InhouseOfflineApp.restore()` (snapshot first; PDF only when needed; dirty ⇒ `driveContentVersion=1`,
   uploaded 0) and revalidate online (`oapp:111-177`). Failure with a dirty draft or offline ⇒ error, **never
   replace a draft with the server PDF**; a clean broken copy falls through to Drive bypassing caches (5.16.34).
2. Otherwise: prepared in-memory upload blob of the same file/version (`app:605-638`) or `drive-open-cache`
   (matching card `modifiedTime`) ⇒ open instantly, revalidate in background (`app:11182-11230`).
3. Else download (metadata + media) and import; schedule the open-cache write.
`InhouseOfflineLibrary.cache()` never overwrites a dirty document (`lib:242-249`). Startup reconciliation replaces the
single-editor cache with a newer clean library copy (`oapp:178-188`); `refreshed()` adopts a background upload of
the open document when the editor has no newer edits (`oapp:189-197`).

Offline creation: new document ⇒ `local-<uuid>` id, captured and kicked (`oapp:385-393`); device PDF upload ⇒
checkpoint `{blob, offlineEdited:true, baseMeta:null}` **before the picker closes** (`app:11744-11760`);
folders ⇒ `local-` entry + folder op (`store:135-143`).

### 2.13 Save-indicator mapping (`getSavePresentation`, `app:31374-31465`)
`phase` = `error` (local error, Drive block, or pending Drive with last error and no save running) > `saving`
(local pending) > `uploading` (Drive save or outbox active for this doc) > `syncing` (pull) > `paused` (Drive pending,
incl. a dirty library record) > `saved`. "Strokes saved" when the ink journal acknowledged the current content
version while the PDF is still pending; a journal receipt never advances the PDF's uploaded version (5.17.0).
Home cards show outbox badges: `Checking…/Preparing…/Merging…/Keeping both versions…/Uploading…`,
`Retry upload` (error, opens details with Copy error/Retry), `Saved offline`, `Waiting to upload` (`lib:130-182`).

### 2.14 Durability guarantees

| moment | guaranteed durable on this device | in Drive |
|---|---|---|
| stroke drawn, pen still writing | memory only (deferred while writing, 5.16.61) | — |
| ~180 ms after input stops | stroke in `stroke-ops` (replayed on reload in this session id) | — |
| local checkpoint `ok` | page bodies + metadata (+ backup) | — |
| offline capture committed | whole document in `documents` + outbox op (survives process death; another launch's Home can upload it) | — |
| Home transition visible | all of the above for the current content version (else editor stays open) | not required |
| `confirmed`/`acknowledge` settled | PDF in `documents.blob`, previous draft in `recoveries` | Drive receipt with matching id |
| page hidden/closed with Drive pending | pending-exit marker + local checkpoint attempt | upload attempted; resumed next launch |

---------------------------------------------------------------------------------------------------

## 3. Service worker, boot, Android host

### 3.1 Precache & versioning (`service-worker.js`, `offline-assets.js`)
- `VERSION='5.17.22'` must equal `APP_VERSION` (`sw:2`); cache `inhouse-shell-<VERSION>`; `importScripts('offline-assets.js',
  'offline-store-v5.js','offline-sync-v5.js')` (`sw:4`).
- Precache list = `self.INHOUSE_OFFLINE_ASSETS` (245 paths, `offline-assets.js:2-248`), generated by
  `scripts/build-offline-assets.mjs:36-47`: the listed core files + every file under `vendor/pdfjs`, `vendor/pdf-lib`,
  `vendor/jspdf`, `vendor/fonts`, `scanner` (excluding `*.md|*.map|*.py|*.zip|*.pdf` and `LICENSE`), sorted, with a
  leading `/`. `offline-assets.js` itself is not in the list.
- `install`: `cache.addAll(list)` (atomic — an incomplete shell never replaces a working one), then `skipWaiting()`.
  `activate`: delete every other `inhouse-shell-*` cache, `clients.claim()` (`sw:5-16`).
- Registration: only by the visible app, `register('/service-worker.js',{updateViaCache:'none'})` (`lib:577-578`).
  The worker iframe never registers.

### 3.2 Fetch handling (`sw:17-54`)
1. `GET` image requests to `/__offline-media__/…` ⇒ answer from `inhouse-media-<active account>` or 404 (requires
   `__session__/active.enabled`).
2. Pass through: non-GET, cross-origin (Drive/OAuth/Google are never cached), `/oauth-callback*`, `?oauth_callback`,
   paths not in the precache list (`/` maps to `/index.html`), and **requests whose `v` query is a semver ≠ VERSION**.
3. `.html` ⇒ `freshPage`: return the cached copy immediately if present, else the network; in the background fetch
   `no-cache` and store it only if `ok` and the body contains `v=<VERSION>` (rejects a mismatched page).
4. Other assets ⇒ cache-first by path (query ignored, so `?native=` and `?v=` variants share one entry); on miss
   fetch and store if ok.

### 3.3 SW background sync (`sw:55-81`)
`sync`/`periodicsync` with tag `inhouse-outbox` run `InhouseOfflineSync.run` without `build`/`merge` (only prepared
or plain PDFs can be uploaded) using `background-auth` (refresh via `https://oauth2.googleapis.com/token` with
`grant_type=refresh_token`, `client_id` when < 60 s left; 60 s per request timeout); posts
`{type:'ihn-offline-updated', result}` to window clients (library reloads, `lib:579`); throws (⇒ browser retries)
while anything is pending. Only one-shot `sync` is ever registered (`lib:370`); no `periodicSync.register` exists.

### 3.4 Boot sequence
`index.html` order: inert `<script type="application/x-inhouse-config" id="inhouse-app-version">const APP_VERSION='…'`
(`index.html:93`), `inhouse-native-assets` JSON map (`:94`), `boot-v5.js` (sync, `:95`), CSS, then deferred modules
in order (`index.html:10431-10453`): jspdf, pdf.js, pdf-lib, rust-core, collaboration-core, peer-transport,
live-collaboration, runtime-core, security-core, timeline-core, document-processing, updater, bullet-template,
offline-store, offline-sync, offline-library, offline-media, offline-app, ink-palette, drive-ink-journal,
drive-live-preview, drive-ink-app, app-v5. CSP: `index.html:84` (`worker-src 'self' blob:`, `connect-src` Google APIs).

`boot-v5.js` (runs before the DOM is complete):
1. `InhouseAssetUrl(path)` appends `native=<sha256>` for same-origin paths in the inline map (`boot:4-10`).
2. Theme from `inhouse-theme`/system; paints `#151515`/`#f5f5f0` and `theme-color` (`boot:12-26`).
3. OAuth callback bounce (`?oauth_callback=1` → `inhousenotes://oauth2callback#…`, then back to `/?inhouse_app=1`
   after 500 ms) (`boot:28-45`).
4. Boot destination `drive` iff a valid token (current scope version, calendar scope, ≥60 s left), or a refresh
   token with current scopes, or `drive-remember-v1==='1'`, or a `drive-hint-email` (`boot:47-74`); Home skeletons
   are painted as soon as `#drive-folder-files` exists (`boot:76-106`, `boot:203-214`).
5. Release watcher (§3.5).
6. If `window.InhouseBackgroundChannel` exists (Android), define `window.InhouseBackground =
   {schedule, cancel, complete(jsonString)}` posting `JSON.stringify({type, result})` (`boot:195-198`).

`app-v5.js` entry (`app:35865`): worker URL ⇒ `InhouseOfflineApp.worker()`; else `init()` (`app:2256-2369`):
load view state, theme, `drive-session-v1`, cached profile; show Home/Welcome fast; start the offline app
(failure disables the library but never blocks boot); start auth; then **in parallel** `loadFromStorage()` and
library init; `reconcileStartup()`; `InhouseOfflineLibrary.resume()`; always land on Home (or a shared-link target).

### 3.5 Update flow (`boot:108-193`)
- Skipped for `?oauth_callback` and `?offline-worker`. 1.2 s after `load`, on `visibilitychange→visible` (≥60 s since
  last check) and on bfcache `pageshow`, when online: `fetch('/service-worker.js?release-check=<now>',{cache:'no-store'})`,
  parse `const VERSION='x.y.z'`; if newer than the running `APP_VERSION` **and the editor is not visible**: guard
  with `sessionStorage['ihn-release-reload']=published` (once per release per tab; storage failure ⇒ do nothing),
  `registration.update()`, delete `/index.html` and `/scanner/index.html` from every other `inhouse-shell-*` cache,
  `location.reload()`. An open document is never reloaded; the check repeats when the app becomes visible.
- A running page whose SW was replaced requests `?v=<old>` assets, which the new SW passes to the network (§3.2).
  The worker iframe URL carries `v=<APP_VERSION>` likewise.

### 3.6 Android host
- **Loader**: the APK's `www/index.html` is `.github/android/app-loader.html` (`.github/scripts/build_android_apk.py:15,70`);
  it paints a local launch mark and after two animation frames `location.replace("https://inhousenotes.com/?inhouse_app=1")`
  (`app-loader.html:27-31`). The web app is **always loaded from the live origin**; the APK ships no app logic.
- Capacitor config: `server.androidScheme:'https'`, `allowNavigation` = inhousenotes.com, *.inhousenotes.com,
  accounts.google.com, *.google.com, *.googleusercontent.com, *.gstatic.com (`builder:1414-1439`).
- `MainActivity` (`builder:1593-1622`): DOM storage + database enabled, UA += ` InhouseNotesApp/<versionName>`
  (with `; wv` and `Version/4.0 ` removed), third-party cookies, `addJavascriptInterface(InhouseNativeBridge,
  "InhouseNative")`, OAuth deep links `inhousenotes://oauth2callback` / `<package>:` forwarded to
  `window.handleInhouseNativeOAuth(payload)` (`builder:1631-1666`); cookies flushed on pause/stop (`builder:1944-1954`).
  App mode in JS: `?inhouse_app=1` or UA regex `InhouseNotesApp/<digits>` (`app:99-121`).
- **Native assets** (`scripts/build-native-assets.mjs`, `.github/scripts/install_native_assets.py`,
  `.github/android/performance/InhouseLocalAssets.java`): `native-assets.json` = `{schemaVersion:1, assets:{"/path":
  {sha256, size, mime}}}` for every precache path except `.html` and `/offline-assets.js`; HTML `src/href` attributes
  and `vendor/fonts/ui.css` font URLs get `?native=<sha256>`. The installer copies each file into
  `assets/inhouse-native/<path>` after verifying size+SHA-256 (stale ⇒ build fails). At runtime
  `shouldInterceptRequest` serves a request only if: `GET`, not main frame, `https://inhousenotes.com` (port default/443),
  path without `..`, `native` query equals the manifest sha256 for that exact path — with headers
  `Cache-Control: public, max-age=31536000, immutable`, `Content-Length`, `X-Content-Type-Options: nosniff`,
  `X-Inhouse-Asset-Source: apk`, UTF-8 for text types. Anything else falls through to Capacitor/network/SW, so a newer
  web release with changed bytes still works with an old APK. Renderer priority IMPORTANT, waived when hidden.
  "No documents, OAuth or account data cross this path" (`InhouseLocalAssets.java:18`).
- **Background sync contract** (`.github/android/background/*.java`, installed by
  `.github/scripts/install_background_sync.py`; `androidx.work:work-runtime:2.10.1`, `androidx.webkit:webkit:1.12.1`):
  - JS→native channel: `WebViewCompat.addWebMessageListener(webView,"InhouseBackgroundChannel",{"https://inhousenotes.com"})`;
    messages accepted only from the main frame with scheme https and host inhousenotes.com; payload JSON
    `{type:'schedule'|'cancel'|'complete', result?}` (`InhouseBackgroundSync.java:18-33`). The visible app's WebView
    (no completion handler) honours `schedule`/`cancel`; the headless WebView honours only `complete`.
  - `schedule`: unique one-time work `inhouse-offline-outbox` (KEEP, network CONNECTED, exponential backoff 30 s) +
    unique periodic work `inhouse-offline-retry` every 15 min (KEEP, CONNECTED). `cancel`: cancel both
    (`InhouseBackgroundSync.java:34-47`).
  - `InhouseSyncWorker` (`InhouseSyncWorker.java:20-58`): on the main thread creates a WebView (JS, DOM storage, DB on;
    file/content access off; UA += ` InhouseNotesApp/Background`; navigation restricted to https://inhousenotes.com),
    attaches the channel with a completion handler and loads `https://inhousenotes.com/?inhouse_app=1&offline-worker=1`.
    It shares the app's origin storage (localStorage tokens, IndexedDB outbox, SW). Waits ≤8 min.
    **Success iff** the completion result is non-null, `busy` is not true, `pending == 0`, and `errors` is empty;
    otherwise `Result.retry()`. `onStopped` destroys the WebView.
  - The web side sends `complete` from `InhouseOfflineApp.worker()` after every run (`oapp:366-367`); the
    headless page (not embedded) runs once, authenticates from localStorage/PKCE refresh, and never registers a SW.

---------------------------------------------------------------------------------------------------

## 4. Historical failure modes v6 must not reintroduce

| # | failure (version fixed) | rule / code |
|---|---|---|
| 1 | Pushing local-only state over another device's strokes (pre-5.3) | merge deferred remote pulls before any upload; final v2-ETag revision check immediately before upload (`app:26874-27018`) |
| 2 | Full PDFs sent with `keepalive` failed instantly (5.4.0) | keepalive only ≤48 KiB (`app:26512`, `app:26562`; smoke `:502`) |
| 3 | Closing from Read mode skipped persistence (5.4.0) | read-only never blocks checkpoints/exit save (`app:26199-26200`; smoke `:503`) |
| 4 | Intermediate flag cleared ⇒ change waited for next edit (5.5.0) | version mismatch is always pending work (`app:26816-26818`, `app:25246-25248`) |
| 5 | Blocked IndexedDB upgrade froze opening (5.9.10) | 1.5 s blocked / 5 s timeout ⇒ continue without DB (`app:22237-22246`) |
| 6 | Metadata published before page bodies (5.9.10) | bodies+ops first, then L3; close never publishes metadata while local work is pending (`app:23908-23911`, `app:26278-26287`) |
| 7 | Silent destructive raster saves (5.11.21–25) | retry pdf-lib once; mark raster fallback; embed clean original (`app:34552-34574`) |
| 8 | Missing-page export permanently blocked autosave (5.11.32) | checkpoint legacy migration and retry (`app:27130-27148`) |
| 9 | Open triggered a re-upload of unchanged legacy PDFs (5.11.33) | opening resets content versions; only real edits dirty (`app:11404`) |
| 10 | Upload restarted from byte 0 / hung after headers (5.11.33, 5.11.37, 5.17.1) | resumable with server-confirmed offsets; body reads inside deadlines; zero-progress cap (`app:26600-26729`, `app:7749-7797`) |
| 11 | Open-cache duplicated stroke arrays (5.11.34) | `embeddedStrokes:null` (`app:665-675`) |
| 12 | Edits during an upload deleted by the acknowledgement (5.12.0) | acknowledge settles only the exact generation (`store:112`) |
| 13 | Lost upload response caused a duplicate upload/create (5.12.0, 5.17.0) | reserved ids + `ihnOfflineReceipt` recovery (`sync:56-64`, `sync:182-190`) |
| 14 | Startup editor cache replaced the durable outbox draft (5.16.42) | boot capture guard + session restore (`oapp:37-41`, `app:24017-24033`) |
| 15 | Prepared PDF/ack from a previous launch matched (5.16.42) | launch-scoped offline session token (`app:24951-24953`); checkpoint clears prepared (`store:95`) |
| 16 | Delayed capture rolled back a newer checkpoint (5.16.44) | same-session `contentVersion` monotonic check (`lib:263`) |
| 17 | v2 media upload via PATCH failed (5.16.39) | v2 ⇒ PUT, v3 ⇒ PATCH (`sync:76-82`, `app:26509-26510`) |
| 18 | Drive v3 without ETag blocked or bypassed conditional writes (5.16.35) | fetch v2 ETag bound to the same revision (`sync:223-233`, `app:26482-26492`) |
| 19 | Non-mergeable flattened remote stuck in `merge-unavailable` / overwritten (5.16.40) | preserve sibling recovery copy idempotently; byte-identical remote needs no merge (`sync:85-115`, `sync:205-217`) |
| 20 | Stale/broken offline probes kept the app "offline" (5.16.39) | probe the Drive API; authenticated responses win (`lib:84-105`) |
| 21 | Home waited on Drive/auth/peers (5.16.6) | Home waits only for local durability (`app:15997-16039`) |
| 22 | Late exit upload dirtied/saved the next document (5.16.6) | persistence context checks; marker cleared only for the same file (`app:21849-21855`, `app:21975-21981`) |
| 23 | Superseded checkpoints shown as storage failures; retries gave up after 3 (5.16.50) | `deferred` outcome; unbounded bounded-backoff retry (`app:26230-26236`, `app:24897-24914`) |
| 24 | Pending draft took the "no upload needed" path when counters matched (5.16.56) | capture `hadPendingDriveContent` before clearing `driveDirty` (`app:26858`, `app:26915`) |
| 25 | Equal-size/hash PDFs reused another upload session (drive-save-truth) | session keyed by Blob identity + ETag (`app:26563-26567`) |
| 26 | Stale page metadata/overlay reused after appending ink (5.16.65) | `invalidatePageMutationCaches` on every op (`app:24590`) |
| 27 | Token refresh outside the request deadline (5.16.65) | deadline covers `ensureDriveToken` (`app:7799-7817`) |
| 28 | Background processor needed its own stored token / used another account's (5.16.38) | `backgroundAccess` from Home; account assertions (`oapp:275-288`) |
| 29 | Background work stole frames while writing; uploads cancelled by hover (5.11.35, 5.16.59–64) | input-idle gating; only accepted canvas contact aborts; chunks pause without resend |
| 30 | Reload kept showing an old release (5.14.0) | release watcher with `no-store`, once per release, never while editing (`boot:130-193`) |
| 31 | Incomplete SW install displaced a working shell | atomic `addAll` before `skipWaiting` (`sw:6-11`) |
| 32 | Background uploads only every 10 s app reloads / never after restart (5.17.22) | event-driven kick, reused processor, forced launch kick (`lib:401-548`, `lib:598-609`) |
| 33 | Inconsistent photo/background register checkpointed via Home (5.17.13–15) | `hasUnresolvedRemotePhotoCheckpoint` gate in normal and exit checkpoints (`app:21897-21900`, `app:23921-23923`) |
| 34 | Ops from an aborted previous-document transaction landing in the new document's page keys | `beginDocumentSession` aborts all page/op/meta transactions (`app:25021-25031`) |
| 35 | Concurrent page reorder vs numeric page keys | single atomic pages+ops+meta rewrite under `collabPageOrderMutationInProgress` (`app:23174-23303`) |

---------------------------------------------------------------------------------------------------

## 5. Tests pinning this behaviour (one line each)

Unit (`npm run test:unit`, `package.json`):
- `tests/offline-sync-schedule.test.mjs:144` idle Home with empty outbox does no background work.
- `:155` failing upload backs off exponentially and reuses one processor.
- `:172` processor released as soon as the outbox is empty.
- `:184` processor never loaded when it could not make progress.
- `:212` backoff honoured by ordinary events, bypassed by launch/reconnect/retry.
- `:227` a new checkpoint of a waiting document is processed immediately.
- `:240` a held sync lease defers the processor until released.
- `:258` a document left with a pending upload is retried right after restart.
- `:268` busy editor defers background work without polling.
- `:279` platform background work only while hidden with work pending.
- `:295` offline probes back off and stop once connected/hidden.
- `:311` runner records attempts only for the queued revision, never for a pause.
- `:337` runner without a processor leaves drafts untouched.
- `:346` owner page never backs off its own open document.
- `:355` unchanged Home refresh writes/re-renders nothing.
- `:372` open document pushed once per outside event, never by its own saves.
- `:386` busy-lease run keeps the processor for its retry.
- `:404` uploads waiting for sign-in resume on next authenticated response.
- `tests/offline-library-index.test.mjs:40,61,82,94,113,124` catalogue aliases, ordering, Home views, atomic reload/logout, recents stability.
- `tests/local-save-status-v5.test.mjs:74` Drive success cannot clear a local checkpoint failure.
- `:92` unfinished remote checkpoint stays pending, session-scoped.
- `:106` dequeued operation stays pending until IndexedDB commits.
- `:132` local retries keep recovering after three failures.
- `:151` read mode still saves earlier edits.
- `:167` late checkpoint/retry cannot affect another document.
- `:188` new document save queued behind an old attempt still completes.
- `:216` superseded checkpoints pending without storage error.
- `tests/pen-writing-idle-v5.test.mjs:69,83,95,155` burst quiet period, exit never waits, offline copy once per burst, non-forced save deferred before copy.
- `tests/document-jobs.test.mjs:8` save bursts share the next checkpoint, never the in-flight snapshot.
- `tests/runtime-core-v5.test.mjs:8,24,48` deadlines, cancellation, retries.
- `tests/native-assets.test.mjs:8,26` APK manifest = exact public bytes; installer rejects stale bytes.
- `tests/drive-ink-journal.test.mjs:21,31,430` late receipts, reserved identities, outbox drain.
- `tests/release-v5-smoke.mjs:478-510,1243-1260` storage key names, exit functions, no keepalive, no metadata before bodies, Home waits locally, SW/boot release logic.

Browser (`npm run test:e2e`):
- `tests/e2e/app-resilience.spec.mjs:22` resumable upload resumes at the server-confirmed byte.
- `:80` upload recovers when the completion body stalls.
- `:107` repeated zero-progress acknowledgements stop.
- `:872` blocked IndexedDB cannot trap startup.
- `:892` failed page transactions return a bounded error.
- `:923` an edit followed immediately by leaving is durable on reload.
- `tests/e2e/local-save-recovery.spec.mjs:8,26,39,52` packing races, page-order lock, real IDB failure, edit during metadata commit.
- `tests/e2e/home-save-navigation.spec.mjs:25,34,44,59,74,89,99` Home local-first semantics and session isolation.
- `tests/e2e/document-lifecycle.spec.mjs:127` a document left before its Drive save finished is uploaded from Home.
- `tests/e2e/drive-save-truth.spec.mjs:9,26,40,51,63` upload-session identity, 410 restart, no false Saved, recovery draft, dirty intent.
- `tests/e2e/fast-drive-save.spec.mjs:4,20,35,53,114,129,140` single conditional request, receipts, ETag binding, conflicts, pen pause.
- `tests/e2e/drive-preparation.spec.mjs:102,132,154,178,197,250` prepared PDF reuse rules.
- `tests/e2e/save-latency.spec.mjs:2,26,37,47,72` journal latency, auth deadline, metadata-only head changes, capture reuse.
- `tests/e2e/save-priority.spec.mjs:8,33` Save now priority and its expiry.
- `tests/e2e/save-status.spec.mjs:46,67,111,135,161` indicator truthfulness.
- `tests/e2e/ink-checkpoint.spec.mjs:33` invalid binary ink lengths/ranges rejected.
- `tests/e2e/offline-library.spec.mjs:26,55,67,80,107` retention, grey unavailable docs, ack vs concurrent edit, cold offline navigation, offline creation.
- `tests/e2e/offline-library-cached.spec.mjs:8,63,81,98,140,156` cached opening, schema upgrade without modifying drafts, new revision never reuses old snapshot, cold Home keeps outbox draft.
- `tests/e2e/offline-reconnect.spec.mjs:27,42,82,107,127,144,161,190` stale editor checkpoint, busy lease retry, corrupt clean copy, failed dirty open, 404 create, missing ETag, revision change, eight-document drain.
- `tests/e2e/offline-sync.spec.mjs:6,28,37,51,70,115` archive+merge+conditional upload, conflict retention, lost response, folder ordering, identical remote, failed preservation.
- `tests/e2e/offline-restart-upload.spec.mjs:96` document left during upload is uploaded after restart.
- `tests/e2e/offline-worker.spec.mjs:2` hidden/headless processor merges and uploads real PDFs (8 modes); `:149` account change during auth rejected.
- `tests/e2e/offline-connectivity.spec.mjs:19,28` probe ordering; persisted failure reasons on cards.
- `tests/e2e/create-upload-menu.spec.mjs:17` device PDF durable offline before the picker closes.
- `tests/e2e/pen-background-priority.spec.mjs:24,51,87` chunk pause without resend; pen aborts upload; native sync once per burst.
- `tests/e2e/editor-performance.spec.mjs:104` incidental pointer does not cancel an upload.
- `tests/e2e/release-check.spec.mjs:12,23,37,63,95` release reload once, never while editing, cached shell first, mismatched copy rejected.
- `tests/e2e/drive-ink-replay-durability.spec.mjs:144,184,225,424` replay acknowledges only durable checkpoints; session isolation.

---------------------------------------------------------------------------------------------------

## 6. Summary of the most important invariants

1. Local durability order is fixed: stroke-op journal → page bodies → metadata (`snapshots/latest`) → localStorage backup; metadata never describes bodies that are not yet committed.
2. Every persisted page/op row is owned by a document session id that is restored from the metadata at boot; rows from another session are invisible, and each new document session aborts the previous session's transactions.
3. `driveContentVersion !== driveUploadedContentVersion` is the only authoritative "Drive is behind" signal; dirty intent is captured before flags are cleared, and only a confirmed receipt of the exact uploaded version advances it.
4. Every Drive overwrite is conditional on the revision that was actually checked/merged (v2 ETag, `If-Match`; v2 media = PUT); a 412 means merge-then-retry, never "Saved".
5. The offline library commits a document, its index row and its outbox op in one transaction; acknowledgement settles only the exact generation, and keeps a last-upload editable draft.
6. Concurrent remote versions are archived to `recoveries` (both full PDFs) before any merge; non-mergeable remotes are preserved as idempotent sibling copies before replacement.
7. Lost responses are recovered idempotently via reserved Drive ids and `ihnOfflineReceipt`, never by blind re-create/re-upload.
8. Home and close wait only for local durability (never Drive/auth/peers); a pending-exit marker plus the outbox guarantee the upload resumes after process death.
9. Background work is event-driven, input-gated (180 ms idle, 1.2 s pen-burst quiet), lease-serialized (`sync` lease + Web Lock), backs off 15 s→15 min, and on Android/SW is requested only while the app is hidden with outbox rows.
10. The shell is version-pinned (`inhouse-shell-<VERSION>`, atomic install, `v=` mismatch passthrough, HTML stored only if it carries `v=<VERSION>`); releases reload once per tab and never under an open editor; the APK only serves byte-exact, hash-addressed public assets and loads the live origin.
