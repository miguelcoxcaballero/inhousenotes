# editor_render porting status

## Done

### U1 stroke_pack: stroke point packing, bounds and legacy cover strokes (done)
- Rust (`rust-core/src/editor_render.rs`): `editor_render_stroke_bounds` (min/max of packed x,y pairs with the exact strict `<`/`>`
  semantics of the old loop, NaN never updates a bound) and `editor_render_cover_widths` (skip rules, 1.05x highlighter /
  1.35x pen widening, `Math.max(1, x)` with NaN propagation). 3.6 KB of Rust code, 3.3 KB of Rust unit tests.
- JS adapter (`rust-core-v5.js`, `stream:editor_render`): `strokeCoords(n)` hands out a Float64Array view of a reusable WASM scratch,
  `strokeBounds(n)` returns the four bounds, `legacyCoverWidths(strokes)` returns one width per stroke (-1 = not covered). Throws
  loudly if the core is missing.
- `drawSmoothStroke`: one pass over the point objects now builds the Path2D and packs the coordinates; the hit-test bounds come
  from Rust and are seeded into `strokeBoundsCache` (also for the throw-away cover stroke objects, which previously rescanned every
  point on every repaint). The old JS bounds scan is no longer used by the draw path.
- `drawLegacyCoverStrokes`: skip/widen rules deleted, widths come from Rust. The spread of the stroke object and the canvas calls stay JS.
- `redrawPage` waits for `InhouseRust.ready` (repaints afterwards) and throws if WASM failed; `drawSmoothStroke` and
  `drawLegacyCoverStrokes` throw when the core is missing instead of degrading.
- Tests: `cargo test` (4 tests), `tests/editor-render-stroke.test.mjs` (differential against the original `app-v5.js` from git:
  fixtures plus 4000 random strokes with repaints, growing live stroke, 300k-point stroke, 3000 random cover pages; byte-exact
  drawing logs and bounds), `tests/e2e/editor-render-parity.spec.mjs` (real canvas pixels, 0 differing bytes).
- Bytes: app-v5.js 1033 B removed, 2449 B added (glue, loud-failure checks and the cold-start gate in `redrawPage`); rust-core-v5.js +3.9 KB
  adapter; Rust +3.6 KB.
- Not ported, with reasons: `cloneLegacyCoverStrokes` is a deep copy of JS object graphs (Rust would need a JSON round trip, slower and
  lossy for `undefined`/extra fields), so it stays JS and unchanged. `getStrokeBounds` (section 4.5x, not owned) still carries the old JS scan
  as a fallback for strokes that were never drawn (eraser hit tests); it should call a Rust helper in a later unit/owner.
- Intentional divergence: for corrupt points whose x/y are numeric *strings* the old bounds kept lexical strings; Rust bounds are numbers
  (consumers subtract a pad first, so results are numeric either way). Path and pixels are identical.

Speed (Chromium, shared 4-core machine, best of 5; `lineTo` dominates so end-to-end is neutral):

| points per stroke | legacy build+bounds | Rust build+bounds | bounds only JS -> Rust |
| --- | --- | --- | --- |
| 150 | 9.2 us | 8.9 us | 3.2 -> 0.5 us (6x) |
| 600 | 31 us | 29 us | |
| 2500 | 141-242 us | 210-245 us (noise) | 98 -> 4.5 us (22x) |
| 10000 | 1.0 ms | 0.56-0.93 ms | 677 -> 24 us (27x) |

The bounds scan itself is 6-27x faster in Rust, but Path2D `lineTo` per point (native, unavoidable) is 95% of the cost, so building a
stroke path is within noise. The measurable win is for repaints of cover strokes (throw-away objects: O(1) seeded bounds instead of an
O(n) scan every paint). The cover-width batch is a few microseconds slower per call (0.1 -> 0.24 us for one stroke, 5.6 -> 30 us for 400),
negligible next to the canvas strokes it precedes.

## Remaining
U2-U8 as planned below.

---

# Original plan

Owned: app-v5.js section 4.6 (13299-13468, 8.5 KB), 4.7 (13469-14661, 60 KB), 4.13 (19751-20813, 52 KB).

