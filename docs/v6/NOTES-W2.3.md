# W2.3 RENDER — notes for the units that build on it

Files: `rust/crates/ihn-core/src/render/*`, `host/canvas.js`, `icons/agenda.svg`, `icons/diary.svg`,
`tests/v6/e2e/internal-canvas.spec.mjs` (new). Branch `v6-w2_3`.

## What is there

- `render::cmd` — `Cmds`, the encoder of canvas family 2, and the handle types `Surf`, `Img`, `PathId`.
- `render::pen` / `highlighter` — `appendPenSplineSegment`, `drawSmoothStroke`, `drawSinglePointStroke`,
  `drawStrokeTailSegment`, `drawLegacyCoverStrokes`, `drawFixedHighlighterStroke`, `buildSmoothPath`.
- `render::template` — `Templates` (bullet template, SVG templates, `_tplCache` 6, `_tplThumbCache` 6, `dotStamps`
  4), `bg_scale`, `Templates::bg_url`, `BgUrls` (which template URL exists at what scale).
- `render::page` — `background`, `repaint` (synchronous), `Staged` (begin / paint(limit) / blit / discard),
  `is_staged`, `resize` (keep the last frame while resizing), `context_defaults`.
- `render::damage` — `Damage::add` (= `recordEraserDamage`), `repaint` (= `repaintEraserDamage`).
- `render::live` — `update` (per frame), `commit` (lift), `clear_cursor`; state `Live`, `Cursor`, `PageTarget`.
- `render::overlay` — lasso path / clear, selection outlines, eraser cursor, remote live redraw / append, timeline
  diff highlights.
- `render::thumb` — `page_preview`, `ScrollPreview`, `panel` (Pages panel thumbnails).
- `render::pdfink::content_stream` — body of the `/IHN6Ink` form (ARCHITECTURE §8.2).
- `host/canvas.js` — the complete executor; `host.surface(id)` / `host.image(id)` for other executors (pdf.js
  renders a scroll preview into `ScrollPreview::surf`).

## How to use it (W3.2, W3.5, W4.x, W5.x)

- **One `Cmds` per app, for the whole page life** (it owns the handle counters). After a batch of calls:
  `let bytes = cmds.take(); let m = out.w.rec(abi::fam::CANVAS, render::cmd::REC_BATCH); out.w.raw(&bytes);
  out.w.end(m);` — one record per flush. Canvas commands are applied in stream order with DOM ops, so insert a
  canvas into the document before the batch that binds it (`cmds.bind(node.0)`).
- `Cmds::ctx` = a `getContext('2d')` call (5.18.0 makes one in almost every function and the trace shows it);
  `Cmds::select` continues on a context without that call. Functions here do it for you.
- **Slices are counted, not timed.** Commands run in JS after the Rust call returns, so `Staged::paint` and
  `ScrollPreview::strokes` take a stroke limit; the editor flushes and re-schedules between slices and keeps what
  5.18.0 keeps in the job (generation guard, pen/pointer pauses, `canvas.isConnected`).
- **Overlay visibility** (`setOverlayCanvasInk`) is a DOM style: functions return `Some(has_ink)` / `bool` where
  5.18.0 calls it; apply `visibility` once per change.
- **`PathCache`**: pass item strokes built with `StrokeRef::of` / `PageItems::collect` (key = item id + body
  stamp). Call `forget` when an item is deleted or its page is released, `clear` on document close; otherwise
  `Path2D` objects stay in the host. Live and remote strokes use `StrokeRef::new` (never cached).
- **Images**: `cmds.image_blob(req, blob_obj)` / `image_url(req, url)`; draw only after `Io(req, 0)` (bytes =
  two f64: natural width, height). Pass `ImageRef { img: None, .. }` until then (5.18.0 `!img.complete`).
- **Async results**: `to_data_url` → `Io(req, 0, bytes = URL text)`; `to_blob` → `Io(req, 0, obj = Blob,
  bytes = f64 size)`; status 1 = failed. Template URLs: `urls.need(kind, zoom, is_pc)` → `tpl.bg_url(...)`.
