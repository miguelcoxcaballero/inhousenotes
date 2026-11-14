# W6.2 PERF — notes (performance part only)

Scope given by the lead: measure v6 against 5.18.0, optimise where v6 is not clearly better, keep pixels identical.
No release steps, no version strings, no deletion of v5 files. Branch `v6-w6_2`, base `v6-int` bc2e028.

## Tools (tests/v6/perf/)

- `bench.mjs --app v5|v6 [--root tree] [--runs n] [--only …] [--throttle n]`: one script, same UI actions, same Drive
  mock and fixtures for both apps. Listener time per event comes from wrapping `addEventListener` before either app
  loads, rAF work from wrapping `requestAnimationFrame`, CPU from CDP `Performance.getMetrics` (TaskDuration). CDP is
  attached only after the boot or open being timed (see "5.18.0 Home stall" below).
- `cpuprofile.mjs <tree> zoom|scroll|erase|lasso`: main-thread CPU profile of one action on `dense-page`, self time
  by function. Used to tell app work from compositor/raster time and noise.
- `open-memory.mjs [scene]`: v6 only; JS heap, wasm memory and the importer's memory marks every 500 ms while a 5.x
  document opens.

Fixtures: `dense-20` (5.x PDF, 20 pages × 120 strokes × 40 points, 1.6 MB) and `dense-page` (2400 strokes × 100
points on one page, 2.1 MB). Desktop Chromium headless shell, 1200×900, no throttling, on a 4-core machine shared
with other agents: CPU-per-step figures carry ±20 % noise; the profiles below decided what was real.

## Results

| Measure | 5.18.0 | v6 before (bc2e028) | v6 after |
|---|---:|---:|---:|
| Boot → first Home card (ms, median; 5.18.0: fastest run) | 458 | 341 | 280 |
| Boot requests / KB | 47 / 3854 | 31 / 2171 | 31 / 2173 |
| wasm compile + instantiate (ms) | — | 14.5 + 0.2 | 10.5 + 0.2 |
| Open dense-20 → first page canvas (ms) | 1684 | 1915 | 1333 |
| Open dense-20 → all 20 pages (ms) | 1696 | 1989 | 1373 |
| Memory after open: JS heap MB (after GC) | 11.92 | 2.1 | 2.09 |
| Memory after open: wasm memory MB | — | 53.81 | 20.94 |
| Memory after open: total MB | 11.9 | 55.9 | 23.0 |
| Pen on dense-page: listener ms / pointermove (mean, p95) | 0.03, 0.1 | 0.03, 0.1 | 0.03, 0.1 |
| Pen: rAF work ms / frame p95 (max) | 0.1 (3.8) | 0.3 (3.6) | 0.3 (4.7) |
| Pen: frame interval p95 / max ms | 16.7 / 66.6 | 16.7 / 50 | 16.7 / 16.8 |
| Pen: renderer CPU ms per move (incl. compositor) | 5.38 | 4.47 | 4.73 |
| Eraser: listener ms / move (mean, p95) | 0.52, 0.8 | 0.05, 0.1 | 0.03, 0.1 |
| Eraser: rAF p95 (max) / frame max ms | 0.8 (11.3) / 66.7 | 1.4 (4.7) / 100.1 | 0.9 (1.3) / 116.7 |
| Lasso: listener ms / move; release ms | 0.08; 15 | 0.04; 17 | 0.04; 8 |
| Lasso: CPU ms per move | 4.54 | 5.52 | 4.59 |
| Scroll (wheel): listener ms / event; frame max | 0.15; 16.8 | 0.21; 33.3 | 0.24; 16.8 |
| Scroll: CPU ms per wheel step (incl. raster) | 6.2 | 8.23 | 8.21 |
| Zoom (ctrl+wheel): listener ms; frame max | 0.1; 16.8 | 0.18; 16.8 | 0.15; 16.8 |
| Zoom step + repaint until idle: CPU ms | 234 | 503 | 484 |
| Pinch: frame p95 / max; CPU ms per move | 16.8 / 766.6; 20.28 | 16.7 / 750; 20.96 | 16.7 / 733.2; 20.36 |
| Export dense-20 → download (ms) | no download in 120 s | 349 | 468 |

## What was wrong and what changed

1. **Opening a 5.x document rebuilt every page once per import batch** (`doc/v5open.rs`). The import delivered its
   21 batches through 21 `with_replica(receive)` calls; each announced changes, the editor ran `renderAllPages`, and
   every row got its template background again. With a 350 KB JPEG data URL as template background that was
   21 × 20 × 350 KB ≈ 74 MB pushed through the out-buffer, plus a 350 KB `String` kept per row. Now one
   `session::receive_owned` call: the batches are moved into the session queue (no copy), the journal transaction
   is reserved once, the editor builds its pages once.
2. **Long style values sent once** (`dom.rs`, `abi.rs`, `host/dom.js`): a STYLE value of ≥ 16 KiB equal to the last
   one written is sent as the marker `0xFFFF_FFFE` and `host/dom.js` re-applies its kept value. The DOM is exactly
   the same (same string, same `style.setProperty`), so pixels and parity DOM are unchanged; a rebuild of 20 template
   pages now moves 350 KB instead of 7 MB. `abi::dump` resolves the marker (unit test
   `a_repeated_long_style_value_is_sent_once`).
