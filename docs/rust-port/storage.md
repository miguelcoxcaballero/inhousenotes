# Rust port plan: storage (Drive sync, local storage, persistence)

Branch `rust/storage`. Status: U1 done (see section 0); U2-U6 planned, nothing else moved yet. Line numbers refer to `app-v5.js` at the branch base.
Rust module: `rust-core/src/storage.rs`. JS adapter: between `stream:storage` markers in `rust-core-v5.js`.

## 0. Status

| Unit | Status | JS bytes (app-v5.js) | Rust bytes added | Measured speedup |
|---|---|---|---|---|
| U1 Ink page checkpoint packing | done | pack/unpack 5667 -> 7133 (glue grew, see below) | storage.rs 9456 (incl. tests), adapter 2758 in `rust-core-v5.js` | pack 2.8-3.5x, unpack 1.4-4.8x (node micro-benchmark) |
| U2-U6 | not started | | | |

### U1 details
- Rust (`storage.rs`, module `ink_words`, exports `ink_pack_words` and `ink_validate_words`): per-stroke portability decision from
  shape tags, word layout (`x, y, p, hasP` as four f64, offsets in words), `inkRange` offsets with per-stroke rewind, and the
  unpack range/pressure-flag validation (`Number.isSafeInteger`, `offset % 4`, bounds, flag in {0,1}; stroke order and messages preserved).
- JS glue that must stay: reading the live point objects (`typeof`, `'p' in`, `Object.keys(point).length` to detect extra keys such as
  `tiltX`) into a `Float64Array` plus one tag byte per point; yielding, staleness and session checks; `structuredClone` of stroke
  attributes and of non-portable strokes (lossless fallback); building the `{x,y,p?}` objects in unpack; the worker dispatch.
- The JS per-point `performance.now()` call is now made every 128 points (the 3 ms budget is unchanged).
- JSON was rejected as the boundary: `JSON.stringify` of 200k points costs ~130 ms against ~25 ms for the whole original loop, and it
  loses `-0`, NaN and Infinity, which `tests/e2e/ink-checkpoint.spec.mjs` requires to survive. Typed arrays keep every value bit-exact.
- JS bytes did not shrink: the algorithm text (validation chain, word fill, range and flag checks, about 1.1 KB) was deleted but the
  explicit extraction/materialisation loops that Rust cannot do (it cannot read JS objects) are larger than the old compact loops.
  The gain is speed (no `Object.keys` array, closure and `performance.now()` per point) and a tested Rust core, not source size.
- Known intentional differences (all unreachable for data the app produces): a stroke destructured with a non-iterable `inkRange`
  throws its `TypeError` before an earlier stroke's invalid-range error; a point whose `x`/`y` come from the prototype chain is
  judged by own-key count. Packing fails loudly without WASM (`Rust core is required to pack ink checkpoints`).
- Parity doubt for the integrator: `tests/e2e/ink-checkpoint.spec.mjs` (modes `no-wasm`, `no-worker`) requires saved pages to stay
  readable when WebAssembly cannot load, so `InhouseRust.validateInkWords` keeps a 10-line JS range/flag check used only when the
  core is absent (read path only; same precedent as the JS xor decoder in `unpackCheckpoint`). Remove it together with that test
  mode if policy says reads must also fail loudly. Differential test covers this path too.
- Other test notes: no test greps removed source text. `tests/native-assets.test.mjs` fails because `app-v5.js` size changed
  (generated `native-assets.json`, rebuilt by the integrator). `app-resilience` scanner specs (308, 371, 443, 800) are flaky or
  failing on the base commit as well (800 fails at base; 308 failed at base once, passed on this branch).
- Tests: `tests/storage-ink-checkpoint.test.mjs` (differential vs the original text from git: fixtures, 1500 random pack pages with
  hostile points, 3000 random unpack pages with corrupt ranges/flags, stale-session checkpoints), `cargo test` in `ink_words`,
  `tests/storage-ink-bench.mjs` (benchmark).

## 1. Inventory and classification

Section extents (bytes of JS): 4.17a Drive HTTP client 7466-7590 (6 KB); 4.24 Auto-sync 18609-19132 (27 KB);
4.18b View-state/token + IndexedDB part 1 21314-23471 (108 KB); 4.18 Local storage 23471-25752 (108 KB);
4.17 Drive sync & save 25752-26885 (58 KB). Total about 307 KB. Over 90% is glue: it is IndexedDB transactions,
timers, fetch, DOM status text and cross-module state mutation. Pure computation is a small, scattered set.

