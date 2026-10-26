# editor_history port plan (undo/redo, three-way merge, version timeline)

Base: branch `rust/editor_history` at 3edf36b (main a2dea10, v5.16.57, identical to origin/main when this unit started).

## Status

| Unit | State | JS removed | Rust added | Measured |
|---|---|---|---|---|
| U1 Item fingerprints and stroke delta kernels (eh-1) | done | 3,567 bytes of algorithm in `app-v5.js` (pointFingerprint, strokeSyncFingerprint, imageSyncFingerprint, itemSyncStampFingerprint, the two `*CollabFingerprint` bodies, simpleHash body, the compare loops of remoteStrokeDataLooksDifferent, the touched-id diff of applyHistoryStrokeSnapshotDelta); 1,467 bytes of glue added back | 18.7 KB (`editor_history/fingerprints.rs`, 9 unit tests), wasm 133,808 to 159,555 bytes | see progress log |
| U2 .. U7 | not started | | | |
Owned JS: `app-v5.js` sections 4.11b, 4.21c, 4.19, 4.19b (the section markers are loose, the real code is
scattered, see inventory), `timeline-core-v5.js` (15.4 KB), `tests/timeline-core-v5.test.mjs`.
Rust module: `rust-core/src/editor_history.rs`. JS adapter only between `stream:editor_history` markers in `rust-core-v5.js`.

## 1. Inventory and classification

Line numbers refer to `app-v5.js` at this base. P = pure computation (portable), G = glue (stays JS), D = decision logic
tangled with glue (extract the decision, leave the effects).

### 4.11b Undo / redo (29844-30318, 30 KB) plus history helpers outside the section
| Block | Lines | Bytes | Class |
|---|---|---|---|
| getHistoryRecordValue/Id, resolveHistoryPageIndex | 29849-29868 | 0.8K | D (page lookup by id, index math) |
| findHistoryItemIndex, cloneHistoryItem | 29869-29886 | 0.8K | P (id/index search; clone stays JS) |
| removeHistoryItems / restoreHistoryItems / replaceHistoryItems | 29887-29940 | 2.5K | D (index planning P, array splice G) |
| applyHistoryStrokeSnapshotDelta | 29941-29988 | 2.2K | D (diff by fingerprint P, splice/clone/track G) |
| saveToHistory | 29989-30023 | 1.3K | D (cursor/truncate/limit arithmetic P) |
| undo / redo | 30024-30318 | 15.4K | G (dispatch on action.type, DOM redraw, page loading, selection) |
| updateHistoryButtons | 30319-30332 | 0.6K | G |
| estimateHistoryMemory, trimHistoryByMemory, trim loop in releaseMemoryOnHide | 1063-1103 | 1.8K | P (outside the section, owned by history) |
| Save indicator/debug popup (30333-30688) | | 11K | G (DOM) , not history, leave alone |

### 4.21c Remote change detection and three-way merge (27927-29844, 109 KB)
| Block | Lines | Bytes | Class |
|---|---|---|---|
| pullRemoteChanges | 27931-28329 | 23K | G (Drive fetch, PDF load, metadata decode, UI) |
| updateLocalRevisionId, updateSyncBaseline | 28330-28374 | 2K | G/P trivial (baseline id sets; small) |
| pointFingerprint, strokeSyncFingerprint, imageSyncFingerprint, itemSyncStampFingerprint, stroke/imageCollabFingerprint | 28375-28419 | 2.5K | P (hot: hashes every point, string join) |
| collaborativeItemLocationFingerprint, chooseCollaborativeItemLocation | 28420-28443 | 1K | P (calls ihnChooseConcurrentItem, owned by collab) |
| reconcileDuplicateItemsAcrossPages | 28444-28575 | 5.9K | D (winner selection P; saves/redraw G) |
| remoteStrokeDataLooksDifferent | 28576-28609 | 2.2K | P (needs state.pages shape as input) |
| applyRemotePages | 28610-29834 | 69K | D, see split below |
|  - option/token/context guards, field-metadata merge, waits | 28610-28736 | 6K | G |
|  - pageId reorder/delete pre-pass | 28737-29092 | 20K | D (about 8K decisions, 12K awaits/flush/IndexedDB) |
|  - page count/append of remote pages | 29094-29198 | 5.6K | D (about 2K decisions) |
|  - per-page stroke/image/tombstone merge | 29199-29470 | 15.2K | P core (ids, stamps, concurrent winner, sort, change flags) |
|  - apply result to page, background/size field merge, saves, redraw | 29470-29694 | 14.2K | G (about 3K field-resolution decisions stay with collab fields) |
|  - dedupe call, content hash, result | 29694-29834 | 6K | G |
| setsEqual | 29835-29843 | 0.3K | P trivial |

