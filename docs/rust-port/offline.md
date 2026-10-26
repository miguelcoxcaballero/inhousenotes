# Rust port plan: offline area

Owner files (JS bytes): offline-library-v5.js 27570, offline-app-v5.js 24897, document-processing-worker-v5.js 16020,
offline-sync-v5.js 15329, offline-store-v5.js 11268, boot-v5.js 10191, service-worker.js 4981,
document-processing-v5.js 4403, offline-media-v5.js 2998. Total 117657 B / 1890 lines.
Rust module: rust-core/src/offline.rs (currently an empty stub). Adapter: stream:offline markers in rust-core-v5.js.
Status: U1 done (see section 6). U2-U6 not started.

## 1. Inventory and classification

Legend: PORT = pure computation, GLUE = DOM/IO/lifecycle, DONE = already Rust.

### document-processing-worker-v5.js (16020 B)
| Block | Lines | Class |
|---|---|---|
| preparedPng (SHA-256 key, PNG decode through pdf-lib, deflate, LRU byte budget) | 10-35 | decode+LRU PORT, SHA-256/deflate native GLUE |
| remember/objectFor/disposeDocument (object table) | 36-47 | GLUE (holds pdf-lib objects) |
| drawStroke | 50-82 | GLUE (canvas calls) |
| compress (deflate stream + 16 KB fromCharCode(...) loop + btoa) | 84-92 | base64 PORT, deflate native GLUE |
| deflateBytes | 94-98 | GLUE (CompressionStream) |
| preparePdfRaster RGBA split | 100-118 | DONE (InhouseRust.splitRgba); JS fallback loop at 105-110 is dead weight and must be deleted |
| embedPreparedRaster | 120-133 | GLUE (pdf-lib dictionaries) |
| pack/unpackInkCheckpoint | 136-142 | DONE (checkpointCodec / unpackCheckpoint) |
| prepareMetadata (compress + canonical hash) | 145-158 | GLUE, hash itself lives in collaboration-core (not mine) |
| timelineHash (31-multiplier string hash) | 159-163 | PORT |
| decodeTimeline (atob + inflate + 96 MB guard) | 164-177 | base64 decode PORT, inflate native GLUE, materializeArchive in timeline-core (not mine) |
| renderOverlay | 178-199 | GLUE (OffscreenCanvas, fetch, bitmap) |
| contentHash | 200-202 | GLUE, delegates to collaboration-core |
| pdf* handlers (load/create/copy/add/embed/draw/rect/metadata/attach/save) | 203-262 | GLUE, vendor pdf-lib; only the Keywords ASCII regex test is pure (trivial) |
| timeline trimming loops | 263-283 | PORT (selection policy), needs encodeArchiveJson from timeline-core |
| onmessage router | 289-300 | GLUE |

### offline-library-v5.js (27570 B)
| Block | Lines | Class |
|---|---|---|
| compareEntries, modifiedTime cache | 19-25, 46-56 | PORT |
| indexCatalogue (folder map, alias map, active list) | 26-45 | PORT |
| reload merge: alias map, newer-remote overlay, parent remap, document map | 89-113 | PORT (data from IDB stays JS) |
| folder, combine | 168-180 | PORT |
| home, compareRecents, recentItems | 181-205 | PORT |
| decorate badge state, syncErrorText, redaction | 117-165 | state computation PORT, DOM GLUE |
| connectivity/probeConnection/networkAvailable | 57-87 | GLUE (fetch, sessionStorage, timers) |
| showSyncFailure, showSettings | 135-160, 330-350 | GLUE (dialogs) |
| captureQueue/capture/scheduleCapture/prepared | 207-262 | GLUE (host callbacks, IDB) |
| openRecord, pin, createFolder, observed, cache | 205-290 | GLUE |
| catalogue() Drive paging | 291-318 | GLUE (fetch); trashed-diff of found ids is PORT but trivial |
| scheduleBackground, kick (iframe worker), init, resume, useAccount, logout | 319-415 | GLUE |