### 4.17a Drive HTTP client (7466-7590)
- `driveFetch` 7469-7557 (3.9 KB): glue (fetch, AbortController, timers, token refresh). Only the status/err classification is portable (-> U3).
- `parseFilenameFromContentDisposition` 7558: pure (U3).
- `isPdfArrayBuffer` 7572: pure byte scan (U3, typed array in, bool out).

### 4.24 Auto-sync (18609-19132) - this is the calendar side-panel sync, not the Drive poll
- `spStartAutoSync`/`spStopAutoSync` 18942-18967: glue (setInterval 5 s, gate checks).
- `spRebuildAllPanels`, `spRefreshPanelsForKeys`, `spSaveEvent`, `spDeleteEvent`, `spAutoAttachCalendarPanel`,
  `reconcileConcurrentCalendarPanels`, `spSyncAllPanels`, `calendarFetch`, `fetchCalendarEventsForMonth`,
  `prefetchNeighborCalendarMonths`, `loadCalendarEventsForVisibleMonth`: DOM + network glue. Calendar-event merge logic
  belongs to the calendar stream (`calendar.rs`); not touched here. No portable unit owned by storage.

### 4.18b View-state & token persistence (21314-23471)
- View state/mode save+load 21318-21369: glue around localStorage; validation (3 numbers) trivial -> U5.
- Token storage 21370-21430 (`hasGoogleScope`, `normalizeGrantedScopes`, token JSON build/validate): small pure parts -> U5; localStorage calls stay JS.
- Drive session `saveDriveSession`/`loadDriveSession` 21431-21458, 21675-21728: pure payload build/normalise (U5); state writes stay JS.
- Pending-exit-upload marker + lifecycle exit save 21467-21674: glue (async, localStorage, state). JSON marker shape only (U5).
- `formatTime` 21752: relative "N min" branch pure, `toLocaleTimeString` needs Intl -> glue. `updateSaveTimestamp` DOM glue.
- IndexedDB open/save/load (`openNotebookDb`, `saveToIndexedDb`, `loadFromIndexedDb`, timeline store, `savePageToIndexedDb`,
  `loadPageFromIndexedDb`, stroke-ops append/load/clear, `clearPageStore`, `deletePageFromIndexedDb`,
  `persistPendingCollabPageStoreRewrite`, `reindexPagesAfterDelete`): IndexedDB transactions, glue.
- `sanitizePageForStorage` 22039, `buildPageMeta` 22190, `stripStoredPageSession` 22183: record shaping, portable decision logic but operate on live JS objects (U4).
- `packInkPageForStorage` 22070-22122 and `unpackInkPageFromStorage` 22123-22155: mixed. The Float64 word packing/unpacking loops are
  pure (U1); yielding, interaction settling, session tokens and worker dispatch are glue. Codec itself is already Rust (`checkpoint.rs`, `InhouseRust.unpackCheckpoint`) = already ported.
- `cloneSanitizedPageForStorage` 22156: structuredClone slicing with UI yields; browser API, glue.
- `getStrokeOpPersistenceKey` 22428, `capture/retirePendingStrokeOpKeys` 22435-22451: tiny pure key/set logic (U4).
- `reindexPageIndexAfterDelete` 22962, `reindexLocalCachesAfterDelete` 22970-23044, `trimLocalCachesToPageCount` 23058: index arithmetic is pure; the Map/WeakMap/DOM cache updates are glue (U4 for the arithmetic).
- Scroll preview queue 23134-23261, `applyPagePreview`, `renderPagePreview`: canvas/DOM, glue.
- `serializePayload`/`saveToLocalStorage`/`scheduleLocalStorageBackup`/`flushLocalStorageBackup` 23262-23332: JSON.stringify + localStorage + idle callbacks, glue (see notPortable).
- `buildMetaPayload` 23333: page-meta map and constant fields (U4).
- `flushDirtyPages` 23345, `hydrateLocalPendingPagesForRemoteMerge` 23428: orchestration over IndexedDB, glue.

