# Inhouse Notes 6.0.0 — Testing

Lean by decision of the user (REQUIREMENTS §5): `cargo test` for the core, about 15 browser smoke tests, one strict
pixel-parity suite against 5.18.0, a two-device sync test, a migration test and a performance check. None of the ~140
v5 test files is ported. Everything lives under `tests/v6/`; `npm test` = `test:cargo && test:e2e && test:parity`.

---

## 1. Cargo tests (`cargo test --locked --workspace`, native target, < 60 s)

| Crate / module | Tests |
|---|---|
| `ihn-core::stamp` | Several batches of one actor within the same millisecond, delivered in reverse order → identical state; far-future clock applied but not observed; VV contiguity + extras. |
| `ihn-core::jsmath` | `sin/cos/atan2/exp/pow/round/hypot` equal values recorded from Node 22 (`tests/v6/fixtures/math/*.json`, W1.4) bit for bit over a grid and edge cases. `jsfmt` equals `Number.prototype.toString` on the same corpus. |
| `ihn-core::doc` | **Convergence property test**: 2–5 replicas, 2,000 random ops each (all `Op` kinds incl. undo-generated inverses, cross-page moves, area splits, page moves/deletes), delivered in random order with duplicates, drops-then-redelivery and gaps; assert identical encoded snapshots and identical `order()`/`page_items()`. Seeded PRNG, failing seed printed. |
| | Idempotence (apply twice = once), commutativity of every op pair, tombstone never resurrects. |
| | Undo/redo: inverse of every `Tx` restores the pre-state; undo under concurrent remote edits keeps remote edits; area-erase undo restores original z; cross-page move puts items on top of the target page; `draw_list` order = covers, images, strokes. |
| `ihn-core::snapshot`, `codec` | encode∘decode = id for snapshots and batches; truncated/corrupt input → `Err`, never panic; size regression guard on the 20-page profile document. |
| `ihn-core::history` | capture/coalesce/prune rules (5 min, same author/device, milestones, 80/30 caps), restore plan reaches the target, diff highlights equal 5.18.0's for recorded page pairs. |
| `ihn-core::geom` | point–segment hit (ported `editor.rs` test), area-eraser split ranges incl. 1-ULP threshold cases (ported from `editor_input`), lasso. |
| `ihn-core::render` | **trace goldens**: for every canvas scene of TESTING §3.3 (`canvas` area), the command stream rendered by Rust, serialised like `trace-init.js`, equals the 5.18.0 trace recorded once by W1.2 (`tests/v6/fixtures/traces/*.json`); bullet-template geometry; `pdfink` streams parse and keep every sample. |
| `ihn-core::v5` | every v5 fixture (§5) imports; counts and ids equal the 5.18.0 import record; deterministic (two imports, identical batches); revisions R0 and R1 imported in both orders give identical state with R1's values winning; xor-v1/raw-v1 bit-exact incl. −0/NaN/±Inf; strict-mode rejections; security-core limits (64 MiB, 2,000 pages, section sizes) with 5.18.0's error strings; `simpleHash` and legacy ids; IH_VERSIONS schema 3 and legacy array; IndexedDB record decoders. |
| `ihn-core::catalog`, `palette`, `motion`, `rows` | ordering and view tests ported from the library-index branch; palette exhaustive luminance sweep (ported); `motion` 29 values equal 5.18.0's `folderMotion` for a grid of rects. |
| `ihn-sync` | SYNC §19.1–19.2 (property tests and the simulator: skew, crash points, removed reader, copied document, scribe/etag churn, forced export, rename outside the editor, throttling, request budget incl. version assets, degraded storage). |
| `ihn-app` (FakeHost) | auth state machine (token expiry, 401 retry, reconnect, sign-out cancels in-flight), store transaction scripts, migration planning over dumped fixtures (incl. the re-dump fingerprint rule), session persistence ordering (journal append before network effect; degraded mode), remote-apply budget during contact, §6.6 status mapping table, DOM-op golden tests for card/folder/page-row templates. |

---

## 2. Browser smoke tests (`npm run test:e2e`, Playwright Chromium, mocks from `tests/v6/mock/`)

All load `http://127.0.0.1:<port>/?e2e=1` with the Google/Drive/GitHub mock, assert no CSP violation and no page
error, and use `__IHN_TEST_API__.state()` only for assertions (actions go through the UI).