### 4.19 / 4.19b Version timeline (code is spread over 3 places)
| Block | Lines | Bytes | Class |
|---|---|---|---|
| zoom clamp, applyTimelineZoomToCanvases | 14086-14114 | 1.5K | G / trivial |
| buildOriginalVersionEntry, getDisplayVersionHistory, getCurrentVersionId, findVersionById | 14115-14201 | 4.2K | D (reads state, pure shape building; small) |
| readPdfPageSizes, needsCleanOriginalAttachmentForSave | 14202-14235 | 1.5K | G |
| findPreviousTimelineEntry | 14236-14242 | 0.3K | P trivial |
| getTimelineItemKey/Signature, StrokeBounds, ImageBounds, clampTimelineBounds, collectTimelineDiffHighlights | 14243-14345 | 5.5K | P (per-point rounding + JSON.stringify per stroke, hot for big pages) |
| drawTimelineDiffHighlights, renderTimelinePreview, updateRevertButton, selectTimelineVersion | 14346-14576 | 11.5K | G (canvas/DOM) |
| getTimelineMonthKey/Label/DayLabel, formatTimelineHourMinute | 14577-14620 | 1.8K | G (Intl/Date locale) |
| groupTimelineEntriesForDisplay | 14621-14661 | 1.8K | P (small; takes label callbacks) |
| renderTimelineList (12.8K), ensureVersionHistoryRestored, open/close/renderTimelinePanel, setupTimelineModalEvents | 14666-15013 | 18K | G (DOM) |
| cloneTimelineValue, timelineSnapshotHash | 32171-32191 | 0.8K | P (JSON clone + FNV-like simpleHash of full JSON) |
| summarizeVersionDelta | 32192-32238 | 3.3K | P (JSON.stringify of images per page) |
| buildTimelineSnapshotFromPages | 32239-32271 | 2.2K | P (deep clones every stroke) |
| normalizeTimelineEntry | 32272-32318 | 2.8K | P |
| pruneTimelineHistory (+Unpruned/normalizeTimelineHistory/normalizeWorkerTimeline/mergeVersionHistories) | 32319-32436 | 6K | P (calls ihnStableStringify on whole entries in comparators) |
| ensureTimelineBaselineFromPages, captureVersionSnapshot | 32437-32509 | 3.9K | D (coalesce decision P; state/persist G) |
| captureTimelineRecoveryPoint, encode/decodeVersionHistoryForKeywords, extractCleanOriginalAttachment, parseOverlayRefs, stripOverlays | 32510-32699 | 9K | G (PDF bytes, async); out of scope for Rust here |
| adoptVersionHistory, persist/load IndexedDB (21935-22060) | | | G (IndexedDB; owned by storage) |

### timeline-core-v5.js (15.4 KB, whole file P)
cloneJson, normalizeForStableJson/stableStringify, hashString, snapshotFromEntry/Hash, copyMetadata, pageIdentity,
snapshotPageMap, buildSnapshotDelta, buildArchive/createArchive/encodeArchiveJson, assertSafeRecord, applySnapshotDelta,
entryFromRecord, materializeArchive, estimateArchiveBytes. Public API (frozen): ARCHIVE_SCHEMA_VERSION,
DEFAULT_CHECKPOINT_INTERVAL, createArchive, encodeArchiveJson, materializeArchive, estimateArchiveBytes, stableStringify, snapshotHash.
Used from app through `timelineArchiveCore` (prepareTimelineArchive worker path and materialize). Already ported: nothing
in this area (rust-core/src/editor_history.rs is an empty stub; `unpackCheckpoint` at app-v5.js:22131 belongs to another area).

### Cross-area dependencies (must be coordinated, not edited here)
- `ihn*` helpers (collaboration-core-v5.js: ihnChooseConcurrentItem, ihnDeletionWinsItem, ihnMerge/NormalizeDeletionStamps,
  ihnStableStringify, ihnMergeStructureMeta) belong to the collab stream. The page merge (U4) and prune (U3) need them.
  Plan: collab exports `pub fn` Rust equivalents in `collab.rs`; editor_history calls them as `crate::collab::...`.
  Until they exist, U4 and U3 keep calling the JS versions through the adapter only if input is pre-resolved; otherwise
  implement a private copy and mark for dedupe with the collab owner. Ask the integrator before starting U3/U4.
