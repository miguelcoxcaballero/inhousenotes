# Inhouse Notes 6.0.0 — non-negotiable requirements

Set by the user. Every design decision, work unit and review must satisfy these.

## 1. Visually identical — exactly
- v6 must look **exactly** like the current app (v5.17.x) in every screen, state, theme and size:
  Home (all tabs, folders, cards, menus, dialogs, pull-to-refresh, empty/loading/offline states),
  welcome/sign-in, the editor (toolbar, tools, menus, popups, page panels, manage-pages, timeline,
  share dialog, presence avatars, save indicators, status bar, toasts, loading overlays), the scanner,
  updater UI, light and dark themes, phone/tablet/desktop widths, Android WebView.
- Same animations with the same timings and curves (e.g. manila folder zoom, menus, spinners).
- Ink, templates, agenda/calendar pages, images and PDF backgrounds must render **pixel-identically**:
  same geometry (current pen spline, widths, caps/joins, highlighter blending), same colours, same fonts.
- How to guarantee it:
  - Keep the existing HTML markup, CSS, fonts and assets as the UI definition (byte-for-byte unless a
    change is invisible); Rust drives that same DOM. No redesign, no "equivalent" restyling.
  - Draw with the same browser Canvas 2D operations in the same order and with the same parameters as
    v5 so antialiasing and blending are identical (do not substitute a custom rasterizer for visible ink
    unless it is proven pixel-identical).
  - A screenshot parity suite is a release gate: the same scripted states rendered by v5 and v6 at
    several viewport sizes, DPRs and both themes must match pixel-for-pixel (any intentional exception
    listed and approved by the user). Live animations compared at fixed paused instants.
- Functions and behaviour also identical: every feature of v5 works the same way, with the same flows.

## 2. Network documents that sync instantly — by design
- Every edit is an operation applied locally at once and propagated to other devices immediately
  (direct peer channel when available, durable Drive-backed relay otherwise). No full-document uploads
  on the edit path; the PDF in Drive is a derived export.
- Conflict-free merge of concurrent edits; no data loss; works offline and converges after reconnect.

## 3. Built from the ground up in Rust, as fast and light as possible
- Application logic, document model, sync, storage logic and rendering decisions in Rust/WebAssembly.
  JavaScript only as minimal glue where the platform requires it.
- Lighter than 5.17.22 on CPU, battery, network and memory; minimal input-to-ink latency.

## 4. Compatibility
- Opens every existing user document and local data (Drive PDFs with embedded strokes, IndexedDB,
  offline outbox, settings) without loss; migration is automatic and safe.
- Interoperates with devices still on v5 during the transition.
- Same Android APK host and updater; the frozen Android installer is never modified.

## 5. Release
- Version 6.0.0 (APP_VERSION and every [data-app-version] label), released to main only after it
  passes the full v5 test batteries adapted to v6, the screenshot parity gate and two-device sync tests.

## 6. Known v5 bugs — policy
The specs (docs/v6/spec/*.md, "quirks" sections) found defects in v5. Policy: keep every intended look and
behaviour identical, but do not reproduce evident bugs. Each fixed bug is recorded in
docs/v6/BEHAVIOUR-CHANGES.md (what v5 did, what v6 does, why) and shown to the user before release; the
screenshot parity gate lists them as the only allowed differences. Intentional-looking oddities are kept
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
