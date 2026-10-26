# Port plan: scan_color

Files owned (bytes / lines): colorTransform.js 64485/1849, colourValues.js 23665/768,
capture-quality.js 16249/265, capture-quality-worker.js 637/16. Total 105 KB.

## Key finding: 88 KB of the area is dead code
`scanner/processing/colorTransform.js` and `colourValues.js` are NOT loaded by `scanner/index.html`, `index.html`,
any worker or any test (grep over the repo, excluding generated `offline-assets.js`, `native-assets.json`, `rebuild-v2/dist`,
finds no `<script>`/`importScripts` reference). Their job (colour transform, tone/contrast/exposure/colour passes, marker
colour scan, dot-grid removal) is already done by the Rust path `InhouseRust.scanColors / balancePaper / separateInk`
called from `scanner/processing/lightweight.js` (lines ~3733, 5244, 5487; owned by another area). The only live
references are `scanner/script.js:134` (`SP.clearColorTransformCache` guarded by `typeof === 'function'`, harmless, other
area) and the generated `native-assets.json`. Deleting them removes the JS without any behaviour change; porting them would
add unused Rust. Recommendation: delete (unit 1), port only the live code (units 2-3).

## 1. Inventory and classification
### colorTransform.js (IIFE, `SP.*` exports)
| Block | Lines | Class |
|---|---|---|
| solve4 (4x4 Gauss) + SP.buildTransform | 6-185 | pure, DEAD (not loaded) |
| edgeBounds/colorRectBounds/getDotColors/pctFromHist/dotColorDist, SP.clearColorTransformCache | 101-204 | pure+cache, DEAD |
| processPixel (per-pixel colour/yellow/snap maths) | 205-446 | pure, DEAD, twin of Rust `scan_colors` |
| floatsToCanvas, applyMatrixPass/Contrast/Exposure/Tone/ColorPass | 453-1002 | pure loops + canvas glue, DEAD |
| SP.applyColorStepsAsync / applyBalanceStepsAsync / applyColorAsync / applyBalanceAsync | 1003-1233 | glue (canvas, await), DEAD |
| restoreMargins, removeEdgeYellowZones, removeDotGrid (flood fill, bounds) | 1234-1849 | pure+canvas, DEAD |

### colourValues.js
| Block | Lines | Class |
|---|---|---|
| colorDist, getColorRect | 6-38 | pure, DEAD |
| SP.scanColors (marker/yellow circle sampling, yellow components) | 39-707 | pure on RGBA, DEAD |
| SP.findBlackBlob, SP.alignMarker | 708-768 | pure+canvas, DEAD |

### capture-quality.js (LIVE: loaded by scanner/index.html, used in live-camera.js and in a Worker)
| Block | Lines | Bytes | Class |
|---|---|---|---|
| readSource (canvas drawImage downscale to <=1100px, getImageData, luma Float32) | 7-22 | 1.1K | glue (canvas); only the luma loop is portable |
| inspect (9-zone fine/broad acutance, quad inside test, frame-edge sampling) | 23-62 | 2.9K | pure, portable |
| clamp/mix/along helpers | 63-74 | 0.7K | pure |
| inspectGrid (lattice/dot detection along quad, contrast sort, coverage) | 75-184 | 7.2K | pure, portable |
| inspectReadiness (orchestration + thresholds .42/.32) | 185-202 | 1.2K | pure orchestration |
| worker lifecycle (inspectReadinessAsync, disposeReadiness, timeout, abort, createImageBitmap) | 203-264 | 3.0K | glue |
### capture-quality-worker.js (16 lines): glue (OffscreenCanvas shim, postMessage). Stays JS.

Already ported: nothing in these files calls Rust yet; the Rust twins (`correct_scan_rows`, `balance_paper_rows`,
`separate_ink`, `scan_colors`) exist in lib.rs and are consumed from lightweight.js, so the JS twins here are dead.

## 2. Units (ordered: speedup first, then lowest risk)

### Unit 1: capture quality kernels (inspect + inspectGrid + inspectReadiness)  [speedup: HIGH, risk: medium]
- Scope: capture-quality.js `inspect` (23-62), `along/mix/clamp` (63-74), `inspectGrid` (75-184), decision logic of
  `inspectReadiness` (185-202). About 11 KB JS removed (of 16 KB).
- Why high: runs on every candidate frame during live capture on up to 1100px rasters, per-pixel gradients at stride 2 plus
  a 5x5 neighbourhood around every frame-edge point; currently inside a 2 s-bounded worker.
