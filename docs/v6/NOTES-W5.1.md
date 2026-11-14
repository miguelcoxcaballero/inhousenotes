# W5.1 PAGES — notes for the other units

Branch `v6-w5_1` (from `v6-int` at `5296f62`). Owns `rust/crates/ihn-app/src/panels/{pages_panel,photo_page,slider}.rs`,
`rust/crates/ihn-app/src/doc/import.rs`, `tests/v6/e2e/{05-pages,15b-soon-scan}.spec.mjs`,
`tests/v6/parity/states/pages.mjs`. Also touched: `editor/pages.rs` (two accessors, below).

## What exists now

- `panels::slider` — 5.18.0 `showConfirmSlider`: `slider::show(app, host, title, message, text, done)` where
  `done: fn(&mut App, &mut dyn Host, bool)` receives the answer (a second `show` answers the pending one `false`).
  Release within 2 px of the end → `.complete`, answer `true` after 120 ms; Cancel / backdrop → `false`.
  **W5.5 (bin) / others:** use it instead of a modal of your own when 5.18.0 used `showConfirmSlider`.
- `panels::pages_panel` — `open` / `close` / `refresh` / `is_open` (`#btn-edit-pages`, `#panel-close`,
  `#panel-overlay`, Read mode, session closed), the default-size line, the list (5.18.0 markup, thumbnails through
  `pages::panel_thumb`, retried while a background/image decodes), click → `viewport::go_to_page` + close, drag
  reorder (rows measured once, one rAF tick: edge auto-scroll, ghost, insertion line, Escape/blur/resize cancel,
  180 ms row animation unless reduced motion), delete through the slider, Photo → `photo_page::open`, Scan →
  `shell::soon`, Import → `doc::import::pick`. Helpers `size_info`, `size_line`, `contained`, `base_size` (5.18.0
  `getPageSizeInfo`, `getContainedRect`, `getDocumentBasePageDimensions`) are public.
  - `move_page(app, host, page, target, before)` and `delete_page(app, host, id)`: one `session::edit` batch each
    (`page_move` / `page_delete`); as in 5.18.0 they are not undo steps.
- `panels::photo_page` — the cover modal (file chooser, drop, Enter/Space, both radios, Escape, backdrop, busy
  state, preview sizes, meta and labels) and the asset: `min(3, 3840/max side, photo/image)` canvas on white →
  JPEG 0.94, or the PNG kept byte for byte when it fills the page; SHA-256 → `session::put_asset` →
  `page_new(after current, Default, size, Background::Custom)`; history cleared (5.18.0).
- `doc::import` — Import PDF: `#pdf-input` → pdf.js open → page sizes + SHA-256 (+ inflate for a v6 container) →
  `v5::import::from_pdf` (lenient, `strict: false`) or the v6 snapshot → a scratch `Doc` → **one native batch**:
  every old page deleted, every imported page/item recreated with this device's ids (collaborators get it as a
  normal edit). Assets: inline images, the source PDF (the file, or `IH_CLEAN_ORIGINAL.pdf`), v6 image attachments.
  Status `PDF imported: N pages, K strokes restored` / `PDF imported: N pages (as background)` / `Error importing
  PDF`; history cleared; view centred on the top. Test command `doc.import.state` → `{busy, log}`.
- `editor/pages.rs` (W3.2's): `pub fn active_index` (5.18.0 `state.activePageIndex`) and `pub fn panel_thumb`
  (5.18.0 `generateThumbnail` on a Pages-panel canvas through `ihn-core::render::thumb::panel`).
- Test commands: `panels.pages_panel.state` `{open, items, current, dragging, moved, deleted, order}`,
  `panels.pages_panel.labels {w,h}`, `panels.photo_page.state`, `panels.slider.state`.

**Listener ids:** `host/dom.js` keeps one listener per id (a second `listen` with the same id replaces the first) —
every module needs a distinct id per registration.

## Verified

- `cargo test --workspace` green (ihn-app 148, 3 new: labels/contained rects, photo layouts, slider answers).
- e2e `05-pages` (2 tests: pull-to-add, panel, thumbnails, drag reorder, slider spring-back and delete, cancel, Read
  mode; photo page Document + Photo ratio, non-image refused, Escape, Import PDF plain → PDF pages with sources,
  Import 5.x `r0` → its pages and strokes, undo cleared) and `15b-soon-scan` (390 and 1280 px) pass.
- Parity `--quick --states pages` (desktop + phone): panel read/edit, drag mid-drag, slider 0/50/100 %, photo modal
  empty / document / photo ratio and Scan Soon are pixel-equal. Remaining cells:
  - the save-status dot after an edit (`deleted`, `added`, `dropped` phone): 5.18.0 reaches `saved` through its
    Drive upload; v6 stays "Waiting to save" until relay/export exist (W4.5/W5.4) — same as NOTES-W3.4 item 1;
  - `photo-modal@added` phone: 1-device-px lines at page edges — 5.18.0 keeps the preview as the wrapper's CSS
    background of *active* pages after `renderAllPages`, v6 (`editor/pages.rs` `rebuild`) clears it once the canvas
    painted; edge antialiasing differs. W3.2's rule, recorded for W6.1;
  - the known 3-px share-button flake on phones.

## Deviations

1. **No "next export" check in smoke 05**: export is W5.4's; Import PDF is checked in the model (pages,
   backgrounds, source assets, strokes).
2. **Stripped 5.x sources**: when `from_pdf` asks for the pdf-lib-stripped source and the file has no
   `IH_CLEAN_ORIGINAL.pdf`, the import stores the file itself (5.18.0's last fallback) instead of running the
   export worker's strip; overlay rasters of such files can show under the vector ink on PDF-background pages.
3. **v6 container import**: images come from the `IHN6-<hash>` attachments; PDF-background sources are left to
   `doc::backgrounds::take_missing` (rebuilt from exports by W5.4).
4. **Over 64 MB**: 5.18.0 throws before any UI (an unhandled rejection, nothing shown); v6 shows `This PDF is larger
   than the 64 MB safety limit.` as an error status.
5. **No structure lock**: 5.18.0's `acquireLocalPageStructureMutation` waits for page-store writes and merges; v6
   edits are single batches applied atomically, so there is nothing to wait for.
6. `imageSmoothingQuality = 'high'` of the photo raster has no canvas op; the default quality is used.
7. Parity scripts toggle Edit with `element.click()` on phones (the floating toggle group covers the button in both
   apps, so a pointer click misses it).
8. Commit trailer names the model given by the workflow.
