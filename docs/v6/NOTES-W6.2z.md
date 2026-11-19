# W6.2z — size work, round three (part of W6.2)

Branch `v6-w6_2z` from 6b4e85d (release candidate 6.0.0 + parity harness fix). No behaviour, pixel or speed change;
no module split, no post-link function cutting, no new tools (rustc 1.90, binaryen 123.0.0 from `node_modules`,
node, python3 only). First round: `NOTES-W6.2s.md`; test API out of production: `NOTES-W6.2t.md`.

## Result

| Step (one commit each, cumulative) | `ihn.wasm` raw | gzip -9 | Δ gzip |
|---|---:|---:|---:|
| Baseline 6b4e85d | 1,589,269 | 697,048 | |
| wasm-opt `--low-memory-unused` | 1,552,191 | 688,078 | −8,970 |
| `ihn_init` writes the `App` slot (`ptr::write`) instead of `Option::insert` | 1,534,243 | 682,406 | −5,672 |
| Workspace `opt-level = "z"`, `ihn-core`/`ihn-sync` explicitly `"s"` | 1,524,831 | 678,112 | −4,294 |
| wasm-opt `--merge-similar-functions` + a second `-Os` round | 1,515,719 | 675,071 | −3,041 |
| `ihn_core::vecmap::{VecMap, VecSet}` for every map/set of `ihn-sync` | 1,393,124 | 637,101 | −37,970 |
| `VersionVector`'s per-actor maps in `VecMap` | 1,379,359 | 632,123 | −4,978 |
| No `BTreeSet<Id>` left (doc page sets → `VecSet`, `Live::applied` → sorted `Vec`) | 1,371,838 | 628,443 | −3,680 |
| `history::needed_assets` sorts with `ihn_core::sort::by` | **1,368,628** | **626,840** | −1,603 |

Total **−220,641 raw (−13.9 %), −70,208 gzip (−10.1 %)**. First-party JS unchanged (124,718). `ihn-test.wasm`
1,446,965 / 663,880. Code section 1,232,817, data 130,985. The build is reproducible (the committed generated files
equal `npm run build`).