- Boundary (typed arrays + small JSON): `gray: Float32Array(width*height)`, width/height, quad as `Float64Array(8)`
  (normalised x,y), frame edges as flat `Float64Array` plus offsets `Uint32Array`. Return a Float64Array/fixed-layout
  result or JSON via `callJson('scan_color.inspect_readiness')` (result object is tiny: valid, clarity, border, interior,
  zones, edges, interiorMinimum, reason, grid{valid,dots,matched,rows,columns,coverage,confidence,reason}). Hot-kernel
  export `scan_color_inspect(ptr,...)` for the raster, JSON for the result. Keep `readSource` (canvas) in JS but move the
  luma loop (RGBA -> Float32 gray, .2126/.7152/.0722) into a Rust `rgba_to_luma` so no JS pixel loop remains; JS passes the
  `ImageData.data` Uint8ClampedArray.
- Adapter: between `// >>> stream:scan_color` markers in rust-core-v5.js (`api.captureQuality(...)`). The worker must
  `importScripts` the rust-core loader or keep using main-thread fallback; verify the worker can instantiate the wasm
  (WASM module is already a single file; worker needs `await InhouseRust.ready`, throw clear error otherwise, never degrade
  silently). If worker instantiation is unreasonable, call Rust directly on the main thread (it will be far faster than the
  JS) and keep the worker only as the abort/timeout wrapper.
- Tests: tests/e2e/scanner-capture-readiness.spec.mjs, scanner-slow-readiness.spec.mjs, scanner-photo-focus.spec.mjs,
  scanner-live-resilience.spec.mjs, live-scanner.spec.mjs, scanner-moving-capture.spec.mjs;
  `node --test tests/scanner-live-detection.test.mjs`. New differential test `tests/scan-color-quality.test.mjs` using
  `loadLegacy('scanner/processing/capture-quality.js')` with a stubbed canvas (the module only needs `readSource` raster:
  pass `raster` as the 3rd arg of `inspect`/`inspectGrid`) over fixtures (tests/e2e/fixtures/scanner-*.js) and seeded random
  gray rasters and quads. Float32 gray + f64 sums: keep identical operation order for bit-exactness (accumulations in f64;
  gray stored as f32 in both). `Math.round`, `Math.floor` and sort order must match.

### Unit 2: delete dead colour pipeline JS  [speedup: none (bundle/parse size), risk: low]
- Scope: remove `scanner/processing/colorTransform.js` (64 KB) and `colourValues.js` (24 KB) in full; both are loaded nowhere.
  Remove `/scanner/processing/colorTransform.js` and `colourValues.js` from the integrator-owned generated lists
  (`native-assets.json`, `offline-assets.js`): NOT committed by this branch, list in the report for the integrator.
  `scanner/script.js:134-137` guard is outside this area; leave it (it is `typeof` guarded) and report it.
- Do it first in time (cheap, biggest percentage change) even though it is ordered after Unit 1 for speed.
- Pre-check before deleting: re-grep the integrated main for any new `<script>`/`importScripts`/dynamic loader of those
  names; run `node tests/release-v5-smoke.mjs` and `scanner-processing.spec.mjs`, `scanner-rust-colors.spec.mjs`,
  `scanner-color-lighting.spec.mjs`, `scanner-reference-colors.spec.mjs`, `scanner-neutral-colors.spec.mjs`, and
  `scanner-processing-visual.spec.mjs`. Data crossing boundary: none.
- Parity doubt to mark in the report: if the product owner wants these as a documented offline reference, move them to
  `tests/helpers/` as legacy fixtures instead of deleting (still not shipped, counts as JS), but nothing requires them.

### Unit 3 (optional, only if the integrator resurrects the dead paths): Rust colour transform  [speedup: medium, risk: high]
Not planned. If it is ever needed: `solve4` + `buildTransform`, `processPixel` and the five passes (453-1002) map onto the
existing `scan_colors` in lib.rs; `removeDotGrid`/`removeEdgeYellowZones` (flood fills, 1277-1849) and
colourValues `scanColors` (39-707) are pure RGBA to RGBA/JSON kernels. They would use `Uint8ClampedArray` in/out and a
`Float64Array` config. Total about 88 KB of JS, so it would need splitting into 3 units of <=40 KB. Current evidence says
no caller exists, so do not spend effort.

## 3. Test coverage summary
- Unit tests: `node --test tests/scanner-live-detection.test.mjs`; `cargo test --manifest-path rust-core/Cargo.toml`;
  `node tests/release-v5-smoke.mjs` (update greps if they reference removed names).
- Visual validation: `node tests/scanner-visual-validation.mjs` and `scanner-synthetic-visual-validation.mjs` exercise the
  lightweight Rust colour path (unchanged by units 1-2); run as regression.
- Browser (via e2e.sh lock only): scanner-capture-readiness, scanner-slow-readiness, scanner-photo-focus,
  scanner-live-resilience, live-scanner, scanner-moving-capture (unit 1); scanner-processing, scanner-rust-colors,
  scanner-color-lighting, scanner-reference-colors, scanner-neutral-colors (unit 2 regression).

