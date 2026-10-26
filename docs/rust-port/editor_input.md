# editor_input port plan and status

Branch rust/editor_input. Owns app-v5.js section 4.11 (lines 4124-4908, ~785 lines, ~45 KB) and the heading
"4.12 Selection & lasso tool" (starts line 4909). IMPORTANT: the 4.12 heading is not closed until
"4.13" at line 19751, so it physically contains ~14.8k lines (~800 KB) of unrelated code (Drive auth/home, timeline,
calendar, viewport pan/zoom, scanner import, page snapshot). I plan only the editor-input functions inside it
(selection, transform, eraser, stroke geometry: lines 4909-5443, 12411-12827, 12828-13145). The rest belongs to the
drive_home / editor_view / calendar / editor_history / scan areas and is listed as out of scope.

## STATUS

| Unit | State |
|---|---|
| U1 Resident stroke store + bounds + circle hit | DONE (see below) |
| U2 Area eraser split | open (splitStrokeByEraser still JS; it now calls Rust `segmentDistance` per segment) |
| U3 Lasso | open |
| U4 Selection bounds + transforms | open |
| U5 Pen input filtering | open |
| U6 Eraser damage rect | open |

### U1 result

What moved: `getStrokeBounds`, `strokeMayIntersectCircle`, `strokeHitsPoint`, `distancePointToSegment`, the hit loop of
`eraseStrokeAtPoint` and the `strokeBoundsCache` WeakMap (app-v5.js), plus the old `strokeHitTest` pack-and-cache wrapper in
rust-core-v5.js and `editor_stroke_hit` in editor.rs. They are replaced by the resident store in `rust-core/src/editor_input.rs`.

Design:
- Rust owns a slot table. A slot holds the stroke bounds (always) and the x/y coordinates (retained for one document generation).
  JS keeps `WeakMap<stroke, {slot, bounds, signature}>`; the signature (array, length, first/last point) is the same validity rule the old
  bounds cache used. Slots are freed by a `FinalizationRegistry`; a 6M-point cap clears the store (epoch bump) and re-uploads lazily.
- Upload happens once per stroke: bounds when first needed (render warm-up or eraser), coordinates only when a stroke passes the
  bounds pre-filter of an eraser query and its retained copy is not from the current `session:pageMutationGeneration`
  (the old guard). Rust reports such strokes as `pending`, JS uploads them and repeats the query.
- A query sends scalars plus a per-page slot/half-width list and gets back indices (descending, the order the eraser removes them).
  The per-page list is validated inside one generation by comparing stroke, point-array and length references; any generation change,
  splice or replaced stroke rebuilds it.
- Parity: V8's `Math.hypot` (scaled, Kahan-compensated) is reproduced bit for bit; `Math.max/min` NaN semantics are kept; a squared-distance
  shortcut only decides cases more than 1e-9 (relative) from the radius, the rest uses the exact formula. The old Rust long-stroke kernel
  (squared compare, 256+ points only) was not exact at the boundary; the new path is exact for every stroke length.
- WASM missing: `window.InhouseRust.*` throws `Rust editor core is unavailable`; `init()` awaits `InhouseRust.ready` and throws if it is false.
- Strokes with more than 4M points or pages that exceed the store are handled (scratch evaluation); only a single stroke above 4M points throws.

Known differences (none observable in the app): a `null` entry inside `points` was skipped by the old bounds scan but threw in the hit scan;
it is now treated as a NaN point (never hits). Shared-file edits outside the area markers: one `await InhouseRust.ready` line in `init()`,
removal of the `strokeBoundsCache` constant (app-v5.js line ~801), removal of `strokeHitTest` from the shared part of rust-core-v5.js.