### 4.18 Local storage & IndexedDB (23471-25752)
- `saveToStorage`, `loadFromStorage` 23475-23650: glue orchestration.
- `normalizeLegacyPage` 23651-23683: pure shaping of old payload pages (U4).
- `migrateLegacyPayload` 23684-23901 (9 KB): orchestration over IndexedDB and state; glue. Only `normalizeLegacyPage`/`buildPageMeta` calls are portable.
- Page load/unload/preview eviction 23902-24141 and stroke-op queue 24142-24356: IndexedDB + timers, glue. `markPageDirty`/`markAllPagesDirty` 24357-24404: Set/Map mutation, glue.
- Save status 24405-24467 (`getLocalSaveStatus`, `noteLocalSaveFailure`, `confirmLocalSave`, `scheduleLocalSaveRetry` delay): small state machine + backoff (U6, U3 for delay).
- Session token/context 24501-24651, interaction/pen idle timers 24652-24910: timers and event state, glue.
- `hasPendingLocalSave`, `hasPendingDriveSave`, `hasUnsyncedLocalDriveChanges`, `isDriveSyncPending`, `shouldPreserveLocalUnsyncedForPull` 24468-24477, 24911-24955: boolean decision tables over many flags (U6).
- Prepared PDF scheduling 24956-25075 and `prepareDrivePdfBlob`: glue (PDF build belongs to export stream).
- Drive version bookkeeping 25076-25146 (content-version counters, hydration flags): trivial state, stays JS.
- ETag/revision bookkeeping 25147-25375: `getRemoteMetaModifiedTime`, `normalizeDriveVersion`, `compareDriveVersions`, `canApplyDriveRemoteMeta`,
  the decision half of `applyDriveRemoteMeta`, `chooseNewerRemoteMeta`, `isRemoteMetaNewerThanTracked`, `driveRevisionMetaMatches` 25502: pure (U2).
- `deferRemotePull`, `flushDeferredRemotePull` 25286-25338: async orchestration; glue.
- Drive revision fetch (`fetchDriveRevisionMeta`, `readDriveBlobResponse`, `fetchDriveRevisionBlob`, `fetchStableDrivePdfSnapshot`) 25376-25501: fetch/stream glue.
- `ensureDriveHydratedBeforeUpload`, `scheduleSave`, `queueSave`, `runLocalCheckpoint`, `flushOnClose` 25527-25751: orchestration, glue.

### 4.17 Drive sync & save (25752-26885)
- Timer scheduling `clearDriveSyncTimers`, `scheduleDeferredDriveSaveRetry`, `scheduleDriveSync` 25756-25861: decision cascade (which delay / defer reason) is pure given a flag snapshot (U6); timers glue.
- `ensureDriveFile`, `createDriveFileInFolder`, `updateDriveFile` 25862-25930: network glue.
- `getDriveResumableAcknowledgedOffset` 25937 and the `uploadDriveBlobResumable` retry/ack/no-progress policy 25943-26112 (9.7 KB): the fetch/Blob loop is glue, the step decision is a pure state machine (U3).
- `hasRemoteActiveSaveSignal`, `waitForRemoteSaveQuietBeforeUpload`, `mergeLatestRemoteBeforeDriveUpload` 26113-26180: glue.
- `queueDriveSave`/`queueDriveSaveOnline` 26191-26616 (~20 KB): glue orchestration (merges, PDF build, upload, status text).
- Collab poll state 26617-26884: `scheduleCollabPullRetry` backoff arithmetic pure (U3); `pollDriveRevision` glue (fetch + decision via U2); `restartDriveSaveForRemoteChange`, fast catch-up timers, start/stop polling: glue.

Totals: already ported about 1% (checkpoint codec); portable pure code about 60-70 KB of the 307 KB (20-25%); glue about 75%.

## 2. Units (ordered by speedup, then lowest risk)

Each unit: Rust in `storage.rs` (submodules inside), JS adapter in the `stream:storage` markers, differential test
`tests/storage-<unit>.test.mjs` using `loadLegacy('app-v5.js')`-style extraction against seeded random inputs, original JS deleted.

### U1 Ink page checkpoint packing (speedup high, risk medium) ~6 KB
- Scope: `packInkPageForStorage` 22070-22122 (point validation + Float64 word fill + `inkRange` rewrite) and
  `unpackInkPageFromStorage` 22123-22155 (word bounds checks, pressure flag decode, point object rebuild).
  Yielding (`pause`, `waitForHotInteractionToSettle`, session assertions) stays JS, run between Rust chunks of N strokes (about 3 ms budget kept via chunk size).
