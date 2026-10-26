# docio port plan: PDF background, import and export helpers

Branch `rust/docio`. Rust module `rust-core/src/docio.rs`; adapter between `stream:docio` markers in `rust-core-v5.js`.
Status: U1 export-meta DONE (committed); U2-U6 remaining.

## 1. Inventory (app-v5.js)

Section sizes: 4.16a lines 6855-7465 (~31.7 KB), 4.16c lines 30975-31722 (~43.4 KB), 4.17b lines 31723-~34246 (~110 KB incl. the
timeline block). Line numbers are approximate, taken from the current worktree.

### 4.16a PDF background management (6855-7465)
| Block | Lines | Size | Class |
|---|---|---|---|
| swapDisplayDocToCleanOriginal, restorePdfBackgroundsIfNeeded | 6859-7135 | ~13 KB | glue (pdf.js docs, Drive fetch, session tokens, state mutation) |
| stripPdfKeywordExtras, DRIVE_SYNC/NORM constants | 7136-7146 | 0.4 KB | portable |
| createDriveSyncEnvelope / encodeDriveSyncEnvelope / decodeDriveSyncEnvelope | 7147-7192 | 1.9 KB | decode portable; create/encode need presence id + hash (glue wrapper, base64/JSON portable) |
| embedded metadata worker plumbing (fail/get worker) | 7193-7289 | ~3.5 KB | glue (Worker, timeouts) |
| parseEmbeddedMetadataPayload | 7290-7350 | 3 KB | mostly glue (worker, cache); sampled-signature cache key portable but tiny |
| decodeEmbeddedStrokesFromPdfKeywords | 7351-7363 | 0.6 KB | portable (prefix parse) |
| pageEntryHasDrawableVectors / pageEntryNeedsPdfBackground / shouldDisplaySavedPdfWithBakedOverlay | 7364-7392 | 1.3 KB | portable |
| getActivePdfEmbeddedStrokes | 7393-7406 | 0.7 KB | glue |
| recoverMissingPageDataFromActivePdf | 7407-7465 | 2.7 KB | page assembly portable, IndexedDB/clone glue |

### 4.16c PDF import (30975-31722)
| Block | Lines | Size | Class |
|---|---|---|---|
| importPDFData | 30979-31709 | 42 KB | mostly glue (pdf.js load, Drive, status UI, session checks, IDB migrate). Portable inside it: legacy same-id/duplicate stroke scan (~31310-31375), importedPages construction loop (~31375-31520, per-page fallbacks for size/background/ids/deleted stamps), page-size refinement planning (~31625-31700), embedded-cal/struct keyword parse (~31190-31250) |
| importPDF (file input handler) | 31710-31722 | 0.6 KB | glue |