3. **Rows share data URLs** (`editor/pages.rs`): template URLs and page previews are `Rc<str>`; a row remembers the
   URL it shows instead of its own `url("…")` copy; `url("…")` is built with its exact capacity.
4. **e2e-only probe copied every worker message** (`shell/mod.rs`): with `?e2e=1` (every benchmark and test) the
   shell kept a lossy UTF-8 copy of each message, including the whole 5.x import passing through the worker channel
   (twice its size). Worker and peer channels and messages over 64 KiB are no longer recorded (no test reads them).
5. **Out-buffer and scratch buffers released** (`ihn-wasm`, `host/main.js`, `store/mod.rs`): the out-buffer gives
   back capacity above 1 MiB after it is applied; the host's event scratch above 256 KiB is freed after the call;
   a store transaction is sent with an exact reservation, the out-buffer is flushed after each megabyte transaction
   (they no longer pile up), and a pending transaction kept for retry drops its growth slack.

Pen, eraser, lasso, scroll, zoom and pinch needed no change in the app: v6's script time per event is equal or
lower than 5.18.0's everywhere (eraser 10× lower) and its frames are steadier. Two renderer-CPU figures stay higher
for v6 and are **not resolved**: scroll (+30 % per wheel step) and "zoom step + repaint until idle" (≈ 2×). Main-thread
CPU profiles (`cpuprofile.mjs`, 8 s after the zoom step, and a 40-step scroll) show the same work in both apps — the
template `toDataURL` (≈ 125 ms), `drawImage`, `save` — with v6's own JS and wasm under 10 ms, and v6's non-idle main
thread time lower than 5.18.0's. The extra `TaskDuration` is therefore outside the main thread's script (raster of
the page wrappers' CSS backgrounds under the canvases is the likely candidate; v6 also keeps light periodic tasks
that the "until idle" window counts). Pinch numbers are dominated by the harness (synchronous CDP touch dispatch,
same 730–770 ms stall in both apps); v6 has no `touchmove` listeners (pointer events), so its listener column is 0.
Export: 5.18.0's download never fired in this harness; v6 349–468 ms for dense-20 (run-to-run noise).

## Remaining memory after opening dense-20

About 12–13 MB live in wasm at the end of the open: the document (3.2 MB: 96,000 points as `f64` pairs plus ids,
stamps and indexes), the journal (2 MB of encoded batches, kept for peers and the IndexedDB journal), the 5.x
version snapshot the timeline caches (1.65 MB), render caches of the two active pages, and transaction buffers until
IndexedDB completes. `memory.buffer` stays higher than live (≈ 19–21 MB) because wasm memory never shrinks and
dlmalloc keeps the peak of the open (store transactions of 2 MB, the compaction snapshot written right after the
open). 5.18.0's JS heap for the same document is ≈ 12 MB (its canvases and pdf.js state are outside that number).

## Deviations

1. ARCHITECTURE §14 "JS heap + wasm memory ≤ 50 % of 5.18.0's" is not met for a large imported 5.x document
   (v6 ≈ 22–24 MB vs 5.18.0 ≈ 12 MB JS heap). Reaching it needs a smaller in-memory document (f32 points or
   compressed stroke storage) or a journal that does not keep imported batches in memory; both change frozen formats
   (SYNC/ARCHITECTURE §4) and were not done here. It was 56 MB before this unit.
2. 5.18.0 export could not be timed: the download never fires within 120 s in this harness (the bench records the
   timeout); v6 is reported alone.
3. **5.18.0 Home stall**: with the Drive mock, 5.18.0 often stays on its Home skeleton ~30 s after a 401 on the first
   `about` (4 of 5 runs). The table uses the fastest run for 5.18.0's boot.
4. Size budget and other release items are outside this part (W6.2s did size; release is not in scope).
5. Renderer CPU for scroll and for the zoom repaint window stays above 5.18.0's (see above); the cause is outside
   v6's script and was not isolated in this unit.
6. ARCHITECTURE §14.1 said "no wasm-opt"; unchanged here (W6.2s owns the build). The perf tools are new files in
   `tests/v6/perf/` (`bench.mjs`, `cpuprofile.mjs`, `open-memory.mjs`); `profile.mjs` (W1.2) is unchanged.

## Tests run

- `cargo test --workspace` (native): ihn-app 166, ihn-core 135 (+1 ignored), ihn-sync 49 — all pass (new:
  `abi::tests::a_repeated_long_style_value_is_sent_once`; `pointer_events_do_not_allocate` still passes).
- `npm run test:e2e` (98 tests, one worker): 94 pass; the 4 failures (`06-images` paste/drop, `08-save-reopen`
  export to the user, two `internal-doc`) fail identically on `v6-int` bc2e028 without this change.
- Parity `--quick` (a = 5.18.0 1d6e8a0): the failures are the same set as on `v6-int` (including
  `canvas/dense-page` @repaint, 3 px Δ2, present on the base too) plus phone-360 flakes of a 2–5 px box (a caret /
  spinner at x 667 or x 995) that move between runs on both trees; the states that differed only here
  (`presence/*`, `sweep/theme-editor`) pass on a rerun. Canvas trace goldens (`internal-canvas`) pass.
