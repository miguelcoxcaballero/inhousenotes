# Rust port plan: collab (collaboration, runtime, security, peer transport)

Owned JS: collaboration-core-v5.js (28.8 KB, 702 lines, all pure), live-collaboration-v5.js (177 KB, 4042 lines, mostly
WebRTC/Drive/timer glue), runtime-core-v5.js (9.5 KB, glue), security-core-v5.js (7.9 KB, mixed), peer-transport-v5.js (6.2 KB, mixed).
Rust: rust-core/src/collab.rs is an empty stub (nothing ported yet). Adapter goes between the "stream:collab" markers in rust-core-v5.js.
Boundary rule: JSON strings in/out via InhouseRust.callJson('collab.*') for structured data; the document hash takes the page JSON as one string
(avoids a JS-side object walk). Stateful pieces (Receiver, snapshot cache, stroke reassembly) keep state in Rust behind integer handles.
All callers must `await InhouseRust.ready`. The core modules are currently synchronous and loaded as plain scripts / required from node,
so the adapters need a sync entry once ready; tests load them through tests/helpers/legacy.mjs for differential comparison.

## 1. Classification

| Area / function | Size | Class |
|---|---|---|
| collab-core: ihnHashString, ihnStableValue/Stringify (19-49) | ~1 KB | pure, portable |
| collab-core: stamps + structure meta (50-300: Normalize*, Record*, ChooseStructureEntry, CanonicalStructurePayload, StructureHash) | ~9 KB | pure, portable |
| collab-core: ihnMergeStructureMeta (302-375) | ~3 KB | pure, portable |
| collab-core: item/deletion merge (376-458) | ~3 KB | pure, portable |
| collab-core: field meta (459-582) | ~4.5 KB | pure, portable |
| collab-core: canonical page + document hash (583-650) | ~3 KB | pure, portable, hottest |
| collab-core: ihnComputeReconnectDelay, ihnChooseMaterializableFallbackPageId (651-680) | ~1 KB | pure, portable |
| peer-transport: Receiver.accept/remove/prune (55-117) | ~3.5 KB | pure state machine (clock injected), portable; setTimeout arm stays JS |
| peer-transport: send chunking (35-52) | ~1 KB | split portable; waitForCapacity/channel.send/crypto.randomUUID glue |
| peer-transport: waitForCapacity (7-33) | ~1 KB | glue (events, timers) |
| live: snapshot cache + delta (3179-3246) | ~3.5 KB | pure, portable, high value (stringifies every page) |
| live: stroke/erase sanitizers + packet reassembly (1427-1449, 1583-1607, 1680-1860 data logic) | ~9 KB | mostly pure; canvas preview calls stay JS |
| live: signal/mailbox codecs (315-326, 606-647, 741-776, 1052-1076) | ~4 KB | pure, portable |
| live: delay schedulers + guards (2390-2399, 3348-3355, 3764-3809, 1251-1262 retry state) | ~3 KB | pure, portable |
| live: ihnEncodeSignal/Decode/key mgmt (334-580, 580-724) | ~14 KB | glue: WebCrypto AES-GCM, Drive property/comment fetch |
| live: rendezvous, ICE, offers/answers, channels, supervisor, polling, lifecycle (857-3110) | ~110 KB | glue: RTCPeerConnection, timers, fetch, BroadcastChannel, localStorage, DOM events |
| live: broadcast/apply/handleWireMessage/ApplyLiveEnvelope (3322-4042) | ~38 KB | glue around state/DOM (state.pages, sanitizePageForStorage, UI); only the decision helpers above port |
| runtime-core: fetchWithDeadline, settleWithTimeout, runWithRetries, delay, CoalescingQueue, WorkerRpcClient | 9.5 KB | glue (Promise, AbortSignal, fetch, Worker, timers); only the backoff arithmetic in runWithRetries is portable (tiny) |
| security-core: isTrustedPdfUrl, hardenPdfDocumentParams, byte-length policy | ~3 KB | pure policy (needs URL parsing in Rust, or JS passes parsed origin/protocol); portable |
| security-core: sanitizeCalendarPanelHtml, assertSafePdfBlob, loadPdfDocument | ~4 KB | glue: DOM/Blob/pdfjs |
| Already ported | 0 | none |

## 2. Units (ordered by speedup then risk)

