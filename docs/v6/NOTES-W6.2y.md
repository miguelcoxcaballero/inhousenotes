# W6.2y — size work, round four (part of W6.2)

Branch `v6-w6_2y` from a95316a (end of W6.2z). No behaviour, pixel or speed change; no module split, no post-link
cutting, no new tools or crates (rustc 1.90, binaryen 123.0.0 from `node_modules`, node, python3 only). Earlier
rounds: `NOTES-W6.2s.md`, `NOTES-W6.2t.md`, `NOTES-W6.2z.md`.

## Result

| Step (one commit each, cumulative) | `ihn.wasm` raw | gzip -9 | Δ gzip |
|---|---:|---:|---:|
| Baseline a95316a | 1,368,628 | 626,840 | |
| wasm-opt `--reorder-functions-by-name` (linker name section kept for it, stripped after) | 1,374,995 | 614,314 | −12,526 |
| Every std `HashMap`/`HashSet` with `IdHasher` (`ihn_core::id::FastMap/FastSet`): SipHash gone | 1,368,306 | 611,272 | −3,042 |
| 5.x `structfields` registers: `VecMap` instead of `BTreeMap<String, _>` | 1,357,226 | 606,600 | −4,672 |
| wasm-opt `-ocimfs 20` (single-caller inlining only up to 20 instructions) | 1,351,182 | 601,181 | −5,419 |
| `VecMap` for sync `peer_vv`, history `needed_assets` bases and `plan_prune` winners | 1,345,650 | 599,283 | −1,898 |
| `Doc::versions` in `VecMap` (+ `VecMap::range`) | 1,339,866 | 597,238 | −2,045 |
| `Doc::inverse`: `[IdSet; 6]` instead of a set of `(u8, Id)` | 1,338,003 | 597,003 | −235 |
| wasm-opt `-aimfs 4` (always inline bodies ≤ 4 instructions) | **1,337,634** | **596,635** | −368 |

Total **−30,994 raw (−2.3 %), −30,205 gzip (−4.8 %)**. Code section 1,201,833, data 130,912. `ihn-test.wasm`
1,416,822 / 631,674. First-party JS unchanged (124,718). Reproducible: a fresh `npm run build` in a copy of the tree
at another path and with an empty `rust/target` gives the same SHA-256 for both wasm files; the committed generated
files equal `npm run build`.

Budgets in `scripts/build-wasm.mjs`: measured + 5 % rounded up to KiB → **1372 KiB raw, 612 KiB gzip** (were
1404/643); JS keeps 125 KiB.

### Why each step is behaviour-neutral

- **Function order** (`--reorder-functions-by-name`): only the order of the code-section bodies and the function
  indices that refer to them (calls, table elements, exports) change; semantics are index-independent. Sorting the
  mangled names groups each module's functions, so gzip's 32 KiB window sees their shared sequences (call indices get
  longer: +6.6 KB raw). The names come from the linked binary (`CARGO_PROFILE_RELEASE_STRIP=debuginfo` for the
  cargo step of `build-wasm.mjs` only, so the linker keeps its name section; DWARF still goes). wasm-opt then
  strips the name, producers and target-features sections: the output has exactly the sections it had before. The
  mangled names contain no paths (remapped, and cargo hashes path dependencies relative to the workspace), so the
  order is the same on every machine (checked from a second checkout path).
- **`IdHasher` everywhere**: the remaining std maps keyed by strings, content hashes, tuples and `PeerRef` used
  `RandomState` (SipHash) only because of their declaration. Every site was checked: membership/lookups only
  (importer sets, history prune/restore sets, versions import), a commutative `min` fold over `values()`
  (`covered`), or entries collected and then sorted by a total order on unique keys (`deletion_stamps`: `cmp_utf16`
  of distinct keys; `plan_prune`: (ts, unique id)). No iteration order reaches an output. The hasher is the one
  `IdMap` already uses; collisions only cost probes.