| # | Spec | Flow → assertion |
|---|---|---|
| 00 | `00-boot` | Boot signed out under CSP → Welcome visible, both `[data-app-version]` = `v6.0.0`, wasm imports = allowlist, no pdf.js loaded. |
| 01 | `01-signin` | Welcome → Continue with Google (GIS stub) → Home lists from Drive; reload keeps session; app mode: PKCE via `InhouseNative.openAuthUrl` + `handleInhouseNativeOAuth('?code=…')`; Sign out → Welcome and keys removed. |
| 02 | `02-home` | Tabs, open folder (zoom animation runs and ends), breadcrumbs, back/Escape, search (250 ms debounce, one request), pull-to-refresh issues one list request, offline → cached lists + offline status. |
| 03 | `03-docs` (+`03b-home-actions`) | Create PDF from FAB in a folder → editor (document id = reserved id; Drive create carries it); rename title → Drive PATCH; Home card appears; ⋮ rename (then reopen: title = new name, and the next export keeps it) / star / colour / copy (copy request nulls `ihn6_*` properties; opening the copy never applies the original's relay) / move; move to bin with slider; Bin restore and delete forever (relay files of that document queued for GC); upload a PDF; the sync folder never appears in any list or search. |
| 04 | `04-ink` | Pen stroke, highlighter, stroke eraser, area eraser (while pen captured), lasso select + move + rotate + recolour + duplicate + delete, undo/redo ×N → model state matches expected ids/points; pen points unchanged after reopen. |
| 05 | `05-pages` | Pull-to-add page, Pages panel reorder by drag, delete with slider, photo page from gallery (file chooser) in both fit modes, **Import PDF** (`#btn-import-pdf`: replaces the pages, clears undo) → page order/backgrounds in state and in the next export. |
| 06 | `06-images` | Paste and drop an image, place, transform, undo → asset stored once (hash), image body as expected. |
| 07 | `07-share` | Share modal: add person (permissions POST), change role, link to anyone, copy link → exact URL format; open that link signed out → read-only document from the downloaded PDF (as 5.18.0); an export made by the owner afterwards appears in the signed-out view within the 2.5 s poll; no request carries `key=`. |
| 08 | `08-save-reopen` | Write, Home, clear local DB, reopen from Drive → identical snapshot; uploaded PDF has fence + `IHN6`, image attachments, no source-PDF attachment, `/IHN6Ink` streams; re-import = same snapshot; a PDF-background page's source rebuilt from the export renders identically; 412 with unchanged head (metadata churn) re-PUTs the same bytes, 412 with a new head merges; **Export to the user**: modal name → forced export → download (web `<a download>` name via `sanitizeFilename`), app mode bridge v2 (`beginPdfExport`, chunks ≤ 240,000 chars, `finishPdfExport`, `handleInhousePdfExportResult`), `navigator.share` path. |
| 09 | `09-offline` | `context.setOffline(true)`, write on two pages, Home shows `Saved offline`; create a new document offline (reserved id, `Created on this device`), write in it; reconnect → relay sealed, the new document created in Drive with its reserved id, badges gone, indicator `saved`. |
| 10 | `10-exit-upload` | Write then close the page immediately → the keepalive relay flush reached the mock (edits sealed); a `?inhouse_app=1&offline-worker=1` headless page with the `InhouseBackgroundChannel` stub completes the export and reports `complete({busy:false,pending:0,errors:[]})`; a fresh context opens the result. The service worker registers no `sync`/`periodicsync` (N-5). |
| 11 | `11-two-devices` | Sub-tests (SYNC §19.3): (a) two contexts, P2P: concurrent pen/erase/page moves → identical state, latency samples (§4); (b) P2P blocked + a signed-in viewer context of another account: relay converges, viewer sees it and never writes or connects; (c) B offline, both edit, B back → converge; B goes Home → export; fresh context opens it; (d) two tabs of one context: tab 2 shows tab 1's stroke at once; closing the `ihn6-net` holder → tab 2 takes over polling and the relay; (e) rename from Home in B while A has the document open → A shows the new title; no export reverts it. Presence avatars show `p2p-connecting` → `p2p-connected`. |
| 12 | `12-open-v5` | Open every 5.x Drive fixture (§5) → page count, strokes, images, backgrounds, timeline entries equal 5.18.0's; a newer 5.x revision written after a v6 export is merged, not overwritten; a plain (non-Inhouse) PDF opens as PDF-background pages; a foreign plain PDF written over a v6 document produces the `(recovered Drive version)` sibling and keeps the v6 state; a 65 MiB PDF shows `This PDF is larger than the 64 MB safety limit.` |
| 13 | `13-migrate` | Seed 5.18.0 storage fixtures, boot v6 → every unsynced stroke present and sealed in the relay (`local-` docs created with reserved ids); 5.x DBs deleted only after every migration batch is sealed and no 5.x window exists; a 5.18.0 page kept open in a second tab writes more strokes after migration → they are migrated on the re-dump before deletion; kill mid-migration and restart → same result. |
| 14 | `14-updater` | `?inhouse_app=1` + `InhouseNative` stub: offer `1.0.28 → 1.0.29`, Update calls `installInhouseUpdate(url, sha)`, progress 50 %; release watcher reloads once for a newer `VERSION`, never with the editor open. **Upgrade from 5.18.0**: a context with the 5.18.0 service worker and its cache installed, then the server switches to the v6 tree → the open 5.18.0 page reloads into v6 at its next Home/visibility check, the registration's `scriptURL` is still `/service-worker.js`, and `v=5.18.0` assets requested by the old page before reload are still served. |
| 15 | `15-soon` (+`15b-soon-scan`) | Diary, Agenda and Scan show the Soon toast for 1.8 s; no camera permission request, no network request, no scanner asset loaded. |
| 16 | `16-timeline` | Edits over two simulated 5-min windows → entries coalesced as 5.18.0 (captured by the editing device, author = itself); restore with slider → pre-restore milestone + restored content; imported 5.x history listed; a second context sees the same entries. |

---

## 3. Pixel-parity harness (`npm run test:parity`) — the release gate

### 3.1 Setup

- **Two servers side by side**: `node tests/v6/server.mjs --root <worktree of 1d6e8a0> --port 4301` (5.18.0) and
  `--root . --port 4302` (v6). Different origins, so storage never mixes. Both get the **same** mock (`tests/v6/mock`)
  with the same fixture state per scripted state, reset before each state.
- **One script per state, run against both**: a state is `{name, setup(ctx), steps(page), snapshot}` where steps use
  only user-level actions (clicks, keys, `page.mouse`/`page.touchscreen`, CDP `Input.dispatchMouseEvent` with
  `pointerType:'pen'` for pens, file choosers) plus storage seeding before load. No app internals, so the same
  script drives both apps.
- **Scene loader (no internals):** editor and canvas states start by opening a fixture document through the shared
  link URL `/?drive=<fixture id>&e2e=1` with the mock serving that fixture PDF (5.x PDFs for 5.18.0 and v6 alike) and
  the mock account signed in. So every canvas scene needs only the open-from-Drive path (W3.1 delivers it first,
  BUILD-PLAN), not tools.
- The v6 server runs with `--overlay build/` (generated files, ARCHITECTURE §14.2).
- **Determinism init script** (`determinism-init.js`, added with `context.addInitScript` to both):
  `page.clock.install({time: 2026-10-30T10:00:00Z})` and explicit `clock.runFor` in steps; `Math.random` and
  `crypto.getRandomValues`/`randomUUID` replaced by a seeded xorshift (same seed for both); `performance.now`
  driven by the Playwright clock; `requestIdleCallback` deterministic; fonts are the vendored `ui.css` files
  (await `document.fonts.ready`); Google Fonts and every external image route return fixture bytes;
  `navigator.onLine`, `matchMedia('(prefers-color-scheme)')`, `prefers-reduced-motion: no-preference` fixed.
- **Paused instants**: before capture, `document.getAnimations()` are paused and set to the state's `atMs`
  (default: finished), CSS transitions likewise via `getAnimations()`; caret hidden (`caret-color: transparent`
  injected in both); the snapshot waits for "settled": no pending fetch in the mock, two rAFs, and
  `__IHN_TEST_API__.ready()` (v6) / `__IHN_TEST_API__.ready()` (5.18.0).
- **Chromium flags**: the same pinned Playwright Chromium for both apps, `--disable-gpu` (software raster),
  `--force-color-profile=srgb`, `--font-render-hinting=none`, `--disable-lcd-text`; W1.2 fixes the final flag set by
  proving that 5.18.0 compared against itself is stable over repeated runs.

### 3.2 What is compared per state

1. **Screenshot — the gate**: full viewport PNG, pixel tolerance **0**, compared byte-wise on decoded RGBA.
2. **DOM — diagnostic**: `document.documentElement.outerHTML` after normalization — attribute order sorted; every
   `style` attribute re-serialised through a scratch element's CSSOM (`el.style.cssText`) so literal template styles
   and `el.style.*` assignments compare equal; every `<script>` element and `#inhouse-native-assets` removed;
   `<link>`/`<meta>` with `?v=`/`native=` queries stripped of the query; `blob:` URLs → `blob:<n>` in order of
   appearance; `data:image/*` URLs → their SHA-256; values of `data-page-id` and `data-version-id` → `id<n>` in order
   of first appearance (N-11). Any other id-like attribute found by W6.1 is added here with a BEHAVIOUR-CHANGES entry.
3. **Canvas call trace — diagnostic** (§3.4) for every canvas in the document and every offscreen canvas.

A state fails only on a screenshot difference, unless the state is tagged with a BEHAVIOUR-CHANGES id whose
expected-diff rule matches (e.g. B-E1 only for states with selection on non-A4 pages). The DOM and trace diffs of a
failing state are attached to its report entry to locate the cause; a state whose pixels match but whose DOM/trace
differ is listed as a warning for W6.1 to look at, not a failure.

### 3.3 Matrix

| Axis | Values |
|---|---|
| Viewport (CSS px) × DPR | phone 360×800 @3, phone 412×915 @2.625, tablet 800×1280 @2 (UA SM-X700, no "Mobile"), desktop 1280×800 @1, desktop 1920×1080 @2 |
| Theme | light, dark (`inhouse-theme` seeded), plus system-dark with nothing stored for Home and Welcome |
| Mode | web; Android app (`?inhouse_app=1`, UA `… InhouseNotesApp/1.0.28`, `InhouseNative` stub) |
| States | every row below |

Full product: ~140 states × 5 sizes × 2 themes ≈ 1,400 captures (+ app-mode subset ≈ 300). On 4 cores with 4
workers ≈ 25 min. **Who runs what:** each unit runs `--quick --states <its files>` (desktop 1280×800 + phone 360×800,
light, web) and fixes its own diffs before merging; the full matrix runs nightly on `v6`, in W6.1 and on the release
PR (ARCHITECTURE §14.2). Sweep states that cross time wait until both apps have settled (5.18.0 upload finished; v6
relay sealed and export done) before capturing (N-1).

| Area (state file) | Scripted states |
|---|---|
| welcome | first paint (boot skeleton off), welcome, welcome sheet open, system-dark |
| home-shell / auth | boot skeletons (signed-in destination), Home empty, signed in with lists, reconnect state, offline line (connectivity probe failed, `ihn-network-offline`), `Sign-in failed. Try again.` |
| profile-menu | Home menu (incl. Refresh, fullscreen button on web), night toggle on, Soon toast visible (t = 0.2 s, 1.0 s), updater settings entry (app mode) — W1.3; editor menu with doc stats — W3.1 (`open.mjs`) |
| updater | offer, details expanded, settings, switch-to-A warning, slider half, progress 50 %, result installed/cancelled |
| home | each tab, long lists (fast-scroll thumb), folder cards for 12 palette colours, opened folder (deep breadcrumbs), folder zoom at 0/25/50/75/100 % forwards and backwards, pull-to-refresh at 0/40/80 px and refreshing, search results, no results, offline badges/pending, skeletons |
| home-dialogs | create menu, folder picker (nested), new folder, card ⋮ menu, colour popup, move-to-bin slider 0/50/100 %, Bin list + menu, download-app modal (web), offline settings dialog with recoveries and queue line (`N pending · …` / `Up to date`), offline sync-error dialog |
| open | loading overlay (`Loading PDF...`, home-save transition at 0/180/360 ms), template doc, PDF-background doc (pdf.js render), photo page, agenda page, diary page, legacy cover strokes doc, viewer/read-only banner |
| editor-view | empty doc, 2 pages at zoom 0.5/1/2/3, panned edge clamps, pull-to-add indicator at thresholds, read/edit toggle, fast scroll, toolbar positions (snap left/right/top/bottom) |
| editor-tools | each tool selected + menus (pen palette, highlighter, eraser menu), `Draw with finger` toggle, live pen paused mid-stroke (frames 1, 5, 20), at lift, highlighter live and committed, stroke/area eraser mid-gesture with cursor, lasso drawing, selection box (move/scale/rotate handles), rotated selection and image (jsmath), cross-page drag, cross-page drag of an old stroke onto a page with a newer highlighter (draws on top), images inserted/selected, undo/redo button states |
| canvas | dense page (2,400 strokes) sync and staged repaint (final), damage repaint after erase, thumbnails (page preview, scroll preview, pages panel), template backgrounds at all scales |
| pages | panel open, list with thumbnails, drag reorder mid-drag, delete slider, photo modal (both modes, preview), Scan Soon |
| share | modal empty, people list (owner/writer/reader, avatars), role menu, link section on/off, copy confirmation, public-link viewer, sign-in gate |
| timeline | panel list (months/clusters expanded/collapsed), preview with added/removed highlights, restore slider, `Original` entry |
| presence | avatars `p2p-connecting` / `p2p-connected` / relay-only, remote save rings, popup (`Actividad del documento`) — mapping in SYNC §9.1 |
| export | export modal, export loading texts, save indicator states (`paused/building/uploading/syncing/saving/saved/error`, driven through the mock: offline, slow relay PATCH, slow export, failing upload) and save details popup (N-2 lines) |
| sweep | cross-area sequences (open doc → Home → reopen; theme switch with editor open; resize phone↔desktop with menus open) |

### 3.4 Proving ink/template/PDF drawing parity per Canvas call

`trace-init.js` (init script, both apps) wraps every method and property setter of `CanvasRenderingContext2D`,
`OffscreenCanvasRenderingContext2D`, `Path2D`, `CanvasGradient`, and `HTMLCanvasElement` size setters/`toDataURL`/
`toBlob`. Each surface gets an id by creation order (`c0`, `c1`, …; Path2D `p0`…; gradients `g0`…). Every call is
logged as `[surface, name, args]` with numbers serialized as their IEEE-754 bit patterns (exact), strings verbatim,
image arguments as `{surface or image src hash, naturalWidth, naturalHeight}`. Normalization: `stroke(path)` is
expanded to the path's recorded construction ops, so cached `Path2D` reuse and fresh paths compare equal only if the
geometry is bit-identical; yields between staged-repaint slices are ignored (the per-surface op sequence must match,
not its timing).

- **Ink**: each canvas scene runs the same input (recorded pen sample streams in `tests/v6/fixtures/ink/*.json`, fed
  through CDP with identical timestamps) in both apps; the traces of `page-canvas`, `cursor-overlay`,
  `lasso-overlay` and offscreen surfaces must be identical for live frames, lift and full repaint.
- **Templates**: bullet template at every backing scale used (2, 2.25, …, 6) and the agenda/diary SVG `drawImage`
  calls; template CSS JPEG data URLs compared by hash.
- **PDF backgrounds**: pdf.js draws into its own canvas in both apps; the trace of the pdf.js render canvas and the
  resulting JPEG blob hash must match (same pdf.js build, same scale and intent prove it); the page canvas trace
  shows identical `drawImage`/CSS background use.
- A trace match plus software rasterization implies identical pixels; the screenshot comparison is still applied as
  the final word.

### 3.5 Expected differences

Only BEHAVIOUR-CHANGES entries. Each entry lists the affected states; the harness requires those states to differ
**only** where the rule says (e.g. B-E1: the selection box geometry on A5/PDF pages) and reports them separately in
`REPORT.md` for the user's review.

---

## 4. Two-device instant sync test (`11-two-devices`, plus `npm run perf:sync`)

- Two Playwright contexts A and B (separate storage, same mocked Google account; a third context with another account
  for the cross-account case), same document, real WebRTC between them (Chromium loopback), Drive relay through the
  mock with configurable latency (0/100/300 ms).
- Script: 200 strokes on A at human pace (120 ms gaps), 50 on B concurrently, area erases on both, a page move on A
  while B draws on that page, B offline for 20 s then back.
- Assertions: final snapshots byte-identical; no stroke lost or duplicated; viewer context never emits ops.
- Latency: time from A's `pointerup` to B's first trace op drawing that stroke id (instrumented through the canvas
  trace timestamps on a shared monotonic clock). Gates (SYNC §1, one number each): **P2P p95 < 150 ms** (in-progress
  stroke visible on B within one frame + RTT); **relay-only p95 < 4 s** with 100 ms mock latency per request.

---

## 5. Migration test and 5.x fixtures

Fixtures are produced by **driving 5.18.0 itself** (`tests/v6/fixtures/make-v5-fixtures.mjs`, W1.2), never
hand-written, and committed with a record of what 5.18.0 reported after import (`*.expect.json`):

- Drive PDFs: a plain non-Inhouse PDF (2 pages, one rotated); two revisions R0/R1 of the same 5.x document (R1
  deletes strokes and changes a page property without stamps); a > 64 MiB PDF (generated on the fly, not committed);
  template doc; imported PDF with overlays (pdf-lib path, `IH_OVERLAY_REFS`); structure-changed doc with
  `IH_CLEAN_ORIGINAL.pdf`; photo page; inline images; agenda and diary template pages with side-panel data; legacy
  document without `IH_NORM` (duplicate ids); `STROKES_DATA` variant; raster-fallback (jsPDF) doc with cover strokes;
  a doc with a 30-entry `IH_VERSIONS`; a dense 20-page doc (profile document).
- Canvas traces: W1.2 records the 5.18.0 canvas trace of every `canvas` scene (`tests/v6/fixtures/traces/`), used by
  W2.3's cargo trace-golden tests before the v6 editor exists.
- Local storage dumps (IndexedDB + localStorage, captured through CDP from a 5.18.0 session): dirty offline doc with
  outbox row; `local-` doc never uploaded; pending local folder; working cache with uncompacted `stroke-ops` newer
  than the offline snapshot; pending-exit marker; recoveries; `background-` DB variant; xor-v1 packed pages.

`13-migrate` restores each dump into a fresh v6 context (via CDP IndexedDB + `addInitScript` for localStorage), boots
v6, and asserts every stroke id from the dump's `expect.json` exists in the v6 state and in the sealed relay files,
that nothing is uploaded twice, and that 5.x databases are removed only after every migration batch is sealed and no
5.x window is open. A second run kills the page after the first document migrated and checks idempotent completion. A
third run keeps a 5.18.0 page open in another tab that keeps writing after the migration; its strokes must appear in
v6 before the 5.x databases are deleted.

---

## 6. Performance check (`npm run perf`, report only; gates in W6.2)

`tests/v6/perf/profile.mjs` is `scratchpad/prof/profile.mjs` generalized: `--app v5|v6 --root <tree>`, same faithful
Drive mock, same 20-page/120-strokes-per-page document (opened from the same fixture PDF), same phases
(`editor-idle` 60 s, `writing` 15 s of CDP pen strokes, `after-writing` 30 s, `home-idle` 60 s), at CPU throttle 1× and
4×. Reported per phase: requests/min, KB/min, CPU %, script %, layout ms, JS heap MB (+ wasm memory for v6), long
tasks, frame p95/max/janky, input-ack p50/p95/max. Additions for v6:

- memory: JS heap + `WebAssembly.Memory` `buffer.byteLength` of the main and export-worker instances, also after
  opening the largest 5.x fixture;
- a remote 20k-op batch delivered while the pen is down (no long task, frame budget kept);

- boot: `performance.mark` around wasm compile+instantiate, time to Home painted from cache, critical-path bytes;
- pen: per-event script time (CDP `Profiler` sampling or `performance.measure` in the e2e build), steady-state
  allocations (allocator counter exported in the e2e build only);
- `writing` with a second device connected (sync cost on the writer);
- battery proxy: total CPU seconds and network wake-ups per minute in `editor-idle` and `home-idle`.

The report (`tests/v6/perf/REPORT.md`) puts 5.18.0 and v6 side by side and checks ARCHITECTURE §14; W6.2 fails the
release if v6 is not lighter than 5.17.22/5.18.0 on CPU, network and memory in every phase.