## Finding
These sections are mostly glue. The stroke path is a plain polyline (`lineTo` per point), highlighter smoothing
(`buildSmoothPath`, `drawFixedHighlighterStroke`), `getStrokeBounds`, `getStrokeWidth`, template drawing
(`drawTemplateBackground`, `_renderTemplateToCanvas` -> `bullet-template.js`, 4 KB) and `getCoverResultLayout`
live OUTSIDE the owned headings (sections 4.1/4.2/4.5x and 12963-13298). The owned code that is real computation is
small (about 20 KB total). Honest expected gain: modest; the biggest hot spots (per-point JS object access in the
repaint loop, timeline diffing on big pages) are covered by units 1-3. Units touching code outside my headings need
integrator approval (marked OUT-OF-SECTION).

## Classification
Portable (pure):
- drawSmoothStroke path-point packing + cache check (13303-13332, 1.6 KB): geometry part only.
- drawLegacyCoverStrokes widen/filter rules (13448-13468) and cloneLegacyCoverStrokes (13429-13436, 0.6 KB).
- redrawPage staging decision `staged` (>128 strokes or any stroke >2048 pts) + per-frame stroke work list (13473-13612 slice, ~1 KB pure).
- getTimelineStrokeBounds / getTimelineImageBounds / clampTimelineBounds / getTimelineItemSignature / getTimelineItemKey /
  collectTimelineDiffHighlights (14243-14345, ~6 KB): pure; signature builds JSON.stringify of every point: the slowest pure code here.
- groupTimelineEntriesForDisplay + month/day key (14577-14661, grouping only; label formatting uses Intl/toLocale, stays JS).
- formatTimelineRelativeTime (13763-13783): pure but uses Date.now/locale; trivial, leave.
- getVisibleStrokeCount (20782-20795), thumbnail scale math in drawThumbnailStrokes (20759-20781).
- getCoverResultLayout + buildCoverImageAsset size math (19575-19597, 19774-19800; ~1.5 KB; layout fn is OUT-OF-SECTION at 19575).
- reindex shift plan in reindexPagesAfterInsert / reindexPagesAfterMove (20202-20255, 20629-20709): index order only; the IndexedDB ops are glue.
- bakeLegacyEraserAreaStrokes filter step (13687-13708): depends on eraseAreaAtPoint (other area).

Glue (stays JS): drawImagesOnContext, setupContext, drawPageBackgroundLayer, redrawPage orchestration/scheduling/Path2D/canvas blits,
renderAllPages, eraser menu (13709-13762), restoreVersion/applyRestoredPages/revertToOriginalPdf/buildPagesFromSnapshot (13784-14073),
timeline UI + renderTimelinePreview (14362-14572), drawTimelineDiffHighlights, all of cover modal, page-structure locks/mutation tokens
(19826-20016), insertPhotoPageAt/insertScannedPagesAt, renderPagesList, initPageListDrag, movePage, generateThumbnail, goToPage.
Already ported: none inside these sections (editor.rs `editor_stroke_hit` and ink.rs serve other areas).

## Units (ordered by speedup, then risk)

### U1 stroke_pack: stroke point packing and path build input — high speedup, low risk, ~3 KB JS
Scope: drawSmoothStroke (13303-13332), cloneLegacyCoverStrokes, drawLegacyCoverStrokes filtering/widening (13429-13468),
legacy cover stroke selection. Rust `editor_render.rs` keeps a per-stroke flat Float64Array (x,y pairs) validated once, bounds and
cache key (len, first, last) computed in Rust; JS builds Path2D from the typed array with a tight loop (no `points[i].x` object hops).
Boundary: Float64Array (ptr,len) in, Float64Array of 4 bounds out (also seeds strokeBoundsCache). Tests: node differential vs legacy.mjs
on seeded strokes; e2e editor-*.spec.mjs, preview-stability, save-*.spec.mjs, pdf-nondestructive-erase. Verify with canvas pixel compare.
Note: Path2D construction itself must stay JS.

