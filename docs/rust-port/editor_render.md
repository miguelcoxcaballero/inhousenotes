# editor_render porting plan (planning only)

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
