# scan_light port plan (scanner/processing/lightweight.js, 296,777 B, 5,781 lines)

Status: U1 (stencil removal) DONE; U2-U10 not started. Rust module: `rust-core/src/scan_light.rs` (+ submodules). JS adapter: between `stream:scan_light` markers of `rust-core-v5.js`.

## 1. Inventory and classification (line ranges are of the pre-port file)

| Lines | Bytes | Block | Class |
|---|---|---|---|
| 1-37 | 3.8K | constants, clamp, yieldToBrowser, createScaledCanvas | glue (canvas/scheduler) |
| 38-71 | | detectionImage (getImageData + exposure gain loop) | glue + tiny kernel (gain loop goes to unit 3) |
| 72-333 | 12.4K | isYellow, hasLocalYellowRidge, estimatePaperYellowBaseline, isFaintYellow, stencil tone/band/sample*/localStencilEvidence, neutralCalibrationSupport, hasPaperOnBothSides | portable (pixel predicates) |
| 334-505 | 6.9K | robustLine, ransacLine, lineIntersection, fallbackPageQuad, polygonArea, isPlausibleQuad, convexHull, cornerFromHull | portable (pure geometry) |
| 506-617 | 6.5K | detectPaperQuad | portable (pixels) |
| 618-675 | | bilinear, extrapolateStencil, quadFitsObservedImage, invertBilinear | portable (pure) |
| 676-1143 | 22.9K | traceFrameSide, traceFrameGuide, refineStencilQuad, traceLowerStencilRails | portable (mask sampling + fitting) |
| 1144-1504 | 17.8K | traceYellowFrame | portable |
| 1505-1634 | 5.6K | sampleCurve, mapFramePoint, `Light.mapDetectedFrame` | portable, but mapFramePoint delegates to `SP.MetricSurface.map` (other area): keep that branch in JS |
| 1635-1898 | 27.2K (with 1899-2217) | chromaticMarkerKind, findChromaticComponents, markerAnchoredPaperCandidates | portable (connected components) |
| 1899-2217 | | guidedRansacLine, medianNumber, estimateCalibrationStrip, fitMarkerAxis, blackMarkerIsPresent | portable |
| 2218-2661 | 21.1K | detectMarkerGuidedStencil | portable |
| 2662-2744 | 4.6K | observedLiveFrame, contourDistance | portable (JSON + pixels) |
| 2745-3306 | 30.6K | detectPageAtScale (canvas creation + mask loops + RANSAC glue + decision logic) | split: 2748-2751 glue; 2756-2830 and 2993-3033 pixel kernels; rest orchestration |
| 3307-3433 | 15.6K | refineObservedFrame, measureFrameEvidence | portable |
| 3434-3604 | | Light.detectPage / detectPageAsync (two-pass decision, MetricSurface/LiveDetection calls, Worker + ImageBitmap) | glue; the 2-pass decision predicate (3445-3473) is portable but trivial |
| 3605-3645, 3702-3744 | | decodeScanBlob, scanPreviewBlob, encodeScanCanvas, separateScanInk | glue (decode/encode); separateInk ALREADY ported (JS fallback loop 3733-3741 remains) |
| 3646-3701 | 5.6K | warpedObservations | portable (pure geometry over mesh) |
| 3745-3978 | 14.4K | processScanBlob, Light.processPageAsync (worker) | glue (canvas, progress, cancel, worker) |
| 3979-4124 | 6.9K | checkProcessingCancelled, projectivePageMap, Light.warp | glue + mesh build (small, stays); inner warp raster ALREADY ported (`InhouseRust.warp`), JS fallback 4068-4113 (2.5K) remains |
| 4125-4373 | 11.8K | medianSample, sampleCalibrationMarker, solve4, buildColorMatrix, percentileFromHistogram, estimatePaperProfile, medianColorSamples, referenceKindForPixel, calibrateFromReference, markerLooksValid | portable |
| 4374-4901 | 28.3K | removePhotographedStencil | portable (per-pixel at output resolution, ~2546x1800: the hottest remaining JS loop) |
| 4902-5240 | 22.2K | smoothPaperIllumination, observedPaperIllumination | portable (the 5190-5193 reduce canvas stays JS) |
| 5241-5282 | 2.1K | balanceObservedPaper | ALREADY ported (`InhouseRust.balancePaper`); JS fallback loop 5251-5278 remains |
| 5283-5764 | 31.0K | Light.correctColors | mixed: ImageData/canvas/progress glue; illumination field fallback 5317-5357 and sampling/quality scoring portable; pixel loop ALREADY ported (`InhouseRust.scanColors`) with JS fallback 5498-5696 (13.6K) remaining |
| 5765-5781 | | createDetectionPreview, SP.detectPageEdges | glue |