- `simpleHash` (app-v5.js:26945) is shared by many areas; Rust copy lives in U1 and is reused by U3.

## 2. Units (7), ordered by speedup then risk

### U1  Item fingerprints and stroke delta kernels  (high speedup, low risk, about 8 KB JS)
Scope: simpleHash, pointFingerprint, strokeSyncFingerprint, imageSyncFingerprint, itemSyncStampFingerprint,
strokeCollabFingerprint, imageCollabFingerprint (28375-28419); remoteStrokeDataLooksDifferent (28576-28609);
the diff part of applyHistoryStrokeSnapshotDelta (29941-29988: touched-id computation); collaborativeItemLocationFingerprint.
Why high: a fingerprint hashes every point of every stroke via string building; it runs on every merge comparison,
every pull, every erase-area undo/redo.
Rust: `fingerprints.rs` submodule. Exact JS semantics to preserve: `Math.round(v*100)/100` (round half toward +inf, not
Rust `round`), -0 prints "0", number-to-string must equal JS `Number.prototype.toString` (shortest round trip; values are
rounded to 2 decimals so exponent forms only appear beyond 1e21; implement the JS exponent rule anyway), `simpleHash`
= 32-bit `(h<<5)-h+code | 0` over UTF-16 code units, `Math.abs(h).toString(36).slice(0,8)`; note abs(-2^31) edge case.
Boundary: points as `Float64Array` triplets (x,y,p; NaN marks a missing p) with `Uint32Array` stroke offsets, strings
(id/tool/color) as one JSON array; result as JSON array of fingerprint strings (callBytes for the string table). Packing in JS
is one linear loop with no string allocation (still far cheaper than the current per-point template strings).
Tests: new differential test `tests/editor-history-fingerprint.diff.test.mjs` using `loadLegacy('app-v5.js')`-extracted
functions (extract by source slice, as release-v5-smoke does at line 668) plus seeded random strokes (include NaN, -0,
1e22, very long strokes, non-ASCII ids). Existing coverage: tests/e2e/collaboration-chaos.spec.mjs, editor-*.spec.mjs.
Risk notes: string hash must be byte-exact; if any 1e21+ coordinate cannot be matched, fall back to returning Err.

### U2  Timeline archive core (timeline-core-v5.js)  (high speedup, high risk, 15.4 KB JS)
Scope: the whole file; keep a thin `timeline-core-v5.js` that exposes the same frozen API and calls Rust (the file is loaded
by index.html and by tests/release-v5-smoke.mjs line 18/553, so it stays as a ~1 KB adapter; the adapter lives in
rust-core-v5.js markers and `InhouseTimelineCore` is re-pointed there. Check with the integrator whether the file may shrink to
the adapter or must be removed; RUST_PORT forbids renaming public globals).
Why high: stableStringify + FNV hash + delta archive over full-document history (up to 80 entries, thousands of strokes) is
what blocks input for seconds (see comment at app-v5.js:32399).
Rust: `timeline_core.rs` submodule. Needs: custom canonical JSON writer (sorted keys by UTF-16 code unit order, which differs
from Rust byte order for astral chars; drop `__proto__`/`prototype`/`constructor`; point-object shortcut with keys x,y[,p]),
JS-exact number formatting (serde_json differs: 1e21, -0, 5e-7, integer-valued floats), `serde_json` default float parsing is
not guaranteed round-trip exact; do not trust it for 17-digit values: parse numbers with a custom path or use the raw text
(`arbitrary_precision` is a Cargo feature we cannot add; record in the report if needed). Checkpoint interval, bounds
(MAX_ARCHIVE_ENTRIES 200, MAX_ARCHIVE_PAGES 2000, MAX_PAGE_ID_LENGTH 256) and fail-closed errors with identical messages.
Boundary: JSON bytes in (history array) and JSON bytes out via `callBytes` (no JS object round trip when possible:
`encodeArchiveJson` returns the final string bytes straight from Rust; `materializeArchive` returns JSON bytes the adapter
`JSON.parse`s once). `snapshotHash`/`stableStringify` take a JSON byte string, return string.
Tests: `node --test tests/timeline-core-v5.test.mjs` (7 tests, kept unchanged, must stay green), plus a differential test
against `loadLegacy('timeline-core-v5.js')` on seeded random histories (edits, reorders, tombstones, tampered chains), compare
byte-for-byte. release-v5-smoke.mjs lines 18-19, 67, 553 reference this file's source text; update only what the port removes.