### 4.17b PDF export helpers (31723-~34246)
| Block | Lines | Size | Class |
|---|---|---|---|
| loadPageDataForExport, getPageCanvasForExport | 31727-31778 | 2.6 KB | glue (IDB, DOM canvas) |
| compressToBase64 / decompressFromBase64 | 31779-31843 | 2.4 KB | glue for CompressionStream; base64 loops portable |
| prewarmPdfCache, _prewarmPdfCacheInner, abortPrewarm | 31844-31987 | ~5.5 KB | glue (canvas drawing, yielding) |
| computePageHash | 31988-32019 | 1.7 KB | portable (depends on strokeSyncFingerprint, simpleHash, ihnStableStringify from other sections) |
| PDF assembly worker plumbing | 32020-32129 | ~4 KB | glue |
| Timeline: generateVersionId, getCurrentVersionAuthor, prepareTimelineArchive | 32130-32170 | 1.5 KB | glue (random/time/profile) |
| Timeline: cloneTimelineValue, timelineSnapshotHash, summarizeVersionDelta, buildTimelineSnapshotFromPages, normalizeTimelineEntry, pruneTimelineHistory, normalizeTimelineHistory*, normalizeWorkerTimeline, mergeVersionHistories | 32171-32436 | ~14 KB | portable (pure JSON in/out; time/ids injected) |
| ensureTimelineBaselineFromPages, captureVersionSnapshot, captureTimelineRecoveryPoint | 32437-32530 | ~4 KB | decision logic (coalesce) portable; state/persist glue |
| encode/decodeVersionHistoryForKeywords | 32532-32560 | 1 KB | glue (worker archive) |
| extractCleanOriginalAttachment | 32561-32592 | 1.3 KB | glue (pdf.js attachments) |
| parseOverlayRefsFromKeywords | 32593-32622 | 1.6 KB | portable |
| stripOverlaysFromPdfBytes | 32623-32709 | 3.5 KB | glue around pdf-lib (stays) |
| retainPdfSource/releasePdfSource | 32710-32722 | 0.4 KB | glue |
| fetchBytesAsImage | 32723-32774 | 2 KB | data: URL decode + magic sniff portable, fetch glue |
| getPdfOverlayContentBounds | 32775-32849 | 4 KB | portable |
| canBuildCurrentPdfWithPdfLib | 32850-32865 | 0.6 KB | glue |
| buildPdfBlobWithPdfLib | 32866-33468 | 33 KB | mostly glue (pdf-lib calls, canvas/worker overlay render, yielding). Portable inside: copyPages planning, page size px/pt, dedupe + compaction + metadata entry (~33100-33170), overlay placement transform, keyword payload assembly |
| markDocumentLegacyBakedAfterRasterFallback | 33469-33482 | 0.5 KB | glue |
| buildPdfBlob (jsPDF raster fallback, worker assemble) | 33483-34013 | 31 KB | mostly glue (canvas, pdf.js render, jsPDF worker). Portable inside: same dedupe/compaction, mm page-size calc, cache-key/hash, export scale selection |
| wait*/fetchLatestDrivePdfForExport/exportDrivePdf/exportPDF | 34014-34128 | 4 KB | glue |
| blobToBase64, waitForNativePdfExportResult, deliverExportedPdf | 34129-34246 | 4.5 KB | glue; base64 + chunk slicing portable |

Already ported: U1 export-meta (see section 5). `docio.rs` + `docio/{jv,strokes,export}.rs` hold the Rust. Note `renderOverlay`, `compress`, `timelineHash`,
`prepareMetadata`, `contentHash` run in `document-processing-worker-v5.js` (not my file, not my section).

## 2. Cross-area dependencies
simpleHash (26945), strokeSyncFingerprint (28383), ihnStableStringify / ihnNormalizeDeletionStamps / ihnCanonicalDocumentHash,
normalizePageDimension (2316), normalizeTemplateKind, cloneStroke are defined in other sections. Until a sibling Rust module
exposes them, docio.rs keeps private byte-exact Rust copies (differential-tested against the JS), and calls switch to the
sibling version when integrated. JS `localeCompare` (used in timeline prune ordering) is ICU-based; Rust uses a documented
codepoint-order comparison, valid for the ASCII ids/device keys produced by the app; parity doubt to be flagged.
Time (`Date.now`) and randomness (ids) are passed in from JS (no host imports).
JSON key order matters (`{...stroke}` spreads, metadata strings feed hashes): requires serde_json `preserve_order`
(Cargo.toml change: record in report if needed, integrator applies) or a hand-written ordered JSON writer in docio.rs.

## 3. Units (ordered: speedup high first, then lowest risk)

### U1  export-meta: stroke dedupe, compaction, page metadata entry, overlay bounds, page hash (speedup HIGH, risk MEDIUM, ~14 KB JS)
Scope: the duplicated block in buildPdfBlobWithPdfLib (~33100-33170) and buildPdfBlob (~33670-33760): fingerprint dedupe
(`id:` or JSON of tool/color/width/points) + point compaction (round to 0.01, drop undefined `p`) + `pageStrokeEntry` JSON;
`getPdfOverlayContentBounds` (32775-32849); `computePageHash` (31988-32019); overlay placement math (x, y flip, scale to pt),
px/pt page size, mm size, export scale/jpeg-quality profile, copyPages plan (stateToSourceIdx).
Boundary: JSON string of strokes/images in; per page returns one metadata JSON string (the same strings pushed in `metadataParts`),
bounds as a 4-float Float64Array, hash as string. Dedupe/compaction currently allocate an object per point and yield every few
strokes; Rust does it in one call per page (JS keeps `yieldToUI` between pages).
Tests: `tests/rust-docio-export.test.mjs` (new differential, seeded random strokes incl. NaN/missing p, id collisions, highlighter),
`tests/e2e/pdf-nondestructive-erase.spec.mjs`, `save-cache.spec.mjs`, `save-status.spec.mjs`, `preview-stability.spec.mjs`.
Risks: number formatting vs JS `JSON.stringify` (-0, exponents), Math.round half-up (use floor(x+0.5)), key order.