Already ported kernels: warp raster, balancePaper, scanColors (correct_scan_rows in lib.rs), separateInk. Their JS fallbacks (about 18.7K) must be deleted.

## 2. Units (ordered by speedup, then risk)

Common: all Rust entries registered in `rust-core-v5.js` markers as `InhouseRust.scanLight.*`; pixels cross as `Uint8ClampedArray` RGBA copied into wasm memory (re-read `memory.buffer` after each call); scalars/quads/point lists cross as JSON via `callJson('scan_light.*')` or as `Float64Array`. Differential test per unit: `tests/scan-light-<unit>.test.mjs` using `tests/helpers/legacy.mjs` `loadLegacy('scanner/processing/lightweight.js')`. Functions are private to the IIFE, so the legacy loader must be given a stub `window/document/performance` and the harness must append a test-only export of the private functions (done in the test, not in the product file). Compare on fixtures in `tests/e2e/fixtures/` plus several thousand seeded random buffers; byte-exact for typed output, exact-or-1e-9 for floats (same operation order, f64 everywhere, mimic `Math.round`/`Math.floor` semantics and `Uint8ClampedArray` round-half-even on stores).

### U1 stencil_removal (speedup HIGH, risk medium, ~28 KB)
- Scope: `removePhotographedStencil` 4374-4901 + `isYellow` (72-90). Keep only the async/yield/cancel loop and the `settings` plumbing in JS: Rust processes bands (`rows(from,to)` style like scanColors) so cancel/yield still work, or one call in a worker.
- Boundary: `data` Uint8ClampedArray RGBA in/out in place; params (width, height, pxPerCm, targetPaper, observed dot points, flags) as JSON; returns JSON stencilCleanup stats.
- Tests: scanner-observed-dot-cleanup, scanner-neutral-colors, scanner-paper-contrast, scanner-reference-colors, scanner-color-lighting, scanner-rust-colors, scanner-processing-visual specs; scanner-visual-validation.mjs and scanner-synthetic-visual-validation.mjs.

### U2 illumination_profile (speedup HIGH, risk medium, ~37 KB)
- Scope: 4125-4373 (calibration sampling, solve4, buildColorMatrix, estimatePaperProfile, calibrateFromReference, ...) + `smoothPaperIllumination` and `observedPaperIllumination` 4902-5240.
- Boundary: in RGBA `data` + canvas dims + dot observations JSON (+ calibration targets JSON); out `Float32Array` fields (illumination, white, neutral, drawing) and JSON (paperPoints, confidence, profile, colour matrices). JS keeps the 192 px reduce canvas drawImage (5190-5193, 5317-5330) and passes its pixels.
- Tests: scanner-lighting-pipeline, scanner-marker-lighting, scanner-reference-colors, scanner-neutral-colors, scanner-color-lighting, scanner-rust-colors, scanner-dot-certification specs.

### U3 detection_pixel_kernels (speedup HIGH, risk medium, ~30 KB)
- Scope: 72-333 predicates, `detectPaperQuad` 506-617 (+ convexHull/cornerFromHull/polygonArea 436-505, bilinear/invertBilinear 618-675 as needed), the exposure-gain loop of `detectionImage` (60-69), the yellow mask / traceMask loop 2756-2788, the live-seed rescue block 2790-2828, and the boundary mask + side-point extraction 2993-3033.
- Boundary: in RGBA analysis pixels (<=800 px long edge), w, h, paperQuad (Float64Array 8, or null), options JSON; out `Uint8Array` mask + traceMask, paperQuad JSON, yellowBaseline JSON, side point lists as Float64Array.
- Tests: scanner-geometry, scanner-layout-detection, scanner-scene-regression, scanner-faint-dots, scanner-paper-contrast, scanner-live-resilience specs; `node --test tests/scanner-live-detection.test.mjs` (shares the detection fixtures; guards that nothing regressed).