### U3  Timeline history normalize / prune / merge / capture  (high speedup, medium risk, about 19 KB JS)
Scope: cloneTimelineValue, timelineSnapshotHash, summarizeVersionDelta, buildTimelineSnapshotFromPages, normalizeTimelineEntry,
pruneTimelineHistory, normalizeTimelineHistoryUnpruned/normalizeTimelineHistory/normalizeWorkerTimeline, mergeVersionHistories
(32171-32436), plus the pure part of captureVersionSnapshot and ensureTimelineBaselineFromPages (coalesce decision, hash,
dedupe vs last entry; 32437-32509). Constants VERSION_HISTORY_MAX 80, MILESTONE_MAX 30, COALESCE_MS 300000, TIMELINE_SCHEMA_VERSION.
Why high: every capture/persist/adopt normalizes and re-stringifies whole entries (`ihnStableStringify(entry)` inside sort
comparators), deep-clones all strokes per page twice, and runs on each save.
Rust: `timeline_history.rs`. Reuses U2 canonical writer and U1 simpleHash. Preserve tie-break order exactly: grouping by
originId then device key, content-hash dedupe, branch id suffix `~simpleHash(device:contentHash)` truncated to 160 chars,
`localeCompare` for ids and device keys (ICU in JS vs byte order in Rust: ids are ASCII `[a-z0-9_~:-]` in practice but
legacy/foreign ids are not guaranteed; either implement the ICU root collation subset for ASCII and fall back to Err on
non-ASCII, or document it as a parity doubt), milestone protection, drop-first-non-milestone loop.
Boundary: JSON bytes in/out (history array in, normalized array out). The `WeakSet` markers `normalizedTimelineEntries` and
`ownedTimelineSnapshots` are a JS-side identity optimization: keep them in the adapter, skipping the Rust call when the
input array is already marked normalized. State mutation, getCurrentVersionAuthor, getPresenceClientId, generateVersionId,
persistTimelineHistory stay JS (captureVersionSnapshot becomes: gather inputs, call `editor_history.timeline_capture`,
assign result, persist).
Tests: tests/timeline-core-v5.test.mjs, tests/collaboration-core-v5.test.mjs (stable-stringify tie-breaks), new differential
test on random multi-device histories; e2e: app-resilience.spec.mjs, save-*.spec.mjs, collaboration-chaos.spec.mjs;
release-v5-smoke.mjs asserts `function normalizeTimelineHistory(` (line 243) and the adoptVersionHistory slice (668): update.

### U4  Per-page stroke/image/tombstone three-way merge  (high speedup on big docs, medium risk, about 15 KB JS)
Scope: applyRemotePages 29199-29470 (the "Stroke-level merge for existing pages" decision core): id sets, baseline-deleted
inference (`lastSyncedStrokeIds`), inferred tombstone stamps, merged deletion stamps, concurrent winner selection
(`ihnChooseConcurrentItem` with fingerprints), localOnly items, sort by id/fingerprint when preserveLocalContent, effective
deletion stamps, deletedStrokeIds sort, local-merge/conflict counters, change flags (stroke/image/ids/deleted/stamps).
Also remoteStrokeDataLooksDifferent if not already in U1 and setsEqual.
Rust: `page_merge.rs`. Input per page: local/remote summaries only. To avoid shipping point arrays across the boundary, JS
sends per item: id, fingerprint strings (from U1, computed once and cached by item identity in the adapter), stamp
(clock, actor), plus deletion stamp maps, baseline id list and options flags. Rust returns the merge plan: ordered list of
`{side: "remote"|"local", id}` for strokes and images, effective deletion stamps, sorted deleted ids and flags. JS then
materializes the arrays from its own object maps (no stroke object is serialized). Awaits, `ensurePageDataLoaded`,
`waitForCollaborativeInteractionIdle`, token checks, field-metadata merge stay JS.
Needs collab Rust (ihnChooseConcurrentItem, ihnDeletionWinsItem, ihnMergeDeletionStamps, ihnNormalizeDeletionStamps) as
`crate::collab` functions: confirm the collab stream's API and exact stamp ordering before starting.
Tests: tests/collaboration-core-v5.test.mjs, differential test generating random local/remote page states and comparing the
final arrays (ids, order, stamps) between legacy JS (extracted function wrapper) and Rust, including tie ties on equal stamps and
fingerprints; e2e tests/e2e/collaboration-chaos.spec.mjs and editor-*.spec.mjs.