## 4. Not portable (stays JS)
- `readSource` canvas drawImage downscale + getImageData: browser canvas API (only the luma loop moves).
- Worker lifecycle in capture-quality.js (203-264), capture-quality-worker.js: Worker, OffscreenCanvas, createImageBitmap,
  postMessage, AbortSignal, timers.
- `SP.CaptureQuality` public export object and `SP.*` global wiring (callers in live-camera.js depend on it).

## 5. Status
Units done: 1/2 (unit 2, deleting the dead colour pipeline, is still open and is not part of unit 1).

### Unit 1: capture quality kernels - DONE
- Rust: `rust-core/src/scan_color/quality.rs` (acutance `inspect`, lattice `inspect_grid`, `readiness` decision, RGBA to luma),
  exported as `scan_color_luma` and `scan_color_quality(mode, ...)` (mode 0 acutance, 1 lattice, 2 full readiness). Fixed-layout
  result: 20 f64 (`RESULT_LEN`); the host maps reason codes to the original strings.
- Adapter (`rust-core-v5.js`, `stream:scan_color`): `InhouseRust.captureQuality.{inspect,inspectGrid,inspectReadiness}` take
  `{width,height,rgba}` or `{width,height,gray}` and return objects with the original key order and values.
- JS removed: `capture-quality.js` 16249 -> 5553 bytes (-10696; `inspect`, `inspectGrid`, helpers, readiness decision and the luma loop).
  Worker file +161 bytes (loads the Rust core, awaits `InhouseRust.ready`, fails loudly). Adapter glue added: about 5.0 KB JS.
- Rust added: `quality.rs` 31221 bytes (about 21 KB code + 10 KB tests).
- Bit-exactness: all sums stay f64 in the original order, gray is f32 as in the Float32Array, `Math.round/max/min/sign` and V8's
  scaled Kahan `Math.hypot` are re-implemented (libm `hypot` differs by an ULP). `tests/scan-color-quality.test.mjs` compares
  against the pre-port source on 2000 small and 40 large seeded acutance scenes, 110 lattice scenes (valid lattices, clipped,
  texture and not-found outcomes), 40 full readiness runs from RGBA (luma included), the 1100 px downscale path and degenerate
  detections. Every result is identical, including key order (the test takes about 4 minutes, almost all of it the slow JS
  original; `QUALITY_SCALE=0.1` for a quick check, `QUALITY_SCALE=5` for a larger sample). A previous larger run (160 lattice,
  60 readiness, 60 large) was also identical. Rust unit tests: 6 in `quality.rs`.
- Intentional differences (documented, unreachable from the app): a non-finite coordinate in frame edges that the lattice
  resampling actually uses throws `RangeError` instead of yielding NaN garbage; a quad that does not have exactly four corners
  throws `RangeError` in `inspect` (the original threw `TypeError` for fewer than four and used a wider bounding box for more).
- Behaviour: the sync `SP.CaptureQuality.inspect/inspectGrid/inspectReadiness` now need the Rust core ready and throw
  `Rust core is not ready` otherwise; `inspectReadinessAsync` awaits `InhouseRust.ready` first. No JS fallback remains.
  The readiness worker is kept as the abort/timeout wrapper and instantiates the wasm itself (`importScripts('../../rust-core-v5.js')`).
- Speed (`node scripts/benchmark-scan-color-quality.mjs`, 1100x825 landscape page, 1855 matched dots, V8 for both, median of
  15-21 runs on a 4-core machine shared with other agents at load 5-10, so only the ratios mean something):
  acutance 16-35 ms JS vs 6-14 ms Rust (2.5-3x), lattice preflight 40-64 ms vs 21-32 ms (1.7-2x), full readiness
  (including RGBA to luma and marshalling) 54-107 ms vs 32-65 ms (1.6-1.7x). The JS original was already JIT-friendly typed-array
  maths, so the gain is a small constant factor, not an order of magnitude; the resampling loop (437k pixels, about 60 flops each)
  dominates the lattice time in both. The win that matters is that the work runs without GC pressure (no per-sample objects, Map
  or string keys). Remaining headroom would need wasm SIMD (not enabled in this build).
- Browser specs run through the lock script: `scanner-capture-readiness` (9) and `scanner-photo-focus` pass, including the
  worker test that instantiates the wasm inside the readiness worker; `live-scanner` (15) passes; `scanner-slow-readiness`
  and `scanner-moving-capture` pass when run alone. `scanner-live-resilience` is load-sensitive: on this machine at load 5-6 the
  pre-port baseline fails 6 of 13 tests and this branch 3 (same timing-bound tests plus a `retained == missed` count), so
  the integrator should re-run it on a quiet machine.