### U4 frame_tracing (speedup MEDIUM, risk medium, ~40 KB)
- Scope: 676-1504 (traceFrameSide, traceFrameGuide, refineStencilQuad, traceLowerStencilRails, traceYellowFrame).
- Boundary: in mask `Uint8Array` + w,h + quad/point JSON; out JSON frame (paths, support, curvature, box).
- Tests: scanner-geometry, scanner-surface, scanner-metric-surface, scanner-lattice-recovery, scanner-grid-parallels, scanner-adaptive-recovery specs.

### U5 marker_components (speedup MEDIUM, risk medium, ~27 KB)
- Scope: 1635-1898 (chromaticMarkerKind, findChromaticComponents, markerAnchoredPaperCandidates) and the neutral band diagnostics block 2833-2992.
- Boundary: RGBA pixels + mask Uint8Array in, JSON candidates out.
- Tests: scanner-marker-lighting, scanner-dot-registration, scanner-dot-registration-geometry, scanner-required-grid, scanner-bullet-pipeline specs.

### U6 marker_guided_stencil (speedup MEDIUM, risk medium-high, ~38 KB)
- Scope: 1899-2217 (guidedRansacLine, estimateCalibrationStrip, fitMarkerAxis, blackMarkerIsPresent, medianNumber), 2218-2661 detectMarkerGuidedStencil, 2662-2744 observedLiveFrame/contourDistance.
- Boundary: RGBA/mask in, JSON in/out. Data are small point lists; the cost is in the pixel probes.
- Tests: scanner-marker-lighting, scanner-geometry, scanner-live-detection.spec, scanner-bullet-refinement, scanner-capture-regression specs.

### U7 geometry_helpers_refine (speedup LOW-MEDIUM, risk low, ~34 KB)
- Scope: 334-505 (robustLine, ransacLine, lineIntersection, fallbackPageQuad, isPlausibleQuad), 629-655 (extrapolateStencil, quadFitsObservedImage), 1505-1634 sampleCurve/rotate (except the `SP.MetricSurface.map` branch), 3307-3433 refineObservedFrame/measureFrameEvidence, 3646-3701 warpedObservations. Pure JSON in/out (random-input differential tests are easy and cheap, so this is the safest unit; can be pulled forward as a warm-up if the schedule requires).
- Tests: scanner-geometry, scanner-surface, scanner-dot-certification, scanner-observed-dot-cleanup specs.

### U8 color_orchestration (speedup MEDIUM, risk medium, ~17 KB after fallback removal)
- Scope: the non-glue remainder of `correctColors` 5283-5497 (reference/marker sampling, localPaperAt, colorParameters assembly) and 5700-5740 (paper quality scoring, coverage regions).
- Boundary: RGBA + illumination arrays in, `Float32Array` colorParameters + JSON quality out; feeds the already ported `scanColors`.
- Tests: scanner-color-lighting, scanner-reference-colors, scanner-neutral-colors, scanner-rust-colors, scanner-processing, scanner-ink-legibility specs.

### U9 detection_orchestration (speedup LOW, risk high, ~30 KB)
- Scope: decision logic of `detectPageAtScale` (2829-2992 remainder, 3034-3306: candidate selection, reliableAttempt, finish, scaleFrame) and the two-pass predicate of `Light.detectPage` (3445-3473) as one `decide_detection` call over the U3-U7 results. Do last, after the pieces exist; JS keeps only canvas scaling and the calls to MetricSurface/LiveDetection/SurfaceParameterization.
- Boundary: JSON in/out.
- Tests: whole scanner-scene-regression, scanner-geometry, scanner-live-resilience, scanner-processing, scanner-worker-pipeline, scanner-speed-profile specs + both visual validation scripts.

### U10 delete_ported_fallbacks (speedup none, risk lowest, ~18.7 KB removed)
- Scope: delete JS fallbacks of kernels already in Rust: Light.warp loop 4068-4113 (and the `disableRust` option), balanceObservedPaper loop 5251-5278, correctColors loop 5498-5696, separateScanInk loop 3733-3741. Throw a clear error if WASM is missing. Can be done at any moment (even first) because it needs no new Rust; it gives the largest immediate percentage drop per hour. Check that tests using `disableRust` (scanner-rust-colors, scanner-rust-surface) are adapted by comparing against stored fixtures/legacy loader instead of the JS fallback, never by loosening them; update `tests/release-v5-smoke.mjs` greps.
- Tests: scanner-rust-colors, scanner-rust-surface, scanner-processing, scanner-raster-regression, scanner-native-template specs.