- **`VecMap` conversions**: same API semantics as `BTreeMap` (W6.2z, unit test against std, now also for `range`).
  Sizes: `structfields` one entry per 5.x page/field (its own page-id loop is already quadratic), `peer_vv` one per
  peer/tab/relay, history maps one per timeline record. `Doc::versions` grows a few entries per hour of writing;
  snapshot loads and `merge` iterate a sorted source, so inserts are appends or hits; the worst case (merging a
  diverged history of thousands of versions of a lower actor) is a few tens of MB of `memmove` once, versus never
  measurable in the bench. `VecMap::range` returns nothing where `BTreeMap::range` would panic (start > end); the
  only caller passes `lo..=lo|u64::MAX`.
- **`Doc::inverse`**: one `IdSet` per register kind gives the same "first time this (kind, id)" answers.
- **wasm-opt inlining limits**: codegen only. `-ocimfs 20` keeps calls to single-caller functions larger than 20
  instructions (LLVM already inlined what it judged worth it); `-aimfs 4` inlines more tiny bodies.

## Tried and not kept

| Variant | raw | gzip | Why not |
|---|---:|---:|---|
| `--reorder-functions` (by call count) | 1,363,328 | 635,079 (+8.2 KB) | Raw −5 KB, but scatters modules. |
| Names demangled before the by-name sort (throwaway name-section rewrite) | 1,371,202 | 614,204 | −0.3 KB more than the mangled order; would need an own post-link tool. |
| `--reorder-locals`, `--duplicate-function-elimination`, `--dae-optimizing`, `--strip-producers`, `--generate-stack-ir --optimize-stack-ir`, `--coalesce-locals-learning`, `--local-cse`, `--rse`, `--code-pushing`, `--merge-locals`, `--precompute-propagate`, `--simplify-globals-optimizing`, `--licm`, `--inlining-optimizing -ifwl` (after the pipeline) | ±0 | −18 … +94 B | No gain. `--optimize-added-constants-propagate`: −3 B raw. `--dfo` needs flat IR. |
| `--gufa-optimizing -Os` | 1,356,287 | 606,100 (−0.5 KB) | ~40 s more per variant for 0.1 %; GUFA's flow assumptions are fine for an open-world module, but the gain does not justify reviewing them. |
| `-const-hoisting` | 1,352,241 | 635,755 | Worse gzip. |
| `-ocimfs` 0 / 5 / 10 / 30 / 50 / 200 / 1000 | | 601,781 / 601,293 / 601,198 / 601,176 / 603,257 / 603,716 / 606,163 | 20 chosen from the 10–30 plateau. |
| `-aimfs` 1 / 3 / 5 / 6 / 8 / 12 | | ±0 / −285 / −457 / +82 / +1,513 / +8,083 | 5 grows raw by 7.7 KB; 4 is the last that shrinks both. |
| `-Oz` instead of `-Os` (with the new flags) | 1,348,891 | 600,305 (−0.9 KB) | Inlines less in the hot crates (W6.2t/W6.2z). |
| `--converge` | 1,350,540 | 601,038 (−0.1 KB) | ~40 s per variant. |
| Importer `page_items` live-id set → sorted `Vec<&str>` | −2.0 KB | −182 B | Sorting 2,400 ids per dense page instead of hashing; too little for the risk. Reverted. |

Not attempted, with the reason: `--type-ssa`/closed-world passes (GC-only, nothing to do on this module);
`-C llvm-args` inline thresholds (global, would also change `ihn-core`/`ihn-sync` codegen); shortening the
`/rustc/<hash>/` panic-location paths (changes panic messages reaching `host_log`); replacing `format!` call sites
(each costs ~30–50 bytes and the output must stay byte-identical; 341 sites, gains spread thin); the
`VersionVector::extras` `BTreeSet<u32>` (kept on purpose by W6.2z); the dec2flt power-of-five table (10.4 KB of
incompressible data, needed for correctly rounded JSON number parsing).

## What remains