### U2  codec: base64 / UTF-8 / data URL / image sniff (speedup MEDIUM-HIGH for big files, risk LOW, ~5 KB JS)
Scope: fetchBytesAsImage data: branch (32723-32750, chunked atob + Uint8Array.from), blobToBase64 (34129-34169), native export
chunk slicing in deliverExportedPdf, base64 loops in compressToBase64/decompressFromBase64, `btoa(unescape(encodeURIComponent()))`
and inverse used for IH_CAL/IH_SYNC/metadata fallback, PNG/JPEG magic sniff.
Boundary: Uint8Array / string via `#[no_mangle]` extern "C" (ptr,len) kernels; typed arrays only. Removes per-char JS loops and the
`setTimeout(0)` chunk yielding on multi-MB payloads (still async at the JS call site).
Tests: new differential `tests/rust-docio-codec.test.mjs` (random bytes, non-ASCII strings, invalid input must match throw/null
behaviour), `save-*.spec.mjs`, `offline-sync.spec.mjs`.

### U3  import-pages: legacy normalisation and page list construction (speedup HIGH on large annotated PDFs, risk MEDIUM, ~16 KB JS)
Scope inside importPDFData: same-id re-id + duplicate fingerprint scan (~31312-31375), importedPages construction loop
(~31375-31520), page-size fallback / refinement index planning (~31625-31700), embedded keyword extras parse (IH_CAL, IH_NORM
flag); plus recoverMissingPageDataFromActivePdf page assembly (7407-7465) and pageEntry* predicates (7364-7392).
Boundary: JSON (embedded pages array + options {firstPageW/H, idbCacheKey, hasLegacyBaked, pendingSwap, preGeneratedIds[]}) in,
JSON `{pages, needsMigrationResave, refinementIdx}` out. New stroke ids are supplied by JS (`generateStrokeId`) in a pre-generated
pool. Yielding between 256-stroke slices disappears (single Rust call per page batch; JS keeps 3-page yield cadence by calling per chunk).
Tests: differential `tests/rust-docio-import.test.mjs`; `pdf-nondestructive-erase.spec.mjs`, `preview-stability.spec.mjs`,
`offline-sync.spec.mjs`, `node --test tests/pdf-clean-original-cap.test.mjs` (source grep for ORIGINAL_BYTES_MAX must stay intact).

### U4  keywords: PDF Keywords codec (speedup LOW, risk LOW, ~6 KB JS)
Scope: stripPdfKeywordExtras, decodeEmbeddedStrokesFromPdfKeywords prefix parse, decodeDriveSyncEnvelope, parseOverlayRefsFromKeywords,
keywordsPayload assembly (IH_NORM, IH_OVERLAY_REFS, IH_CAL, IH_STRUCT, IH_FIELDS, IH_SYNC, IH_VERSIONS), sampled-signature key of
parseEmbeddedMetadataPayload.
Boundary: strings in, JSON/strings out (`callJson('docio.keywords_*')`). Parsing only; worker, cache and Date stay JS.
Tests: differential `tests/rust-docio-keywords.test.mjs`, `save-*.spec.mjs`, `drive-save-truth.spec.mjs`.
Done early because it is trivial and unlocks U1/U3 payload assembly.