- Why first: runs on every page checkpoint with tens of thousands of points; the per-point `Object.keys`, `typeof` and `performance.now()` calls dominate.
- Boundary: pack takes a chunk of strokes as JSON (`[{...attrs, points:[{x,y,p?}]}]`) and returns attribute JSON with `inkRange` plus a
  Float64 word buffer appended into one shared `Float64Array`; non-portable strokes (extra point keys) return a flag and keep the structuredClone path. Unpack takes the
  decoded Float64 bytes (typed array) plus stroke ranges (JSON) and writes points into a JSON array or, preferably, returns a Float64 array of x,y,p,flag per stroke that JS wraps lazily. Bytes are byte-compatible (`raw-v1`/existing codec word layout 4 x f64: x, y, p, hasP).
- Parity risk: point objects must keep key order and `p` presence; NaN/Infinity words; error messages for invalid ranges.
- Tests: `save-cache.spec.mjs`, `save-priority.spec.mjs`, `offline-sync.spec.mjs`, `app-resilience.spec.mjs`, `tests/rust-core.test.mjs` (checkpoint), new differential test over random strokes including malformed ones.

### U2 Drive revision/ETag ordering (speedup low-medium, risk low) ~9 KB
- Scope: `getRemoteMetaModifiedTime` 25147, `normalizeDriveVersion` 25153, `compareDriveVersions` 25158, `canApplyDriveRemoteMeta` 25167,
  decision part of `applyDriveRemoteMeta` 25209 (returns which of head/modified/version fields to adopt), `chooseNewerRemoteMeta` 25258,
  `isRemoteMetaNewerThanTracked` 25339 (with explicit `now`, own-write meta and constants passed in), `driveRevisionMetaMatches` 25502.