Code section by area (names-kept build of the final sources): `ihn-app` doc 125 KB, editor 118 KB, home 114 KB,
panels 79 KB, migrate 56 KB, collab 41 KB; `ihn-core` v5 importer 72 KB, doc 35 KB; `ihn-sync` relay 43 KB; std:
iterator adapters 35 KB, slice sorts 29 KB (the hot `doc.rs`/`session.rs` ones), `Vec` 19 KB, drop glue 16 KB,
hashbrown 25 KB (`IdMap`s of `ihn-core::doc`, importer string sets), B-tree 9 KB (`extras`), fmt 10 KB, dlmalloc
7 KB. Data section 131 KB raw / 65 KB gzip, of which ~46 KB gzip are binary tables (panic locations, `&str` slice
tables, dec2flt and Unicode tables). The large single-function bodies (`home::dialogs::event` 28 KB, `migrate::merge`
17 KB, …) are their modules' handlers with callees inlined, not generic bloat. Further size needs either the hot
crates at `"z"` (~45 KB gzip, ~4 % slower open, W6.2z) or the lazily loaded module of W6.2t (c).

## Verification (this machine, 4 CPUs shared, load average 1–2)

- `cargo test --workspace`: 170 + 138 (+1 ignored) + 49 passed (the `vecmap` test now also checks `range`).
- `npm run build`; full e2e (`IHN_E2E_PORT=4371 npx playwright test --workers=1 --trace off`): **98/98 passed**
  (8.5 min).
- Quick parity against 5.18.0 (`$S/r518`), one state file per run: home, open, canvas, editor-tools, selection,
  pages, share, presence, export, timeline: **0 fail/error in every state** (119 ok cells, 25 expected, warnings
  only — the same counts as W6.2z).
- Performance, `tests/v6/perf/bench.mjs --runs 3`, two alternating rounds, base = the unmodified a95316a build:

| Measure | base #1 | base #2 | final #1 | final #2 |
|---|---:|---:|---:|---:|
| Boot → Home (ms); wasm ready (ms) | 225; 59.6 | 197; 53.1 | 230; 57.5 | 230; 58.1 |
| Open dense-20 first canvas / all pages (ms) | 1397 / 1406 | 1356 / 1367 | 1310 / 1318 | 1340 / 1348 |
| Export dense-20 (ms) | 438 | 329 | 471 | 421 |
| Pen CPU ms/move; script ms/move | 4.58; 0.49 | 4.33; 0.44 | 4.22; 0.45 | 4.37; 0.44 |
| Eraser listener; CPU; script ms/move | 0.04; 5.70; 1.32 | 0.04; 5.32; 1.27 | 0.03; 5.03; 1.21 | 0.03; 5.20; 1.38 |
| Lasso release ms; CPU ms/move | 10; 5.01 | 7; 4.65 | 9; 4.52 | 7; 4.37 |
| Scroll CPU ms/step; script | 7.32; 0.59 | 7.20; 0.61 | 6.64; 0.52 | 7.23; 0.55 |
| Zoom CPU ms/step; repaint CPU ms | 5.62; 513 | 5.54; 458 | 5.37; 511 | 5.43; 222 |
| Pinch CPU ms/step; script | 18.96; 13.93 | 20.80; 15.45 | 18.50; 13.52 | 19.25; 14.02 |

  Boot/open/export again with `--runs 7`, four alternating rounds (home median ms / first canvas ms / export ms):
  base 238/1322/442, 231/1415/454, 222/1324/441, 215/1394/442; final 212/1346/466, 236/1344/358, 236/1344/450,
  232/1360/451. Same within noise; nothing consistently slower, so no technique was reverted for speed.
- Reproducibility: the committed `wasm/ihn.wasm`, `offline-assets.js`, `native-assets.json` and `index.html` equal
  `npm run build`; a second checkout at another path with an empty `rust/target` builds the same `ihn.wasm`
  (08e5a19f…) and `ihn-test.wasm` (b58c4a69…).
