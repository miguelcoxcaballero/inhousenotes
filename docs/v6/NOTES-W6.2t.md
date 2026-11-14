# W6.2t — size work, round two (part of W6.2)

## Result

| Build (measured on `v6-int` bc2e028 + this branch) | raw | gzip -9 |
|---|---:|---:|
| `ihn.wasm` before (W6.2s, test API included) | 1,658,239 | 728,164 |
| **`ihn.wasm` now (production, no test API)** | **1,577,352** | **692,278** |
| `ihn-test.wasm` (e2e/parity/perf only, never in the shell or the APK) | 1,658,239 | 728,142 |

−81 KB raw, −36 KB gzip (−5 %). The build is reproducible (same SHA-256 on rebuild). Budgets in
`scripts/build-wasm.mjs` are now measured + 5 %: **1618 KiB raw, 710 KiB gzip, 125 KiB JS** (JS is 121,245 bytes).
5.18.0 shipped 456,055 gzip bytes of first-party JS: v6 still downloads ~236 KB more compressed bytes. Reversing that
needs the split module of (c) below; nothing else measured comes close.

## (a) Why the wasm compresses to only 44 %

- Code section: 1,513,077 → 655,672 gzip (43 %); data section: 139,846 → 69,325 (50 %). The code is 95 % of the
  compressed bytes. Wasm bytecode is already a compact binary encoding (LEB128 immediates, local and function
  indices), so it has far less redundancy than JS source text (5.18.0's JS compresses to 20 %). Compare gzip bytes,
  not ratios.
- No embedded compressed, base64 or other high-entropy blobs. The only binary tables in the data section are std's
  Unicode case-mapping tables (~10 KB, `to_lowercase`) and the float formatting/parsing power tables (~10 KB).
- Duplicated templates: repeated runs of ≥ 48 bytes add up to ~6 KB raw (mostly the 24×24 SVG prefix and
  21 copies of the `/rustc/<hash>/` panic-location prefix); indentation whitespace in templates is ~2.2 KB. Gzip's
  32 KB window already removes most of it (< 2 KB gzip in total).
- wasm-opt variants on the production binary (raw / gzip): `-Os` 1,577,439 / 692,215 (kept);
  `-Os --converge` 1,575,347 / 690,693; `-Oz` 1,568,671 / 690,221; `-Oz --converge` 1,566,645 / 688,684;
  `--strip-producers --strip-target-features`: identical (cargo's `strip` already removes both sections). At most
  −3.5 KB gzip (0.5 %), and `-Oz` inlines less in the hot crates: not changed.
- Brotli q11 would be 543,927 (−21 %), but **GitHub Pages serves gzip only** (verified with curl on
  inhousenotes.com: `Accept-Encoding: br` gets identity, 1,715,623 bytes for `app-v5.js`; `gzip` gets
  `content-encoding: gzip`, 343,212 bytes; `application/wasm` is gzipped too: `wasm/inhouse_notes_core.wasm`
  53,994 → 21,547). Brotli does not change what users download.

## (b) Test API out of production — done

- Cargo feature `testapi` (`ihn-wasm` → `ihn-app/testapi`). Without it, `App::dispatch` ignores `Event::Test` and the
  module table holds `no_test_cmd` instead of each module's `test_cmd`, so LTO drops `testapi::dispatch`, every
  `test_cmd` (app, core and sync) and what only they use (selftests, state dumps). `cargo test` keeps them
  (`cfg(test)`).
- `scripts/build-wasm.mjs` builds both variants from the same sources with the same profile and wasm-opt pass, and
  checks imports/exports/ABI for both: `build/wasm/ihn.wasm` (production, budgeted) and `build/wasm/ihn-test.wasm`.
- `host/main.js` loads `ihn-test.wasm` only when `?e2e=1` on localhost (the condition that already enabled
  `host/test-api.js`). The service worker adds it to the shell only on localhost, so offline e2e behaves like
  production. It is not in `offline-assets.js` nor `native-assets.json`.
- **Exact difference between what e2e/parity/perf run and what ships:** the test build additionally contains
  `testapi::dispatch`, every module's `test_cmd` and the functions only they call, and its module table points at
  the real `test_cmd`s. Every other function is compiled from the same source with the same flags; the only
  possible codegen difference is LTO inlining/ordering around the extra code. The test build is the size of the
  previous production build to the byte (1,658,239). `00-boot` now checks that `?e2e=1` fetches `ihn-test.wasm`
  and that the plain page fetches `ihn.wasm` and has no test API.

## (c) Lazy second module — not done: blocked by the toolchain, measured saving below

Attribution of the code section by the name section of an unstripped build (same code, test API included):
editor + panels + collab + export + doc sessions/versions/import 365 KB (24.5 %), migration + 5.x importer +
snapshot 153 KB (10.2 %), `ihn-sync` 143 KB (9.6 %) → **661 KB (44 %) not needed by Home at boot**; boot code
(shell, auth, Home, store, Drive, updater, app/abi/dom) 259 KB, `ihn-core` doc/render/history 117 KB, std/generic
350 KB (shared). Estimate for a split: primary ≈ 810 KB raw / ≈ 355 KB gzip (data section stays there), secondary
≈ 780 KB / ≈ 340 KB, total a few KB more than today. That is the only measured route under 400 KB gzip at boot.

Why it is not in this unit:
- Rust/LLVM cannot emit two wasm modules that share one memory, allocator, stack pointer and function table;
  two separately linked cdylibs would each place their statics at the same addresses and run their own dlmalloc.
- The tool that does it after linking is binaryen's `wasm-split`. The pinned `binaryen@123.0.0` npm package
  ships `wasm-opt`, `wasm-merge`, `wasm-metadce`, `wasm-ctor-eval`, … but **not `wasm-split`**, and binaryen.js
  exposes no module-splitting API. The native release has it, but running a downloaded native binary was refused in
  this environment, and the build would need a new pinned download with a checksum (§2.4/§14.1 build policy).
- A hand-written splitter (binaryen.js: stub each deferred function with `if table[slot] is null: call
  env.host_load; call_indirect slot`, secondary = same module with primary functions turned into imports, memory,
  table and `__stack_pointer` imported) plus a host loader (prefetch and compile after Home is idle, synchronous
  instantiate in `host_load`, the same in the export worker) and a 10th ABI import is a new build tool and loader,
  not a mechanical edit. Proposed as a follow-up unit with that design; hot paths would not change because the
  editor, render and sync code would all live in the secondary module.

## (d) Data section — not changed

148 KB → 139,846 bytes after (b); 64.5 KB of it is text in 917 strings. Dedup/whitespace gains are < 2 KB gzip
(see (a)) and every template must stay pixel-exact; the remaining 75 KB are std tables, `fmt::Arguments` piece
tables, vtables and `&str` slice tables that cannot be re-encoded from the source.

## (e) Generic bloat — not changed

Remaining std code: BTreeMap/BTreeSet 120 KB (≈30 key/value instantiations), std sorts 53 KB, hashbrown 28 KB,
iterator adapters ~25 KB. The BTrees are the sync engine's deterministic-order state (`ihn-sync` peers, relay,
live strokes, version vectors in `ihn-core::stamp`, the doc index) on the remote-apply/tick/frame paths, and the
std sorts left are the ones W6.2s kept on purpose (`doc.rs` index, `session.rs` remote batches). Replacing them with
sorted-vector helpers changes their complexity on hot paths, and those files are being edited in parallel. The only
clearly cold instantiations are three `BTreeMap<String, _>` in `ihn-core/src/v5/structfields.rs` (≈10 KB raw,
≈4 KB gzip), whose types are public and used by the importer and migration: left for the split module, where they
would leave the boot path anyway.

## Tests

- `cargo test --workspace`: 165 + 135 + 49 passed.
- e2e 00, 02, 04, 08, 11, 12: all pass except `08-save-reopen.spec.mjs:180` (export to the user), which fails the
  same way on `v6-int` without these changes (`inked` false at :198): pre-existing.
- Quick parity: see the report summary in the unit result.