- Runs on every 0.6 s collab poll and each save; main value is correctness and testability. Arbitrary-length digit strings compare as strings, as in JS.
- Boundary: JSON in, small JSON out (`callJson('storage.canApplyRemoteMeta', {meta, tracked})`). Dates parse ISO-8601 in Rust (must match `new Date(x).getTime()` for Drive's RFC3339 millisecond form; non-parseable returns 0).
- Tests: `offline-sync.spec.mjs`, `app-resilience.spec.mjs`, `home-save-navigation.spec.mjs`; differential test with random meta pairs (versions with leading zeros, empty, non-numeric; equal timestamps).

### U3 Retry, backoff and upload-step planning (speedup low-medium, risk medium) ~25 KB
- Scope:
  a. `driveFetch` outcome classification 7469-7557 (401-once retry decision, accepted-status check, `Drive API error N` message, auth-regex
     `/auth|token|sign.?in|refresh|No refresh token|Silent token/i`, timeout/abort mapping).
  b. `getDriveResumableAcknowledgedOffset` 25937 and a pure step function for `uploadDriveBlobResumable` 25943-26112:
     `next(state, event) -> action` covering chunk size (256 KiB multiples, 1 MiB), `Content-Range` strings, no-progress counter (>3 stall), retry counter (>3), 404/410 session expiry, backoff `min(1500, 250*retry)`, completion detection. The async fetch loop stays JS.
  c. Delay calculators: `scheduleLocalSaveRetry` 24449 (`min(30000, 800*2^n)`, attempts cap 6, deferred 400), `scheduleCollabPullRetry` 26639
     (`min(15000, 700*2^(n-1))` with jitter factor supplied by JS `Math.random()`), `scheduleDriveSync`/`scheduleDeferredDriveSaveRetry` delay selection 25763-25861.
  d. `parseFilenameFromContentDisposition` 7558, `isPdfArrayBuffer` 7572.
- Boundary: JSON for state/event/action; `isPdfArrayBuffer` takes a `Uint8Array` view of the first 1024 bytes through `callBytes`.
- Tests: `offline-sync.spec.mjs`, `save-status.spec.mjs`, `save-priority.spec.mjs`, `app-resilience.spec.mjs`; differential tests on header strings and event sequences. Browser upload specs mock Drive; the resumable path is verified by a simulated-server unit test comparing the JS loop (legacy) against the Rust-driven loop on identical scripted responses.

### U4 Persisted page record shaping (speedup medium for large docs, risk medium) ~8 KB
- Scope: `sanitizePageForStorage` 22039 and `buildPageMeta` 22190 (field defaults, `backgroundSource` inference, blob: background drop, finite checks),
  `stripStoredPageSession` 22183, `normalizeLegacyPage` 23651, page-map part of `buildMetaPayload` 23333, `getStrokeOpPersistenceKey` 22428 (key string choice),
  `reindexPageIndexAfterDelete` 22962 and the index remap table behind `reindexLocalCachesAfterDelete` 22970-23044 (JS applies the remap to Map/WeakMap caches).
- Dependencies owned by other streams are passed as arguments, not ported: `getPageDimensions`, `normalizeTemplateKind`, `ihnNormalizeDeletionStamps`, `generatePageId`, `generateLegacyDocumentPageId`, `shouldRenderPageCalendarPanels`, `getVisibleStrokeCount`. JS resolves them and passes plain values (`{width,height,templateKind,deletedStrokeStamps,generatedId,renderPanels}`); Rust only does the shaping. Heavy members (`strokes`, `images`) are never serialised across the boundary: Rust returns the scalar fields and JS assembles the object with the same array references.
- Boundary: JSON for scalar fields (small), indices as Int32Array for remap.
- Persisted formats stay byte-identical (key order matters for structuredClone-equality tests, so output field order is fixed to the original).
- Tests: `save-cache.spec.mjs`, `home-save-navigation.spec.mjs`, `offline-library*.spec.mjs`, `tests/local-save-status-v5.test.mjs`, differential tests with random page objects.

### U5 Session, token and view-state persistence codecs (speedup low, risk low) ~22 KB
- Scope: payload build and validation for `saveDriveSession`/`loadDriveSession` 21431, 21675 (field normalisation, `canEdit`/`ownedByMe` fallbacks),
  `saveViewState`/`loadViewState` 21318-21352 (3-number validation), `applyViewState` zoom clamp 21729,
  token payload build/validate (`saveDriveTokenToStorage`, `loadDriveTokenFromStorage` expiry window 60 s, scope-version checks,
  `hasGoogleScope`, `normalizeGrantedScopes`) 21370-21430, pending-exit-upload marker parse/build 21467-21499.
  localStorage reads/writes, `state.*` assignment and timers stay JS.
- Boundary: JSON string in (raw localStorage value), normalised JSON out; `now` passed explicitly.
- Tests: `save-timestamp-format.spec.mjs`, `home-save-navigation.spec.mjs`, `offline-sync.spec.mjs`, `app-resilience.spec.mjs`; differential on corrupted/legacy session blobs. Stored key names and JSON key order unchanged.

### U6 Save-status and pending-flag decision tables (speedup low, risk medium) ~6 KB
- Scope: `getLocalSaveStatus`, `noteLocalSaveFailure`, `confirmLocalSave` 24415-24447 (error source precedence), `hasPendingLocalSave` 24911, `hasPendingDriveSave` 24924,
  `hasUnsyncedLocalDriveChanges` 24934, `shouldPreserveLocalUnsyncedForPull` 24950, `isDriveSyncPending` 24468, `formatTime` relative branch 21752.
  JS passes a flag snapshot object and applies the result.
- Risk: called on hot paths and watched by source-text tests; per-call JSON crossing could cost more than the JS it replaces, so this unit is only worth doing as a batched `snapshot -> {pending, status, label}` call per status refresh, not per query. Lowest priority; skip if measurement shows no gain.
- Tests: `tests/local-save-status-v5.test.mjs` (greps source; update per protocol), `save-status.spec.mjs`, `save-timestamp-format.spec.mjs`.

## 3. Order of work
1. U1 (largest user-visible win: smoother checkpoints while writing)
2. U4 (shared by U1 caller, reduces checkpoint work)
3. U2 (low risk, safety critical for multi-device sync)
4. U3
5. U5
6. U6 (only if measured benefit)

## 4. Honest expectation
The storage area is dominated by IndexedDB, fetch, timers and cross-module state; Rust can take about 20-25% of the JS bytes
(roughly 60-70 KB) and only U1 changes frame-time. Rust added is expected to be larger than JS removed per unit (serde types and tests).

## 5. Not portable
See `notPortable` in the structured report: IndexedDB, fetch/AbortController, localStorage, timers/idle callbacks, DOM status, canvas previews,
Intl time formatting, structuredClone of live objects, `JSON.stringify` of the backup payload, calendar auto-sync (calendar stream).