### offline-store-v5.js (11268 B)
| Block | Class |
|---|---|
| open, transaction, read, list, put, patch, withLease | GLUE (IndexedDB, timers) |
| documentInfo (strip big fields) | GLUE (object projection, structured clone dominates) |
| settings/setRetention validation | PORT (trivial) |
| checkpoint record build (generation, preparedBlob reset, dirty merge, outbox row) | PORT as pure record transform |
| acknowledge (settled decision, recovery row, next record) | PORT as pure record transform; blobs stay JS |
| cacheEntries newer-wins merge | PORT (trivial) |
| prune cursor decision | PORT; note it scans the full `documents` store (clones every blob), should scan `document-index` instead |

### offline-sync-v5.js (15329 B)
| Block | Class |
|---|---|
| changed(a,b) revision comparison | PORT |
| folder op ordering by ancestor chain, deferral passes | PORT |
| receipt/recoveryKey/parent selection, error record shaping | PORT |
| multipart body assembly (boundary text, headers) | PORT for the text parts; Blob assembly GLUE |
| sameBytes SHA-256 compare | GLUE (WebCrypto is native and hardware backed) |
| run(): leases, fetch, metadata, upload, acknowledge | GLUE (network, IDB) |

### Fully GLUE files
- offline-app-v5.js (24897 B): editor integration (capture, restore, merge, worker, queueSave, saveNow). Every function touches `state`, the editor DOM, Drive fetch or IndexedDB. Nothing pure worth moving.
- document-processing-v5.js (4403 B): Worker RPC facade over pdf-lib proxies. Pure message plumbing.
- offline-media-v5.js (2998 B): Cache API + blob URL LRU (128 entries). The LRU bookkeeping is ~10 lines and not on any hot path.
- boot-v5.js (10191 B): runs before WASM exists and must paint synchronously. Contains compareVersions, readAppVersion regex, bootDestination decision and the release-check fetch; all tiny and boot-critical.
- service-worker.js (4981 B): fetch routing, Cache API, sync events. A service worker cannot be Rust-only and loading the WASM module into it would slow install/start for ~1 KB of logic.

## 2. Units (ordered by speedup, then lowest risk)

Common rules: every unit adds a differential test in tests/ using loadLegacy on the pre-port file, with fixtures plus seeded random inputs; JS algorithm deleted after; adapter between `stream:offline` markers; `await InhouseRust.ready` before use.

### U1 worker-kernels (speedup high, risk low, JS about 2.0 KB)
Scope in document-processing-worker-v5.js: `compress` base64 step (84-92), `decodeTimeline` base64 decode and size guard (164-177), `timelineHash` (159-163), RGBA fallback loop deletion in `preparePdfRaster` (105-110), PNG cache LRU planner in `preparedPng` (key to evict list, 10-35).
- Rust: `offline::b64_encode(bytes) -> bytes(ascii)`, `b64_decode`, `timeline_hash(json utf8) -> string` (hash over UTF-16 code units, length in UTF-16 units; JSON.stringify output is well formed so UTF-8 transport is lossless), `lru_plan(sizes...)`.
- Boundary: typed arrays (Uint8Array in, Uint8Array/ASCII out, no JSON for payloads); only tiny JSON for the LRU plan. Deflate/inflate stays on CompressionStream (native, faster than any no-crate Rust).
- Gain: removes the 16 KB `String.fromCharCode(...)` + btoa string churn on every autosave metadata compress, and atob + Uint8Array.from per-char callback on timeline decode (multi-MB).
- Tests: tests/document-jobs.test.mjs, tests/rust-core.test.mjs, new differential test; e2e processing-lifecycle.spec.mjs, save-cache.spec.mjs.

