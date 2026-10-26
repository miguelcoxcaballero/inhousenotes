# editor_view porting plan (planning only; no code moved yet)

Area: app-v5.js section 4.8 (Page lifecycle, lines 2708-3487), 4.10 (Visible-page tracking, 3488-4123),
4.9 (Viewport transform, 16426-17405), 4.20 (Manage-Pages panel, 19132-19750; mostly scanner/cover/page-size helpers).
Rust module: rust-core/src/editor_view.rs (currently an empty `call`, nothing ported yet).
Adapter: rust-core-v5.js between the `stream:editor_view` markers.

Important coordination note: the layout cache that feeds almost everything here (`getPageLayoutMetrics`,
`findFirstPageWhoseBottomReaches`, `findLastPageWhoseTopPrecedes`, `getPageBounds`, `getContentHeight`, lines 16330-16425,
and `getPageCenterOffset` at 2358) sits just OUTSIDE my headed sections. Unit 1 needs it. Either the integrator confirms
I may move those few lines (pure, ~3 KB, no one else should own them), or Unit 1 receives the 4 arrays from the JS builder
unchanged. Plan assumes the first option and falls back to the second.

## 1. Inventory and classification

Legend: P = pure computation (portable), G = glue (DOM/canvas/IO/timers, stays JS), A = already ported (none).

### 4.8 Page lifecycle (37 KB)
| Function | Bytes | Class |
|---|---|---|
| addPage, createPageElement, attachPageCanvases (7.2 KB), ensurePageCanvas, releasePageCanvas | 14.8K | G (DOM/canvas creation) |
| cancelPageActivationScheduler, schedulePageActivationProcessing, activateVisiblePage, processPendingPageActivations | 3.8K | G (timers, async) |
| queuePageActivation | 0.4K | P (priority map insert, tiny) |
| getPdfBackgroundScale | 0.7K | P |
| resetPdfState, cancelPdfRenderResume, cancelActivePdfBackgroundRender, pausePdfRenderingDuringInteraction | 2.3K | G |
| bumpPageStructureVersion, invalidatePageStructureAsyncState | 0.8K | G (state flags) |
| touchPdfBackgroundCache, prunePdfBackgroundCache | 1.2K | P (LRU eviction choice; the Map itself stays JS) |
| applyPageBackground, queuePdfPageRender, requeue..., processPdfRenderQueue, schedulePdfRenderResume, renderPdfPageToBackground (5.2K) | 9.6K | G (pdf.js, canvas, timers) |
| isViewportInteracting | 0.2K | G (reads live input flags) |
| getVisiblePageRange, getViewportVisiblePageIndices, expandPageIndices | 2.8K | P (after the DOM rect read) |

### 4.10 Visible-page tracking (32 KB shown in section scan)
| Function | Bytes | Class |
|---|---|---|
| updateVisiblePages | 6.6K | mixed: selection/ordering plan is P (about 4.5K), ensure/activate/release calls are G |
| scheduleVisiblePageUpdate | 0.6K | G (rAF) |
| remote live stroke/erase preview functions (redrawRemoteLive..., drawRemoteLiveStrokeAppend, applyRemoteLiveStrokePreview, clear..., commitRemoteLiveStroke, flushRemoteLiveStrokeCommits, getRemoteLiveErasePreviewStrokes, applyRemoteLiveErasePreview, clearRemoteLiveErasePreviews; lines 3659-4123, about 14 KB) | 14K | G (Maps of packets plus canvas drawing, TTL timers) and collaboration-owned; not touched here |

### 4.9 Viewport transform (about 38 KB over 16426-17405; 20K of it is pointer/toolbar glue)
| Function | Bytes | Class |
|---|---|---|
| clampPanToVisible, getBottomLimit | 1.3K | P |
| updateAddPageIndicator | 1.3K | mixed: opacity maths P, style writes G |
| applyVerticalPan | 3.3K | P state machine (overscroll, rubber band) with timer glue (overscrollResetTimeout stays JS) |
| finalizePendingPageCreation | 1.2K | decision P, addPage/animatePanTo G |
| animatePanTo, stopInertia, applyViewportTransformStyle, scheduleViewportTransformRender, updateTransform | 2.4K | G (CSS transform, rAF) |
| startInertia | 1.5K | step maths P (decay, stop test, per-frame cap); rAF loop G |
| handleWheel | 3.1K | maths P (zoom around cursor, delta normalise, cap); event/DOM guard G |
| handleViewportPointerDown / Move / Up | 13.7K | maths P (pinch zoom anchor, velocity smoothing, scroll-lock classification); pointer map, cursor, flags G |
| clampPanDelta, capPanStep, getPointerDistance | 1.5K | P |
| preserveViewOnResize | 0.8K | anchor maths P, rAF G |
| setZoom | 0.5K | P clamp |
| findTodayCalendarPageIndex | 2.8K | P (date key logic; today key supplied by JS) |
| scrollToTodayCalendarPage | 3.0K | fit-zoom and pan maths P, DOM reads/rAF correction G |
| setupToolbarDrag | 10.7K | G (DOM drag of toolbar, snapping tied to element rects) |