Budgets in `scripts/build-wasm.mjs`: measured + 5 % rounded up to KiB → **1404 KiB raw, 643 KiB gzip**; JS keeps
125 KiB (the W6.2t number; +5 % of today's JS would loosen it).

### Why each step is behaviour-neutral

- `--low-memory-unused`: rustc's wasm32 layout is stack first (`__stack_pointer` starts at 1 MiB and grows down),
  statics at 1 MiB, heap after them. Nothing addresses `[0, 1024)` except a stack 1 KiB from overflow; binaryen
  only folds constants < 1024 into load/store offsets, which changes results only when an address computation
  wraps into that range.
- `ihn_init`: the host calls it once per instance (`host/main.js`; the export worker returns before creating an
  `App`). `Option::insert` dropped the (never present) previous `App`, which linked and inlined the whole `App` drop
  glue (421 `deallocate` call sites). No `Drop` impls exist in the crates, so not dropping has no side effect.
- Opt levels and wasm-opt passes change codegen only.
- `VecMap`/`VecSet`: same observable semantics as `BTreeMap`/`BTreeSet` for the API used (unique keys, ascending
  iteration, `insert` keeps the key and returns the old value, `entry().or_*`, `retain` in key order, `collect` and
  `extend` = inserts in order so the last value of a repeated key wins, `Debug` printed as a map/set, derived
  `Eq`/`Ord`/`Hash` over the sorted entries). Unit test `vecmap::tests::behaves_like_btree` runs 20,000 random
  operations against std. Every converted collection holds one entry per device, actor, peer, connection, request
  in flight, Drive property or live stroke, or the pages one transaction touched. Kept as B-trees on purpose: the
  per-actor seq sets above a gap (`VersionVector::extras` values, can grow while batches arrive out of order and
  `settle` pops from the front), `Doc::versions` (uses `range`), `ihn-core::v5::structfields` (one entry per 5.x
  struct field, importer), test/simulator code.
- `Live::applied`: membership in a sorted `Vec<Id>` (`binary_search`) instead of a `BTreeSet<Id>` built from the
  same ids: same answer, O(n log n) for a large batch.
- `needed_assets`: both sorts are stable with the same comparator.

## Tried and not kept

| Variant (measured on the step-1 binary unless noted) | raw | gzip | Why not |
|---|---:|---:|---|
| `ihn-core` + `ihn-sync` at `"z"` | 1,434,175 | 643,094 (−45 KB) | Open dense-20 slower: 4×7 runs base 1256/1277/1293/1317 ms vs 1366/1347/1337/1345 ms (first canvas); wasm self time +6–10 ms per open (codec `Reader`, `snapshot::put_body`, hashbrown probes no longer inlined). |
| `ihn-core` at `"z"` only | 1,470,069 | 654,274 | Same regression (it is the core part). |
| `ihn-sync` at `"z"` only | 1,533,278 | 681,555 | Browser open unchanged, but a native 30,000-step sync-simulator run 314 → 353 ms (+12 %). |
| core `"z"` + `#[inline(always)]` on the codec primitives | 1,441,814 | 645,168 | +2 KB gzip and did not recover the open time; reverted. |
| wasm-opt `-Oz` | 1,580,381 | 695,111 | Base binary; inlines less in the hot crates (W6.2t); `-Os` + the passes above give more. |
| `--converge` instead of the second `-Os` | 1,515,657 | 674,999 | −72 B more for ~40 s per variant. |
| `--zero-filled-memory` | identical | identical | No gain. |
| `-Zlocation-detail=none` | — | — | Nightly only; not on the pinned 1.90 stable. Panic messages and locations also reach `host_log` through the panic hook, so they are behaviour. |
| `HashMap`/`HashSet` → `VecMap` in `ihn-app`/`ihn-core` | — | — | Not done: several sites iterate std hash maps, whose order would change; each needs its own proof. |

## Verification (all on this machine, 4 CPUs shared with other jobs)

- `cargo test --workspace`: 170 + 138 (+1 ignored) + 49 passed (one more than before: the `VecMap` test).
- Full e2e (`IHN_E2E_PORT=4370 npx playwright test --workers=1`): **98/98 passed** (8.9 min).
- Quick parity against 5.18.0 (`$S/r518`), one state file per run: home, open, canvas, editor-tools, selection,
  pages, share, presence, export, timeline: **0 fail/error in every state** (119 ok cells, 25 expected, warnings only).
- Performance, `tests/v6/perf/bench.mjs --runs 3` (two alternating rounds; base = unmodified 6b4e85d build):

| Measure | base #1 | base #2 | final #1 | final #2 |
|---|---:|---:|---:|---:|
| Boot → Home (ms) | 296 | 200 | 238 | 236 |
| Open dense-20 first canvas / all pages (ms) | 1538 / 1547 | 1426 / 1441 | 1400 / 1415 | 1364 / 1375 |
| Pen CPU ms/move; script ms/move | 3.92; 0.45 | 4.49; 0.48 | 3.84; 0.49 | 4.30; 0.47 |
| Eraser listener ms/move; CPU ms/move; script | 0.04; 4.63; 1.19 | 0.04; 5.13; 1.27 | 0.04; 4.49; 1.26 | 0.04; 4.91; 1.25 |
| Lasso release ms; CPU ms/move | 9; 4.36 | 9; 4.65 | 8; 3.61 | 8; 4.28 |
| Scroll CPU ms/step; script | 6.70; 0.50 | 7.38; 0.62 | 6.45; 0.52 | 6.77; 0.52 |
| Zoom CPU ms/step; repaint CPU ms | 5.65; 484 | 5.66; 241 | 5.33; 231 | 5.26; 215 |

  Boot/open again, `--runs 7`, four alternating rounds (home ms / first canvas ms): base 263/1311, 288/1479,
  254/1452, 214/1484; final 258/1522, 230/1396, 252/1452, 266/1430. Same within noise (load average 1–3 during the
  runs). Earlier, quieter rounds of the intermediate builds gave base 1252–1317 ms and 1198–1285 ms for steps 1–3.
  Native sync simulator (3 devices, 30,000 live-ink steps + edits, release build): 320 ms before the `VecMap` step,
  285 ms after.

## What remains (code section by area, from a names-kept build of the final sources)

`ihn-app`: doc 125 KB, editor 118 KB, home 114 KB, panels 79 KB, migrate 56 KB, collab 41 KB, store 34 KB, drive
33 KB, export 32 KB, shell 15 KB, auth 13 KB. `ihn-core`: v5 importer 72 KB, doc 35 KB, snapshot 21 KB, history
19 KB, render 18 KB, catalog 15 KB. `ihn-sync`: relay 43 KB, rest ~45 KB. std: iterator adapters 36 KB, sorts
(the hot `doc.rs`/`session.rs` ones W6.2s kept) 33 KB, hashbrown 29 KB (mostly `IdMap` in `ihn-core::doc`),
`Vec` 25 KB, B-trees 22 KB (was ~120 KB), `str` 21 KB, fmt + float formatting/parsing 30 KB, dlmalloc 7 KB.
Data section 131 KB (templates, texts, std tables). The largest lever left inside the rules is the hot crates'
opt level (~45 KB gzip), which costs ~4 % on open; the next ones need per-site work (hash maps with iteration,
the app's UI code itself).
