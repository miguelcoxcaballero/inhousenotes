# W3.4 V5-OPEN — notes for the other units

Branch `v6-w3_4` (from `v6-int` at `58039fc`). Owns `rust/crates/ihn-app/src/doc/v5open.rs`, `host/export-worker.js`,
`tests/v6/e2e/12-open-v5.spec.mjs`. Also touched: `home/mod.rs` (signal codes, below).

## What exists now

- `doc::v5open` claims `session::sig::FOREIGN` (W3.1's seam) for every PDF that is not a v6 container:
  1. Plain PDF and the session already holds pages → **foreign overwrite** (SYNC §15.1): the v6 state is kept,
     the revision is recorded as a `Recovery {doc_id, name: "<name> (recovered Drive version).pdf", head, blob,
     size}` and the open finishes. **W5.4: call `v5open::take_recovery(app, doc_id)` before the next export and
     write the copy next to the file** (you own the blob). Memory only: a reload before the export loses it (the
     revision is still in Drive's history).
  2. Otherwise: page sizes (`pdf::pages`, `jsmath::round`) and SHA-256 of the bytes (`fx::digest`) → `PdfFacts`
     (`strict: true`, name = Drive name without `.pdf`), import actor `ActorId::import(headRevisionId)` (no
     revision id, as for a signed-out public download: the hex SHA-256), `rev_time_ms` = `modifiedTime` of the same
     `files.get` → `v5::import::from_pdf`.
     - Keywords > 256 KiB → in the **export worker** (import role, `worker_role::IMPORT = 0`, op 1): it streams one
       message per batch and per asset, then an end record (source, v5 flag, worker memory). A worker over 64 MiB is
       terminated after the job. If the worker dies, the parse runs on the main instance.
     - Otherwise on the main instance (plain PDFs and the usual 5.x documents are a few KiB of Keywords).
  3. Batches are kept encoded until the import is complete; then `supersede(doc, skeletons, &[])` is computed
     against the document **as it was** (it must be before applying: an older revision arriving after a newer one
     needs the old state), then the batches go through `Replica::receive(PeerRef::Import)` one by one and each is
     freed. Inline image assets → `blob_new` + `session::put_asset`.
  4. The source PDF of `Background::Pdf` pages, by `Import::source`: `Saved` = the downloaded blob;
     `Attachment` = `IH_CLEAN_ORIGINAL.pdf` via `pdf::attachment`; `Stripped` = pdf-lib in the worker (op 2, port
     of `stripOverlaysFromPdfBytes`). Failures fall back Stripped → Attachment (if present) → Saved, as 5.18.0's
     recovery order. Then `open::finish_foreign(true)` (records the head as known, shows the document).
- Errors (strict 5.x metadata, page count, failed pdf.js/digest) end the open with 5.18.0's
  `Could not open this document. Try again.` (5.18.0 shows that text for every Drive open failure, the 64 MB limit
  included; the specific text belongs to "Import PDF", `doc/import.rs`).
- Later 5.x revisions: `doc::open` revalidates the local copy, downloads an unknown head and hands it here again;
  the new import actor + `supersede` merge it (R0 → R1: R1's deletions apply).
- `host/export-worker.js`: classic worker; instantiates the module host/main.js sends (`{type:'init', module,
  version}`) with stub imports, `ihn_init` with `mode::WORKER`, forwards `{bytes}` to `ihn_worker` and posts its
  `WORKER_POST` records back. Role 0 op 2 (strip) is pure JS. **W5.4 adds the export role in the same file**;
  `v5open` starts the worker itself (`fx::WORKER_START` once, `State::worker`) — when W5.4 starts it too, share one
  flag (a second `WORKER_START` terminates the running worker).
- Test commands `doc.v5open.{state, doc, legacy}`: outcome log (`worker`, `v5:N`/`plain:N`, `source:<variant>`,
  `recovery`, `error:<msg>`), recoveries, main-instance memory marks; the open document in order (ids, sizes,
  template, background, asset stored, stroke ids in draw order, images, cover strokes); `Id::legacy` of 5.x ids.

## Signal codes (lead's request)

`home::sig::OPEN_FILE/SYNC_ERROR/RETRY_SYNC` moved from `0x40..0x42` (collided with `session::sig::OPENED/CHANGED/
COMMITTED`) to **`0x30..0x32`** (free: shell uses `0x20..0x2a`, session `0x40..0x46`, editor view `0x60`).

## Verified

- `cargo test --locked --workspace` green (ihn-app 123, incl. 5 new: worker protocol round trips, streaming,
  error answer, skeletons, names).
- e2e `12-open-v5` (5 tests, 33 s): every one of the 15 fixtures equals its `expect.json` (name, page count, page ids,
  sizes, templates, backgrounds/`pdfPageIndex`, source asset stored, stroke ids in draw order, images, covers;
  `legacy-no-norm` and `plain` by counts: 5.18.0's ids there are random); `imported-overlays`/`photo-page` show the
  pdf-lib-stripped original; dense-20 parsed in the worker; R0 then R1 merged with R1's deletions; a plain PDF over
  an open document → kept + recovery record; a 65 MiB PDF fails with 5.18.0's text.
- Memory: see deviation 2.
- `00-boot`, `02-home`, `04-ink`, `07-share`, `internal-doc`, `internal-editor-view` still pass (25 with 12).

## Parity reruns for W3.2 and W3.5 (lead's request), `--quick`, A = `release/5.18.0` (`1d6e8a0`)

The 5.x fixtures now open, so these states are pixel-comparable for the first time.

| States | Comparisons | Pixel-equal | Differ |
|---|---|---|---|
| `editor-view` | 20 | 19 (DOM/trace warnings only) | 1, flaky: `pull-to-add` phone, 3 px Δ7 at the share button's edge; a 2-run rerun passed |
| `canvas` | 18 | 16 | `dense-page`@repaint desktop 3 px Δ2 (raster AA, stable); `photo-page` phone = the same 3-px share-button flake |
| `editor-tools` | 18 | 12 | see below |

Remaining `editor-tools` differences (not fixed here — none is in the importer, each belongs to a unit that does not
exist yet or to a component outside this unit; recorded for W6.1):
1. **Save indicator after a stroke** (`pen-lift`, `highlighter`@committed, `single-tap-dot`@live, desktop 9×9 px at
   the dot): 5.18.0 reaches `saved` (green, Drive upload done by the mock); v6 stays `paused` / "Waiting to save to
   Drive" because nothing uploads yet — relay/export are W4.5/W5.4.
2. **`undo-redo`@redone, phone**: the column is 36.8 px lower in v6 and `#canvas-viewport` shows `cursor: grabbing`.
   5.18.0 positions the phone toolbar as a floating panel (`#toolbar` inline `inset`, `--toolbar-scale`); v6's
   `#toolbar` has no such style, so the state's forced click on `#btn-redo` lands on the viewport and pans it. The
   floating-toolbar layout is the toolbar unit's (5.18.0 `positionToolbar…`), not W3.2/W3.5/W3.4.
3. `pen-live-1`/`pen-live-20` phone: the 3-px share-button flake again.

## Deviations

1. **No recovered copy request yet** — `drive/revisions.rs` is W5.4's; the record and `take_recovery` are the seam.
2. **"Main-instance memory unchanged after the largest fixture" is measured as: the import leaves < 16 MiB more
   main-instance memory than reopening the same document from the local copy.** Opening dense-20 (2,400 strokes,
   2 MB snapshot) grows the main instance from 6 MB to ~46 MB; reopening it locally without any import already needs
   ~37 MB. The parse itself is not the cause (forcing it onto the main thread gave the same peak): the cost is in
   `session::receive`/load and the editor (W3.1/W3.2) — worth a look by their owners or W6.2. This unit streams the
   worker answer per batch and frees each batch after applying, so its own transient memory is small.
3. **Worker threshold is on the Keywords length (256 KiB)**, not on the inflated size (unknown before inflating).
4. **`keep` for `supersede` is empty**: this installation's migration actors are W5.x's (migration); pass them there.
5. **Versions (`Import::versions`) are not stored**: the record codec is W5.3's.
6. **Size**: the importer links `ihn-core::v5` into the app for the first time: +150 KB raw (968,713 → 1,118,390;
   450 KB gzip). Report-only gate.
7. **Commit trailer** names the model given by the workflow.
