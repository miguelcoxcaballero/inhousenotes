# v6 input: existing Rust work, the TypeScript rebuild, and the WASM toolchain

Status: research input for the Inhouse Notes v6.0.0 plan (Rust/WebAssembly rebuild, identical UI and
features, network documents that sync instantly by design). Written 2026-10-29 against:

- `main` as checked out in this tree (`66c4afa`, v5.17.22);
- the unmerged branches `rust/*` (14 branches) and `rust-on-5.17` in `/home/user/inhousenotes`;
- `rebuild-v2/` in this tree.

Everything marked **measured** was run for this report in a scratch copy (no repository or branch was
modified). Everything marked **estimate** is reasoning, not a measurement, and needs a device check.

---

## 0. Recommendations in 15 lines

1. Write the v6 core from scratch as a **sans-IO Rust library**: document model, op log, causal merge, undo, layout, hit testing and render geometry live in Rust; it imports no browser API.
2. Keep a **thin hand-written TypeScript host** for DOM, canvas, IndexedDB, fetch, WebRTC, Workers and timers. Do not use `web-sys` for IndexedDB or WebRTC.
3. Use a **versioned binary ABI** between them: Rust owns the state, JS sends input events, and Rust returns command and effect buffers. Do not use a JSON bridge for anything on the editing path.
4. **Rust owns the strokes.** The v5 port kept JS objects as the source of truth, so marshalling capped its speedups. App JS shrank only 1.7%, while the adapter grew from 30 KB to 123 KB.
5. Render through command buffers. Rust computes v5.17.22's centripetal Catmull-Rom Bézier controls and JS replays them into `Path2D`, which gives the same f64 maths and the same pixels.
6. Skip wasm-bindgen for v6.0. Per-call cost was the same as raw imports (≈32 ns per call, measured), it adds about 10 KB gz of glue, and it requires a CLI pinned to the exact crate version. Revisit it only for ergonomics.
7. Set a **size budget**: editor core ≤ 400 KB raw / ≤ 130 KB gz. Ship the scanner kernels and the v5-compatibility code as **separate, lazily loaded modules**. The monolithic port reached 916 KB (304 KB gz).
8. Reuse as-is: the scanner pixel and geometry kernels, `checkpoint.rs`, `pdf_ink.rs`, `docio/codec.rs`, `drive_home/palette.rs` and `folder.rs`. These are pure kernels with tests. Keep only one base64 implementation and one `hypot`.
9. Reuse the algorithms but rewrite the interfaces for: the resident stroke store and area-eraser split, the page layout and visible-window planner, the timeline diff, the library index and the work scheduler.
10. Isolate the v5-compatibility code (canonical hash, ES number formatting, STROKES_Z entries, v5 checkpoint words) in a `v5compat` module. Updates are optional and can be snoozed for 14 days, so v5 and v6 clients will share documents for a while.
11. Carry over from rebuild-v2: stable ids, every write as an invertible op, crash journal with idempotent replay, keys by identity, sidecar instead of PDF as the truth, and pure camera maths.
12. Fix rebuild-v2's gaps:
    - add causal clocks and op-level sync, not whole-snapshot "local wins";
    - use `If-Match` conditional writes;
    - make tombstones expire safely (FIFO eviction can resurrect deleted ink);
    - store f64 points;
    - share version history;
    - wire up the PDF worker;
    - match v5 rendering (no predicted tip).