### U2 library-index (speedup high on large Drives, risk medium, JS about 6.5 KB)
Scope in offline-library-v5.js: `compareEntries`, `modifiedTime`, `indexCatalogue`, merge section of `reload` (alias map, remote-newer overlay, parent aliasing), `folder`, `combine`, `home`, `compareRecents`, `recentItems`, home-view alias resolution.
- Design: Rust keeps the compact catalogue in a thread-local table filled by one call per reload; JS keeps the original entry objects (cards need them) in an array. Queries return `Uint32Array` of indexes into that array, so no object round trip per card.
- Calls: `offline.catalogue_load` (JSON projection of id, remoteId, parents, mimeType, trashed, modifiedTime, starred, ownedByMe, pending, name, plus document table id/remoteId/lastOpenedAt, driveRoot remoteId, saved views), then `folder(parent, needle)`, `combine`, `home` returning index arrays.
- Parity risk: `localeCompare` for folder names. Plan: JS computes a collation rank for folder names once per load with Intl.Collator and passes ranks as a Uint32Array; Rust sorts by rank. Case-insensitive `includes` search must match JS `toLowerCase` for non-ASCII (Rust `to_lowercase` differs for a few characters such as final sigma); verify in the differential test, otherwise lowercase in JS once at load.
- Tests: tests/offline-library-index.test.mjs (it runs the source in `vm` with no InhouseRust, so the sandbox must be given the Rust stub/real wasm; update that harness, do not weaken assertions), tests/rust-folder.test.mjs, e2e offline-library.spec.mjs, offline-library-cached.spec.mjs, folder-rust.spec.mjs (read only, other area).

### U3 timeline-trim planner (speedup high, risk medium, JS about 1.6 KB)
Scope: `type === 'timeline'` handler in the worker (263-283): the two `while` loops that splice history and re-run `encodeArchiveJson` / `compress` each iteration (quadratic).
- Rust: given per-entry flags (isMilestone, kind==='baseline', index) and a target budget, produce the drop order in one pass; the JS calls the encoder once on the final set, then only repeats if the compressed size still exceeds the budget (bounded retries). Output must equal the legacy sequence of removals.
- Dependency: `encodeArchiveJson` lives in timeline-core-v5.js, owned by another area (ts_engine/collab). Do not edit it; coordinate through the report. If that area ports it to Rust first, call it from here.
- Boundary: JSON (small array of flags in, array of indexes out).
- Tests: tests/document-jobs.test.mjs, differential test against the legacy loop with a fake encoder of deterministic length; e2e processing-lifecycle.spec.mjs.

### U4 PNG raster decode (speedup high on imported PNG pages, risk high, JS about 1.2 KB plus the vendor decoder it bypasses)
Scope: `preparedPng` decode step (`PDFLib.PngEmbedder.for(bytes)` returning rgbChannel/alphaChannel).
- Rust: PNG chunk parse, inflate (hand written, std only), unfilter, 1/2/4/8/16-bit gray, RGB, palette, gray+alpha, RGBA, tRNS and Adam7 handling exactly as pdf-lib's decoder, producing rgb and alpha planes. Needs no crate; if a crate is wanted (png, miniz_oxide) record it in the report instead of adding it.
- Boundary: Uint8Array in, rgb and alpha Uint8Array out (transferable).
- Fallback: throw a clear error on unsupported variants; never silent degradation, so each pdf-lib variant must be covered before deletion, otherwise the unit is dropped and the JS stays.
- Tests: differential byte test against pdf-lib on a PNG corpus (all color types and depths), e2e scanner-processing.spec.mjs, processing-lifecycle.spec.mjs (read; verify via produced PDF bytes).

### U5 store record plans (speedup medium, risk low, JS about 3.0 KB)
Scope in offline-store-v5.js: `setRetention` validation, `checkpoint` record build (generation increment, prepared blob reset, pinned/dirty merge, outbox row), `acknowledge` settle decision and next record, `cacheEntries` newer-wins merge, `prune` selection.
- Boundary: JSON for metadata fields only. Blob/snapshot/originalBlob fields stay in JS and are re-attached after the call (Rust sees them as opaque presence flags).
- Perf fix included: `prune` must walk the `document-index` store (already holds pinned, dirty, lastOpenedAt, id) instead of cursoring `documents`, which structured-clones every PDF blob and snapshot. Same decisions, far less IO. Rust decides which keys to delete from (index rows, outbox key set).
- Tests: tests/local-save-status-v5.test.mjs, offline-library-index.test.mjs; e2e save-cache.spec.mjs, offline-sync.spec.mjs, offline-reconnect.spec.mjs.