### 4.20 Manage-Pages panel (31 KB; maths/layout parts only)
| Function | Bytes | Class |
|---|---|---|
| openPagesPanel, closePagesPanel, scanner open/close/post/handleBridgeMessage, importScannedPagesFromBridge (4.5K), clonePageSnapshot, applyPageSnapshot, createBlankPageData, cover modal UI functions (updateCoverModalPreview, setCoverModalFile, etc.) | about 17K | G (DOM, Blob, postMessage, state mutation; createBlankPageData depends on external id/normalise helpers) |
| validateScannerPages | 2.6K | G-ish (Blob duck typing, throws UI errors) |
| validateScannedInk | 1.4K | P (numeric sanitising of stroke points; verified in unit 5 before moving) |
| getScannedPageLogicalSize, getContainedRect, formatPageMillimetres, formatPageAspectRatio, getStandardPageName, getPageSizeInfo, getDocumentBasePageDimensions, getCoverResultLayout | about 5.6K | P |

Already ported: nothing.

## 2. Units (ordered: highest speedup first, then lowest risk)

Call convention: hot per-frame paths use `#[no_mangle] extern "C"` functions over a shared Float64Array scratch region
(no JSON allocation at 120-240 Hz). Per-event-rare paths use `callJson('editor_view.*')`.
Differential tests: tests/editor-view-diff.test.mjs using `loadLegacy('app-v5.js')`-style extraction of the original
functions, seeded random inputs, byte-exact (f64 identical; the JS maths is plain IEEE ops, so use the same operation order).

### Unit 1: Page layout cache + visible-window planner (speedup HIGH, risk medium, about 10 KB JS)
Scope: `getPageLayoutMetrics` build loop, `findFirstPageWhoseBottomReaches`, `findLastPageWhoseTopPrecedes`, `getPageBounds`,
`getContentHeight` (16330-16425, see coordination note); `getVisiblePageRange`, `getViewportVisiblePageIndices`,
`expandPageIndices` (3437-3487); the planning half of `updateVisiblePages` (3492-3643: direct/visible set, distance sort around
view centre, MAX_ACTIVE_PAGES cap, ordered active list with selection/active-page priority, pending-activation prune list,
release list).
Why first: layout rebuild is O(pages) with a per-page dimension lookup and 4 array pushes, and the visible plan runs on every
scroll settle with sorts over the buffered window; on 500+ page PDFs this is the main non-render cost of scrolling.
Boundary: JS uploads per-structure-change `Float64Array widths, heights, calendarSpace` (once per page-count/size/calendar change, cached
inside WASM, a version integer guards stale use) and `maxWidth, padding, gap, A4 fallback`. Per update it passes a small f64 scratch
(panX, panY, zoom, rectW, rectH, buffer, maxActive, selection index, activePage index) plus an `Uint8Array` of currently active
pages; Rust returns `Int32Array` regions: directlyVisible, visibleIndices, orderedActive, toRelease, rangeStart/End. Layout arrays
for the few JS readers (`metrics.tops` etc.) are exposed as typed-array views over WASM memory (re-read `memory.buffer` per call).
DOM calls (ensurePageCanvas, applyPagePreview, queuePageActivation, releasePageCanvas) stay in JS and iterate the returned lists.
Tests: pages-panel-performance.spec, editor-performance.spec, editor-repaint-continuity.spec, preview-stability.spec,
app-resilience.spec, pdf-nondestructive-erase.spec; `node tests/release-v5-smoke.mjs` (folder-navigation layout parity);
differential fuzz on page counts 0..2000 with mixed sizes and zoom 0.5..3.

