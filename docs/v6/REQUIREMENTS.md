# Inhouse Notes 6.0.0 — requirements

Set by the user. Every design decision, work unit and review must satisfy these.

Reference behaviour: 5.18.0, the cleaned-up 5.x release (scanner, agenda and diary replaced by "Soon",
legacy compatibility and dead code removed). The specs in docs/v6/spec/ describe 5.17.22; where they
cover the scanner, agenda/diary/calendar, or code that 5.18.0 deletes, they are out of scope.

## 1. Looks and works like v5 — as close as possible, not pixel-perfect
(Revised by the user on 2026-10-29: "lo más parecida posible pero no pixel perfect si no es necesario".)
- v6 must look and behave like the current app (v5.17.x) in every screen, state, theme and size:
  Home (tabs, folders, cards, menus, dialogs, pull-to-refresh, empty/loading/offline states),
  welcome/sign-in, the editor (toolbar, tools, menus, popups, page panels, manage-pages, timeline,
  share dialog, presence avatars, save indicators, status bar, toasts, loading overlays), updater UI,
  light and dark themes, phone/tablet/desktop widths, Android WebView. A user should not notice a difference.
- Same layout, colours, fonts, icons, texts and animations (timings and curves, e.g. the manila folder zoom).
- Ink, templates (including existing agenda/diary page backgrounds), images and PDF backgrounds look the same: same pen geometry,
  widths, colours, highlighter blending. Pixel-exact output is NOT required; take the simplest approach
  that is visually indistinguishable.
- How: keep the existing HTML markup, CSS, fonts and assets as the UI definition (reuse them, do not
  redesign or restyle); Rust drives that DOM. Draw with browser Canvas 2D like v5 rather than a custom
  rasterizer. Visual check = screenshots of key screens v5 vs v6 compared with a small tolerance, plus a
  side-by-side review of the differences; no pixel-for-pixel gate.
- Functions and flows identical, except what §1b removes.

## 1b. Removed in 6.0.0, shown as "Soon": scanner, agenda, diary
(Decided by the user on 2026-10-29: "quita lo del escáner y pon un cartel de 'soon'" and "elimina también
la agenda y el diario y sus funciones y ponlo en soon también".)
- Scanner: no camera, scanner iframe, workers or kernels; nothing in `scanner/` is loaded. The "Scan pages"
  button (`#btn-scan-page`, Manage-Pages panel) stays in place with a small "Soon" badge; tapping it only
  shows a "Soon" notice in the app's existing toast style. Photo pages from the gallery keep working.
- Agenda and diary: no Agenda/Diary quick access, no creating agenda/diary/academic-year documents, no
  Google Calendar (API calls, side panels, month/day modals, event editor, 5 s sync, "Add to pages",
  calendarPageConfig/IH_CAL handling). Their entry points stay in place with a "Soon" badge and show the
  same "Soon" notice. Existing documents whose pages use the agenda/diary template still open and show the
  same page background (the SVG drawn behind the ink) so no user document looks broken; calendar side
  panels are simply not shown. New pages added to such a document use the default template.
- Existing documents with scanned pages (images + strokes) open, render and sync normally.

## 2. Network documents that sync instantly — by design
- Every edit is an operation applied locally at once and propagated to other devices immediately
  (direct peer channel when available, durable Drive-backed relay otherwise). No full-document uploads
  on the edit path; the PDF in Drive is a derived export.
- Conflict-free merge of concurrent edits; no data loss; works offline and converges after reconnect.

## 3. Built from the ground up in Rust, as small, fast and light as possible
- Application logic, document model, sync, storage logic and rendering decisions in Rust/WebAssembly.
  JavaScript only as minimal glue where the platform requires it.
- Lighter than 5.17.22 on CPU, battery, network and memory; minimal input-to-ink latency.
- As little code as possible: no dead code, no unused features, no compatibility layers beyond §4.

## 4. Compatibility — user data yes, old app versions no
(Revised by the user on 2026-10-29: remove old compatibility code and everything unused; reduce code as much as possible.)
- Every document the user already has in Drive opens without loss (all PDF/stroke formats that can exist in
  Drive files), and unsynced local data from 5.x (IndexedDB documents, offline outbox, pending exit uploads,
  settings) is migrated once, automatically and safely. Nothing else from older versions is kept: no
  legacy migrations of stores older than 5.x, no old cache/key names, no old protocol versions.
- No live interop with devices still running 5.x: the APK loads the website, so every device moves to 6.0.0
  together; a 5.x client that is still open is told to reload. If a 5.x device saved a document in Drive
  after v6 last saw it, v6 imports that newer version instead of overwriting it.
- Same Android APK host and updater; the frozen Android installer is never modified.

## 5. Release and tests — lean
(Revised by the user on 2026-10-29: "reduce el número de tests".)
- Version 6.0.0 (APP_VERSION and every [data-app-version] label).
- Do NOT port the ~140 v5 test files. A lean, fast suite instead:
  - `cargo test` unit/property tests for the core: document ops and merge, sync protocol, 5.x format
    readers (Drive PDF strokes, IndexedDB records), migration.
  - About 15 browser end-to-end smoke tests covering the main flows: sign-in/boot, Home browsing and
    folders, create/open/rename/delete documents, write/erase/select/undo, pages (add/move/delete,
    photo page), share, save to Drive and reopen, offline then reconnect, exit upload,
    two devices editing the same document with instant sync, opening documents saved by 5.x,
    migration of 5.x local data, updater/version label, the "Soon" entry points.
  - Screenshots of key screens v5 vs v6 (both themes, phone and desktop width) with tolerance (§1).
  - The existing profiler to show v6 is lighter than 5.17.22.
- Released to main only after that suite is green and the user has seen the behaviour changes.

## 6. Known v5 bugs — policy
The specs (docs/v6/spec/*.md, "quirks" sections) found defects in v5. Policy: keep every intended look and
behaviour identical, but do not reproduce evident bugs. Each fixed bug is recorded in
docs/v6/BEHAVIOUR-CHANGES.md (what v5 did, what v6 does, why) and shown to the user before release; the
screenshot comparison lists them. Intentional-looking oddities are kept
unless the user decides otherwise. Collected so far:
- Public share links never download the PDF (fetchWithDeadline argument mix-up), so signed-out
  recipients always get the embedded Drive preview, recreated every ~7.5 s (spec/home.md §8.3, §12).
- The signalling key property `ihn_live_key_v1` matches the `ihn_live_` preview-hint prefix and is fetched
  as a preview file, a 404 every 3 s (spec/collaboration.md §16).
- Editor (spec/editor.md §11 Q1–Q10): selection move / image placement / cross-page drag assume A4 page
  size; a single-tap pen dot vanishes on the next repaint; undoing an area erase restores strokes on top
  of later ink; others listed there.
- Security: the live signalling key is readable by viewers through file properties and peers are not
  checked for edit rights (spec/collaboration.md §8) — v6 must not keep this.