### U5  timeline: version-history snapshot, normalise, prune, merge, delta summary (speedup MEDIUM-HIGH on long histories, risk HIGH, ~22 KB JS)
Scope: cloneTimelineValue, timelineSnapshotHash, summarizeVersionDelta, buildTimelineSnapshotFromPages, normalizeTimelineEntry,
pruneTimelineHistory, normalizeTimelineHistory(Unpruned), normalizeWorkerTimeline, mergeVersionHistories, and the coalesce/decision
core of captureVersionSnapshot (32171-32510).
Boundary: JSON string in/out (history array up to 80 entries with page snapshots, can be MBs: the cost of crossing is paid once
instead of repeated stringify tie-breaks in prune). `now`, new version id, author, device id injected by JS. Wrapper keeps the
`WeakSet` identity short-circuits (ownedTimelineSnapshots / normalizedTimelineEntries) so unchanged histories never cross the boundary.
Risks: stable-stringify ordering ties, `localeCompare`, simpleHash UTF-16 semantics, in-place mutation expectations of callers
(captureVersionSnapshot replaces last entry). Persist/IDB (persistTimelineHistory) stays JS.
Tests: differential `tests/rust-docio-timeline.test.mjs` with seeded multi-device histories (branch ids `~hash`, milestones, >80
entries, >30 milestones), `offline-sync.spec.mjs`, `save-*.spec.mjs`, timeline e2e if present (grep `timeline`).

### U6  zlib/deflate for STROKES_Z (speedup MEDIUM, risk MEDIUM, ~3 KB JS; needs crate) 
Scope: compressToBase64/decompressFromBase64 core (31779-31843) and the bounded main-thread fallback in parseEmbeddedMetadataPayload.
Needs `miniz_oxide` (no_std-friendly, pure Rust) in Cargo.toml: cannot be added by me, record for the integrator; otherwise this unit is
dropped. Output must be a valid zlib stream (CompressionStream 'deflate'); byte equality with the browser is not required, round trip is.
Boundary: Uint8Array in/out. Enforces the 96 MB decompressed cap in Rust. Worker copies in document-processing-worker-v5.js are
not mine; coordinate with its owner.
Tests: round-trip + cross decode of browser-produced streams in node, `save-*.spec.mjs`.

(Total 6 units, order U1 U2 U3 U4 U5 U6; U4 can land first as a warm-up if the integrator prefers lowest risk.)

## 4. Not portable
- pdf-lib / pdf.js / jsPDF calls (copyPages, embedPng/Jpg, drawImage, setKeywords, attach, save, stripOverlaysFromPdfBytes, getAttachments, render): vendored JS libraries stay.
- Canvas 2D drawing (drawSmoothStroke, drawImagesOnCanvas, drawTemplateBackground, pre-warm and raster-fallback rendering, canvasToBlobSafe JPEG/PNG encode): browser rasteriser and encoder; Rust pixels would not be identical (antialiasing) and the stroke smoothing belongs to the ink area.
- Worker plumbing (embedded metadata worker, PDF assembly worker, InhouseDocumentProcessing calls) and `document-processing-worker-v5.js`: outside my section.
- importPDFData / restorePdfBackgroundsIfNeeded / swapDisplayDocToCleanOriginal control flow: session tokens, abort errors, Drive fetch, status UI, IndexedDB migration, timers.
- exportPDF / exportDrivePdf / deliverExportedPdf / native bridge (`InhouseNative`), Blob/File/URL/share: browser and host APIs.
- loadPageDataForExport, getPageCanvasForExport, flushDirtyPages and yield helpers (yieldToUI, hasHotUserInteraction): DOM/IDB/event-loop glue.

## 5. Progress log

### U1 export-meta: DONE
Moved to Rust (`rust-core/src/docio.rs`, `docio/jv.rs`, `docio/strokes.rs`, `docio/export.rs`); JS adapter `api.docio` between the
`stream:docio` markers of `rust-core-v5.js`:
- stroke duplicate filter, point compaction and the per-page metadata entry of **both** export paths (`docio.page_export`, modes `pdflib`
  and `raster`; the entry key order, `points` rules and `deletedStrokeStamps` normalisation of each path are kept);
- `getPdfOverlayContentBounds` (`docio.page_export` computes it with the entry, `docio.overlay_bounds` standalone);
- `computePageHash` (`docio.page_hash`: strokeSyncFingerprint, imageSyncFingerprint, simpleHash, ihnNormalizeDeletionStamps +
  ihnStableStringify are private byte-exact Rust copies until the collab area exposes its own);