### U5  Undo/redo ledger and item-index planning  (medium speedup, low risk, about 7 KB JS)
Scope: getHistoryRecordValue/Id (29849-29857), resolveHistoryPageIndex, findHistoryItemIndex, removeHistoryItems,
restoreHistoryItems, replaceHistoryItems (index planning only), saveToHistory cursor logic, estimateHistoryMemory,
trimHistoryByMemory and the trim loop in releaseMemoryOnHide (1063-1103).
Why: `estimateHistoryMemory` rescans every action and every point after each save and each trim iteration (O(n^2) with
the while loop); `findIndex`-per-item is O(n*m) on 10k-stroke pages.
Rust: `history_ledger.rs`: a small ledger holding per-action point counts and the cursor. `history.push(count)` returns
`{truncateFrom, dropFront, newIndex}`; `history.trim(limit)` returns drop count; id search takes a `Array<string>` JSON of
page item ids and the targets, returns `Int32Array` of positions/insert plan (so JS only splices). The action objects stay in
JS (they hold live stroke references and DOM-free closures); Rust only sees counts and ids. The point count per action is
computed once at push time in a tiny JS loop (reads `.length`, no allocation) and cached on the action.
Boundary: typed arrays (Int32Array indices, Uint32Array counts) and a JSON array of id strings.
Tests: new differential test of the ledger vs legacy trim/save semantics (random push/undo/redo sequences);
e2e editor-*.spec.mjs (undo/redo specs), pdf-nondestructive-erase.spec.mjs, preview-stability.spec.mjs.
undo/redo bodies stay JS (DOM/page loading), only helper calls change.

### U6  Timeline diff highlights and display grouping  (medium speedup, low risk, about 9 KB JS)
Scope: getTimelineItemKey, getTimelineItemSignature, getTimelineStrokeBounds, getTimelineImageBounds, clampTimelineBounds,
collectTimelineDiffHighlights (14243-14345), groupTimelineEntriesForDisplay (14621-14661; labels via a JS callback result:
JS passes precomputed month keys/labels per entry, Rust groups), findPreviousTimelineEntry, buildOriginalVersionEntry shape
building (page list from sizes).
Why medium: runs only when opening a timeline preview, but signature = JSON.stringify of every point of every stroke twice.
Boundary: per-page strokes as packed `Float64Array` points + offsets and metadata JSON (same packer as U1); highlights returned
as `Float64Array` (x,y,w,h) plus `Uint8Array` types (0 added, 1 changed, 2 removed). Rounding `Math.round(v*10)/10` same rule as U1.
drawTimelineDiffHighlights stays JS (canvas).
Tests: differential random test on pages; e2e: editor-*.spec.mjs timeline cases, preview-stability.spec.mjs.

### U7  Page structure reorder/append planner and duplicate-location resolver  (low-medium speedup, high risk, about 17 KB JS decision subset)
Scope: decisions extracted from the pageId pre-pass (28737-29092: localById/remoteById matching, pageId adoption by position,
`structuralPageChangeExpected`, desired order from `ihnMergeStructureMeta`, deletions of synced-but-missing pages, remote-only
inserts, local-only appends) and the append block 29094-29198 (shouldAppendRemotePages, localHasUnsynced*), plus the winner
selection of reconcileDuplicateItemsAcrossPages (28444-28575) and chooseCollaborativeItemLocation.
Why last: little compute (O(pages)), but the plan must be exactly identical; it is intertwined with awaits and checkpoints, so
the port is an "extract planner, JS executes" refactor with the highest regression risk. Do only after U1-U4 are stable.
Boundary: JSON (page id lists, flags, structure meta in, ordered operation list out).
Tests: tests/collaboration-core-v5.test.mjs, tests/e2e/collaboration-chaos.spec.mjs (page reorder/delete races), differential
test with random page-id permutations, tombstones and `preserveLocalUnsynced` combinations.

## 3. Not portable (stays JS)
See `notPortable` in the structured result. Summary: pullRemoteChanges (Drive/PDF/network), undo()/redo() bodies (DOM, canvas
redraw, selection, async page loading), all timeline DOM/canvas rendering, Intl date labels, IndexedDB persistence, PDF
keyword encode/decode and overlay stripping, save indicator UI, async/await token orchestration in applyRemotePages.

