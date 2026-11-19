# W6.3p PERF — notes (memory after a 5.x open, renderer CPU)

Branch `v6-w6_3p`, base `v6-int` 5015183. Two cases were still worse than 5.18.0: memory after opening `dense-20` and
renderer CPU for scroll / the zoom repaint window. Pixels, formats on disk and on the wire are unchanged.

## How it was measured

- `tests/v6/perf/memory.mjs <tree>` (new): opens `dense-20` and every 100 ms reads `Runtime.getHeapUsage` of the page
  **and of every worker** (CDP auto-attach): JS heap + `backingStorageSize` (ArrayBuffer backing stores: a wasm
  `memory.buffer`, pdf.js buffers, typed arrays). Tab total = sum. Peak = largest sampled total during 12 s; steady =
  after a forced GC in every isolate at 12 s. The same for both apps, so 5.18.0's ArrayBuffers and workers count too.
  (Its `openedMs` is late when a worker is busy: use `bench.mjs --only open` for open time.)
- `--features ihn-app/memstat` (new, `ihn-app/src/memstat.rs`, never in a shipped build): a counting global allocator
  with a 16-byte header per block; every allocation is tagged with the module whose `event`/`frame`/`init` runs
  (`app.rs` sets the tag, nested `shell::signal` calls restore it). `__IHN_TEST_API__.cmd('app.mem')` reports live
  bytes by module, the peak and the breakdown at the peak, the largest live blocks and every linear-memory growth
  with the allocation that caused it. `memory.mjs --overlay <build with that wasm>` prints it.
- `tests/v6/perf/trace.mjs <tree> scroll|zoom` (new): CDP trace (devtools.timeline, blink, cc, gpu, viz, v8) of the
  same scripted action; events summed by thread and name; canvases with backing sizes, layer-forming elements;
  `--calls` counts canvas 2D calls and DOM mutations.
- `bench.mjs` probes (env): `BENCH_TRACE` (trace of the repaint window), `BENCH_CALLS` (canvas calls per canvas
  incl. OffscreenCanvas, DOM mutations, and the Drive writes against the phase times), `BENCH_TOTAL` (CPU of the
  whole dense-page session until idle, background saves and export included), `BENCH_QUIET=ms` (quiet period before
  scroll and zoom).

Desktop Chromium headless shell, 1200×900, 4 shared cores (other agents running): CPU figures ±10–20 %.

## Results

| Measure | 5.18.0 | v6 before (v6-int 5015183) | v6 after |
|---|---:|---:|---:|
| Memory after opening dense-20, steady: tab total MB (JS heaps + ArrayBuffers, page + workers) | 31.2–31.4 | 51.7–53.7 | 12.5–13.2 |
| … of which page JS heap / page ArrayBuffers / workers MB | 11.9 / 8.5 / 10.9 | 2.1 / 20.1–22.2 (wasm 18.9–20.9) / 29.2 | 2.1 / 10.4–11.2 (wasm 9.2–9.9) / 0 |
| Main wasm `memory.buffer` = its peak (never shrinks) MB | — | 18.9–20.9 | 9.2–9.9 |
| Main wasm live allocations after the open / at the peak MB (memstat) | — | 6.2 / 13.5 | 4.5 / 6.7 |
| Tab peak during the open (sampled, 12 s) MB | 92.4–103.2 | 74.0–74.1 | 65.2–74.6 |
| Open dense-20 → first page canvas / all pages ms (bench, median of 3) | 1652 / 1665 | 1331 / 1393 | 1243 / 1276 |
| Scroll, no edits before: CPU ms per wheel step | 4.73–5.07 | — | 4.70–4.96 |
| Zoom step + repaint, no edits before: CPU ms | 218–220 | — | 226–231 |
| Scroll after pen/erase/lasso, quiet period first: CPU ms per step | 6.55–7.12 | 6.42–6.82 | 6.54–7.93 |
| Zoom + repaint after edits, quiet period first: CPU ms | 235–292 | 222–235 | 217–242 |
| Zoom + repaint after edits, bench order (no quiet): CPU ms | 219–254 | 209–519 (the Drive export lands in the window in most runs) | 230–502 (same) |
| Whole dense-page session (pen, erase, lasso, scroll, zoom, saves, export) until idle: CPU ms | 4236–5016 | 3473–4234 | 3919–4569 |