### U1 canonical-hash: hashing + canonical document hash (high, low risk, ~6 KB JS)
Scope: collaboration-core-v5.js ihnHashString, ihnStableValue, ihnStableStringify (19-49); ihnCloneFieldValue, ihnNormalizeFieldMeta, ihnFieldMetaHash
(459-487, 573-582); ihnCanonicalContentItem, ihnCanonicalLivePage, ihnCanonicalDocumentHash (583-649) with the structure payload it needs
(ihnCanonicalStructurePayload/ihnStructureHash 281-301, which pull ihnNormalizeStructureMeta 71-130; port those normalizers here, U2 reuses them).
Run on every broadcast/apply (live 3305, 3883, 3921, 3995), so it is the hottest path. Note JS number formatting inside stable stringify
must match JS exactly (use a ryu/ES-compatible f64 to string; test on floats like 1e21, -0, 0.1+0.2).
Boundary: JSON text in (pages array, structure meta, calendarPageConfig, exportName, fieldMeta), hash string out. Stroke point arrays are the bulk: keep
them as a JSON string rather than building JS objects twice.
Tests: tests/collaboration-core-v5.test.mjs, live-collaboration-v5.test.mjs (hash assertions), new differential test over seeded random pages vs loadLegacy.

### U2 structure CRDT (medium, medium risk, ~12 KB)
Scope: ihnNormalizeCollabStamp .. ihnMergeStructureMeta (50-375): stamps, rank math (ihnStructureRankForIndex, Rebalance, MIN_GAP float math),
ihnRecordPageAdded/Moved/Deleted/StructureReplacement, ihnChooseStructureEntry, ihnIsPageTombstoned, ihnMergeStructureMeta, ihnChooseMaterializableFallbackPageId.
Float rank rounding and Date.now defaults: pass `now` explicitly from the JS adapter. Sort order must be code-unit order like JS (not Rust char order for non-BMP ids).
Boundary: JSON (meta + page id arrays) in, JSON out; small payloads, speedup mainly via sharing the normalizer with U1.
Tests: collaboration-core-v5.test.mjs (628 lines, covers merge convergence), chaos spec tests/e2e/collaboration-chaos.spec.mjs.

### U3 item/deletion/field merge + reconnect delay (medium, medium risk, ~9 KB)
Scope: ihnGetItemStamp, ihnChooseConcurrentItem, ihnNormalizeDeletionStamps, ihnMergeDeletionStamps, ihnDeletionWinsItem (376-458);
ihnSeedFieldValue, ihnRecordFieldValue, ihnChooseFieldRecord, ihnMergeFieldMeta, ihnResolveFieldValue (488-572); ihnComputeReconnectDelay (651-656).
Boundary: JSON in/out; fingerprints are strings. Reconnect delay returns an integer (use f64 rounding half away like Math.round: round half up for positives).
Tests: collaboration-core-v5.test.mjs, chaos spec, differential random merges.

### U4 snapshot cache + delta builder (high, medium risk, ~4 KB)
Scope: live-collaboration-v5.js ihnSerializeSnapshotPage, ihnRememberLiveSnapshot, ihnBuildPeerSnapshot (3179-3246) with IHN_LIVE_SNAPSHOT_CACHE_LIMIT / MAX_CHARS eviction.
Each page serialised via the U1 stable stringifier (avoids JS stringify on large stroke lists, then compares per-page strings).
State: Rust-side LRU keyed by contentHash (handle-free singleton, cleared on stopLiveCollaboration at 3111-3178 through a `reset` call).
Boundary: snapshot JSON string in; JSON out ({partial, baseHash, pages indices} so JS rebuilds the page array without re-serialising: return changed page indices).
Tests: live-collaboration-v5.test.mjs (delta/nack cases), live-handshake.spec.mjs.