- **SVG templates**: `Templates::new(agenda_req, diary_req)`; route those `Io`s to `tpl.svg_loaded(cmds, req, ok)`;
  when it returns true do what 5.18.0's `onload` does: `urls.clear()`, warm the URLs, update wrappers, schedule
  the visible-page update.
- **Eraser hit radius / swept test** stay in `Doc::hit_strokes`; `damage` only needs the removed strokes' bounds
  and width.
- Covers: `Tool::LegacyCover` strokes keep their original width in the model; the renderer applies 5.18.0's
  white + 1.35× rule.

## Acceptance

- `cargo test -p ihn-core render` — 17 tests. Nine are the trace goldens: `template-backgrounds`, `dense-page`,
  `dense-page-erase`, `ink-pen`, `ink-highlighter`, `thumbnails`, `page-preview`, `page-preview-images`,
  `scroll-preview` — every surface, size and call of the recorded file, argument bits included. The others cover
  what has no recorded scene (overlays, PDF placeholder, covers, images, path cache, resize, B-E3 dots, `pdfink`
  parse + every sample kept, `PageItems::collect`, the JS op table).
- `tests/v6/e2e/internal-canvas.spec.mjs` (6 tests): the same command batches replayed by the real
  `host/canvas.js` in Chromium under `trace-init.js` equal the 5.18.0 trace files (scenes without images).
  `IHN_CMD_DUMP=<dir> cargo test -p ihn-core render::tests` writes the batches; the spec runs it itself.

## Deviations and things the lead should know

1. **Scene tests script the editor.** A recorded scene contains calls that belong to the editor (canvas sizes,
   `applyPageDimensionsToWrapper`, which frame holds which samples, the scroll-preview queue). The tests make
   those through the same `render` functions/`Cmds` in the order the trace shows; the frame grouping (two samples
   per frame; eraser frames after even samples) is read off the recording. W3.2/W3.5 own the real scheduling.
2. **Calls of other units inside two recordings are emitted by hand in the test** so that the whole file
   compares: `dense-page` — a context probe and a Home card cover crop (24 × 16); `dense-page-erase` — the 5.x
   export's `toBlob` of the template canvas and its 1 × 1 processing canvas (v6 does not make these: ink is
   exported as a form XObject).
3. **Stroke inputs of document scenes are read back from the recordings** (`trace::strokes_of`: segment end
   points, colour, width). `expect.json` only has point counts, the PDFs hold the points compressed, and the v5
   reader (W2.2) is not merged. Cross-file where possible (`dense-page-erase` uses `dense-page.json`,
   `page-preview` uses `scroll-preview.json`); `thumbnails`, `page-preview-images`, `scroll-preview` and
   `dense-page` read their own file. Control points, line/curve choice, state calls, order, culling and damage
   rectangles are computed. Ink scenes use the recorded pen samples (client y rounded to f32 as Chromium does).
   The highlighter's stored points after lift (5.18.0 simplifies them) are also read from the recording: the
   simplifier is W3.5's.
4. **Colour case.** 5.18.0 passes the stored string (`#4D4D4D` from the palette); `Color::parse` folds
   `#rrggbb` to lowercase. Pixels are identical; the live trace *diagnostic* will show the case difference. The
   goldens build `Color::Css` with the recorded text. If the diagnostic matters, the model has to keep the case.
5. **Template SVG identity.** v6 loads `icons/agenda.svg` / `icons/diary.svg` (bytes equal to the 5.18.0
   constants, checked by the golden through the trace hash of the data URL); in a live trace the image argument's
   `src` hash differs from 5.18.0's `data:` URL. Diagnostic only.
6. **B-E3 is implemented here**: a one-point pen stroke is drawn as the live dot in repaints, thumbnails,
   previews and `pdfink`.
7. **`Tool::LegacyCover` has no "was a highlighter" bit**, so every cover is redrawn as a pen (white, 1.35×);
   5.18.0 used 1.05× and the highlighter shape for highlighter covers. Affects only legacy raster-cover
   documents with highlighter covers.
8. **Canvas record = one batch** (`[2][1][len][commands…]`, commands without their own length) instead of one
   record per call: a dense repaint is ~50,000 calls.
9. `scripts/check-math.mjs` currently reports three `.log(` lines of `ihn-app/src/store/selftest.rs` (W2.4);
   nothing in `render`.