### U2 timeline_diff: timeline diff engine — high speedup, low risk, ~7 KB JS
Scope: getTimelineStrokeBounds, getTimelineImageBounds, clampTimelineBounds, getTimelineItemSignature, getTimelineItemKey,
collectTimelineDiffHighlights (14243-14345). Rust computes 64-bit hash of the rounded (0.1) signature instead of JSON strings
(keys only compared for equality, not persisted; confirm collisions policy: use 128-bit), and highlight rects.
Boundary: JSON in (prev/next page items, minimal: tool,color,width,points flat arrays, image fields) -> Float64Array rects + Uint8Array types.
Tests: tests/timeline-core-v5.test.mjs, new differential test vs legacy, e2e editor-* (version history specs), save-*.
Rounding rule must match `Math.round(x*10)/10` (half toward +inf): implement with floor(x*10+0.5).

### U3 page_plan: repaint planning and stroke work list — high speedup, medium risk, ~4 KB JS
Scope: pure part of redrawPage (13473-13612): staged decision, visible stroke list filter (eraser-stroke / min points / highlighter 1),
remote-erase preview merge, repaint-skip key comparison stays JS (identity checks). Boundary: Uint8Array tool codes + Uint32 point
counts in, Uint32 index list out (order preserved). Tests: preview-stability, app-resilience, editor-*, collaboration-chaos.
Risk: must keep generation/cancel logic in JS; only the selection loop moves.

### U4 thumb_geometry: thumbnail and page-visible stroke counts — medium speedup, low risk, ~2.5 KB JS
Scope: getVisibleStrokeCount (20782-20795), drawThumbnailStrokes scale/filter (20759-20781), generateThumbnail inset/font math (20710-20758).
Boundary: Float64Array/Uint8Array tool codes. Tests: pages-panel-performance, preview-stability.

### U5 reindex_plan: page-move/insert index shuffle plan — medium speedup, low risk, ~4 KB JS
Scope: shift order and permutation computation in reindexPagesAfterInsert (20202-20255) and reindexPagesAfterMove (20629-20709);
Rust returns Uint32 pairs (from,to) in a safe (non-overwriting) order; IndexedDB reads/writes stay JS. Boundary: Uint32Array.
Tests: pages-panel-performance, save-*.spec.mjs, collaboration-chaos.

### U6 timeline_group: timeline grouping — low speedup, low risk, ~2 KB JS
Scope: groupTimelineEntriesForDisplay clustering (14621-14661), month key. Locale labels stay JS. Boundary: Float64Array timestamps +
Uint8Array original flags in, Uint32 cluster/month boundaries out. Tests: timeline-core-v5.test.mjs + version-history e2e.

### U7 cover_layout: cover/photo layout math — low speedup, low risk, ~2 KB JS (partly OUT-OF-SECTION)
Scope: buildCoverImageAsset size math (19774-19800), getCoverResultLayout (19575-19597, outside heading), getContainedRect users.
Boundary: Float64Array in/out. Tests: e2e cover/photo specs, release-v5-smoke.

### U8 template_geometry (needs integrator OK): bullet template dot/line geometry — medium speedup, medium risk, ~4 KB JS
Scope: bullet-template.js render geometry (outside my sections; file not in my ownership). Rust emits dot/line coordinates into
Float32Array; canvas calls stay JS. Tests: scanner-template/scanner-native-template specs, pixel compare. Skip if integrator refuses.

## Order of work
U1, U2, U3, U4, U5, U6, U7, U8. Each unit: Rust + cargo tests + differential test, delete JS, commit "rust(editor_render): ...".

## Not portable
- All canvas 2D calls, Path2D, drawImage, getContext, createProcessingCanvas, canvas blits (browser API).
- DOM building (renderAllPages, renderPagesList, timeline list), event/drag handling (initPageListDrag), modals.
- setTimeout/performance.now scheduling in staged repaint (time-sliced 4 ms budget; browser scheduler).
- Image decode/cache (getCachedImage, decodeImageSource, canvasToBlobSafe, dataUrlFromBlob).
- IndexedDB transactions, async locks/mutation tokens (19826-20016), Drive/PDF restore flows (restoreVersion, revertToOriginalPdf).
- Intl/toLocale date labels in timeline.
- Template SVG rendering for agenda/diary (img + base64 SVG decode by browser).