- export profile / scale / JPEG quality and the copyPages plan (`docio.export_plan`), px/pt/mm page geometry and orientation
  (`docio_page_geometry`, `docio_pdflib_geometry`), overlay placement in points (`docio_overlay_placement`; typed-pointer kernels).

Boundary: strokes cross as a JSON shell plus Float64Array/Uint8Array point data (`packStrokes` in the adapter; pages whose strokes lack an
id or have non-numeric coordinates fall back to plain JSON, handled identically by Rust). Header/scalar fields travel as JSON; the entry
comes back as one string plus index/bounds binary header. `rust-core/src/docio/jv.rs` is an ordered JSON reader/writer with JavaScript
number formatting (ECMAScript tie-to-even shortest digits), WTF-8 strings (lone surrogates survive) and integer-key property order, so
no `serde_json/preserve_order` crate change is needed.

JS deleted from `app-v5.js`: `getPdfOverlayContentBounds`, the body of `computePageHash`, the dedupe + compaction + `pageStrokeEntry`
blocks of `buildPdfBlobWithPdfLib` and `buildPdfBlob`, `pxToMm`/`getPageOrientation`, the inline overlayScale/exportScale/jpegQuality/
exportProfile selection, the copyPages planning loop, the px/pt arithmetic and the overlay placement arithmetic; also the prewarm
profile constants. Net: 18,375 bytes removed from `app-v5.js`, 7,463 added (thin glue calls), `app-v5.js` 1,926,082 -> 1,915,385 bytes.
Adapter in `rust-core-v5.js`: 10,364 bytes. Rust added: 81,724 bytes (of which about a third is `#[cfg(test)]`).

Behaviour notes (deliberate, all invisible in the produced bytes):
- the pdf-lib path no longer keeps `metadataPages` objects (compact objects were rebuilt per point); the timeline snapshot parses
  `metadataParts` instead (the raster path always did);
- overlay bounds are computed together with the entry (before the `yieldToUI(true)` instead of after it) when the cached overlay is
  stale; when the cache turns out stale only after the yield they are computed with `docio.overlay_bounds`;
- yielding inside the per-point loops is gone: one synchronous Rust call per page replaces many 4 ms slices (the page-level yields stay);
- non-finite coordinates (NaN/Infinity) round-trip exactly on the packed path; on the plain-JSON path JSON turns them into `null`
  (practically unreachable: pointer coordinates are finite and persisted ink never holds NaN);
- a `null` point element fails with an error (the original threw a TypeError); a non-object stroke in a page with more than one stroke
  likewise errors instead of being silently mishandled.
Parity doubts to flag for the integrator: `Math.cos/sin` of image rotation come from the wasm libm; 3,000 random images and 20,000 swept angles
matched V8 bit for bit.

Tests: `tests/rust-docio-export.test.mjs` (differential against the original code cut out of `git show a2dea10:app-v5.js`: 1,500 random
pages per path by default, 20,000 with `DOCIO_ROUNDS=20000`, plus 60,000 random doubles through the number writer);
`cargo test` (21 docio unit tests); `tests/release-v5-smoke.mjs` updated (it grepped for `function getPdfOverlayContentBounds` and ran
that source in a vm; it now asserts the call to `docio.overlayBounds` and exercises the Rust function). Browser specs run: see
final report.

Speed (`node tests/bench/docio-export.bench.mjs`, median, shared 4-core box under load, both sides in the main realm):
| page | step | JS ms | Rust ms | speedup |
|---|---|---|---|---|
| 40 strokes x 60 pts | changed page: dedupe+compaction+entry+bounds | 1.1 | 1.0 | 1.0-1.4x |
| 40 strokes x 60 pts | page hash (every page, every save) | 1.5 | 0.42 | 3.6x |
| 300 strokes x 80 pts | changed page | 14.3 | 9.2 | 1.6x |
| 300 strokes x 80 pts | page hash | 14.6 | 4.8 | 3.1x |
| 1200 strokes x 120 pts | changed page | 120-134 | 50 | 2.4-2.7x |
| 1200 strokes x 120 pts | page hash | 90 | 28 | 3.3x |
The in-browser original additionally slept between 220-point/5-stroke slices (`yieldToUI`), so wall time per save was longer
than these synchronous numbers; the Rust call is one uninterrupted 1-10 ms block per page.
