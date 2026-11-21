# 6.2.0 — ihn.wasm in two parts

`scripts/split-wasm.mjs` (binaryen.js, already a build dependency) splits each build after wasm-opt:

- **First part** (`wasm/ihn.wasm`, 571,242 bytes / 260,990 gzip): the functions that run while the app boots and
  shows Welcome or Home (`scripts/wasm-split-keep.txt`), the exported entry points and every unnamed function.
- **Second part** (`wasm/ihn-2.wasm`, 818,156 / 365,833 gzip): everything else. It imports the first part's
  memory, table, globals and the functions it calls (`p.t`, `p.g<i>`, `p.f<n>`), has no data segments and fills
  table slots reserved for it with an active element segment when it is instantiated.
- Each moved function stays in the first part as a stub with the same index and signature: if its table slot is
  empty it calls `env.host_split()`, then `call_indirect`s through the slot. Function pointers to it keep working.

`host/main.js` fetches and compiles the second part right after boot and instantiates it as soon as it is compiled.
If the first part needs it earlier, `host_split` compiles and instantiates it synchronously (a 1.3 MB module compiles
synchronously on the main thread in about 5 ms), from the fetched bytes or a synchronous request. The export worker
gets the compiled second module and instantiates it against its own instance.

Profile: `IHN_SPLIT_PROFILE=1 npm run build` writes instrumented, unsplit builds (each function calls
`env.host_mark(i)`); `IHN_SPLIT_PROFILE=1 npx playwright test zz-split-profile` boots both builds to Welcome and to
Home and writes the keep list with stable names (Rust symbol hashes removed, so the test and production builds
match). A stale list is safe: it only makes the second part load earlier.

Measured: Rust tests, 100/100 e2e (the production boot fetches each part once; boot, offline, export worker,
5.19.0 → v6 upgrade), quick parity `canvas` 0 failures. First load on an emulated slow mobile network (1.6 Mbps,
150 ms, empty cache): app ready 11.7 s → 7.9 s. On a local server boot and document open are unchanged within noise.
