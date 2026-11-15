# Inhouse Notes web architecture

Since 6.0.0 the app is a Rust/WebAssembly program behind a thin JavaScript host. The full design is
[`docs/v6/ARCHITECTURE.md`](v6/ARCHITECTURE.md); sync is [`docs/v6/SYNC.md`](v6/SYNC.md). The site is still served as
static files from GitHub Pages and the Android APK loads it.

## Runtime boundaries

| File | Responsibility |
| --- | --- |
| `index.html`, `offline-v5.css` | 5.18.0's markup and styles (byte-identical UI); version block, native asset map, script tags |
| `boot.js` | Theme, skeleton reveal, release watcher, `InhouseAssetUrl`, `InhouseBackground` |
| `host/main.js` | Wasm loader, ABI, out-buffer dispatcher, events, frames and timers |
| `host/dom.js`, `host/canvas.js` | DOM operations and canvas command replay |
| `host/idb.js`, `host/fetch.js` | IndexedDB transactions, Web Locks; fetch with deadlines, uploads, digests, compression |
| `host/pdf.js`, `host/export-worker.js` | pdf.js adapter; export worker (pdf-lib + its own core instance), large 5.x imports |
| `host/peer.js`, `host/platform.js` | WebRTC/BroadcastChannel transports; Google sign-in, Android bridges, updater, files, clipboard |
| `service-worker.js` | Shell cache `inhouse-shell-6.0.0`; keeps `inhouse-shell-5.18.0` for open 5.18.0 tabs until 6.0.1 |
| `rust/crates/ihn-core` | Document model, codecs, render geometry, exact maths, 5.x readers (no I/O) |
| `rust/crates/ihn-sync` | Sync protocol state machines (P2P and Drive relay) |
| `rust/crates/ihn-app` | UI state machines, input, scheduler, storage, Drive, auth, export, migration of 5.x data |
| `rust/crates/ihn-wasm` | The `extern "C"` boundary to `host/main.js` |

The 5.18.0 runtime files (`*-v5.js`, `rust-core-v5.js`, `wasm/inhouse_notes_core.wasm`, `boot-v5.js`,
`bullet-template.js`, `ink-palette.js`, `vendor/jspdf/`) stay deployed but unreferenced in 6.0.0, for 5.18.0 tabs that
are still open (`docs/v6/ARCHITECTURE.md` §10.4). They are deleted in 6.0.1.

## Build and release verification

`npm run build` compiles `rust/` to `build/wasm/ihn.wasm` (and the e2e-only `ihn-test.wasm`), checks the ABI, the
import allowlist, the exact-maths gate and the size budgets, and writes `build/offline-assets.js`,
`build/native-assets.json` and `build/index.html`. A release commit copies those four files over the tree.
`npm run test:cargo`, `npm run test:shell`, `npm run test:e2e` (Playwright, `tests/v6/e2e`) and
`npm run test:parity` (pixel parity against 5.18.0) are required by `.github/workflows/production-checks.yml`
(`docs/v6/ARCHITECTURE.md` §14.2, `docs/v6/TESTING.md`).