13. Run the core on the main thread for latency, and run heavy jobs (export, scanner, large merges) in Workers on separate instances of the same compiled `WebAssembly.Module`. GitHub Pages cannot send COOP/COEP headers, so there are no wasm threads.
14. Load with `instantiateStreaming` (served as `application/wasm`). At boot, check the WebView version (≥ Chrome 96, required by Rust 1.90's default target features) and give a clear message instead of a silent failure.
15. Keep the reproducibility gate:
    - pinned `rust-toolchain.toml`, `--locked`, `--remap-path-prefix` once crates are used;
    - CI rebuild followed by `git diff --exit-code` on the `.wasm`;
    - run `cargo test` in CI (main does not today);
    - reuse the v5-as-oracle differential tests.

---

## 1. The Rust core on `main` (v5.17.22)

### 1.1 Layout and build

| Item | Value |
|---|---|
| Crate | `rust-core/` (`inhouse_notes_core`, edition 2021, `cdylib` + `rlib`), **no dependencies** (`Cargo.lock` lists only itself) |
| Source | 9 files, 1,696 lines: `lib.rs` (299), `ink.rs` (419), `surface.rs` (416), `ink_repair.rs` (146), `folder.rs` (123), `pdf_ink.rs` (101), `document_work.rs` (85), `checkpoint.rs` (72), `editor.rs` (35) |
| Profile | `opt-level=3`, `lto=true`, `codegen-units=1`, `panic="abort"`, `strip=true` |
| Toolchain | `rust-toolchain.toml`: channel `1.90.0`, profile `minimal`, target `wasm32-unknown-unknown` |
| Build | `scripts/build-rust-core.mjs` (`npm run build:rust`): `cargo build --release --target wasm32-unknown-unknown --locked`. It then asserts `WebAssembly.Module.imports(module).length === 0` and `core_abi_version() === 1` and copies the result to `wasm/inhouse_notes_core.wasm` |
| Output (measured) | 126,798 B raw, 49,901 B gzip -9, 186 functions, 33 function exports plus `memory`, `__data_end`, `__heap_base`; **0 imports**; SHA-256 `2cddd99269306979d68029fd9612ca398a0d9ab0eebbd1db3d2239519112d27c` |
| Loader | `rust-core-v5.js` (475 lines, 29,966 B), a classic script that sets `globalThis.InhouseRust`. It is loaded by `index.html` (`<script defer>`), `scanner/index.html`, `document-processing-worker-v5.js` and `scanner/processing/detection-worker.js` (`importScripts`), and is listed in `offline-assets.js` and `native-assets.json` |

**Reproducibility (CI).** `.github/workflows/production-checks.yml` runs `rustup toolchain install 1.90.0
--profile minimal --target wasm32-unknown-unknown`, then `npm run build:rust` and `npm run build:offline`, then
`git diff --exit-code -- wasm/inhouse_notes_core.wasm offline-assets.js native-assets.json index.html scanner/index.html vendor/fonts/ui.css`.
**Measured:** rebuilding from a different absolute path produced the identical SHA-256. The build is path-independent because
cargo passes workspace-relative paths for the local crate (`src/lib.rs`) and std paths are `/rustc/<commit>/…`.
With registry crates this no longer holds (see §2.4).

**Tests.**

- 7 `#[test]`s in Rust: `lib.rs` 2, `checkpoint.rs` 2, `document_work.rs` 1, `editor.rs` 1, `pdf_ink.rs` 1. All pass (measured), but **`cargo test` is not run in CI**.
- Node tests in `test:unit`, which load the committed wasm and the adapter in a `vm`:
  - `tests/rust-core.test.mjs` (12 tests: no imports, lossless RGBA split, warp parity with clamped canvas bytes, ink separation, surface contexts, ink axes, paper balance, cached-core fallback, checkpoint codec, PDF ink);
  - `tests/document-work-rust.test.mjs` (8);
  - `tests/rust-folder.test.mjs` (7). All 27 pass (measured).
- Browser specs: `rust-pixels`, `scanner-rust-colors`, `scanner-rust-surface`, `folder-rust`, `editor-rust-hit`, `ink-checkpoint` and several scanner specs.

### 1.2 Every export, what it does, and its JS callers

The adapter wraps every export. All wrappers return `null` or fall back to JavaScript when the core is missing,
and `ready` resolves to `false` on any load failure. Handles are released with `dispose()`.

| Export(s) | Module | What it does | `InhouseRust.*` wrapper | Production JS callers |
|---|---|---|---|---|
| `core_abi_version` → 1 | lib | ABI gate checked by the loader and the build script | `ready`, `available()` | every caller awaits `InhouseRust.ready` |
| `alloc(len)` / `free(ptr,len)` | lib | 8-byte-aligned host buffers (≤ 128 MB), **pre-filled with 255** (white pixels) | internal `allocation`, `release` | all wrappers |
| `warp_cell(src,sw,sh,dst,dw,dh,l,r,t,b, 4 corners)` | lib | Bilinear quad-to-rectangle mesh sampling with `Uint8ClampedArray` ties-to-even rounding | `warp(input,w,h)` → `{cell, copyTo, dispose}` | `scanner/processing/lightweight.js` `Light.warp` (page dewarp, ~l.4049) |
| `split_rgba(src,rgb,alpha,n)` | lib | Lossless RGBA → RGB + alpha planes; reports transparency | `splitRgba` | `document-processing-worker-v5.js` `preparePdfRaster` (PNG overlay cache) |
| `balance_paper_rows(...)` | lib | Spatially varying illuminant correction from paper rings around bullet dots, with an optional neutral (flare) correction and gamut projection | `balancePaper` | `lightweight.js` `balanceObservedPaper` |
| `correct_scan_rows(...)` (13 args, 61 f64 params) | lib | Per-pixel scan colour correction: paper field, drawing mask, reference-colour calibration, stencil/margin removal, optional coverage plane, row bands for cancellation | `scanColors` | `lightweight.js` `Light.correctColors` |
| `separate_ink(data,n)` | lib | Paper → transparent ink (alpha from the minimum channel) | `separateInk` | `lightweight.js` `separateScanInk` |
| `surface_fit_new/step/result/drop` | surface | "Developable pinhole" least-squares page-surface fit (stepped, cancellable context) | `surfaceFit` | `scanner/processing/metric-surface.js` `optimize` |
| `ink_geometry_version` → 2, `ink_axes(...)` | ink | Scan ink → palette-labelled medial axes plus distance radii (bounded budget; 2 = rejected) | `inkAxes` | `scanner/processing/ink-vectorizer.js` (`fromPixels`) |
| `ink_repair_discs(...)` | ink_repair | Chooses small pen discs inside the measured silhouette to restore omitted ink | `inkRepair` | **none.** Dead in production: `tests/e2e/scanner-ink-repair.spec.mjs` asserts `repairBackend === 'javascript'` |
| `folder_plan_version` → 1, `folder_motion(...)` | folder | 29-value plan for the Drive folder open/close animation (scale, offsets, perspective, easing, keyframes), handed to WAAPI | `folderMotion` | `app-v5.js` `playDriveFolderTransition` (3 sites) |
| `folder_fade(...)` | folder | Opacity keyframes for the label fade | `folderFade` | `app-v5.js` `playDriveFolderTransition` (2 sites) |
| `folder_rows_new/range/span/drop` | folder | Variable-height grid rows: prefix sums plus binary search for the visible range and span | `folderRows` | `app-v5.js` `readDriveFolderGridPlans` |
| `ink_checkpoint_version` → 1, `ink_checkpoint_pack/unpack` | checkpoint | Lossless 4-lane XOR + length-prefixed codec over 8-byte IEEE words (`xor-v1` format of offline ink checkpoints) | `checkpointCodec`, `unpackCheckpoint` (has a JS decoder fallback) | worker `packInkCheckpoint` and `unpackInkCheckpoint` tasks; `app-v5.js` `unpackInkPageFromStorage` |
| `editor_stroke_hit(ptr,n,x,y,r)` | editor | Exact segment-distance hit test (used only for strokes of ≥ 256 points; coordinates cached per document generation, LRU 64 entries / 4 MB) | `strokeHitTest` | `app-v5.js` `strokeHitsPoint` (stroke eraser) |
| `document_work_version` → 1, `document_work_new/add/cancel/poll/drop` | document_work | Input-priority scheduler: quiet-time deadlines, a held pen pauses jobs, stale sessions are rejected, at most 8 resumptions per turn | `documentWorkQueue` | `runtime-core-v5.js` `DocumentWorkQueue` |
| `pdf_ink_path(values,len,x,bottom,out,cap)` | pdf_ink | Writes PDF content-stream operators (`m`/`l`/`S`, `w`, `RG`) for pen strokes and keeps every sample | `pdfInk` | worker `renderOverlay` task (vector ink for sparse pages; dense pages are rasterised) |

Observations that matter for v6:

- The core is **all numeric kernels**: mostly the scanner (pixel kernels, surface fit, ink axes), a few editor and Home helpers, and two codecs.
  The document model, persistence and sync live entirely in JS.
- The loader uses `fetch` + `arrayBuffer` + `WebAssembly.instantiate`, not streaming instantiation. That means no
  overlap of download and compile, and no Wasm code cache.
- Every wrapper must re-create its typed-array views after any call, because another kernel may have grown memory.
  The comments document this as a recurring bug class.

---

## 2. The unmerged Rust port: `rust/*` and `rust-on-5.17`

### 2.1 Topology

- All 14 `rust/*` branches start from `3edf36b` ("rust: bridge, per-area modules and porting rules…"), which was
  made on `a2dea10` (v5.16.57). They are dated 2026-10-26 and were written by parallel agents under `docs/RUST_PORT.md`.
- `rust-on-5.17` (`086d4fe`, 133 commits) merges the port branch into `fcfb1ee` (**v5.17.19**). The merge is
  `6c6cc09`, and its second parent `ffce1ad` is the earlier integration branch that merged all areas. Later commits
  re-merge the areas, rebuild the wasm and assets (`3f6ee64`) and add the differential tests to `test:unit` (`086d4fe`).
- Every `rust/*` branch is an ancestor of `rust-on-5.17`. **None of it is on `main`.** Main moved to 5.17.22 through the `perf/*` and `fix/*` branches.
- `rust/calendar` contains only the bridge commit. The `calendar` and `ts_engine` modules are empty stubs.

### 2.2 Shared infrastructure (commit `3edf36b`)

- `rust-core/src/bridge.rs` adds a single export, `bridge_call(name,len,input,len,out_len)`, that dispatches
  `"area.method"` strings to per-area `call(method, bytes)` handlers. It returns a block of `[status byte | payload]`
  and adds the JSON helper `bridge::json`.
- New dependencies: `serde` and `serde_json` (std, derive). This is the first time the crate uses registry dependencies.
- `rust-core-v5.js` gains `callBytes`/`callJson`, a `runtime` escape hatch (`allocation`, `view`, `release`, `exports`),
  and one marker region per area (`// >>> stream:<area>`).
- `tests/helpers/legacy.mjs` loads the **pre-port JS from git** (`a2dea10`) into a `vm` and provides a seeded PRNG,
  which is the basis of every differential test.
- `scripts/build-rust-core.mjs` adds `--remap-path-prefix=$CARGO_HOME=/cargo` and `--remap-path-prefix=<root>=.`,
  so registry paths in panic locations do not break reproducibility.
- Rules in `docs/RUST_PORT.md`: delete the JS algorithm (**no JS fallback**; throw if WASM is missing), byte-exact
  parity with the old JS unless a 1-ULP difference is proven harmless, and a per-area report in `docs/rust-port/<area>.md`.

### 2.3 What each area ported

Units done are counted against each area's own plan. Speedups are the agents' Node 22 micro-benchmarks on a
shared, loaded 4-core box, so the ratios matter more than the absolute times.

| Area (branch) | Units done | What moved to Rust | Evidence of quality | Measured speedup | v6 reuse |
|---|---|---|---|---|---|
| `scan_light` | 1/10 | Photographed-stencil removal (`removePhotographedStencil`, 28 KB JS) as a stepped state machine (`scan_stencil_*`), plus `is_yellow` | 8 cargo tests; differential over all 16.7 M colours for `isYellow`, plus 3,000 noise pages and 1,543 damaged pages, byte-exact including statistics | ≈27–32× (5.0–6.6 s → 155–246 ms per 2546 px page) | **As-is** (pure pixels) |
| `scan_live` | 1/7 | Live-frame colour references and illumination normalisation (`scan_live_*`); `scan_live/js.rs` holds the JS number semantics | Over 6,000 seeded frames byte-exact | 1.5–3.2× | **As-is** |
| `scan_color` | 1/3 | Capture-quality acutance, lattice preflight and readiness (`scan_color_quality`, `scan_color_luma`). Found that 88 KB of colour JS is dead (never loaded) | 6 cargo tests; 2,000 + 110 + 40 scenes identical including key order | 1.6–3× | **As-is** |
| `scan_geometry` | 1/10 | Metric-surface model, projection, Coons rails, refinement and registration at map time (`sg_*`) with LRU handle caches | Differential: `lineDistance`/`inside`/`coons` byte-exact; `project`/`map` within 2.7e-12 px (1-ULP libm `sin`/`cos`/`pow`) | 13–33× (batched map) | **As-is** |
| `editor_input` | 2/6 | Resident stroke store (slots, bounds, coordinates per document generation), exact circle hit for the stroke eraser, and range splitting plus a query loop for the area eraser | 11 cargo tests; 7,200 erase queries plus 2,064 area erases with 1-ULP threshold cases; reproduces V8 `Math.hypot` bit for bit | 1.8–8× | **Algorithm.** The store design is the right idea, but in v6 Rust owns the strokes, so the upload/generation protocol disappears |
| `editor_render` | 2/8 | Stroke bounds, legacy cover widths, and the timeline diff-highlight engine (`editor_render_timeline_diff`) | Differential with 6,000 random page pairs per seed; e2e pixel parity of 0 bytes | Bounds 6–27×, but path build is neutral (95% is `lineTo`); timeline diff 2.7–9× | **Algorithm** (timeline diff) |
| `editor_view` | 1/5 | Page layout cache plus visible-window planner (binary searches, active-page cap, prune lists; `ev_*`) | 12 cargo tests; about 11,000 random editors bit-exact | Layout 3.7–5.7×, `updateVisiblePages` 1.8–5.8×, range lookup no gain | **Algorithm** |
| `editor_history` | 1/7 | Item fingerprints, `simpleHash`, stroke-delta kernels (`history_*`), full ES `Number::toString` | 9 cargo tests; 9 differential tests | 2–17× (small calls are slower) | **v5 compatibility only** (v5 fingerprint strings) |
| `collab` | 2/8 | v5 canonical document hash and stable stringify (UTF-16 strings, ES number printing, `__proto__` drop); live snapshot cache plus peer delta planner (thread-local) | 19 cargo tests; byte-exact on 850k numbers, 5k strings and 4k documents | Hash 10–23×; snapshot 2.4–3.3× | **v5 compatibility only** |
| `docio` | 2/6 | Export metadata (STROKES_Z page entry, dedupe, compaction, overlay bounds, page hash, export plan and geometry) with an ordered JSON reader/writer (`jv.rs`); base64/UTF-8/data-URL/image-sniff codec | 21+ cargo tests; 1,500–20,000 random pages per path | Page hash 3.1–3.6×; changed page 1.0–2.7× | Codec **as-is**; export metadata **v5 compatibility** |
| `offline` | 3/6 | Worker base64 and `timelineHash`; PNG cache plan; library catalogue merge, index, order and Home views (thread-local table); timeline trim planner | 37 cargo tests; differential over 3,000 random catalogues and 1,500 histories | base64 14–30×; Home 27–40×; folder sort 10–26×; reload about equal | Library index: **algorithm**; trim and `timelineHash`: **v5 compatibility** |
| `storage` | 1/6 | v5 ink page checkpoint words: per-stroke portability and range/flag validation (`ink_pack_words`, `ink_validate_words`) | Differential: 1,500 pack plus 3,000 unpack pages | pack 2.8–3.5×, unpack 1.4–4.8× | **v5 compatibility only** (local migration) |
| `drive_home` | 1/7 | Folder colour palette maths (HSL, luminance, warm hue, tint) | 9 cargo tests; exhaustive 2^24 luminance sweep plus 48.7k lattice, byte-exact | Repaint of 300 folders 3–200× (cache) | **As-is.** Needed for identical folder colours |
| `calendar`, `ts_engine` | 0 | Empty stubs | none | none | none |

### 2.4 Integration result (`rust-on-5.17`), measured

| Metric | main (v5.17.22) | rust-on-5.17 |
|---|---|---|
| Rust source | 1,696 lines | 11,454 lines (about a third tests) |
| Wasm | 126,798 B / 49.9 KB gz / 186 functions / 33 function exports | **916,475 B / 304 KB gz** / 843 functions / 97 function exports |
| `opt-level="s"` / `"z"` (same code) | not measured | 792,251 B (262 KB gz) / 694,194 B (234 KB gz) |
| `wasm-opt -O3` / `-Oz` (binaryen 123) | `-Oz` 112,791 B (48 KB gz) | 838,815 / 828,049 B (≈ 303 KB gz). Only about 10% smaller: this is real code volume, not bloat |
| Largest functions | `ink_axes` 14 KB | `offline::call` 37.5 KB, `docio::call` 21 KB, `library::load` 20.5 KB, `Stencil::step` 19.4 KB, `collab::call` 16 KB. The JSON dispatchers inline serde code; `serde`/`serde_json` ≈ 49 KB and `core::fmt`/float formatting ≈ 43 KB |
| JS adapter `rust-core-v5.js` | 29,966 B / 475 lines | **122,737 B / 1,866 lines** |
| `app-v5.js` | 1,952,641 B (at v5.17.19) | 1,919,738 B (**−1.7%**) |
| Compile + instantiate (Node 22, x86, 4 cores) | 1.4 ms | 9.6 ms |
| Reproducibility | identical from any path | identical (SHA-256 `989beaa6…0658d`) **only with** `--remap-path-prefix`. Without it the hash differs (`c58c60cb…`) |
| Rust tests | 7 | **188 pass, 3 ignored** (`cargo test`, measured) |
| Node unit suite (`unit517.log`) | n/a | 319/319 pass; v5.17.19 smoke passed |
| Differential suite (`diff517.log`) | n/a | 149/150. The one failure (export profile `vector` vs `raw`) was fixed in `086d4fe` |
| Browser suite (`e2e517.log`) | n/a | **Not completed**: the log stops at test 64 of 536 with one timing failure (scanner calibration 1,444 ms > 1,400 ms) |

### 2.5 Quality assessment

Strengths worth keeping:

- **Differential testing against the old implementation from git** (`legacy.mjs`) with seeded fuzzing and replayable
  seeds. This is the best asset of the port. In v6, v5 JS becomes the *oracle* for render geometry, v5 document
  import and any v5-compatible output.
- Rigorous parity engineering: V8 `Math.hypot`, ES `Number::toString` ties, `Math.round` half-up, UTF-16 key order,
  `Uint8ClampedArray` rounding, libm 1-ULP probes. These are documented and tested per unit.
- Boundary validation everywhere (lengths, finiteness, sentinels instead of panics), bounded work, and stepped or
  cancellable kernels for long pixel jobs.
- Honest per-unit reports, including where the gain was zero or negative (tiny calls, `recentItems`, path building).

Weaknesses that argue against reusing the port wholesale:

- **Piecemeal porting kept JS objects as the source of truth.** Every call reads `{x,y,p}` objects into typed arrays
  (about 20 ns per point, which dominated several benchmarks), or mirrors JS state in Rust behind generation guards.
  Speedups are therefore capped at 1.5–8× on editor paths, the adapter quadrupled in size, and app JS shrank only 1.7%.
- **Duplication across areas**: a V8-exact `hypot` exists in 5 modules (`scan_color`, `scan_light`, `scan_live`,
  `scan_geometry`, `editor_input`); JS `Math.round` re-implementations exist in about 6; there are two base64 export
  families (`offline_b64_*`, `docio_b64_*`); `docio` carries private copies of the collab hash helpers.
- **Global mutable state** through `thread_local!` (`collab/snapshot`, `editor_view`, `scan_geometry`, `editor_input`,
  two in `offline/library`) behind integer handles. This is acceptable in a single-threaded host but couples unrelated
  callers, and none of it is document-scoped.
- **JSON bridge cost**: serde monomorphisation makes the dispatchers the largest functions, and values must be plain
  JSON (no `-0`, NaN, `Map`, lone surrogates without workarounds).
- **"No JS fallback" with no capability gate**: `init()` throws if WASM fails, and there is no UI path for an old
  WebView. v6 cannot have a fallback either, so it needs an explicit boot check (§4.6).
- 19 of the 89 units the areas planned are done, and the full browser suite was never recorded green.

### 2.6 Reuse verdict for a from-scratch v6 core

| Category | Code | How |
|---|---|---|
| **A. Take as-is** (pure, tested, no v5 data shapes) | main: `warp_cell`, `split_rgba`, `balance_paper_rows`, `correct_scan_rows`, `separate_ink`, `surface.rs`, `ink.rs`, `checkpoint.rs`, `pdf_ink.rs`, `folder.rs`; branches: `scan_light/stencil.rs`, `scan_live/illumination.rs`, `scan_color/quality.rs`, `scan_geometry.rs`, `docio/codec.rs`, `drive_home/palette.rs` | Move into a `scanner-kernels` crate or module (lazily loaded) and a `ui-kernels` module. Merge the duplicated `js.rs` shims into one `jsnum` module (hypot, round, clamped rounding) and keep one base64 |
| **B. Take the algorithm, rewrite the interface** | `editor_input.rs` (spatial pre-filter, exact circle hit, eraser split ranges), `editor_view.rs` (layout prefix plus visible-window and active-page planning), `editor_render/timeline_diff.rs`, `offline/library.rs` (catalogue merge, ordering, Home views; collation rank stays in JS `Intl`), `document_work.rs` | Rust owns the data, so the slot, upload and generation protocols and the columnar-JSON inputs go away. Keep the existing unit tests as specifications |
| **C. v5 compatibility only** | `collab/json.rs` + `canon.rs` (v5 canonical hash), `collab/snapshot.rs` (v5 peer delta), `docio/export.rs` + `jv.rs` + `strokes.rs` (STROKES_Z page entry, page hash), `editor_history/fingerprints.rs`, `storage.rs` (`ink_words`), `offline/timeline_trim.rs`, `timelineHash` | Put these in a separate `v5compat` module, needed to (a) migrate local v5 IndexedDB data and Drive PDFs and (b) write v5-readable data while v5 clients still exist. Its differential tests against v5 JS stay valid unchanged |
| **D. Drop** | `bridge.rs` JSON dispatch for hot paths, the per-area marker adapter, `ink_repair_discs` (unused), V8-exact shims where no compatibility is needed, the `alloc` 255-fill quirk outside pixel buffers | Replace with the v6 ABI (§4.5) |

---

## 3. `rebuild-v2/` (TypeScript rebuild)

### 3.1 What it is

- A TypeScript + Vite rebuild with `vite-plugin-singlefile`. It produces one `dist/index.html` (2,479,824 B, with
  pdf.js, pdf-lib and the fonts inlined) for the Android builder.
- `src/` has 12,852 lines in 65 TypeScript files. The plan (`REBUILD_PLAN.md`) and status (`STATUS.md`, dated 2026-07-17) are in Spanish.
- It was **never shipped**. `STATUS.md` lists open items before cut-over: real PDFs and `.sdocx` files, two-device
  Drive QA, Calendar, APK on a Samsung, then replacing the legacy HTML.
- Production CI runs `npm ci && npm test && npm run build` in `rebuild-v2/` on every push.
- **Measured:** 68/68 Vitest tests pass in 4.4 s, and `tsc --noEmit` is clean.

### 3.2 Architecture

| Layer | Implementation (file) | Notes |
|---|---|---|
| Ids | `core/ids.ts`: 9-char base36 ms timestamp + 2-char per-ms counter + 12 random base36 chars | Lexicographic ≈ creation order, which keeps IndexedDB ranges tidy. No actor or clock component |
| Model | `core/model.ts`: `Doc{id, rev, pageOrder, pages: Map, pageTombstones: Set, meta}`, `Page{id, w, h, background (template\|pdf\|custom), strokeOrder + strokes: Map, imageOrder + images: Map, tombstones: Set, sidePanel}`, `Stroke{id, tool, color, width, points: Float32Array [x,y,p…], bbox}` | Identity everywhere; indices only in the presentation layer. Tombstones are capped at 4,000 per page with **FIFO eviction** |
| Ops | `core/ops.ts`: 11 types (`splice-strokes`, `transform-strokes`, `splice-images`, `transform-images`, `add-page`, `remove-page`, `move-page`, `set-page-background`, `set-page-side-panel`, `set-meta`, `replace-doc`) | Addressed by id; `applyOp` is idempotent on replay; `invertOp` is computed from the pre-state. Restore and remote merge both use `replace-doc` |
| Store | `core/store.ts`: `DocStore.apply(op, source)` with `local\|remote\|history\|restore` | The single write path. Subscribers handle render, persistence, undo and sync |
| Undo | `core/history.ts`: inverse ops; caps of 100 entries / 200k points | Id-addressed, so concurrent remote changes do not corrupt undo |
| Merge | `core/merge.ts`: three-way by id. Union minus tombstones on either side; order follows remote, with local-only ids inserted after their surviving local predecessor; a conflict on the same id is decided by the base snapshot; if both sides changed, **local wins** | Signatures are string concatenations (`points.join(',')`); `mergeOrder` is O(n²) (`includes`/`indexOf`) |
| Versions | `core/versions.ts`: full JSON checkpoints captured on compaction (works offline); same-author coalescing within 5 min; caps of 100 entries / 24 MB raw; milestones survive eviction; restore = `replace-doc` plus tombstones for discarded ids | **Local only**: versions are not in the sidecar, so the timeline is not shared between devices or collaborators |
| Persistence | `persist/idb.ts`: DB `inhouse-notes-v2` with stores `docs`, `pages [docId,id]`, `oplog [docId,seq]`, `versions [docId,id]`, `assets [docId,id]`. `persist/oplog.ts`: append per microtask, drain loop, retry after 250 ms. `persist/persistController.ts`: debounced compaction (800 ms, max 5 s), dirty-page tracking, Web Locks, flush on `visibilitychange`/`pagehide`, version capture (30 s debounce, 2 min max). `persist/docRepo.ts`: incremental page writes plus oplog trim in one transaction; crash replay. `persist/migrate.ts`: legacy localStorage (`notebook-data-v3/v2`) and IndexedDB `notebook-data-db-v1` (snapshots, pages, stroke-ops) | Sound crash-safety model; reordering pages rewrites only the doc record |
| Sync | `sync/syncMachine.ts`: states `idle / localDirty / syncing / pdfStale / conflict`; on sync it reads the remote sidecar, merges, then uploads; auto-sync 1.8 s after a local op. `sync/collab.ts`: polls `refreshFromDrive` every 5 s; presence through Drive file `properties` | `conflict` and `pdfStale` are **never entered** (no `setState('conflict')`; `markPdfStale` has no caller) |
| Sidecar schema 3 | `sync/sidecar.ts`: `{version: 1\|2\|3, docId, rev, pageOrder, pages, pageTombstones, meta, savedAt, sidecarVersion: 3, assets: [{id, name, mimeType, createdAt, data? \| fileId, resourceKey?}], writeId, parentWriteId}`. Stored as `<name>.ihn.json` in the app folder, as plain JSON or `IHN3:` + base64url(deflate(JSON)), whichever is smaller. Source PDFs are uploaded once as `<name>.<assetId>.source.pdf` and referenced by `fileId` | The `writeId`/`parentWriteId` lineage is the right idea, but it is not used as a precondition |
| Viewport | `viewport/camera.ts`: pure zoom/pan maths (zoom 0.5–3, `clampPan` keeps 80 px visible, `zoomAround`, inertia with decay 0.94 and a 90 px step cap, `fitPageZoom`). `transformApplier.ts`: CSS transform during gestures, re-raster 220 ms after the zoom settles. `pageWindow.ts`: activate/deactivate deltas | `PageWindow` scans every page on each update and **ignores its `bufferPages` option** |
| Input | `input/pointerPipeline.ts`: pointer capture, coalesced plus predicted events, **no `pointerleave` stroke end**, a touch draws only until a pen has been seen, a second finger hands over to the pinch. `router.ts`: palm rejection while the pen draws, UI-target filter. `gestures.ts`, `tools.ts` (pen, highlighter, erasers, lasso). `render/strokesIndex.ts`: uniform 96 px grid spatial index | Good structure; predicted points are drawn into the live tail |
| Render | `render/pageView.ts`: 3 canvases per page (bg, ink, live), dirty-rect erase repaint, 4,096 px backing cap. `strokePainter.ts`: **quadratic-midpoint smoothing** | Not v5's current curve (see §3.4) |
| PDF | `import/pdfImport.ts` (`/Ink` annotations become strokes), `import/flattenedInk.ts` (chromatic flattened-ink recovery), `export/pdf.ts` (strokes exported as `/Ink` annotations), `sync/pdfArtifact.worker.ts` | The worker **is not referenced** anywhere, so PDF work stays on the main thread |

### 3.3 Decisions that are sound and should carry into v6

1. **Identity over position**: stable ids for documents, pages, strokes and images; IndexedDB keys by `[docId, id]`; page order is a separate list.
2. **One write path**: every mutation is a plain-data op applied by the store, with a computable inverse. Undo, the journal, version deltas and sync all consume the same ops.
3. **Crash journal with idempotent replay**: append on a microtask, compact on a debounce in one transaction, flush on `pagehide` and when hidden. Use Web Locks for cross-tab writes.
4. **Tombstones** for deleted pages and items, so a deletion beats a concurrent remote copy, and a restore propagates as tombstones.
5. **The sidecar is the truth and the PDF is a derived artifact**. Upload source PDFs once as separate files and reference them by id; this keeps the per-edit payload small.
6. **Version capture independent of upload success** (works offline), with a size-based cap and milestones.
7. **Pure camera maths**, CSS transform during gestures, and re-raster only after the zoom settles.
8. **Input rules**: pointer capture, no `pointerleave` stroke end (legacy bug C4), coalesced events, palm rejection, and touch-to-pinch handover.
9. **Layered page rendering** (background, committed ink, live) with dirty-rect erasing and a spatial index for erasers and the lasso.
10. **Pure, DOM-free core with fast unit tests**. In v6 these become `cargo test` plus the v5-oracle differential tests.

### 3.4 What it lacked, or got wrong

- **Not a network document design.**
  - Sync uploads a **whole-document snapshot** and merges whole snapshots. Ops never travel over the wire.
  - Collaboration is Drive polling every 5 s. There is no real-time channel, while v5 has WebRTC live P2P in `live-collaboration-v5.js`.
  - There is **no causal metadata** (no actor id, Lamport or hybrid clock, or version vector). When both sides changed the same element, "local wins" on each device, so devices converge only after a ping-pong of uploads, and with more than 2 writers the result depends on timing.
- **Lost-update window.** It downloads, merges, then calls `updateFileMedia` with no `If-Match`. v5 already uses ETag preconditions in `app-v5.js`, `offline-sync-v5.js` and `drive-ink-journal-v5.js`.
- **Tombstone FIFO cap (4,000 per page)** can resurrect deleted ink when an old replica merges after eviction. Tombstones need causal expiry (for example, garbage collection after every known replica has acknowledged them).
- **Float32 points** lose the f64 coordinates that v5 persists. That makes migration lossy and changes v5 hashes and fingerprints.
- **Version history is per device** (not in the sidecar), unlike v5, and is stored as full snapshots instead of the planned checkpoint plus delta.
- **Dead states and modules**: `conflict` and `pdfStale` are never entered, and the PDF worker is not wired.
- **The UI is not identical to v5.17.22**:
  - Pens are drawn with quadratic midpoints. v5.17.22 draws a **centripetal Catmull-Rom spline converted to cubic Béziers** through every sample, with straight segments below 2 px (`appendPenSplineSegment` in `app-v5.js`).
  - rebuild-v2 draws the **predicted** tail; v5.17.21 removed the predicted pen tip.
  - There is no `desynchronized` context (v5 does not use one either).
- **Scaling**: `PageWindow` is O(pages) per update and ignores its buffer option. `mergeOrder` is O(n²).
- **Coverage**:
  - no browser end-to-end tests;
  - no scanner (declared out of scope);
  - no Drive home parity work beyond basics;
  - no ink journal;
  - no performance budget checks.

### 3.5 Tests (68, all passing)

`core/ops.test.ts` (11), `persist/persist.test.ts` (11, fake-indexeddb: incremental save, oplog recovery, compaction),
`persist/migrate.test.ts` (9), `ui/syncStatusBar.test.ts` (7), `core/merge.test.ts` (7), `core/versions.test.ts` (6),
`core/history.test.ts` (4), `sync/sidecar.test.ts` (3), `sync/syncMachine.test.ts` (3, Drive concurrency with a fake
client), `import/flattenedInk.test.ts` (3), `core/pageSize.test.ts` (3), `pdf/pdfRoundtrip.test.ts` (1, `/Ink` export
and import). The merge and sync tests use hand-written scenarios with no randomised or convergence (multi-replica)
testing. v6 should add property tests: N replicas, random op interleavings, assert convergence.

---

## 4. Toolchain facts and the WASM boundary for v6

### 4.1 Verified environment

| Fact | Value (measured) |
|---|---|
| Toolchains | `stable` (cargo 1.97.0) is the default. `1.90.0` is installed with `wasm32-unknown-unknown` and is selected automatically through `rust-toolchain.toml` |
| Rust 1.90 default wasm target features | `bulk-memory`, `multivalue`, `mutable-globals`, `nontrapping-fptoint`, `reference-types`, `sign-ext` (from `rustc --print cfg`). `simd128` is opt-in (`-C target-feature=+simd128`) |
| Browser minimums those imply | reference-types → Chrome/WebView 96+ (the highest); multivalue 85+; bulk memory 75+; SIMD 91+ if enabled. The current v5 core is already built with these defaults |
| crates.io | Reachable. Latest `wasm-bindgen` is **0.2.129** (`rust-version` 1.81, so it builds on 1.90). `serde`/`serde_json` resolved for the port |
| wasm-bindgen CLI | `wasm-bindgen 0.2.100` is installed in `~/.cargo/bin`, built from source at about 20:05 today by another session (about 7 min to compile on this box). The CLI must match the crate version **exactly** |
| binaryen | `npm i binaryen@123.0.0` works and provides `wasm-opt` version 123 under Node. It saves only about 10% on these modules |
| Android shell | `android app/html_to_apk_builder.py` generates a Capacitor project (`androidScheme: "https"`) that navigates to `inhousenotes.com`, with native asset hashes pinned in `native-assets.json`. The builder pins `@capacitor/*` to `"latest"`, which is a separate reproducibility gap |
| Hosting | GitHub Pages (`CNAME`, `deploy-pages.yml`) cannot set COOP/COEP headers, so there is no `SharedArrayBuffer` and no wasm threads without a service-worker workaround |

### 4.2 Prototype: wasm-bindgen + web-sys versus hand-written glue (measured)

I built two crates (Rust 1.90.0, release, LTO, `panic=abort`, stripped) with the same surface: draw a polyline from
packed points on a 2D context, open an IndexedDB database, start a fetch, create an `RTCPeerConnection` with a data
channel, call a host function N times, and sum an f32 array.

| | Hand-written ABI (raw imports and exports) | wasm-bindgen 0.2.100 + web-sys 0.3.77 (`--target web`) |
|---|---|---|
| `.wasm` | 8,952 B (4.0 KB gz), including a `Vec`-based allocator | 15,352 B (6.9 KB gz) after the CLI (523 KB before it) |
| JS glue | Written by hand: an import object plus whatever marshalling the app needs | 13,352 B generated (3.2 KB gz), plus `.d.ts` |
| 2M wasm→JS import calls | 85.7 ms (≈ 43 ns per call) | 64.9 ms (≈ 32 ns per call). No penalty: V8 inlines the shim |
| Sum over 900k f32 | 1.98 ms (data resident in linear memory) | 2.84 ms (`&[f32]` copied in on every call) |

Takeaways:

- wasm-bindgen costs nothing per call. Its costs are fixed glue (about 10 KB gz for a modest surface, growing with
  each web-sys method used), a second pinned tool, and slices that copy by default.
- The real performance lever in both approaches is the **design**: keep data resident in Rust memory and batch
  commands. The v5 port shows what happens otherwise.

### 4.3 Per-API assessment

| Browser API | Via web-sys from Rust | Via the JS host (recommended) |
|---|---|---|
| **Canvas 2D** (pen, highlighter, templates, Path2D) | Works, but every `lineTo`/`bezierCurveTo` is a boundary call anyway | Rust emits a command buffer (op codes plus f64 coordinates in linear memory, including the Catmull-Rom Bézier controls), and a small JS loop replays it into `Path2D` or the context. Same pixels as v5 because it is the same IEEE f64 maths (only `sqrt`, which is correctly rounded in both). Caches `Path2D` per stroke as v5 does |
| **DOM** (toolbar, menus, Drive home cards, modals, timeline UI) | Verbose and slow to iterate; large glue | JS/TS templates. Rust supplies view models (ordering, palette colours, layout plans) |
| **IndexedDB** | Callback-style `IdbRequest` events plus `Closure` lifetimes and `wasm-bindgen-futures`; re-entrancy into `RefCell` state panics (and `panic=abort` kills the app) | JS performs reads and writes from an **effects queue** Rust produces (`PutPage`, `AppendJournal`, …) and feeds completions back as events. Rust stays synchronous and deterministic |
| **fetch / Drive REST** (OAuth, `If-Match`, resumable upload) | Promise-based through futures; error mapping in Rust | JS does the I/O; Rust decides *what* to send (op batches, base revision) and merges responses |
| **WebRTC** (live P2P ops) | Event-heavy (ICE, `ondatachannel`, `onmessage`) with many closures | JS keeps the transport (v5's `peer-transport-v5.js` is a model). Rust encodes and decodes op frames and acknowledgements |
| **Workers** | Possible, but adds glue in each worker | Compile once and `postMessage` the `WebAssembly.Module` (structured-cloneable) to Workers. Each instantiates its own memory. Use this for PDF export, scanner and large merges |

### 4.4 Startup on a mid-range Android WebView

- **Measured** (Node 22, x86, 4 cores, `WebAssembly.compile` + `instantiate`): 127 KB core in 1.4 ms; 916 KB port in 9.6 ms (similar with `--liftoff-only`).
- **Estimate**: a mid-range ARM phone is typically several times slower than this desktop for Liftoff compilation (assume 4–8×). That gives roughly 6–12 ms for a ~130 KB module and 40–80 ms for ~900 KB, before download or disk read, glue parse and first-frame work. Measure on the user's Samsung through a boot trace (`performance.mark` around instantiate) before fixing budgets.
- **Code caching**: V8 caches compiled Wasm only for **streaming** compilation of sufficiently large modules (`instantiateStreaming` / `compileStreaming` with `Content-Type: application/wasm`). The v5 loader (`fetch` → `arrayBuffer` → `instantiate`) gets neither the overlap nor the cache.
  - v6 should stream and check that both the GitHub Pages response and the service-worker or native asset cache path keep the `application/wasm` MIME type.
  - Keep a small fallback path for when streaming is not available.
- **Budget proposal**:
  - editor core ≤ 400 KB raw / ≤ 130 KB gz, on the critical path;
  - `scanner-kernels` and `v5compat` as separate modules, loaded on demand (scanner page, first open of a v5 document);
  - avoid `serde_json` and `core::fmt` float formatting in the editor core (≈ 90 KB together in the port). Use a compact binary encoding for the ABI and the journal, and keep JSON parsing for v5 import inside `v5compat`.

### 4.5 Recommended boundary (v6.0)

1. **No wasm-bindgen and no web-sys in v6.0.** One `cdylib` with a C ABI and **no imports**, the property the current
   build script already enforces. It is testable in Node and with `cargo test`, and needs no extra tool.
2. The ABI is versioned by `core_abi_version`. Calls are coarse:
   - `core_input(ptr, len)` takes a batch of events (pointer samples, tool changes, remote op frames, IndexedDB and fetch completions, timers);
   - `core_frame()` returns pointers to a **render command buffer** and an **effects buffer** (persist, send, schedule).
   - Hot read-only queries (hit testing, visible range) are direct typed exports.
3. Encode with one small hand-rolled little-endian binary format shared by the ABI, the IndexedDB journal and the wire
   frames (versioned, length-prefixed). JSON is used only to read and write v5 data.
4. A single TS host module (target ≤ 2k lines) owns all `memory.buffer` views and re-creates them after every call.
   If the ABI grows, generate its TS bindings from a Rust-side schema with an in-repo script, so no external CLI is needed.
5. Reconsider wasm-bindgen (exports only, no web-sys) only if hand-maintained bindings become an error source. If it
   is adopted:
   - pin `wasm-bindgen = "=0.2.x"` in `Cargo.toml`;
   - install the identical CLI in CI with `cargo install wasm-bindgen-cli --version 0.2.x --locked` (cached), or download the release binary with a checksum;
   - commit the generated glue and include it in the diff gate.

### 4.6 Runtime gates

- At boot, check `WebAssembly.validate` against a tiny module that uses reference-types and multivalue, plus the
  Chrome/WebView version. If either fails, show a clear "update Android System WebView" screen. v6 has no JS fallback,
  and the port's "throw on missing WASM" policy needs this UI.
- Run the editor core on the main thread: pen input to ink must not cross a Worker hop. Run export, scanner, large
  catalogue and merge work in Workers, each on its own instance.
- Enable `simd128` only in the `scanner-kernels` module, and only after measuring. The scanner reports name SIMD as
  the next lever, while the editor core is not pixel-bound.

### 4.7 Keeping the build reproducible in CI

1. Keep `rust-toolchain.toml` pinned (1.90.0, `profile = "minimal"`, `targets = ["wasm32-unknown-unknown"]`), keep
   `Cargo.lock` committed, and always build with `--locked`. Add `cargo fetch --locked` as a separate step, or vendor
   crates (`cargo vendor`) if crates.io reachability in CI must not matter.
2. As soon as any registry crate is used, keep the `--remap-path-prefix` flags from the port's build script for
   `$CARGO_HOME` and the repository root. **Measured**: without them the integration build's hash changed.
3. Keep `lto = true`, `codegen-units = 1`, `panic = "abort"`, `strip = true`; avoid `build.rs` scripts that embed time
   or environment; do not depend on `wasm-opt` (about a 10% gain). If it is used, pin `binaryen@123.0.0` through npm
   with the lockfile.
4. Keep the existing CI gate: rebuild, then `git diff --exit-code` on every committed `.wasm` and generated glue.
   Also run `cargo test --locked` and `cargo clippy -D warnings` (main runs neither today), plus the Node differential
   suites.
5. Report the size of each module in CI and fail the build if it exceeds its budget (§4.4).
6. Treat the Android builder's unpinned `@capacitor/*: "latest"` as a separate reproducibility item for the APK
   track. It is not part of the frozen installer, but it needs the user's decision.

---

## 5. Sources examined

- `rust-core/**`, `rust-core-v5.js`, `scripts/build-rust-core.mjs`, `rust-toolchain.toml`, `.github/workflows/production-checks.yml`, `package.json`, `tests/rust-core.test.mjs`, `tests/document-work-rust.test.mjs`, `tests/rust-folder.test.mjs`, `tests/e2e/scanner-ink-repair.spec.mjs`, and the call sites listed in §1.2 (this tree, `66c4afa`).
- Branches `rust/{calendar, collab, docio, drive_home, editor_history, editor_input, editor_render, editor_view, offline, scan_color, scan_geometry, scan_light, scan_live, storage}` and `rust-on-5.17` (`git log`/`show`/`diff` in `/home/user/inhousenotes`), and `docs/RUST_PORT.md` plus `docs/rust-port/*.md` in the `rust-on-5.17` worktree. Integration logs `unit517.log`, `diff517.log` and `e2e517.log` from the session scratchpad.
- `rebuild-v2/` (`REBUILD_PLAN.md`, `STATUS.md`, all of `src/core`, `src/persist`, `src/sync`, `src/viewport`, `src/input`, `src/render`).
- Measurements were reproduced in scratch copies: main and integration wasm builds and hashes, `cargo test`, opt-level and `wasm-opt` sizes, a per-function size breakdown from the name section, compile timings, the wasm-bindgen versus raw-ABI prototype, and rebuild-v2 `vitest` and `tsc`.