### U5 realtime stroke/erase packet logic (medium-high, medium risk, ~9 KB)
Scope: ihnSanitizeLiveStrokePoints, ihnSanitizeLiveStrokeStamp (1427-1449), ihnSanitizeLiveEraseId/Stroke (1583-1607), the sequence/offset/final-slot
reassembly of ihnHandleRealtimeStrokePacket and ihnHandleRealtimeErasePacket (1680-1860), ihnRememberRealtimePacket backlog trimming (1463-1475).
Canvas preview publish/clear calls (clearRemoteLive*, publishLive*Preview 1488-1679) stay JS. Pure part: validate packet, dedupe by sequence, assemble points
into a Float32Array/Float64 buffer, enforce MAX_TOTAL_POINTS and ERASE limits, decimate to 1200 points with round(index*stride).
Boundary: points as Float64Array (x,y,p triplets) via typed pointers; metadata as JSON. Number coercion (Number(x)||0, NaN, strings) must be done in the adapter or
mirrored precisely.
Tests: live-collaboration-v5.test.mjs (stroke/erase sections), e2e collaboration-chaos.spec.mjs; keep a JS adapter test (vm harness at test line ~140).

### U6 peer-transport framing (medium, low risk, ~4.5 KB)
Scope: peer-transport-v5.js Receiver (55-117: limits, reserved accounting, TTL pruning, duplicate/conflicting chunk handling, JSON.parse of the assembled text)
and the chunk plan inside send (35-52). Clock passed in per call (`now`), so ttl is deterministic; setTimeout arming and unref remain JS.
Boundary: Receiver handle (int) + JSON message text in, assembled payload as JSON string out (parse once in JS). Chunk plan returns start frame + slices count;
JS slices strings (UTF-16 offsets are JS strings, so splitting by chars is done in JS or Rust must count UTF-16 units: keep slicing in JS).
Tests: tests/peer-transport.test.mjs plus transport sections in live-collaboration-v5.test.mjs.

### U7 signalling codecs and timing policy (low, low risk, ~6 KB)
Scope: ihnBytesToBase64Url/ihnBase64UrlToBytes (315-326), ihnFailedSignalId/Prune/Remember/ShouldSkip (606-647), ihnMailboxPeerHash/PrefixFor/KeyFor/EncodeMailboxValue/
PropertyFits/DecodeMailboxValue/PruneMailboxSeen (741-776), ihnSerializeIceCandidate/IceCandidateKey (1052-1076), ihnNextSignalPollDelay arithmetic (2390-2399),
ihnLiveBroadcastRetryDelay (3348-3355), ihnTrackMergeCurrentState/ihnShouldQueueMergeUpload/ihnPeerAlreadyHasLiveHash (3764-3809).
Depends on simpleHash (owned elsewhere): call stays JS or confirm its owner and take the hash as parameter. AES-GCM encrypt/decrypt stays WebCrypto.
Boundary: JSON / small strings. Speedup is small; do this last, mainly for language share.
Tests: live-collaboration-v5.test.mjs, live-handshake.spec.mjs.

### U8 security + runtime policy (low, low risk, ~3.5 KB)
Scope: security-core-v5.js isTrustedPdfUrl, hardenPdfDocumentParams (policy part), byteLengthOfPdfData size checks (81-147); runtime-core-v5.js runWithRetries backoff arithmetic.
URL parsing: adapter passes `new URL()` fields (origin, protocol) and Rust applies the allow-list, or Rust parses itself; to stay identical to WHATWG URL prefer the adapter route.
Boundary: JSON. Tests: tests/pdf-clean-original-cap.test.mjs, runtime-core-v5.test.mjs, tests/release-v5-smoke.mjs (greps security-core source text; update).