### Unit 2: Gesture and pan/zoom maths (speedup MEDIUM, risk low, about 10 KB JS)
Scope: `clampPanToVisible`, `getBottomLimit`, `clampPanDelta`, `capPanStep`, `getPointerDistance`, `setZoom` clamp,
`preserveViewOnResize` anchor maths, wheel branch of `handleWheel` (delta-mode scale, ctrl zoom around cursor, capped pan),
pinch-zoom anchor maths and one-finger velocity smoothing / scroll-lock classification inside `handleViewportPointerDown/Move/Up`,
per-frame decay/stop/cap step of `startInertia`, fit-zoom and pan target maths in `scrollToTodayCalendarPage`.
Boundary: scratch Float64Array in/out (fixed field layout documented in the module: pan, zoom, rect, bounds, velocities,
pinch initial state, lock state encoded as 0/1/2/3). Each call returns the new pan/zoom/velocity/lock. JS keeps listeners, pointer map,
`state` writes, cursor, rAF scheduling.
Tests: editor-pen-input.spec, editor-performance.spec, editor-repaint-continuity.spec, app-resilience.spec,
pages-panel-performance.spec; differential fuzz for each function; new unit test that replays recorded pointer traces through legacy and Rust.

### Unit 3: Overscroll / add-page state machine (speedup LOW-MEDIUM, risk medium, about 6 KB JS)
Scope: `applyVerticalPan`, the opacity computation of `updateAddPageIndicator`, the decision of `finalizePendingPageCreation`
(canCreate, pendingPageCreate, overscrollY, pageCreateLocked), and the overscroll reset values (the 600 ms timer and style writes stay in JS).
Boundary: scratch Float64Array (panY, overscrollY, flags, contentHeight, viewport height, zoom, readOnly, constants OVERSCROLL_*)
in, new values plus an action code (none / schedule reset / create page) out.
Tests: editor-*.spec (editor-performance, editor-repaint-continuity), save-priority.spec (page creation through overscroll),
differential fuzz over sequences of dy values compared against legacy state evolution.

### Unit 4: Activation and PDF-cache scheduling policies (speedup LOW, risk low, about 4 KB JS)
Scope: `queuePageActivation` priority rule, ordering choice of `processPendingPageActivations`, LRU eviction plan of
`prunePdfBackgroundCache` / `touchPdfBackgroundCache` (Rust returns the keys to evict from a typed list of
(page, lastUsed, bytes) entries; Map and data URLs stay JS), `getPdfBackgroundScale` (pure tiers by page count and file size).
Boundary: Int32Array/Float64Array triples in, Int32Array out; `getPdfBackgroundScale` via callJson scalar.
Tests: pdf-nondestructive-erase.spec, pages-panel-performance.spec, preview-stability.spec, large-pdf checks in release-v5-smoke.

### Unit 5: Page-size, cover-layout and calendar-page helpers (speedup LOW, risk low, about 9 KB JS)
Scope: `getContainedRect`, `formatPageMillimetres`, `formatPageAspectRatio`, `getStandardPageName`, `getPageSizeInfo`,
`getDocumentBasePageDimensions` (mode-of-sizes with first-seen tie break), `getCoverResultLayout`, `getScannedPageLogicalSize`,
`validateScannedInk` (after confirming it has no DOM dependency), `findTodayCalendarPageIndex` (JS passes today key and the
per-page dateKeys list plus `calendarPageConfig`, since Date stays a browser value).
Boundary: JSON (small, event-driven, not per frame). Number formatting (`toFixed`, rounding) must match JS exactly; use a shared
formatter with tests on 100k random values, otherwise leave the string formatting in JS and port only the numbers.
Tests: pages-panel-performance.spec (size labels), save-*.spec indirectly through page size persistence, scanner-live-detection.test.mjs,
folder-navigation spec in release-v5-smoke; differential fuzz.

Estimated JS removed once all units land: about 45 KB of the 138 KB owned (the rest is DOM glue).

## 3. Not portable (stays JS)
See the `notPortable` list in the structured output: DOM element/canvas creation and release, pdf.js rendering and its queues/timers,
rAF/timeout schedulers, pointer event plumbing and pointer map, CSS transform writes, toolbar drag, remote live-stroke preview
canvases (collaboration-owned), Blob/scanner bridge and modals, cover modal DOM, state-flag resets.