### U6 sync planner and status view-model (speedup low, risk low, JS about 5.0 KB)
Scope: offline-sync-v5.js `changed`, folder-op ancestor filtering and deferral passes, receipt and recovery key builders, multipart header text, error record shaping; offline-library-v5.js `syncErrorText`, error redaction regex, the per-card badge phase/text decision inside `decorate` (DOM writes stay JS).
- Boundary: JSON in, JSON out. `decorate` runs over every card on each progress event, so batching card states into one call (array of ids in, array of states out) is the only part with measurable benefit.
- Tests: tests/offline-library-index.test.mjs (decorate cases), e2e offline-sync.spec.mjs, offline-worker.spec.mjs, offline-connectivity.spec.mjs, offline-reconnect.spec.mjs.

## 3. Order and status table

| # | Unit | Speedup | Risk | JS bytes | Status |
|---|---|---|---|---|---|
| U1 | worker-kernels | high | low | ~1400 removed | DONE |
| U2 | library-index | high | medium | ~6500 | todo |
| U3 | timeline-trim planner | high | medium | ~1600 | todo (depends on timeline-core owner) |
| U4 | png raster decode | high | high | ~1200 | todo (optional) |
| U5 | store record plans | medium | low | ~3000 | todo |
| U6 | sync planner / view-model | low | low | ~5000 | todo |

Portable total about 19 KB of 117 KB; the remainder is IndexedDB, Cache API, fetch, DOM, canvas, pdf-lib and service-worker lifecycle.

## 4. Not portable
- IndexedDB layer (open/transaction/read/list/put/patch/withLease), Cache API, fetch, timers, BroadcastChannel/postMessage: browser APIs.
- service-worker.js: install/activate/fetch/sync events cannot be Rust; the URL gating logic is about 1 KB and loading WASM in the worker costs more than it saves.
- boot-v5.js: executes before WASM is available and must reveal the first view synchronously; its logic is tiny.
- offline-app-v5.js and document-processing-v5.js: editor state, DOM and Worker RPC plumbing.
- offline-media-v5.js: Cache API + object URL lifetime.
- drawStroke/renderOverlay: OffscreenCanvas calls; deflate/inflate: CompressionStream is native.
- pdf-lib object handling (pdf* handlers, embedPreparedRaster, doc.save): vendor library, protected.
- ihnCanonicalDocumentHash, encodeArchiveJson, materializeArchive: live in collaboration-core-v5.js and timeline-core-v5.js, other areas.
- SHA-256 via WebCrypto: hardware accelerated.

## 5. Open parity questions
- localeCompare and toLowerCase vs Rust (U2): resolve with JS-side rank/lowercase.
- tests/offline-library-index.test.mjs loads the source in vm without InhouseRust; its harness needs the wasm injected once U2 lands.
- tests/release-v5-smoke.mjs greps some of these sources; update it per unit.

## 6. Unit status