## 3. Order of work
U1 -> U4 -> U2 -> U3 -> U5 -> U6 -> U7 -> U8. U1 shares normalizers with U2; land them in the same Rust submodule (rust-core/src/collab/*.rs, mod declared inside collab.rs).
Each unit: cargo test, differential test via loadLegacy, replace JS with call, delete JS, run the node tests above and only collaboration-chaos.spec.mjs / live-handshake.spec.mjs through e2e.sh.

## 4. Not portable
- live-collaboration-v5.js WebRTC lifecycle, rendezvous/ICE/offer/answer, supervisor timers, Drive comment/property fetches, BroadcastChannel/localStorage leader election: browser APIs (about 110 KB).
- ihnEncodeSignal/ihnDecodeSignal key handling: WebCrypto AES-GCM keys are non-extractable browser objects and calls are async.
- ihnApplyLiveEnvelope/ihnHandleWireMessage/broadcast: orchestration over app state (state.pages, sanitizePageForStorage, UI previews); only extracted decision helpers move.
- runtime-core-v5.js fetchWithDeadline, WorkerRpcClient, CoalescingQueue, delay: fetch, AbortSignal, Worker, Promise scheduling.
- security-core-v5.js sanitizeCalendarPanelHtml (DOM tree walk), assertSafePdfBlob (Blob), loadPdfDocument (pdfjs).
- peer-transport-v5.js waitForCapacity and channel.send: RTCDataChannel events/timers.
- tests that run live-collaboration inside a vm harness remain JS adapter tests.

## 5. Progress

### U1 canonical-hash: DONE
Ported to rust-core/src/collab/{json,canon}.rs (methods `collab.hash_utf8|hash_utf16|stable|structure_hash|normalize_structure|field_hash|normalize_fields|document_hash`).
- Replaced in collaboration-core-v5.js (now thin glue calling `InhouseRust.collab`): ihnHashString, ihnStableValue (deleted), ihnStableStringify, ihnNormalizeStructureMeta,
  ihnCanonicalStructurePayload (deleted), ihnStructureHash, ihnCloneFieldValue, ihnNormalizeFieldMeta, ihnFieldMetaHash, ihnCanonicalContentItem / ihnCanonicalLivePage (deleted),
  ihnCanonicalDocumentHash. New export `ihnCanonicalDocumentHashJson(pagesJson, ...)` takes the pages as JSON text (no JS-side walk).
- Adapter: `InhouseRust.collab = { call(name, text), hashString(text) }` between the stream:collab markers in rust-core-v5.js (cached method-name blocks, text in/text out).
- Strings are UTF-16 inside Rust (lone surrogates, code-unit key order, `{...}` index-key ordering, `__proto__` drop, ES Number::toString incl. the round-half-even
  tie that Rust's formatter rounds up) so output is byte-identical with the old JS.
- Rust also carries its own copy of ihnNormalizeDeletionStamps for the page canonicalizer (plain-JSON forms; Map input is not supported across the JSON boundary).
  The JS version stays until U3 and must then be replaced by a call into `canon::normalize_deletions`.
- Tests: `cargo test` (14 collab tests incl. golden values from the original JS), tests/collab-hash-differential.test.mjs (850k random numbers, 5k strings,
  5k stable values, 3k structure metas, 4k field metas, 4k random documents, large realistic pages; byte-exact vs loadLegacy), existing collaboration/live tests now
  import tests/helpers/collab-rust.mjs (installs globalThis.InhouseRust from the built wasm). Chaos fixture loads rust-core-v5.js and exposes `__CHAOS__` after `InhouseRust.ready`.
- Bytes: JS algorithm removed 7.6 KB (collaboration-core-v5.js 28782 -> 24454), JS glue added 3.1 KB there + 2.6 KB adapter, Rust added 46.6 KB (excluding 8 KB tests).
- Speed (node 22, shared noisy machine, legacy loaded via loadLegacy): document hash, 28 KB page 8.7 -> 0.74 ms (11.8x; 22x from JSON text), 619 KB 208 -> 19.5 ms (10.7x; 23x
  from JSON text), 8.1 MB 3660 -> 368 ms (10x; 21x from JSON text); ihnStructureHash with 60 pages 6x. Tiny calls are break-even (stableStringify of a small object 0.93x,
  hashString of a short string 0.84x) and ihnNormalizeStructureMeta of 60 pages is 0.5x slower until U2 moves its callers into Rust (the call now crosses the JSON boundary).
- Known tolerances (all outside JSON-clean data): values must be plain JSON data (Date/toJSON/Map/Set/typed arrays differ: JSON.stringify semantics); +-Infinity in numeric
  meta fields (clock, rank, pageWidth) arrives as null; `Map` deletedStrokeStamps; a `__proto__` page id. None occur in persisted or wire data.
- Integrator notes: (1) every caller must await `InhouseRust.ready` before the first hash, the functions throw otherwise. document-processing-worker-v5.js already does importScripts
  rust-core first but its `contentHash` / `prepareMetadata` tasks need `await InhouseRust.ready`, and should call `ihnCanonicalDocumentHashJson(data.json, ...)` to skip the JSON.parse
  (not my file). The main app needs the same gate before the first Drive sync / live snapshot. (2) add tests/collab-hash-differential.test.mjs to package.json test:unit.

### Remaining: U2, U3, U4, U5, U6, U7, U8 (not started).