The CPU rows depend on machine load (shared cores): before/after and 5.18.0 were interleaved for the session row.
The bench-order row keeps varying with where the export lands.

## What the memory was made of

Before (v6-int, dense-20, steady 52–54 MB):
1. **The export worker, 29 MB**: the 5.x parse ran in the worker (its own wasm memory grew to ~30 MB parsing 842 KB
   of Keywords) and the worker was kept unless it passed 64 MB. ARCHITECTURE §9 says it is terminated after such a
   job.
2. **Main wasm `memory.buffer` 19–21 MB for 6.2 MB live**: wasm memory never shrinks, so it is the peak of the
   open plus fragmentation. memstat's growth log showed the peak (13.5 MB live) and what built it: the Keywords
   held 4–5 times (pdf.js result, `Opened` clone into `Foreign`, a worker message buffer, its out-buffer copy); the
   1.6 MB 5.x version snapshot asset arriving from the worker (message buffer + `to_vec` + `blob_new` out-buffer
   copy); the import batches held three times (journal, the `ops` store transaction, its out-buffer copy); the
   compaction right after the open writing a 2 MB snapshot through a `StoreTx` that doubled to 3.35 MB plus its
   out-buffer copy; `Vec::reserve` on the out-buffer doubling instead of reserving the payload.
3. Live after the open: document 3.2 MB (2400 items: 1 MB `IdMap<Item>` table at 240 B/item, 1.5 MB of `f64` points,
   draw lists and indexes), the timeline preload's 1.6 MB snapshot copy, the template data URL twice (345 KB in
   `editor.pages`, 345 KB in the out-buffer's long-style memo), page rows 0.4 MB.

After (steady 12.5–13.2 MB): main JS heap 2.1, wasm 9.2–9.9 (live 4.5: document 3.2, pages/template 0.8, view 0.3),
no worker, other ArrayBuffers 1.2.

## What changed (memory)

- `doc/v5open.rs`: the worker is stopped after a parse and after a strip (unless an export is using it); the
  Keywords are written to the worker straight into the out-buffer (no message buffer); the job (Keywords) is dropped
  before the open goes on to compaction and the editor; worker `ASSET` parts arrive as Blobs (`part::ASSET_BLOB`).
- `host/export-worker.js`: `postAsset` turns an import `ASSET` part into a Blob in the worker, so the page's wasm
  never holds a version snapshot or image of the import.
- `doc/open.rs`: the foreign PDF hands `opened` over (`mem::take`) instead of cloning the Keywords.
- `abi.rs` `OutBuf::room` (exact reservation for payloads ≥ 64 KiB) used by `blob_new`, `Src::Bytes`, store
  transactions, worker posts and long styles; `OutBuf::trim` (called at the start of every `App::event`) gives
  out-buffer capacity above 256 KiB back once the host applied it (ihn_out_reset only shrinks above 1 MiB).
- `store/mod.rs`, `store/tx.rs`, `host/idb.js`, `store/fake.rs`: a transaction above 512 KiB leaves as `TX_PART`
  records (FX 0x26) of whole operations (~256 KiB each, flushed one by one) followed by the `TX` record with flag 4;
  idb.js collects the decoded operations and runs **one** IndexedDB transaction as before. `Tx` records operation
  offsets (`marks`). Test: `a_large_transaction_leaves_in_pieces_and_commits_as_one`.
- `doc/session.rs` compaction: trims the in-memory journal before encoding, encodes chunks into one reused buffer
  (twice: sizes first) and reserves the transaction exactly (`Tx::reserve`, `snapshot::encode_chunk_into`).
- `doc/session.rs` `receive_owned`: import batches of a Drive revision (import actor, not the migration actor) older
  than the journal's 24 h age limit are applied but not journalled and not appended to `ops` (`folded`), when this
  tab will compact (persistent, net holder, store healthy, no compaction in flight): the compaction that follows
  every import (forced by `set_meta` / `write_record`) would have trimmed them from both at once. The end state of
  IndexedDB is the same (chunks, no `ops`), the bytes are written once. A failed compaction now reschedules one
  (`full_compact` was set but nothing ran it). Recent revisions keep the journal path (SYNC §7.3 retention for
  peers). Test: `old_import_batches_go_straight_into_chunks`.
