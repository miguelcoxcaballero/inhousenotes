# Inhouse Notes v6 — changelog of the Rust rewrite (work in progress)

The owner approved 6.0.0 for web release on 2026-11-19. The release includes the
5.19.0 mainline (`35d0f22`), including the Android 1.0.29 / versionCode 30 channel A metadata.

Rust retains the Notes UI, profile, refresh scheduling and Drive request waiters. The host
delegates GIS, Android grant discovery, PKCE, callback validation, invalidation and local
logout to the canonical OAuth module. Visual parity and upgrade checks now target 5.19.0.
Historical measurements below retain their original baseline.

User-facing release notes: `RELEASE_NOTES_v6.0.0.md`. Per-unit details: `docs/v6/NOTES-W*.md`.

## 6.0.0

### What it is
- The whole app is one Rust program (`rust/`: `ihn-core`, `ihn-sync`, `ihn-app`, `ihn-wasm`, no external crates)
  compiled to `wasm/ihn.wasm`. A thin JavaScript host (`boot.js`, `host/*.js`) carries out the browser calls that
  Rust decides on. `service-worker.js`, pdf.js and pdf-lib remain JavaScript.
- Same screens as 5.18.0: same markup and CSS. A pixel-parity suite (`tests/v6/parity/`) compares every state with
  5.19.0.
- New document model built on operations, with instant sync over P2P or through small sync files in Drive (`SYNC.md`).
  The PDF in Drive is rebuilt after 10 s of inactivity, when the user leaves the document, and on Export.
- Existing Drive documents open as before, and local 5.x data is migrated once on first start.

### Measured against 5.18.0 (desktop Chromium, Drive mock)
- Home first card: 280 ms instead of 458 ms. Requests: 31 / 2.2 MB instead of 47 / 3.9 MB.
- Opening a dense 20-page 5.x notebook: 1.24 s instead of 1.65 s. Memory after opening: 12.5–13.2 MB instead of
  31.2–31.4 MB.
- Eraser: 0.03 ms of script per move instead of 0.52 ms. Pen worst frame: 16.8 ms instead of 66.6 ms.

### Size work after the release candidate (W6.2z, W6.2y, W6.2x)

| Step | `ihn.wasm` raw | `ihn.wasm` gzip (build) | First-party JS |
|---|---:|---:|---:|
| Release candidate W6.2r | 1,589,269 | 697,048 | 124,718 |
| W6.2z: sorted-vector maps in sync, `opt-level` tuning, wasm-opt passes | 1,368,628 | 626,840 | 124,718 |
| W6.2y: functions ordered by name, one hasher, binaryen inlining limits | 1,337,634 | 596,635 | 124,718 |
| W6.2x: host logic moved from JS to Rust (export plan, Android chunks, pdf limits, event tables) | 1,340,739 | 597,941 | 121,316 |

- 5.18.0 downloads 456 KB of gzipped first-party JS, so 6.0.0 is still about 140 KB larger. The one remaining step
  with a large saving is splitting the wasm into two modules, which needs the owner's authorisation.
- Every step was verified at the time: `cargo test` (173 + 138 + 49), e2e 98/98 and quick parity with 0 failures
  (22 state files in W6.2x). Perf matched the earlier build within noise, and exported PDFs were byte-identical in
  W6.2x.
- Tried and rejected: compiling the hot crates for minimum size (−45 KB, but opening documents was 4 % slower) and
  several wasm-opt passes. Numbers are in the notes.

### Verified on `v6-int`
- 5.19.0 integration: 360 Rust tests passed (one ignored), six shell checks passed. Production wasm is 1,334,340 bytes (595,562 gzip). The offline shell includes the canonical OAuth script.
- [Browser and reproducible build checks](https://github.com/miguelcoxcaballero/inhousenotes/actions/runs/44502820292): all 98 existing e2e cases plus the Android shared-account regression passed (99/99).
- [Full 5.19.0 matrix](https://github.com/miguelcoxcaballero/inhousenotes/actions/runs/44498736786): 22 state files, 2,387 comparisons, 2,088 pixel-identical with DOM/trace diagnostics, 298 expected differences, one failure and zero errors. See `tests/v6/parity/REPORT.md`.
- The failure is the first capture of `home/folder-colours` on phone-412, system-dark. It differs in 26 pixels in a 41×38 rectangle at (654,57). Three independent runs with the pinned Playwright Chromium reproduced the same 26-pixel difference **between two runs of 5.19.0 itself**. A separate reference-only run (`--self --states home --filter home/folder-colours --viewports phone-412 --runs 3`) also reproduced variation. The original verdict and zero tolerance are retained. The previous 5.18.0 matrix had a clean run; its notes also record intermittent variation at this icon (`NOTES-W6.1e.md`).
- Deploy emulation (W6.2r): the site shows `v6.0.0`, the service worker activates, and there are no failed requests.

### Known limits and open items
- Images and version snapshots do not travel over P2P or the relay; they arrive with the next PDF export.
- Drive sync has been verified only against the mock, not real Google accounts.
- Production checks ran on the integration branch. The owner approved the documented behaviour changes.
- 5.18.0 waits for structure mutations to settle before some repaints; v6 does not port that wait (states pass).
- The 5.19.0 runtime files (`*-v5.js`, `wasm/inhouse_notes_core.wasm`) stay deployed unused until 6.0.1, so tabs
  still open on 5.x keep working until they reload.
- Some commits on unit branches carry different co-author lines; review the trailers when preparing the `main`
  commit.

### Decisions pending from the owner
1. Keep or delete `stencil.html`.
2. Supply real test accounts/devices for end-to-end Google and cross-app Android acceptance.

Current scope keeps one wasm module and the existing APK. A split or Android release needs a separate request.