### U1 worker-kernels: DONE
Moved to Rust (`rust-core/src/offline/worker_kernels.rs`, adapter between the `stream:offline` markers in `rust-core-v5.js`):
- base64 encode (worker `compress`) and decode (worker `decodeTimeline`): `InhouseRust.b64Encode(Uint8Array) -> string`, `b64Decode(string) -> Uint8Array`. The decoder implements the WHATWG forgiving-base64 rules of `atob` (ASCII whitespace ignored, optional padding, leftover bits dropped) and throws the same `InvalidCharacterError`. Encoding is chunked at 24 MB so the 128 MB WASM allocation cap never limits payloads.
- `timelineHash`: `InhouseRust.timelineHash(json) -> "<abs(hash) base36>:<length base36>"`. JSON goes into WASM memory as UTF-8 via `TextEncoder.encodeInto`; strings with lone surrogates (cannot be UTF-8) travel as raw UTF-16 units, so every input hashes exactly as the old loop did. ASCII text is hashed four units per step.
- Prepared-PNG cache eviction plan: `InhouseRust.pngCachePlan(limit, sizesOldestFirst, incoming) -> {store, evict}` (bridge method `offline.png_cache_plan`). The worker keeps the `Map` (it holds the rasters) and drops the first `evict` keys. The running byte counter was removed.
- Dead JS RGBA split fallback in `preparePdfRaster` deleted. A missing core now throws `Rust core is unavailable; reload the app` from every worker task that needs a kernel (`compress`, `timelineHash`, `decodeTimeline`, `preparedPng`, `renderOverlay` with `pdfRaster`).
- Stayed JS (browser APIs): CompressionStream/DecompressionStream deflate/inflate, SHA-256 key of the PNG cache (WebCrypto), the expansion guard in the inflate read loop, the 48 MB payload guard (one comparison).

Behavior notes (none changes output bytes):
- Cache accounting: the old counter double counted when two concurrent `preparedPng` calls decoded the same bytes (second `set` replaced the entry but added its size again, so the cache could evict early forever after). The plan now sums the real entries and replaces the key first. Identical to the old behavior in every non-concurrent sequence (differential test).
- Error text of an invalid payload is the engine's own `atob` wording in the old code and `The string to be decoded is not correctly encoded.` now; the exception name `InvalidCharacterError` is identical. Callers only log it.
- A page raster over 128 MB RGBA (about 32 megapixels) used to be handled by the JS loop; `splitRgba` cannot allocate it and the export now fails loudly. Overlays are bounded by the page size, far below that.

Tests: `cargo test` (8 new tests), `tests/offline-worker-kernels.test.mjs` (differential against the pre-port worker loaded from git and driven through its `onmessage` router: ~400 compress inputs, 2300 btoa buffers, 6000 mutated atob strings, 3000 timelineHash strings with lone surrogates, 120 decodeTimeline payloads, 3000 PNG cache operations with eviction, 40 RGBA rasters), `tests/offline-worker-kernels.bench.mjs` (micro-benchmark). Existing `document-jobs`, `offline-library-index`, `local-save-status-v5`, `rust-core`, `rust-folder`, `release-v5-smoke` unchanged and green. E2E `processing-lifecycle.spec.mjs` and `save-cache.spec.mjs` green (8/8).

Bytes: worker JS 16020 -> 15673 (net -347; about 1400 B of algorithm removed, about 1050 B of glue added). Rust added 13.5 KB source (of which about 5 KB tests). `rust-core-v5.js` adapter +5.2 KB of marshalling glue.

Measured (node 22, V8, shared 4-core box, median of 7; `node tests/offline-worker-kernels.bench.mjs`):

| kernel | JS ms | Rust ms | speedup |
|---|---|---|---|
| b64 encode 1 MB | 66 | 3.1 | 22x |
| b64 encode 8 MB | 523 | 23 | 22x |
| b64 encode 32 MB | 2325 | 168 | 14x |
| b64 decode 1 MB | 181 | 7.4 | 24x |
| b64 decode 8 MB | 1436 | 47 | 30x |
| b64 decode 32 MB | 4302 | 198 | 22x |
| timelineHash 0.1 MB | 1.04 | 0.18 | 5.9x |
| timelineHash 1.2 MB | 2.4 | 1.6 | 1.5x |
| timelineHash 5 MB | 11.3 | 6.1 | 1.9x |

The hash gain is small because V8 already JIT-compiles the old loop well; the cost left is marshalling the string into WASM memory. The base64 gain is large because the old code built a binary string 16 KB at a time with spread arguments, and decoded with a per-character callback.