## 4. Parity risks to track (record in report if unresolved)
1. JS number to string and `JSON.stringify` number output vs serde_json (1e21, -0, exponent forms).
2. `serde_json` float parsing is not guaranteed correctly rounded without `float_roundtrip`; Cargo.toml is not ours.
3. Key ordering: JS sorts by UTF-16 code units; Rust by UTF-8 bytes (differs for astral characters).
4. `localeCompare` (ICU) vs byte comparison in prune ordering.
5. `Math.round` half-up semantics for negatives; `|0` int32 wraparound in simpleHash; UTF-16 code unit iteration for hashing.
6. Cross-stream dependency on collab Rust (ihn*), and ownership of `simpleHash`.

## 5. Progress log

### eh-1 Item fingerprints and stroke delta kernels (done)
Ported to `rust-core/src/editor_history/fingerprints.rs`; adapter `InhouseRust.history` between the `stream:editor_history`
markers of `rust-core-v5.js` (10.4 KB of JS packing glue: typed-array packing, scratch buffer, record composition).

What is in Rust: `Math.round(v*100)/100` (half toward +inf, exact for all doubles), -0 prints "0", full ECMAScript
`Number::toString` (shortest digits, exponent forms, tie-to-even on exact 16/17 digit ties, which Rust's own `{:e}` gets wrong:
`-1590602947399020.25` prints `...020.2` in JS but `...020.3` in Rust), int32 wraparound hash over UTF-16 units (ASCII fast path
through `TextEncoder.encodeInto`, 4M-unit chunking so no 128 MB allocation limit is hit), stroke point hash over packed
(x,y,p) triplets (NaN x marks a hole in a sparse array, which `join` renders as empty text), width/count/clock tails, image
geometry/rotation tails. A direct decimal fast path for rounded values below 1e15 is proven equal to the general path by
a 200k-sample Rust test and by the differential test.

What stays in JS on purpose: id/tool/color/actor/src strings (so lone surrogates, numeric ids and `||` coercions behave as
before) and the Map/Set bookkeeping of `applyHistoryStrokeSnapshotDelta`. `collaborativeItemLocationFingerprint` is three lines of
string joining over the new functions and was left unchanged. Deviation from the unit text: strings do not cross the boundary
as a JSON array; Rust returns ASCII tails and JS prefixes `id|tool|color|`.

Behavioral additions that cannot change results: identical object references skip fingerprinting (same object, same
fingerprint); `touchedStrokeIds` fingerprints each unique stroke once instead of four times; `collabItemsDiffer` stops at the
first differing block of 128 items. `init()` now awaits `InhouseRust.ready` and throws if WASM is missing; every adapter entry
point and `rustHistory()` throw a clear error without the core (tested).

Tests: `cargo test` (9 in this module), `node --test tests/editor-history-fingerprint.diff.test.mjs` (9 differential tests against
`loadLegacy`-era source: fixtures plus seeded random strokes/images/pages/snapshots, 4M chunk boundary, tie-prone magnitudes
and random bit patterns, WASM-missing behavior). No existing test needed a change.

Speed (`node tests/editor-history-fingerprint.bench.mjs`, best of N, Node 22 main context, machine at load average 7 on 4 cores, so
absolute numbers are noisy and the ratios are what counts):

| Case | legacy | Rust | speedup |
|---|---|---|---|
| one stroke, 200 points | 0.075 ms | 0.023 ms | 3.2x |
| one stroke, 12 points | 4 us | 2 us | 2.0x |
| page, 2000 strokes x 60 points | 72.9 ms | 13.4 ms | 5.5x |
| page, 300 strokes x 800 points | 205 ms | 41.7 ms | 4.9x |
| remote vs local diff, 2000 equal strokes | 184 ms | 51 ms | 3.6x |
| history delta touched ids, 2000 strokes | 493 ms | 29 ms | 17x |
| simpleHash, 1.1 MB data URL | 4.0 ms | 1.4 ms | 2.8x |
| image collab fingerprint with 1 MB data URL | 3.4 ms | 1.5 ms | 2.3x |
| simpleHash, 20 chars | about 0.1 us | about 1 us | slower (boundary cost), not used on a hot path |

Note for later units: benchmarks and differential tests must not execute hot JS inside `vm.createContext` (global lookups go
through interceptors and made the adapter look 10x slower than it is); use `vm.runInThisContext` for timing.

Parity doubts: none open. Not done in this unit: caching fingerprints by item identity (belongs to U4 where the merge consumes them).