- `doc/versions.rs`: version snapshots are read when the tab may edit (Edit mode, `editor::view::sig::MODE`, or a
  local commit), not at open: a document opened to read holds no 1.6 MB snapshot.

Not changed: in-memory point storage (f64 pairs, 1.5 MB here) and the `Item` layout (240 B). With the budget met
(≈ 41 % of 5.18.0's tab) they were left alone; a compact lossless point store (scaled-integer deltas when every
value round-trips, f64 otherwise) would save about 1 MB more on this document but touches ~70 sites in 23 files.

## Renderer CPU: what caused the difference

Traced on both apps with the same script (`trace.mjs`, `BENCH_TRACE`, `BENCH_CALLS`):
- Scroll and zoom **without edits before**: the same canvases (page 1588×2246, lasso 1072×1516, cursor and remote
  993×1404 in both), the same event counts and durations within noise (scroll: 129 vs 130 main frames, 56 vs 54
  `ProduceCanvasResource`, 1481 vs 1420 software quads; zoom: both draw only the new template background, 1785 dot
  stamps on a 2881×4075 canvas + `toDataURL`, ≈ 125 ms, and decode it once). v6's only extra layers are two hidden
  Home dropdowns. bench: scroll 4.70–4.96 vs 4.73–5.07 ms/step, repaint 226–231 vs 218–220 ms.
- The +30 % / 2× figures of W6.2 came from the bench order (pen, erase, lasso, then scroll and zoom): the Drive
  export after those edits lands in v6's scroll or repaint window (v6's scribe exports 10 s after the last edit,
  SYNC §14), while 5.18.0's save runs ~4 s earlier, inside the lasso settle, and is not counted. In the outlier
  windows the extra work is the export's template JPEG (1985×2808 canvas, 1785 stamps, `toBlob`) and the save
  indicator's colour transitions (`building` → `uploading` → `saved`, `background-color 0.25s` on `::after`), which
  repaint the frame and re-produce the page canvas resource in every commit: 74 `ProduceCanvasResource` / ≈ 120 ms.
  The indicator costs the same in 5.18.0 (forced class changes: 91 vs 92 `ProduceCanvasResource`, 191 vs 185 ms).
  With a quiet period before each phase, or counting the whole session, v6 is equal or lower (whole session
  3.6–3.7 s vs 4.2–4.7 s).
- One thing v6 did more of: 5.18.0 caches the encoded template JPEG per template canvas across saves
  (`buildPdfBlobWithPdfLib.templateJpegCache`); v6 encoded it on every export. Now cached the same way
  (`export/pdf.rs` `tpl_jpegs`, dropped when the canvas leaves the template cache; `Templates::holds`).

## Tests

- `cargo test --workspace --locked`: ihn-app 168 (2 new), ihn-core 135 (+1 ignored), ihn-sync 49 — all pass.
- e2e (one worker, port 4731): 04-ink, 04b-erase, 04c-select, 05, 06, 08, 11, 12, 16, internal-canvas,
  internal-editor-view, internal-doc, internal-store — 46 passed, 0 failed.
- Parity `--quick` vs 5.18.0 (canvas, editor-view, editor-tools, pages, open, timeline): 108 comparisons, 0 fail,
  0 error, 10 expected (BEHAVIOUR-CHANGES), 98 warnings with equal pixels — PASS. Canvas trace goldens
  (internal-canvas) pass.

## Still worse than 5.18.0

- Tab peak while opening: 65–75 MB vs 92–103 MB for 5.18.0, i.e. better but not half: two short-lived workers
  dominate it (pdf.js ≈ 29 MB in both apps, v6's parse worker ≈ 30 MB for ~0.5 s before it is terminated). The
  parse builds a JSON tree of the whole STROKES payload; a streaming 5.x parser would cut it.
- Against 5.18.0's JS heap alone (11.9 MB, the number W6.2 compared with) v6's 12.5–13.2 MB total is not below
  50 %; counting what 5.18.0's tab really holds (31.2 MB with its ArrayBuffers and three workers) it is 40–42 %.
- Scroll/zoom per step: equal within noise, not clearly better; the work is the browser's (raster, canvas resource
  production per commit) and the same in both apps.

## Deviations

1. ihn-wasm (`ihn_out_reset`'s 1 MiB shrink threshold) was not touched (another unit owns it); `OutBuf::trim` does
   the shrink from ihn-app at the next entry instead.