## 3. Not portable (stays JS)
- createScaledCanvas, decodeScanBlob, encodeScanCanvas, scanPreviewBlob, createDetectionPreview, all getImageData/putImageData/drawImage calls: Canvas/ImageBitmap/OffscreenCanvas/Blob browser APIs (browser downscale with `imageSmoothingQuality` also defines pixels, so resampling here cannot be reproduced bit-exactly).
- Light.detectPageAsync / Light.processPageAsync worker spawn, postMessage, `moduleUrl`, terminate: Worker API.
- processScanBlob, progress callbacks, checkProcessingCancelled, yieldToBrowser, `performance.now` profiling: scheduler/events/cancellation glue.
- Calls into other modules (`SP.LiveDetection.*`, `SP.MetricSurface.*`, `SP.SurfaceParameterization`, `SP.InkVectorizer`, `SP.ScanTemplate`): owned by other areas (scan_live, scan_geometry, ink); the call sites stay and receive Rust output.
- Light.warp mesh/option plumbing and projectivePageMap (small, closure over callbacks and `mapFramePoint` into MetricSurface).

## 4. Risks / parity notes
- JS `Uint8ClampedArray` stores round half to even; `Math.round` rounds half up (toward +inf); sort stability and NaN handling in RANSAC must be reproduced exactly.
- RANSAC / candidate ordering depend on iteration order and tie-breaks: port line by line, no refactors.
- U1 and U2 change time at 2546 px output; keep the band API so the UI thread still yields.

## 5. Progress log

### U1 stencil_removal: DONE
- Rust: `rust-core/src/scan_light/stencil.rs` (state machine `Stencil`, `is_yellow`, exports `scan_stencil_new/step/result/drop`, `scan_is_yellow` for tests). Adapter: `InhouseRust.scanLight.stencil(...)` / `.isYellow` between the `stream:scan_light` markers of `rust-core-v5.js`.
- JS: `removePhotographedStencil` (527 lines, 28.3 KB) replaced by a 17-line loop that awaits `InhouseRust.ready`, throws if WASM is missing, calls `step()` until done and yields/cancels between slices (a yield every 8 slices). Pixels cross as one copy in and one copy out of wasm memory; observations as a `Float64Array` (5 values per dot); stats come back as 7 f64 and keep the original object shape.
- `isYellow` (JS, 19 lines) stays for now: it is still called per pixel by `detectPageAtScale`, `estimatePaperProfile`, `referenceKindForPixel`, `correctColors` (units U3/U8/U2). Delete it with the last of those; Rust already has the port and a differential test over all 16.7 M colours.
- Slices: ~120 per page (observed-dot path 117, phase search path 122), each ~1-3 ms at 2546 px, so the UI thread can still yield and cancel.
- Parity details reproduced on purpose: `Math.round` ties up (not `f64::round`), V8's `Math.hypot` Kahan algorithm (glyph disc and reference dot edges depend on it), typed-array out-of-range reads (NaN, counted in the glyph histogram but without a bin), linear pixel offsets wrapping onto adjacent rows, `Uint8Array` truncation vs `Uint8ClampedArray` round-half-even stores, NaN-propagating comparisons.
- Tests: `cargo test` (8 unit tests), `node --test tests/scan-light-stencil.test.mjs` (differential vs the pre-port file from git: all 16.7 M colours for isYellow, 12 observed-dot pages, 12 recovered-phase pages, 16 chromatic/sparse/jittered variants, malformed observation records, 3000 seeded noise pages with tiny sizes and fractional targets, 40 random scales, the browser-spec scene), all byte-exact including statistics. Benchmark: `node tests/scan-light-stencil.bench.mjs`.
- Speed (2546 x 1800 page, Node 22, loaded shared machine): original JS ~5.0 s (observed-dot path) and ~6.6 s (phase search) vs Rust ~155 ms and ~246 ms including the memory copies: about 32x and 27x.
- Bytes: JS removed 27,350 B (`lightweight.js` 296,777 -> 269,427 B); JS adapter added 3.9 KB to `rust-core-v5.js` (not counted by GitHub); Rust added about 35 KB logic + 5 KB tests.
- Remaining units: U2-U10 unchanged.