Tests: `rust-core` unit tests (8 new), `tests/editor-input-erase.test.mjs` (differential against the pre-port app-v5.js from git: 120 seeded pages,
7,200 eraser queries with 1,947 exact-threshold and 1-ulp cases, 200,000 random/non-finite segment distances, invalidation and capacity cases),
`tests/e2e/editor-rust-hit.spec.mjs` rewritten for the new API (it called the removed `strokeHitTest` and the global `distancePointToSegment`; the
assertions are the same plus a bit-exact comparison with Chromium's own `Math.hypot`). `node tests/bench-editor-input.mjs` is the micro-benchmark.

Measured (node 22, main realm, 400-200 queries, microseconds per eraser query, original JS loop vs Rust store):

| Page | JS | Rust | Speedup |
|---|---|---|---|
| 1 stroke, 3000 points, r=15 | ~70-100 | ~19 | 3.5-5x |
| 300 strokes x 100 points | ~37-53 | ~12-30 | 1.8-3x |
| 2400 strokes x 100 points (dense) | ~300-380 | ~35-40 | 8x |
| 600 strokes x 600 points | ~430-480 | ~70-130 | 3.6-6x |

The first query after a document edit costs about what the old loop cost (strokes near the cursor are re-uploaded once).

Browser results (lock script, 14 spec files, 85 tests): 81 pass. Failures that also fail on the untouched base commit 3edf36b:
`editor-pen-input` "stationary pen ... 3000 pens" (30 s timeout) and the scanner specs "faded yellow stencil" / "calibration strip" in app-resilience.
"scanner starts offline" failed once in the batch and passed twice when rerun alone (flaky, scanner area).
`tests/native-assets.test.mjs` fails until the integrator regenerates native-assets.json (changed hashed files); not committed here by rule.

Bytes: JS removed from app-v5.js 3,951 B deleted / 750 B glue added (net -3,201 B); rust-core-v5.js 2,531 B (`strokeHitTest`) deleted and
8,164 B adapter added (the adapter is the upload/validation layer; it is the price of keeping geometry resident). Rust: editor_input.rs +16,236 B
(11.8 KB code, 4.6 KB tests), editor.rs -1,342 B. Wasm 123,290 -> 131,025 B.

## 1. Inventory and classification

### 4.11 (4124-4908)
| Function | Lines | Bytes | Class |
|---|---|---|---|
| setupCanvasEvents (closure) | 4128-4635 | 26 KB | glue (events, capture, state machine, history, save). Embedded pure bits: collectEraserPositions min-step filter, appendPenSamples point filter (dist<.001, highlighter min step), getCanvasPointerPos affine, erase-gesture loops |
| clearLassoCanvas, drawLassoPath, releaseLivePenSurface, drawEraserPreview, clearEraserPreview | 4636-4667,4702-4710,4865-4912 | ~4 KB | glue (canvas calls). Only dirty-rect bounds arithmetic is pure and tiny |
| drawStrokeTailSegment, drawSinglePointStroke | 4668-4701 | 1 KB | glue (ctx drawing) |
| drawStrokePreviewTail | 4711-4757 | 2.4 KB | glue + pure tail bounds (min/max loop over tail points, padding) |
| drawLiveHighlighterAppend | 4758-4815 | 3.2 KB | glue + pure incremental bounds |
| commitLiveStrokeToCanvas, finalizeStrokeSegments, updateLiveStroke | 4816-4864 | 2.3 KB | glue (state machine over canvas) |

### 4.12 editor functions
| Function | Lines | Bytes | Class |
|---|---|---|---|
| clearSelection, updateSelectionBox, drawSelectionOutlines, deleteSelection(+Images), duplicateSelection, changeSelectionColor, track/untrackDeletedStrokeIds, getStrokeIdsFromItems | 4913-5443 minus below | ~17 KB | glue (DOM, history, state, sync). Pure kernels inside: duplicate offset clamping (dx,dy) and point translate |
| selectStrokesWithLasso, strokeIntersectsPolygon, pointInPolygon, getImageCorners, getImageBounds, segmentsIntersect, polygonIntersectsRect | 4930-5038 | 4.2 KB | PURE portable (lasso) |
| getSelectionBounds | 5039-5075 | 1.4 KB | PURE (needs stroke points + image corners) |
| getPagePosFromEvent/FromClient, getPageIndexFromClientPoint, clamp | 5444-5488 | 1.6 KB | glue (reads DOM rects); affine is trivial, keep JS |
| tool switching (setActiveTool etc.), doc title, Drive, theme ... | 5489-12043 | - | out of scope (other areas) |
| captureStrokes/Images | 12411-12429 | 0.8 KB | glue (clones, history payload) |
| moveSelectionToPage | 12430-12517 | 3.9 KB | glue + pure y-shift of points |
| applyTransformToSelection | 12518-12571 | 2.5 KB | PURE kernel (scale/rotate/translate of points and image rects) + glue |
| setupSelectionBoxEvents, startSelectionTransform, flushSelectionMove, onSelectionPointerMove/Up | 12572-12675,12788-12827 | 6 KB | glue |
| applySelectionPointerMove | 12676-12787 | 6.2 KB | glue + pure transform computation (move/scale/rotate parameters, cross-page decision) |
| recordEraserDamage, repaintEraserDamage, scheduleEraserRedraw | 12828-12908 | 4.2 KB | damage rect union is pure (tiny); repaint is canvas glue (belongs to editor_render) |
| splitStrokeByEraser | 12909-12962 | 2.1 KB | PURE portable (id generation stays JS) |
| getStrokeBounds | 12963-13008 | 1.6 KB | PURE (WeakMap cache is JS-object identity based) |
| strokeMayIntersectCircle, strokeHitsPoint, distancePointToSegment | 13009-13053 | 2 KB | PURE; strokeHitsPoint already calls Rust `InhouseRust.strokeHitTest` for >=256 points (editor.rs editor_stroke_hit) = partially ported |
| eraseAreaAtPoint, eraseStrokeAtPoint | 13054-13126 | 3 KB | PURE loop over page strokes + glue (state mutation, cloneStroke, changeSet) |
| getCanvasPos | 13127-13145 | 0.8 KB | glue |
| Already ported: editor_stroke_hit (hit test for >=256 pts, f64 pairs) | editor.rs | - | ported, needs extension |

Out-of-area but touched by my hot path (owned by others; do not edit): simplifyStrokePoints (app-v5.js:881, section earlier than 4.11),
cloneStroke (839), shouldAcceptDrawInput (1916), drawSmoothStroke/buildSmoothPath/drawHighlighterCaps/getStrokeWidth (editor_render).
The pen-writing hot path is mostly canvas calls; its pure arithmetic is small, so the real wins are the O(strokes x points)
eraser, lasso and selection loops, and the per-event JS-object point marshalling.

## 2. Design principle (the big win)

Strokes are arrays of `{x,y,p}` objects. Every Rust call currently would need a copy of the points. Keep a
**resident stroke store in WASM** (module `editor_input::store`): per page, flat `Vec<f64>` (x,y,p interleaved) +
per-stroke offsets, bounds (min/max, width) and eraser flag, keyed by stroke id and revalidated by the existing
`${sessionToken}:${pageMutationGeneration}` guard used by strokeHitTest. JS pushes a stroke into the store only when it
is created/changed (finalized stroke, erase split, transform commit, page load). Hot queries (erase at point,
lasso, bounds) then take only scalars/small typed arrays and return indices. Never a JS fallback.

## 3. Units (ordered: speedup first, then lowest risk)

### U1 - Resident stroke store + bounds + circle/segment hit (stroke eraser)  speedup HIGH, risk LOW
- Scope: getStrokeBounds, strokeMayIntersectCircle, strokeHitsPoint, distancePointToSegment (12963-13053),
  eraseStrokeAtPoint's hit loop (13102-13126), extends editor.rs `editor_stroke_hit`; JS WeakMap strokeBoundsCache deleted.
- Rust: `editor_input::store` (page -> strokes), `editor_input_hits_circle(page, x, y, r, out_ptr, cap) -> count`
  returning stroke indices (reverse order, skipping eraser tools, bounds pre-filter with pad = r + 2 + width/2, then exact segment
  distance, 1-point strokes use point distance, same <= comparison as JS; note JS uses hypot, Rust must use the same
  expression order for byte-equal booleans; the existing Rust version uses squared distance, so parity fixtures with
  exact-boundary cases are required).
- Boundary: typed arrays. Upload: Float64Array [x,y,p,...] + Uint32 offsets (transferred once per stroke, incremental append for live strokes).
  Query: scalars in, Int32Array indices out.
- Tests: editor-rust-hit.spec.mjs, editor-pen-input.spec.mjs, editor-performance.spec.mjs, pdf-nondestructive-erase.spec.mjs,
  differential node test (new, tests/editor-input-erase.test.mjs via tests/helpers/legacy.mjs) with seeded random strokes.

### U2 - Area eraser: splitStrokeByEraser + eraseAreaAtPoint loop  speedup HIGH, risk MEDIUM
- Scope: splitStrokeByEraser (12909-12962), the loop of eraseAreaAtPoint (13054-13101: skip rules <2 pts, eraser tools,
  bounds prefilter, unchanged-detection `segments.length===1 && same length`).
- Rust: `editor_input::erase_area(page, x, y, r) -> segment descriptors`: for each affected stroke index returns list of
  (start,end) point ranges (a segment is a sub-run of the original points, except a crossing segment restarts at the
  crossing point, which is an original point, so ranges suffice: JS never inserts interpolated points). JS then builds
  stroke objects (generateStrokeId, color/tool/width copies, `{x,y,p}` point objects) from ranges, so ids/changeSet/history stay JS.
- Boundary: Int32Array ranges [strokeIdx, nSegments, (a,b)*]; scalars in.
- Tests: editor-*.spec.mjs, pdf-nondestructive-erase.spec.mjs, collaboration-chaos.spec.mjs (change-set/removed ids), save-*.spec.mjs;
  differential test on seeded random polylines incl. exact-radius cases.

### U3 - Lasso selection: pointInPolygon / polygonIntersectsRect / selectStrokesWithLasso  speedup HIGH, risk LOW
- Scope: selectStrokesWithLasso selection loop, strokeIntersectsPolygon, pointInPolygon, getImageCorners, segmentsIntersect,
  polygonIntersectsRect (4930-5038). Preserve the 0.00001 epsilon and the `(yi>py)!==(yj>py)` form exactly.
- Rust: `editor_input_lasso(page, poly_ptr, n, out_ptr, cap)` -> stroke indices (store from U1) and a second call
  `lasso_images(poly, images_f64[x,y,w,h,rot]*) -> indices`.
- Boundary: Float64Array polygon (lasso points collected into a typed array growing during the gesture),
  images as Float64Array 5-tuples; Int32Array out.
- Tests: editor-pen-input.spec.mjs (lasso cases), editor-performance.spec.mjs; differential test (random polygons, include
  collinear/degenerate polygons and det===0 segments).

### U4 - Selection bounds + transform kernels  speedup MEDIUM-HIGH (drag at 60 fps over thousands of points), risk MEDIUM
- Scope: getSelectionBounds (5039-5075), getImageBounds, applyTransformToSelection point/image maths (12518-12571),
  the transform parameter maths in applySelectionPointerMove (move/scale/rotate branches, 12676-12787, minus cross-page glue),
  duplicateSelection offset clamp + translate (5271-5366), moveSelectionToPage y shift (12430-12517), changeSelectionColor is
  not numeric (stays JS).
- Rust: `selection_bounds(page, idx[], images[])`, `transform_points(page, idx[], matrix params, out Float64Array)`,
  `transform_params(mode, handle, bounds, center, startPos, startAngle, pos) -> [tx,ty,sx,sy,rot]`.
  Output Float64Array written back into new `{x,y,p}` objects by JS (objects must still be created because history/save use them).
  Use the same operation order (`x*=sx; y*=sy; rx=x*cos-y*sin`) for bitwise equality.
- Boundary: typed arrays; params as 8-f64 array.
- Tests: editor-repaint-continuity.spec.mjs, editor-pen-input.spec.mjs, save-*.spec.mjs, preview-stability.spec.mjs; node differential test.

### U5 - Pen input filtering and live-tail bounds (hot path while writing)  speedup MEDIUM, risk MEDIUM
- Scope: appendPenSamples decision logic (dedupe distance<.001, highlighter min step max(.6,width*.12), timestamp monotonic filter,
  finite check, pressure fallback p = sample.pressure||last.p||.5), collectEraserPositions min-step filter, getCanvasPointerPos affine,
  and the tail-bounds loops in drawStrokePreviewTail / drawLiveHighlighterAppend (min/max over unfinalized points + padding).
- Rust: `pen_filter(state, samples Float64Array[clientX, clientY, t, pressure, pointerId]*, geom 4 f64, tool/width, out)` appending
  directly into the live stroke buffer in the store (U1) and returning accepted count + tail bounds `[minX,minY,maxX,maxY]`.
  JS still builds `currentStroke.points` objects for rendering and collab (live preview publishes them), so the benefit is mostly
  bounds and filtering; mark as lower priority. No smoothing/prediction exists in this section today (raw measured paths are
  intentional per commit a2dea10); do not add any, to keep pixels identical.
- Boundary: Float64Array samples in (batch per pointer event, coalesced events), Float64Array out.
- Tests: editor-pen-input.spec.mjs, editor-performance.spec.mjs, editor-repaint-continuity.spec.mjs, app-resilience.spec.mjs.

### U6 - Eraser damage rect union (small, merge with U2)  speedup LOW, risk LOW
- Scope: recordEraserDamage math (12828-12844) and the `intersects` bounds test used by repaintEraserDamage (stroke bounds vs damage rect),
  as a batch `strokes_intersecting_rect(page, rect) -> indices` used by the repaint filter loop.
- Tests: pages-panel-performance.spec.mjs, editor-repaint-continuity.spec.mjs, pdf-nondestructive-erase.spec.mjs.

(Total 6 units; each well under 40 KB of JS.)

## 4. Estimated JS removed (approx.)
U1 ~3 KB, U2 ~5 KB, U3 ~4 KB, U4 ~7 KB, U5 ~3 KB, U6 ~1 KB = ~23 KB of JS logic; Rust added roughly 20-30 KB with tests.
app-v5.js is over GitHub's 1.5 MB cutoff, so the language percentage changes through added Rust only.

## 5. Not portable / stays JS
- setupCanvasEvents event wiring, pointer capture, pointer-id state machine, rAF scheduling, palm rejection (shouldAcceptDrawInput): DOM events.
- All canvas drawing: drawLassoPath, drawStrokeTailSegment, drawSinglePointStroke, drawStrokePreviewTail/HighlighterAppend drawing, drawEraserPreview,
  drawSelectionOutlines, repaintEraserDamage, clear*Canvas, releaseLivePenSurface: Canvas2D calls (need browser).
- Selection box DOM/handles (updateSelectionBox, setupSelectionBoxEvents, color menu), clearSelection, tool buttons.
- History, save, sync stamping (saveToHistory, scheduleSave, enqueueStrokeOp, stampCollaborativeItems, publishLive*Preview, trackDeletedStrokeIds): app state / IO / collaboration, owned by other areas.
- getPagePosFromEvent/Client, getPageIndexFromClientPoint, getCanvasPos: read getBoundingClientRect (trivial affine kept JS).
- generateStrokeId and object construction (cloneStroke, id assignment): persistence format must stay JS objects.
- WeakMap caches keyed by JS objects (penPathCache, strokeBoundsCache, livePenSurfaces): replaced in Rust by the resident store, but Path2D cache is canvas (editor_render).
- ~14.8k lines of non-editor code under the 4.12 heading (Drive, theme, timeline, calendar, viewport, scanner import, page snapshot): other areas' scope; not planned here.
- simplifyStrokePoints (line 881) lies outside my sections: report to the integrator as a candidate for editor_render/docio.

## 6. Risks and cautions
- Exact parity of floating-point: Math.hypot vs sqrt of squares differ in the last ulp; boundary-case booleans must be differential-tested (existing Rust hit test uses squared compare; confirm identical results at threshold or keep hypot semantics).
- Store coherence: any code outside my area that mutates `stroke.points` (remote sync, undo, bake, restore) must invalidate the store. Keep the existing `sessionToken:pageMutationGeneration` guard and rebuild lazily from JS when it changes; never serve stale geometry.
- Lowest-latency requirement: the first eraser contact must not upload every stroke (current code deliberately avoids scanning every point); upload lazily per stroke when its bounds pass the pre-filter, or at page load slices.
- Wasm unavailable: throw a clear error (rule), no JS copy.
