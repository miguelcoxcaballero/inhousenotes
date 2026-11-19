# Inhouse Notes v6 spec — Editor surface (ink, canvas, viewport, selection, tools)

Source of truth: Inhouse Notes **v5.17.22** (`index.html:93` `APP_VERSION = '5.17.22'`).
Scope: `app-v5.js` § 4.6, § 4.7, § 4.8, § 4.9, § 4.10, § 4.11 (+ the undo/redo it feeds, § 4.11b),
§ 4.12, § 4.14a; the editor markup/CSS in `index.html`; the shared helpers `ink-palette.js`,
`bullet-template.js`; and the editor kernels in `rust-core-v5.js` / `rust-core/src/editor.rs`.

All citations are `file:line` in the v5.17.22 checkout. "Logical px" = page coordinate units
(an A4 page is 794×1123 logical px). "Client px" = CSS viewport px. Persistence, Drive,
collaboration transport, timeline, Manage-Pages, calendar panels, scanner and PDF export
are other specs; they are mentioned only where they touch the editor surface.

The rebuild must reproduce *behaviour*, including the quirks listed in § 11 (each one is
flagged; fixing one is a product decision, not a refactor).

---------------------------------------------------------------------------------------------

## 0. Coordinate systems and the one transform

| Space | Definition | Where |
|---|---|---|
| Client | `PointerEvent.clientX/Y`. | browser |
| Content | Unscaled space of `#canvas-container`. Pages are laid out in it at CSS px == logical px. | `index.html:1467` |
| Viewport transform | `#canvas-container.style.transform = translate(panX px, panY px) scale(zoom)` with `transform-origin: 0 0`. client = viewportRect.left/top + pan + content·zoom. | `app-v5.js:17701-17705`, `index.html:1467-1478` |
| Page (logical) | Origin = top-left of a `.page-wrapper`; x∈[0,pageWidth], y∈[0,pageHeight]. All stroke points, image centres and selection boxes live here. | — |
| Canvas backing | page logical × `renderScale` (≥2). The 2D context carries `setTransform(scaleX,0,0,scaleY,0,0)`. | `app-v5.js:2440-2586` |

Client→page conversion while drawing (cached once per contact):
`x = (clientX − rect.left) · pageWidth / rect.width`, `y = (clientY − rect.top) · pageHeight / rect.height`
where `rect = pageCanvas.getBoundingClientRect()` (`app-v5.js:4250-4277`).

Selection transforms and image insertion use a different helper that **hard-codes A4**
(`getPagePosFromEvent/getPagePosFromClient`, `app-v5.js:5583-5605`): `x = (clientX−rect.left)·794/rect.width`,
`y = (clientY−rect.top)·1123/rect.height` with `rect` = wrapper rect. See § 11 Q1.

There is **no `devicePixelRatio` handling anywhere** (`grep devicePixelRatio app-v5.js` → none).
Canvas resolution depends only on zoom and device class (§ 6.2).

---------------------------------------------------------------------------------------------

## 1. Data model (in memory)

### 1.1 Global editor state (`state`, `app-v5.js:1240-1318`)

Editor-relevant fields and defaults:

| Field | Default | Meaning |
|---|---|---|
| `pages` | `[]` | Ordered page objects (§ 1.2). Index = visual order. |
| `currentTool` | `'pen'` | One of `pen`, `highlighter`, `eraser-stroke`, `eraser-area`, `lasso`. (`'eraser'` is only a button name.) |
| `currentColor` | `'#002FD9'` | Colour for pen/highlighter strokes. |
| `currentWidth` | `2` | Width (logical px) for new pen/highlighter strokes. |
| `eraserWidth` | `12` | Eraser diameter (logical px). Hit radius = `eraserWidth/2`. |
| `eraserMode` | `'stroke'` | `'stroke'` or `'area'`; selects which eraser tool the eraser button activates. |
| `zoom`, `panX`, `panY` | `1, 0, 0` | Viewport transform (§ 3). |
| `overscrollY` | `0` | Accumulated bottom overscroll used for "pull to add page" (§ 3.7). |
| `pageCreateLocked`, `pendingPageCreate` | `false` | Overscroll page-creation flags. |
| `isPanning`, `scrollLocked`, `lastPanX/Y`, `panStartX/Y` | — | Single-pointer pan state; `scrollLocked ∈ {false,'vertical','horizontal','free'}`. |
| `initialPinchDist`, `initialScale`, `initialPanX/Y`, `pinchCenterX/Y` | — | Pinch state captured when 2nd touch lands. |
| `activePointers` | `Map` | Viewport (non-pen) pointers: id → `{clientX, clientY}`. |
| `panVelocityX/Y`, `lastPanTime`, `inertiaFrame`, `inertiaLock` | `0,0,0,null,'free'` | Fling state. Velocity unit = client px per ms. |
| `lastNonEraserTool` | `{tool:'pen',color:'#002FD9',width:2}` | Restored when toggling out of the eraser. |
| `selection` | `{pageIndex:null, strokeIndices:[], imageIndices:[]}` | Current lasso selection (indices into the page arrays). |
| `selectionTransform` | `null` | Active move/scale/rotate drag (§ 5.6.4). |
| `activePageIndex` | `0` | Last page touched; gets activation priority. |
| `history`, `historyIndex`, `maxHistory` | `[], -1, 10` | Undo stack (§ 8). |
| `isReadOnly` | `true` | Read mode. Every input path is a no-op when true. |

Module-level editor state worth porting (`app-v5.js:1859-2111`): `touchDrawEnabled` (default
`false`, `app-v5.js:1989`), `isPenWriting`, `activeDrawingPointers: Map<canvas,pointerId>`
(`app-v5.js:1936`), `lastPenEventTime` (palm rejection), `lastPenBackgroundTap`,
`viewportGestureGeometry` (cached `{rect,bounds}` for a gesture), page-layout caches,
`activePageIndices`, `desiredActivePageIndices`, `pendingPageActivations`,
`addPageIndicator` element (`app-v5.js:2112-2115`).

### 1.2 Page object

Constructor for new pages (`addPage`, `app-v5.js:2800-2814`; `createBlankPageData`, `app-v5.js:19737-19757`):

```
{
  pageId: 'pg_' + Date.now().toString(36) + '_' + random(9 base36),   // app-v5.js:743-745
  strokes: Stroke[],            // null when unloaded
  images: Image[],              // null when unloaded
  deletedStrokeIds: string[],   // tombstone ids (sorted keys of deletedStrokeStamps)
  deletedStrokeStamps: {id: {clock, actor}},
  backgroundImage: string|null, // data:/blob: URL. PDF pages: rendered JPEG blob URL (never persisted)
  backgroundSource: 'template'|'pdf'|'custom',
  templateKind: 'default'|'agenda'|'diary',
  pdfPageIndex: number|null,    // 1-based page in the active PDF
  pageWidth, pageHeight,        // logical px, normalized: round(), min 64 (app-v5.js:2392-2404)
  needsRedraw: true,
  preview: string|null,         // JPEG data URL thumbnail shown as wrapper background
  strokeCount: number,          // visible (non-eraser) strokes, valid even when unloaded
  unloaded: false,
  // optional / transient:
  pendingUnload, missingData, scrollPreviewDirty, scrollPreviewVerified,
  bakedBackground (legacy raster, read-only), legacyCoverStrokes (legacy white covers),
  sidePanel (calendar panel data; other spec)
}
```

* New pages inherit `templateKind` and the template's default size from the **previous last page**
  (`app-v5.js:2796-2809`). Template default sizes: `default` → A4 794×1123; `agenda`/`diary` → A5 559×794
  (`app-v5.js:294-310`).
* `isPageLoaded(page) = page && !page.unloaded && Array.isArray(page.strokes)` (`app-v5.js:24336-24338`).
* Persisted shape (IndexedDB page record) is `sanitizePageForStorage` (`app-v5.js:22413-22437`):
  `pageId, strokes, images, deletedStrokeIds, deletedStrokeStamps, backgroundImage (null for pdf / blob:),
  backgroundSource, templateKind, pdfPageIndex, legacyCoverStrokes, pageWidth, pageHeight, sidePanel`.

### 1.3 Stroke object

Finalized at pen-up (`app-v5.js:4684-4690`):

```
{ id, tool, color, width, points: [{x, y, p}, ...], syncStamp?: {clock, actor} }
```

* `id`: `Date.now().toString(36) + '-' + Math.random().toString(36).substr(2, 9)` (`app-v5.js:736-738`).
  Legacy documents get deterministic `st_legacy_<hash><hash>` ids (`app-v5.js:753-776`).
* `tool`: `'pen'` or `'highlighter'`. Legacy data may contain `'eraser-area'` / `'eraser-stroke'` strokes:
  `eraser-stroke` strokes are never drawn; `eraser-area` strokes are converted at boot
  (`bakeLegacyEraserAreaStrokes`, `app-v5.js:13993-14013`, called from `init`, `app-v5.js:2273`).
* `color`: hex string (palette § 5.1). `width`: logical px (pen 2, highlighter 16 from the toolbar).
* `points[i].x/y`: logical page px, raw float (never rounded). `p`: pressure — first sample
  `e.pressure || 0.5`; later samples `sample.pressure || last.p || 0.5` (`app-v5.js:4435`, `4379`).
  **Pressure is stored but never used for rendering** (`getStrokeWidth` returns `stroke.width`,
  `app-v5.js:13497-13499`).
* `syncStamp = {clock: max(structure.clock+1, Date.now()), actor}` set on every create/modify
  (`stampCollaborativeItems`, `app-v5.js:835-852`).
* Live-only fields during contact (never persisted): `lastMid` (unused), `finalizedIndex`,
  `liveCurveIndex`, `livePreviewPointCount` (`app-v5.js:4497-4505`, `4805-4825`, `4896-4953`).
* Pen points are **never simplified** (`simplifyStrokePoints` returns early unless highlighter,
  `app-v5.js:914-922`). Highlighter simplification: § 5.3.
* `cloneStroke` copies exactly `{id, tool, color, width, points[{x,y,p}], syncStamp?}` (`app-v5.js:872-882`).

### 1.4 Image object (inserted pictures on a page)

```
{ id, src /* data URL */, x, y /* CENTRE, logical px */, width, height /* logical px */,
  rotation /* radians, about centre */, syncStamp? }      // app-v5.js:12652-12660
```

### 1.5 Selection and transform state

* `state.selection = {pageIndex, strokeIndices[], imageIndices[]}` — indices, not ids.
* `state.selectionTransform = {pageIndex, mode:'move'|'scale'|'rotate', handle, bounds, center,
  startPos, startAngle, originalStrokes[{index,stroke}], originalImages[{index,image}], pointerId}`
  (`app-v5.js:12933-12962`).

### 1.6 Ids, z-order

Draw order = array order: images (all) first, then strokes in `page.strokes` order (§ 6.4).
New strokes/duplicates/moved items are appended (top of z-order).

---------------------------------------------------------------------------------------------

## 2. Page layout

### 2.1 Constants (`app-v5.js:294-353`)

`A4_WIDTH=794, A4_HEIGHT=1123, A5_WIDTH=559, A5_HEIGHT=794, PAGE_PADDING=30, PAGE_GAP=30,
PAGE_CALENDAR_PANEL_HEIGHT=184, PAGE_CALENDAR_PANEL_GAP=14, VISIBLE_PAGE_BUFFER=1, MAX_ACTIVE_PAGES=3,
MIN_PAGE_VISIBLE=5, OVERSCROLL_THRESHOLD=360, OVERSCROLL_FRICTION=0.24, OVERSCROLL_MARGIN=100,
CREATE_PAGE_MIN_BOTTOM_RATIO=0.45`.

### 2.2 DOM structure of the page column

```
#canvas-viewport                         (flex:1; overflow hidden; bg #2c2c2c / dark #121212; cursor grab)
  #canvas-container                      (absolute 0,0; flex column; align-items:flex-start;
                                          padding 30px (20px ≤768px); gap 30px; will-change: transform)
    .page-row[data-page-row=i]           (flex column, align-items center, gap 14px, flex-shrink 0)
      .page-wrapper[data-page=i]         (position relative; width/height = page logical px inline;
                                          bg white; box-shadow 0 4px 20px rgba(0,0,0,.4);
                                          border-radius 2px; overflow hidden)
        canvas.page-canvas[data-page=i]  (display block; cursor crosshair; touch-action none; 100%×100%)
        canvas.lasso-overlay             (absolute inset 0; pointer-events none)
        canvas.cursor-overlay            (absolute inset 0; pointer-events none)
        canvas.remote-live-overlay       (absolute inset 0; pointer-events none)
        div.selection-box.hidden         (§ 9.4)
        div.page-number                  (text i+1)
      .page-side-panel                   (optional calendar panel; other spec)
    #add-page-indicator                  (always last child of the container)
```

Built by `renderAllPages` (`app-v5.js:13919-13991`) / `createPageElement` (`app-v5.js:2830-2887`);
canvases + selection box are only present for *active* pages (`attachPageCanvases`,
`app-v5.js:2889-3015`). Child insertion order inside the wrapper is exactly as listed
(canvas inserted before first child; each overlay/selection box inserted before the page number).

CSS references: `.page-row` `index.html:1656-1661` and `3432-3438`; `.page-wrapper` `2270-2286`;
`.page-canvas` `2298-2302`; overlays `2304-2315`; `.page-number` `2537-2545`
(`position:absolute; bottom:10px; right:15px; font-size:.75rem; color:var(--text-secondary); opacity:.6; pointer-events:none`).
New pages added by overscroll get class `new-page` for 600 ms → `pageFadeIn` 0.5 s opacity 0→1 (`app-v5.js:2842-2845`, `index.html:2279-2296`).

### 2.3 Layout metrics (`getPageLayoutMetrics`, `app-v5.js:16656-16712`)

Cached per `(state.pages identity, length, calendarEnabled)`; invalidated by
`invalidatePageBoundsCache` (`app-v5.js:16740-16746`).

```
y = PAGE_PADDING
for each page i: top[i]=y; bottom[i]=y+h; left[i]=PAGE_PADDING+(maxWidth−w)/2; right[i]=left[i]+w
                 y += h + (sidePanel && calendarEnabled ? 184+14 : 0) + (i<last ? PAGE_GAP : 0)
contentBottom = y;  bounds = {left:30, top:30, right:30+maxWidth, bottom:y}
getContentHeight() = contentBottom + PAGE_PADDING            (app-v5.js:16651-16654)
```

`maxWidth = max(794, widths)` (`app-v5.js:2406-2414`). Binary searches
`findFirstPageWhoseBottomReaches` / `findLastPageWhoseTopPrecedes` (`app-v5.js:16714-16734`).
Note § 11 Q3 (metrics centre narrower pages but the DOM left-aligns rows; CSS padding is 20 px at ≤768 px).

### 2.4 Add-page indicator (`#add-page-indicator`)

Created in JS (`app-v5.js:2112-2115`), text `+`. CSS `index.html:2518-2535`: 56×56 circle,
`2px dashed rgba(255,222,0,.95)`, colour same, font 28px/600, bg `rgba(20,20,20,.25)`,
shadow `0 6px 18px rgba(0,0,0,.25)`, `opacity:0; transition: opacity .5s ease; pointer-events:none`.
Position (`updateAddPageIndicator`, `app-v5.js:16800-16828`): `left:50%; transform:translateX(-50%);
top: contentBottom + 20 px` (content space). Opacity: 0 in read-only or no pages; else if
`overscrollY>0`: `min(1, 0.2 + min(1, overscrollY/360)·0.8)`, ×0.6 when page creation is not yet
allowed (`bottomInView ≥ 0.45·viewportHeight`). Read-only CSS forces opacity 0 (`index.html:1280-1282`).

---------------------------------------------------------------------------------------------

## 3. Viewport: pan, zoom, scroll, inertia, overscroll

### 3.1 Applying the transform

* `applyViewportTransformStyle` (`app-v5.js:17701-17705`): sets the CSS transform, schedules the
  fast-scroll thumb and queues scroll previews.
* `scheduleViewportTransformRender` (`app-v5.js:17707-17720`): ≤1 rAF; in the frame: clamp (§ 3.2),
  apply; only when *not* interacting also schedule canvas-resolution sync and visible-page update.
* `updateTransform` (`app-v5.js:17722-17736`): cancels pending frame, clamps, applies immediately,
  then canvas resolution sync (immediate if idle and no gesture geometry, else 90 ms debounce),
  `scheduleViewSave()` (250 ms → localStorage `notebook-view-v1` = `{panX,panY,zoom}`,
  `app-v5.js:21675-21693`) and `scheduleVisiblePageUpdate()`.
* Zoom range everywhere: **[0.5, 3]** (`app-v5.js:17344`, `17519`, `17739-17742`, `applyViewState` `22103-22114`).

### 3.2 Clamp (`clampPanToVisible`, `app-v5.js:16752-16772`)

Keeps ≥5 client px of the content bounds visible:
`panX ∈ [5 − right·zoom, vw − 5 − left·zoom]`, `panY ∈ [5 − bottom·zoom, vh − 5 − top·zoom]`;
if min>max use the midpoint. Uses cached gesture geometry when present.

### 3.3 Mouse wheel / trackpad (`handleWheel`, `app-v5.js:17316-17377`; listener `{passive:false}` `app-v5.js:16517`)

1. If the target is inside `.page-side-panel-body`, the wheel is mostly vertical, no Ctrl/Meta, and
   the panel can scroll in that direction → return (native panel scroll).
2. `preventDefault`, interrupt background work, note activity.
3. Normalise: deltaMode 1 → ×16; deltaMode 2 → ×max(240, 0.85·vh); else ×1.
4. Ctrl or Meta (and trackpad pinch, which Chrome reports as ctrl+wheel): `zoom' = clamp(zoom·(dy>0 ? 0.94 : 1.06), 0.5, 3)`,
   anchored at the mouse: the content point under the cursor stays under it.
5. Otherwise: `panX −= cap(dx·0.92)`, `applyVerticalPan(−cap(dy·0.92))` with `cap = clamp(±100)`.
6. `scheduleViewportTransformRender()`; 300 ms after the last wheel event: `finalizePendingPageCreation()`.

### 3.4 Pointer routing on the viewport (`handleViewportPointerDown`, `app-v5.js:17380-17451`)

Listeners on `#canvas-viewport`: pointerdown/move/up/cancel/leave (`app-v5.js:16507-16511`).

* **Pen**: never pans. Only checks the pen background double-tap (§ 4.4), then returns.
* Pointer on `canvas.page-canvas`, not read-only, tool is a drawing tool, and `shouldAcceptDrawInput(e)`
  → return (the canvas handles it). In read-only the touch falls through to pan.
* **Touch while `isPenWriting`** → ignored entirely (palm rejection for pan).
* Else: note activity, `stopInertia()`, cache gesture geometry. If tool is lasso and a selection box
  is visible and the target is outside it → `clearSelection()`.
* **Mouse** (any button): start pan, `cursor:grabbing`. No inertia for mouse.
* **Touch**: add to `activePointers`. 1 pointer → start pan (velocity reset, `lastPanTime=now`).
  2 pointers → start pinch: centre = midpoint, `initialPinchDist`, `initialScale=zoom`, initial pan.

### 3.5 Move (`handleViewportPointerMove`, `app-v5.js:17492-17598`)

Ignores pen and pointers not in `activePointers`. Refreshes the interaction idle deadline (no
per-sample interruption work).

* **Teleport filter** (`clampPanDelta`, `app-v5.js:17469-17484`): if |dx|>100 or |dy|>100 client px
  between consecutive events, the delta is dropped (but `lastPan` is updated).
* Mouse: `panX += dx; applyVerticalPan(dy)`.
* Pinch (2 touches): `zoom = clamp(initialScale·dist/initialDist, 0.5, 3)`; the content point that was
  under the *initial* pinch centre is placed under the *current* centre (pinch-zoom and two-finger pan in one).
* One touch: axis lock decided once cumulative |Δ| > 10 px from the start: vertical if Δy > 2Δx,
  horizontal if Δx > 2Δy, else free. Locked axes ignore the other component.
  Velocity: `dt = now−lastPanTime`, `dt<8 → 16`, `dt>64 → 64`; `v = 0.72·v + 0.28·(d/dt)`.

### 3.6 Up / fling (`handleViewportPointerUp`, `app-v5.js:17600-17673`; inertia `app-v5.js:17036-17091`)

* Mouse up: end pan, finalize page creation, stop inertia, snap back any overscroll, `updateTransform()`.
* Touch: when going 2→1 touches, the remaining finger restarts a pan with `lastPanTime=now` and zero
  velocity (prevents a 2800 px/s spike). When the last touch lifts: finalize page creation; if no
  overscroll and `hypot(v) > 0.015` → inertia with the final axis lock; else `updateTransform()`.
* Inertia per rAF: `v *= 0.935^(dt/16.67)`; stop when both |v| < 0.015 px/ms; step = `cap(v·dt)` (±100);
  apply respecting the lock (`applyVerticalPan` for y); clamp + apply transform inside the same frame.
* `window` blur releases all viewport pointers and inertia (`app-v5.js:1950-1956`, `16774-16784`).

### 3.7 Vertical pan & "pull to add page" (`applyVerticalPan`, `app-v5.js:16942-17007`)

`minPanY = vh − 100 − contentHeight·zoom` (`getBottomLimit`, `app-v5.js:16793-16798`).

* Read-only: `panY = max(minPanY, panY+dy)`; no overscroll.
* Scrolling up past `minPanY`: excess accumulates into `overscrollY`, capped at
  `max(1080, 1.1·max(0,(0.55·vh − 100)/0.24))`; displayed `panY = minPanY − overscrollY·0.24` (rubber band).
  `pendingPageCreate = overscrollY ≥ 360 && (panY + contentHeight·zoom) < 0.45·vh`.
  A 600 ms timer after the last overscroll movement snaps back (`overscrollY=0`, `panY=max(panY,minPanY)`).
* Scrolling back down consumes overscroll first.
* `finalizePendingPageCreation` (`app-v5.js:17009-17034`), called on touch/mouse release and 300 ms
  after the last wheel: if pending and still allowed → `addPage(null, animate=true)`, reset, and
  `animatePanTo(previous minPanY)`: container `transition: transform .35s ease` for 360 ms
  (`app-v5.js:16830-16837`).

### 3.8 Other view operations

* `centerViewOnTop` / `alignViewportTopToWidth` (`app-v5.js:2752-2773`): keeps zoom,
  `panX = (vw − w·zoom)/2 − 30·zoom` (page horizontally centred), `panY = 20`.
* `goToPage(i)` (`app-v5.js:21153-21165`): centre page i vertically and horizontally at current zoom.
* `scrollToTodayCalendarPage` (`app-v5.js:16891-16940`): zoom-to-fit `clamp(min(vh/(h+60), vw/(w+60)), 0.5, 3)`,
  centre the target page, re-measure next frame and correct residual >1 px.
* `preserveViewOnResize` (`app-v5.js:17683-17699`): keep the content point at viewport centre fixed.
* Boot (`init`, `app-v5.js:2256-2365`): view = today's calendar page, else saved view state
  (`applyViewState`, `app-v5.js:22103-22114`: zoom clamped 0.5–3), else `centerViewOnTop()`.
* Opening a Drive document shows a 50 %-opacity A4 thumbnail placeholder `.page-wrapper.loading-doc-preview`
  and aligns the top to A4 (`app-v5.js:11355-11373`). New documents start at zoom 1, pan 0 (`app-v5.js:11792-11795`).

### 3.9 Fast-scroll thumb (`installDocumentFastScroller`, `app-v5.js:35797-35862`; CSS `offline-v5.css:24-28`)

`div#document-fast-scroll` (role scrollbar, aria-valuemin 0 / max 100, tabindex 0) appended to body;
29×48, `border-radius:48px 4px 4px 48px`, bg `var(--accent-color,#df8143)` (dark `#87502f`), white
SVG double arrow, `position:fixed; right:safe-area`. For the editor: `max = contentHeight·zoom − vh + 40`,
`offset = 20 − panY`; thumb `translateY(top + ratio·travel)` where `top = max(0,rect.top)+12`,
`travel = visibleHeight − 72`. Hidden if `max<2` or `travel<30`. Drag sets `panY = 20 − ratio·max`.
Keys: ArrowUp/Down ±4 %, PageUp/Down ±20 %, Home/End.

---------------------------------------------------------------------------------------------

## 4. Input acceptance, palm rejection, gestures

### 4.1 Who may draw (`shouldAcceptDrawInput`, `app-v5.js:1991-2001`)

| pointerType | Draw/erase/lasso on canvas? |
|---|---|
| `pen` | Always. Records `lastPenEventTime = Date.now()`. |
| `mouse` | Only when "Draw with finger" (`touchDrawEnabled`) is on. Otherwise mouse drags **pan**. |
| `touch` | Only when `touchDrawEnabled` and ≥ 2000 ms since the last pen event (palm rejection). |
| other | Never. |

`touchDrawEnabled` = checkbox `#touch-draw-toggle` in the editor profile menu, persisted as
localStorage `touchDrawEnabled` = `'true'|'false'` (`app-v5.js:15972-15980`, `index.html:9605-9608`).

### 4.2 Gesture matrix (resulting behaviour)

* Fingers (draw-with-finger off): 1 finger pans (axis-locked, fling); 2 fingers pinch-zoom + pan.
  Touch is ignored for panning while `isPenWriting` (pen contact + 180 ms idle tail).
* Pen: draws/erases/lassos on pages; can never pan; tapping the grey background twice toggles eraser.
* Mouse: left-drag pans (draw-with-finger off) or draws (on); wheel scrolls; Ctrl/Meta+wheel zooms.
* With draw-with-finger on, a finger on a page draws. A 2nd finger landing on a page canvas is rejected
  by the canvas (`activePointerId !== null` and not pen → return, `app-v5.js:4394-4399`) **and** by the
  viewport (it is a drawing-tool touch on a canvas, `app-v5.js:17389-17396`), so it does nothing; only
  touches on the grey background pan/pinch. Pinch over pages is therefore unavailable in this mode.
* No barrel-button/eraser-end support, no two-finger-tap undo, no long-press, no contextmenu handling.

### 4.3 Locks

All canvas, selection and image inputs are ignored when `state.isReadOnly`, while a local page-structure
mutation holds `localPageStructureMutationToken`, or while `remotePageStructureApplyInProgress`
(`app-v5.js:4400`, `5304`, `5346`, `12877`). A pointerdown on a canvas whose page data is not loaded
only triggers loading (the contact is dropped, `app-v5.js:4403-4406`). Any pointerdown on a
`.page-wrapper` sets `activePageIndex` and activates a non-active page (`app-v5.js:2869-2879`).

### 4.4 Pen double-tap on background (`handlePenBackgroundDoubleTap`, `app-v5.js:5716-5735`)

Pen pointerdown on the viewport *outside any `.page-wrapper`*; two taps within 320 ms and 42 client px
→ `toggleEraserFromGesture()` (eraser ↔ last non-eraser tool, `app-v5.js:5706-5714`).

### 4.5 Keyboard

* `Ctrl+z` → undo, `Ctrl+y` → redo, only when not read-only; compares `e.key` to lowercase letters,
  `ctrlKey` only (no Meta, no Ctrl+Shift+Z) (`app-v5.js:16349-16359`). Button titles say "Undo (Ctrl+Z)"/"Redo (Ctrl+Y)".
* `Escape` closes modals/menus (not the selection) (`app-v5.js:16142-16155`).
* No Delete/Backspace for selections; no keyboard zoom/scroll (except fast-scroll thumb § 3.9).
* Doc title (`#doc-title`, contenteditable): click in read mode switches to edit mode and focuses;
  Enter commits, Escape cancels (`app-v5.js:16075-16097`).

### 4.6 Activity/idle bookkeeping (affects every heavy job)

* `notePenActivity()` (`app-v5.js:25413-25430`) on every accepted pen/eraser/touch-draw down and move:
  `isPenWriting=true`, re-arm idle timer **180 ms** (`DOCUMENT_SAVE_IDLE_MS`, `app-v5.js:363`);
  pauses interruptible saves (first event, then ≤ every 140 ms).
* `finishPenWritingIdle` (`app-v5.js:25317-25339`): if a drawing pointer is still down → re-arm 80 ms;
  else `isPenWriting=false`, flush stroke-op queue, run deferred "after pen" tasks (visible pages,
  canvas resolution), resume saves.
* `noteInteractionActivity()` / `refreshInteractionIdleTimer()` (`app-v5.js:25255-25300`): viewport
  gestures; `isGestureInteracting` until 100 ms (`INTERACTION_IDLE_MS`) of no input *and* no held
  pointer/inertia/selection drag; then resolution sync, visible pages, previews, view save.
* `isViewportInteracting()` (`app-v5.js:3514-3517`) = panning ∨ held viewport pointers ∨ inertia ∨
  `isPenWriting` ∨ gesture active ∨ any user activity in the last 240 ms.
* Capture-phase document listeners interrupt background processing on pointerdown/touchstart/wheel/keydown
  and (throttled 60 ms, only with buttons/pressure or touch) on pointermove/touchmove (`app-v5.js:16157-16195`).
* Pen bursts: strokes < 3000 ms apart form a burst; whole-document work waits 1200 ms after the last
  pen activity inside a burst (`app-v5.js:1966-1967`, `25364-25369`).

---------------------------------------------------------------------------------------------

## 5. Tools

### 5.1 Palette and toolbar presets

`window.InhouseInkPalette = ['#4D4D4D', '#002FD9', '#E81010']` (black, blue, red), shared with the
scanner (`ink-palette.js:4-5`); `InhouseInkDefaults = {width:2, pageWidth:794, pageHeight:1123}`.
Toolbar presets (`index.html:9651-9683`): Black Pen `#4D4D4D` w2, Blue Pen `#002FD9` w2 (initially
`.active`), Red Pen `#E81010` w2, Highlighter `#ffde00` w16. There is no width or colour picker.

### 5.2 Pen

Contact lifecycle (`setupCanvasEvents`, `app-v5.js:4220-4736`):

1. **pointerdown** (`app-v5.js:4393-4513`): if another contact is active: non-pen → ignore; pen → end the
   old stroke as `pointercancel` first (a new pen down is always a new stroke). Accept per § 4.1;
   cache geometry; `clearSelection()`; `preventDefault/stopPropagation`; `setPointerCapture`;
   register in `activeDrawingPointers`; `lastSampleTime = e.timeStamp`; interrupt background work;
   `notePenStrokeStart/notePenActivity`. Create `currentStroke = {id, tool:state.currentTool,
   color, width, points:[{x,y,p}], lastMid:null, finalizedIndex:-1}`, remember its `pageId`, and paint
   immediately: the first point is drawn as a dot on the page canvas (§ 6.5).
2. **pointermove and pointerrawupdate** (both bound to `moveDrawing`, `app-v5.js:4516-4553`):
   * other pointer ids ignored;
   * pen with `buttons===0 && pressure===0` (hover after a lost release) → end the stroke as
     `pointercancel` and **do not** append the hover position;
   * `pointerrawupdate` is only used for pen while a stroke exists;
   * `appendPenSamples(e)` (`app-v5.js:4353-4381`): iterate `e.getCoalescedEvents()` plus the event
     itself if it differs from the last coalesced sample (empty coalesced lists on Android are handled);
     skip samples of another pointer id, samples with `timeStamp < lastSampleTime` (dedupes raw+move),
     non-finite coords, and samples closer than `minDistance` to the last point
     (pen 0.001 px; highlighter `max(0.6, 0.12·width)`; squared-distance compare). No allocation
     of copied arrays, no `Math.hypot`.
   * If any point was appended → `schedulePenPaint()` (one rAF per frame; nothing scheduled for
     duplicate batches).
3. **rAF paint**: `updateLiveStroke` (§ 6.5) then `publishLiveStrokePreview(pageId, stroke)` (collab).
4. **pointerup / pointercancel / pointerleave / lostpointercapture** → `endDrawing`
   (`app-v5.js:4556-4730`):
   * `pointerleave` is ignored while the canvas still holds capture (strokes continue across page edges);
   * on `pointerup` only, append the release samples (coalesced + event);
   * cancel the pending rAF; release capture; arm the 180 ms pen idle timer;
   * resolve the target page **by `pageId`** captured at down (survives remote reorders); if the page
     no longer exists, drop the stroke (and cancel the live preview);
   * `commitLiveStrokeToCanvas` (draw remaining segments on the page bitmap), `simplifyStrokePoints`
     (no-op for pen), push `{id,tool,color,width,points}`, stamp, publish final preview,
     `strokeCount += 1`, enqueue stroke op (durable journal), history `add-stroke`,
     `scheduleSave(false, page, 'stroke')`; clear the cursor overlay.
   * `pointercancel` **commits** the partial stroke (it is not discarded).
5. Document-level safety nets (`app-v5.js:1943-1956`): a `pointerup/pointercancel` for a captured drawing
   pointer that did not pass through its canvas is re-dispatched to the canvas; `window` blur sends
   `pointercancel` to every drawing canvas (commits partial ink).
6. Read-only switched on mid-contact: the contact is discarded (no stroke stored) (`app-v5.js:4565-4587`).

### 5.3 Highlighter

Same pipeline as the pen with: live drawing on the cursor overlay only (§ 6.5); committed to the page
bitmap at lift with the final highlighter renderer; then simplified **after** commit
(`simplifyStrokePoints`, `app-v5.js:914-955`): requires ≥6 points; distance filter keeps points
≥0.4 px from the last kept (first and last always kept); iterative Douglas–Peucker
(`simplifyPoints`, `app-v5.js:891-911`) with ε = 0.45·1.3 = 0.585 and a work budget
`min(250000, 40·n)` (overflow → keep all); if the result keeps < `max(3, ceil(0.5·filtered))` points,
use the filtered list instead. A single-point highlighter is kept and rendered as a dot.

### 5.4 Stroke eraser (`eraser-stroke`)

* Down: snapshot ids for live-erase publishing; erase at the down point; draw eraser cursor.
* Move: `collectEraserPositions(samplesForEvent(e))` (`app-v5.js:4279-4298`): uses coalesced samples;
  keeps a sample if ≥ `max(0.8, 0.18·eraserWidth)` from the last kept one, always the batch's last.
* For each position: `eraseStrokeAtPoint(page, x, y, list, eraserWidth/2)` (`app-v5.js:13405-13428`):
  iterate strokes from top (last) to bottom; skip `eraser*` tools; remove every stroke for which
  `strokeHitsPoint` is true, recording `{index, stroke clone}` and damage.
* `strokeHitsPoint` (`app-v5.js:13324-13342`): bbox prefilter padded by `r + 2 + width/2`; then the
  **centre-line** polyline distance ≤ r (stroke width is *not* added). Single point: point distance.
  Strokes with ≥ 256 points use Rust (§ 13) with identical results.
* Erasing is sampled at positions, not swept between them.
* Up: if anything was erased → tombstones (`trackDeletedStrokeIds`), history `erase-strokes`,
  `scheduleSave`; final live-erase packet; clear cursor.

### 5.5 Area eraser (`eraser-area`)

* Down: snapshot `areaEraseBefore` = shallow copies (points arrays shared — stored point arrays are only
  ever replaced, never mutated) (`app-v5.js:4476-4478`).
* Each position: `eraseAreaAtPoint` (`app-v5.js:13357-13402`): skip strokes with < 2 points and
  `eraser*` strokes; bbox prefilter; `splitStrokeByEraser(stroke, x, y, r)` (`app-v5.js:13212-13264`):
  walk points; a point is *inside* if within r. If a point is outside but the segment from the
  previous point passes within r, the current run is closed and a new run starts at this point.
  An inside point closes the current run. Runs with ≥ 2 points become new strokes
  `{id: new, tool, color, width, points: copies}` (no syncStamp until commit). If the stroke is
  unchanged (one run with all points) it is kept. Result: whole sample-segments touching the circle are
  removed; there is no exact circle clipping.
* Up: stamp every new fragment; tombstone fully removed originals; history `erase-area`
  `{before: clones of removed originals, after: clones of surviving new fragments}` — only touched
  strokes are copied (`app-v5.js:4627-4646`).
* Images, backgrounds and PDF content are never erased by either eraser.

### 5.6 Lasso & selection (§ 4.12)

#### 5.6.1 Drawing the lasso
Tool `lasso`, accepted input per § 4.1 (`app-v5.js:4407-4423`): `clearSelection()`, capture, start
polygon with the down point; each `pointermove` (not rawupdate, not coalesced) appends a point and
redraws (`drawLassoPath`, `app-v5.js:4748-4771`): on the lasso overlay, clear, `strokeStyle #002FD9`,
`lineWidth 1.5`, dash `[6,4]`, open polyline. Lasso does not set `isPenWriting`.

#### 5.6.2 Hit test on release (`selectStrokesWithLasso`, `app-v5.js:5067-5097`)
If the polygon has > 2 points: a stroke is selected if **any** of its points is inside the polygon
(even–odd ray casting with `+0.00001` in the denominator, `app-v5.js:5108-5118`); eraser strokes skipped.
An image is selected if its rotated rectangle intersects the polygon (corner-in-polygon, polygon-point-
in-rect, or edge intersection, `app-v5.js:5120-5174`). Nothing selected → `clearSelection()`. Then the
lasso overlay is cleared and selection outlines drawn.

#### 5.6.3 Selection box (`updateSelectionBox`, `app-v5.js:5213-5253`)
Bounds = min/max of all selected stroke points and image corners (stroke width not included;
`getSelectionBounds`, `app-v5.js:5176-5211`). Box `left/top/width/height` = bounds in logical px
(it scales with zoom since it lives in the scaled container). Classes: remove `hidden`; `has-selection`
when anything selected; `has-image` when images selected. Action bar: above the box
(`bottom: calc(100% + 28px)`) if `minY > 62`, else below (`top: calc(100% + 12px)`); centred
(`left:50%; translateX(-50%)`). Outlines (`drawSelectionOutlines`, `app-v5.js:5545-5581`): on the
lasso overlay, `rgba(0,47,217,0.45)`, width 1.5, dash `[3,3]`, round caps/joins, straight polyline
through each selected stroke's points (strokes with < 2 points skipped; images get no outline).

#### 5.6.4 Transform drags (`setupSelectionBoxEvents`, `app-v5.js:12875-12931`)
Pointerdown on the box (any pointer type; only when tool is lasso and the selection is on this page):

* `.selection-color-swatch` → change colour (§ 5.6.6), close menu.
* `.selection-color-btn` → toggle `.selection-color-menu.open`.
* Anything else closes the colour menu; `.selection-duplicate` → duplicate; `.selection-trash` → delete;
  `.rotate-handle` → rotate; `.selection-handle` → scale with `data-handle`; else → move.
* Start (`startSelectionTransform`, `app-v5.js:12933-12962`): centre = bounds centre; capture original
  strokes/images; `startAngle = atan2(start−centre)`. No pointer capture; document-level
  `pointermove/pointerup/pointercancel` drive it (`app-v5.js:16512-16514`).
* Moves are coalesced to one per rAF (`app-v5.js:12964-12978`). For each frame
  (`applySelectionPointerMove`, `app-v5.js:12979-13089`) a transform `{tx,ty,sx,sy,rot}` is computed
  **from the originals** (not incremental) and applied (`applyTransformToSelection`,
  `app-v5.js:12821-12873`): `p' = centre + R(rot)·S(sx,sy)·(p−centre) + t`; images: centre transformed
  the same way, `width·sx`, `height·sy`, `rotation += rot`. Then the selection box is recomputed
  (axis-aligned; the box never rotates).
  * move: `t = pos − startPos`.
  * scale (always about the centre): `halfW = max(10, w/2)`, `halfH = max(10, h/2)`; corners →
    uniform `max(0.2, |pos−centre| / hypot(halfW, halfH))`; `e/w` → `sx = max(0.2, |pos.x−cx|/halfW)`;
    `n/s` → `sy = max(0.2, |pos.y−cy|/halfH)`.
  * rotate: `rot = atan2(pos−centre) − startAngle`.
  * Cross-page move: if the moved centre goes above `−50` and a previous page exists, or below
    `A4_HEIGHT + 50` and a next page exists → if that page is not loaded, load it and skip this frame;
    else apply the current transform, then `moveSelectionToPage` (`app-v5.js:12733-12819`) with
    `yOffset = +1123` (to previous) / `−1123` (to next): items are spliced out (reverse index order), y
    shifted, appended to the target page, stamped, tombstoned on the source and un-tombstoned on the
    target; selection re-pointed; history `cross-page-move`; both pages marked dirty and redrawn;
    the drag continues on the new page with `startPos` shifted by ±1123.
  * Repaint during drag: images present → full `redrawPage(…, {duringErase:true})`; strokes only →
    damage-region repaint of old+new bounds (§ 6.7).
* Up (`onSelectionPointerUp`, `app-v5.js:13091-13126`): flush the pending frame; if before/after JSON
  differ → stamp, history `transform-selection {before:{strokes,images}, after:{…}}`, `scheduleSave`.

#### 5.6.5 Duplicate (`duplicateSelection`, `app-v5.js:5408-5502`)
Offset `(18, 18)` clamped (`app-v5.js:5419-5434`) so the duplicate's bounds stay within `[6, 794−6] × [6, 1123−6]`
(if impossible on an axis, offset 0). New ids; appended; stamped; history `duplicate-selection`;
selection moves to the duplicates; `setActiveTool('lasso')`; status "Selection duplicated".

#### 5.6.6 Change colour (`changeSelectionColor`, `app-v5.js:5504-5543`)
Only palette colours (case-insensitive map, `app-v5.js:357-360`); only selected strokes with a truthy
colour (pens **and highlighters**); images untouched; image-only selection → no-op. History
`color-change`; redraw; status "Color changed".

#### 5.6.7 Delete (`deleteSelection`, `app-v5.js:5345-5406`)
Remove selected strokes/images (reverse index order), tombstone their ids, history `delete-selection`
(`{strokes:[{index,stroke}], images:[{index,image}]}`), clear selection, redraw, refresh pages list;
status "Selection deleted" (> 1 item) / "Item deleted". (`deleteSelectedImages`, `app-v5.js:5303-5343`, is
unreferenced dead code.)

#### 5.6.8 Clearing the selection (`clearSelection`, `app-v5.js:5050-5065`)
Hide the box (remove `has-selection`, `has-image`, close colour menu), clear the lasso overlay, reset
indices. Triggered by: starting any drawing contact, starting a new lasso, clicking a non-lasso tool
button, touch/mouse down outside the box with the lasso tool, releasing the selected page's canvas,
`renderAllPages`, read-only.

### 5.7 Images on pages

* Sources: document `paste` with image items/files (`app-v5.js:15772-15784`) at the last pointer
  position (else viewport centre); drag-and-drop of image files on `#canvas-viewport`
  (`app-v5.js:12690-12712`, `16520-16521`). Target page = wrapper nearest to the point (distance to rect,
  `app-v5.js:5607-5622`).
* Compression (`compressImageForDocument`, `app-v5.js:12527-12559`): scale =
  `min(1, 3200/max(w,h), sqrt(8 MiPx/(w·h)))`; PNG ≤ 4 MiB at scale 1 is kept byte-exact as a data URL;
  otherwise one encode: PNG stays PNG, else JPEG q 0.94 on white.
* Placement (`addImageToPage`, `app-v5.js:12603-12688`): page resolved by id (loads it if needed);
  `max = (0.6·794, 0.45·1123)`; `scale = min(maxW/natW, maxH/natH, 1)`; centre = drop point +
  `18·n` for the n-th image of a batch, clamped to keep the image inside 794×1123; `rotation 0`.
  Push, stamp, history `add-image`, select the new image, `setActiveTool('lasso')`, redraw,
  status "Image added" / "N images added" / "No usable image in clipboard".
* Images are moved/scaled/rotated/duplicated/deleted only through the lasso selection.

### 5.8 Tool switching (§ 4.14a) and the eraser menu

* Toolbar click (`app-v5.js:15791-15819`): ignore in read-only; remove `.active` from all
  `.tool-btn[data-tool]`, add to the clicked one. Eraser clicked while an eraser tool is already active
  → toggle the eraser menu and stop. Non-lasso → `clearSelection()`. Eraser → `currentTool =
  eraserMode==='stroke' ? 'eraser-stroke' : 'eraser-area'`; otherwise `currentTool = data-tool`,
  `currentColor = data-color`, `currentWidth = parseFloat(data-width)` when present. Non-eraser closes
  the menu. `rememberNonEraserTool(currentTool)` (ignores erasers and lasso).
* Programmatic `setActiveTool(name, {color,width})` (`app-v5.js:5670-5686`) and
  `setActiveToolButton` (`app-v5.js:5641-5664`): highlights the button whose tool matches and, for
  pen/highlighter, whose colour (case-insensitive) and width match.
* `restoreLastNonEraserTool` (`app-v5.js:5692-5704`).
* Eraser menu (`#eraser-menu`): sizes 4 (Tip), 12 (Small, default), 22 (Medium), 32 (Large) set
  `eraserWidth` and the `.active` size button (`app-v5.js:16273-16281`). Switch `#eraser-stroke-toggle`
  checked = stroke mode; changing it updates `eraserMode` and, if an eraser is active, `currentTool`
  (`app-v5.js:16282-16290`). `syncEraserMenuState` on open/init (`app-v5.js:14015-14022`). Opening adds
  `.visible`, `aria-hidden=false`, positions next frame (`positionEraserMenu`, `app-v5.js:14024-14044`):
  vertical toolbar → `left = btn.right + 12`, vertically centred on the button; horizontal toolbar →
  centred above the button `top = btn.top − menuH − 12`; clamped to `[12, innerWidth−w−12]` and
  `[headerOffset+12, innerHeight−h−12−mobileInset]`. Closes on pointerdown outside menu and button
  (`app-v5.js:16299-16304`), on choosing another tool, on read-only. Re-positioned on resize.

---------------------------------------------------------------------------------------------

## 6. Rendering

### 6.1 Layers per active page (bottom → top)

1. `.page-wrapper` CSS background: thumbnail `preview` (`background-size:cover`) or, for non-PDF pages,
   the template JPEG (`100% 100%`) (`applyPagePreview`, `app-v5.js:23505-23537`). Cleared once the
   canvas has painted (`clearPagePreview`, `app-v5.js:24351-24356`; PDF pages keep it until their raster
   exists).
2. `canvas.page-canvas` CSS background: template JPEG URL (`100% 100%`), or the page's background
   image (`100% 100%` for pdf, `cover` for custom), `background-color: transparent`
   (`applyPageBackground`, `app-v5.js:3302-3343`).
3. `canvas.page-canvas` bitmap: background layer + baked raster + legacy covers + images + all strokes
   (§ 6.4). For template/custom/loaded-PDF pages the bitmap is fully opaque.
4. `canvas.lasso-overlay`: lasso polyline and selection outlines.
5. `canvas.cursor-overlay`: live pen tip, live highlighter, eraser cursor.
6. `canvas.remote-live-overlay`: other devices' in-progress strokes (collab spec).
7. `.selection-box`, `.page-number`.

Empty overlays are hidden with `style.visibility='hidden'`, toggled only on state change
(`setOverlayCanvasInk`, `app-v5.js:4883-4895`, comment + function); every drawing path reveals its overlay first.

### 6.2 Backing-store resolution

* Page canvas (`getTargetPageCanvasScale`, `app-v5.js:2440-2452`; `getTargetPageCanvasBackingSize`, `2542-2550`):
  ```
  desired = 2 · max(1, zoom)                     // CANVAS_SCALE = 2 at 100 %
  isPc    = detectDeviceType() === 'pc'          // app-v5.js:27644-27650
  cap     = isPc ? 6 : 4.5
  pxCap   = sqrt((isPc ? 28 : 16)·2^20 / (w·h))
  scale   = max(2, floor(min(desired, cap, pxCap) / 0.25) · 0.25)
  backingW = round(w·scale); applied = backingW / w; backingH = round(h·applied)
  ```
  A4 @ zoom 1 → 1588×2246. Device type: Android UA without "Mobile" → tablet; iPad/tablet/
  touch-Mac ≥768 → tablet; mobile UAs → mobile; else pc. Tablets/mobiles use the touch budget.
* `resizePageCanvas` (`app-v5.js:2552-2577`): when the size changes, copy the previous bitmap
  (if it had a complete paint) into a temp canvas and draw it scaled into the new bitmap so ink never
  disappears during zoom refinement; `dataset.renderScale = applied`. Then context
  `setTransform(applied,0,0,applied,0,0)`, round caps/joins (`app-v5.js:2680-2685`, `2983-2988`).
* Overlays (`app-v5.js:2454-2464`): cursor & remote-live overlay scale `clamp(1.25 + 0.35·(zoom−1), 1.25, 1.8)`;
  lasso overlay `clamp(1.35 + 0.45·(zoom−1), 1.35, 2.2)`; sizes `round(w·s) × round(h·s)`;
  `dataset.overlayScale` stores the applied ratio. (The live pen tip is therefore drawn at a lower
  resolution than the page bitmap until it is committed.)
* Resolution sync (`app-v5.js:2588-2656`): after zoom changes, wait until no interaction for 90 ms
  (`CANVAS_RESYNC_IDLE_MS`), deferring to pen-up while writing; resize only active pages whose target
  size differs; a resized page is marked `needsRedraw` and redrawn. During a pinch the compositor scales
  the old bitmaps (blurry until resync).
* Contexts are plain `getContext('2d')` — **no `desynchronized`** (an e2e test asserts
  `getContextAttributes().desynchronized === false`; v5.16.53's low-latency contexts caused invisible ink
  and were reverted in v5.16.54).

### 6.3 Stroke geometry

#### Pen: centripetal Catmull–Rom through every sample (`appendPenSplineSegment`, `app-v5.js:4779-4800`)
Segment *i* joins `P1=points[i]` and `P2=points[i+1]` using `P0=points[max(0,i−1)]`,
`P3=points[min(last,i+2)]`:
```
if |P2−P1|² < 4:  lineTo(P2)                                        // < 2 px: straight
d1 = |P1−P0|^½ ; d2 = |P2−P1|^½ ; d3 = |P3−P2|^½                    // centripetal α = 0.5
C1 = d1 ≥ 1e-3 ? (d1²·P2 − d2²·P0 + (2d1² + 3d1d2 + d2²)·P1) / (3d1(d1+d2)) : P1
C2 = d3 ≥ 1e-3 ? (d3²·P1 − d2²·P3 + (2d3² + 3d3d2 + d2²)·P2) / (3d3(d3+d2)) : P2
bezierCurveTo(C1, C2, P2)
```
Full stroke = `moveTo(points[0])` + segments 0…n−2, one `stroke()` (`drawSmoothStroke`,
`app-v5.js:13606-13636`); `Path2D` cached in a `WeakMap` keyed by the points array, validated by length
and first/last coordinates. Style (`setupContext`/`setStrokeCaps`, `app-v5.js:13660-13677`, `13501-13509`):
`source-over`, alpha 1, `strokeStyle = color`, `lineWidth = width`, `lineCap = lineJoin = 'round'`.
No pressure modulation, no smoothing of stored points, no prediction (v5.17.20's predicted tip was
removed in v5.17.21). Pen strokes with < 2 points are **not** drawn by repaints (§ 11 Q4).
The same function exists in the PDF/thumbnail worker (`document-processing-worker-v5.js:45-107`).

#### Single-point pen dot (`drawSinglePointStroke`, `app-v5.js:4865-4881`)
Filled circle radius `max(0.7, width/2)` in the stroke colour (used live only).

#### Highlighter (`drawFixedHighlighterStroke`, `app-v5.js:13534-13565`)
`width = max(4, stroke.width || 12)`, colour default `#ffde00`, `globalCompositeOperation='multiply'`,
round caps/joins. 1 point → circle radius `0.52·width`, alpha 0.24. ≥ 2 points → path
`buildSmoothPath` (`app-v5.js:13567-13586`: `moveTo P0`; 2 points → `lineTo P1`; else
`quadraticCurveTo(Pi, mid(Pi,Pi+1))` for i=1…n−2, then `lineTo(Plast)`), stroked twice: alpha 0.22 at
`width`, then alpha 0.10 at `max(2, 0.62·width)`.

#### Legacy eraser-area strokes
`setupContext('eraser-area')` = `destination-out`, opaque — they would punch through the whole bitmap
(including the painted template). They are converted away at boot (§ 1.3).

#### Legacy cover strokes (`drawLegacyCoverStrokes`, `app-v5.js:13754-13773`)
For old PDF documents whose page raster contains burnt-in ink: each cover is redrawn opaque white with
width ×1.35 (pens) or ×1.05 (highlighters, still multiply). Skipped while `pendingCleanOriginalSwap`.

### 6.4 Full page repaint (`redrawPage`, `app-v5.js:13779-13917`)

Preconditions: canvas exists (else `needsRedraw=true`); page loaded (else show preview and load).

* **Coalescing**: if a pending staged paint for this canvas matches (same page object, mutation
  generation, `strokes` array identity and length, canvas size, background, `images` identity, baked
  background), there are no remote erase previews, and the call is not `assetsReady`/`duringErase` →
  return (reuse). Otherwise cancel the pending one.
* **While the pen writes** (and not `duringErase`): do nothing now; record a job and retry every 40 ms
  until the pen idles (the visible bitmap keeps its content).
* **Staged** when `strokes.length > 128` or any stroke has > 2048 points: paint into an offscreen canvas
  (OffscreenCanvas if available) of the same size: background synchronously, then legacy covers one per
  step, then images, then strokes, yielding (`setTimeout 0`) every ≥ 4 ms; pause 40 ms whenever the pen
  is writing or any viewport pointer is held; abort if the canvas disconnects/resizes/page replaced;
  restart if the page's mutation generation or stroke count changed; finally clear and blit the
  offscreen bitmap in one `drawImage`, set `hasPageDraw='1'`, clear the preview.
* **Synchronous** otherwise, in this order with transform `(cw/w, 0, 0, ch/h)`:
  1. `clearRect`;
  2. background layer (`drawPageBackgroundLayer`, `app-v5.js:13679-13733`):
     template page → draw the template at **backing resolution** (identity transform,
     `round(w·max(1,renderScale)) × round(h·…)`); PDF page without raster → queue PDF render, leave
     transparent; otherwise fill white and draw `backgroundImage` stretched to the page;
  3. `bakedBackground` image stretched (legacy);
  4. legacy covers;
  5. images (`drawImagesOnContext`, `app-v5.js:13639-13658`): translate to centre, rotate, draw
     `[-w/2,-h/2,w,h]`; not-yet-loaded images trigger a redraw with `{assetsReady:true}` on load;
  6. strokes in array order (remote live-erase previews applied, `app-v5.js:4114-4146`): skip
     `eraser-stroke`, pens with < 2 points, highlighters with < 1 point; `drawSmoothStroke`.
  Then `needsRedraw=false`, `canvas.dataset.hasPageDraw='1'`, clear the wrapper preview.

### 6.5 Live ink (§ 4.11)

**Pen** (`updateLiveStroke`, `app-v5.js:4980-4998`):
* 1 point: draw the dot directly on the **page bitmap** (transform `(cw/w,0,0,ch/h)`), `finalizedIndex=0`.
* ≥ 2 points, once per frame:
  `finalizeStrokeSegments` (`app-v5.js:4971-4978`) → `drawStrokeTailSegment(ctx, stroke)` (`app-v5.js:4805-4825`):
  draws segments `[liveCurveIndex ?? 0, last−1)` on the **page bitmap** as one path (only segments whose
  following sample is known, so they are geometrically final), sets `liveCurveIndex = last−1`.
  Then `drawLivePenTip` (`app-v5.js:4827-4863`) on the **cursor overlay**: clear the previous tip rectangle,
  draw the newest segment `points[last−1] → points[last]` (spline with `P0=points[last−2]`, `P3=P2`),
  remember its bbox padded by `max(4, width+2)`.
* Pen-up: `commitLiveStrokeToCanvas` (`app-v5.js:4955-4969`) draws the remaining segments up to `last`
  on the page bitmap (`final=true`) with explicit `(cw/w, ch/h)` transform; the cursor overlay is
  cleared. Result: no stored-ink replay during contact; lifting changes only pixels around the last
  sample; live pixels == repaint pixels (up to join antialiasing).

**Highlighter** (`drawLiveHighlighterAppend`, `app-v5.js:4896-4953`): on the cursor overlay only;
first point → highlighter dot; then each frame appends a **polyline** from `points[start−1]` to the
newest sample (multiply, alpha 0.22 at width, then 0.10 at `max(2, 0.62·width)`), accumulating a padded
bbox. At lift the stroke is drawn on the page bitmap with the full highlighter renderer and the overlay
is cleared (the visual changes slightly at lift: polyline → quadratic; per-batch joins were double-blended).

**Eraser cursor** (`drawEraserPreview`, `app-v5.js:5000-5026`): on the cursor overlay, clear the previous
rect, circle radius `eraserWidth/2`, stroke `rgba(0,0,0,0.45)` width 1.2, fill `rgba(255,255,255,0.25)`.
Shown only during contact (no hover cursor). Cleared at lift (`clearEraserPreview`, `app-v5.js:5028-5044`).

### 6.6 Eraser repaint (damage region)

`recordEraserDamage` (`app-v5.js:13131-13146`) unions the bbox of each removed/added/transformed stroke,
padded by `max(3, 1.5·width)`. `scheduleEraserRedraw` (`app-v5.js:13197-13210`) batches pages per rAF.
`repaintEraserDamage` (`app-v5.js:13148-13195`): if the canvas has no complete paint → full
`redrawPage(…, {duringErase:true})`; else cancel any pending staged paint, clip to the damage rect,
`clearRect`, then redraw background layer, baked raster, intersecting legacy covers, **all** images and
every intersecting stroke (stroke bbox padded the same way); mark `needsRedraw=true` so a later full
repaint refreshes the cache. This runs even while the pen is down (erasing must be visible before lift).

### 6.7 Templates (drawn under ink)

* Kinds: `default` (bullet journal, native canvas), `agenda`, `diary` (embedded SVG, A5)
  (`app-v5.js:174-244`).
* Bullet template (`bullet-template.js:41-117`), drawn in backing pixels for canvas size W×H with
  `u = W/21` (geometry is width-based):
  white fill; border `roundRect(1.5u, 1.43u, 18u, 27u, r=0.03u)` stroke `#f0db4c` width `0.06u`;
  dot grid 35 columns × 51 rows at `((2.0 + 0.5c)u, (1.93 + 0.5r)u)`, each dot a stamped radial gradient
  `#a8a8a8` solid to radius `0.035u − 0.02u`, fading to transparent at `0.035u + 0.02u`;
  footer: two boxes `roundRect(2u, 27.43u, 8u, 0.5u)` and `roundRect(11u, 27.43u, 8u, 0.5u)` with
  vertical ticks every 0.5u at x = 2.5…9.5 and 11.5…18.5 (height 0.5u); four circles at
  `y = 27.68u`, `r = 0.125u`, `x = 10.125u + k·0.25u`, filled `#ff0000, #000000, #0000ff, #6eff12`
  and stroked with the border style. Line cap butt, join miter.
* Agenda/diary: white fill + the SVG image drawn stretched to W×H; when the SVG finishes loading,
  caches are flushed and pages re-rendered (`app-v5.js:176-197`).
* Caches: rendered template canvases keyed `kind|WxH` (max 6, FIFO, `app-v5.js:224-238`); CPU
  thumbnail copies (max 6, `app-v5.js:246-270`); CSS background JPEG data URLs per kind at scale
  `clamp(base·(1 + 0.35·(zoom−1)), base, cap)` with base 3.6/cap 4.8 on PC, 3.0/4.2 on touch, JPEG q 0.86
  (`app-v5.js:2466-2506`), regenerated only when a higher scale is needed.

### 6.8 Page backgrounds

* Template: CSS JPEG under a bitmap that also paints the template (§ 6.4).
* Custom (photo page): `backgroundImage` data URL; CSS `cover`; bitmap draws it stretched.
* PDF: pdf.js renders the page at `pdfBackgroundScale` (default `min(3, 2·1.5) = 3`; 1.6 for >10 MB or
  ≥ 30 pages, 1.25 for > 30 MB, 0.9 for > 80 MB, 1.5 for ≥ 200 pages, 1.2 for ≥ 500 pages,
  `app-v5.js:3163-3174`) on white, encoded JPEG q 0.92 to a blob URL (fallback data URL q 0.85)
  (`renderPdfPageToBackground`, `app-v5.js:3405-3512`). Queue processed serially, one page per frame,
  only for active pages, paused/cancelled by any interaction or activity within 240 ms
  (`app-v5.js:3237-3250`, `3345-3403`). At most 6 cached rasters for non-active pages (LRU; evicted
  pages drop `backgroundImage`) (`app-v5.js:3273-3300`).

### 6.9 Thumbnails / previews

* Synchronous page preview (`renderPagePreview`, `app-v5.js:23631-23675`), used when a page is released:
  CPU canvas (`willReadFrequently`) `floor(w·0.18) × floor(h·0.18)`; white; background image (non-template)
  or template thumbnail; baked raster; images; strokes (same renderer); JPEG q 0.6. Returns null for a PDF
  page whose raster is not decoded.
* Scroll previews (`app-v5.js:23540-23629`): serial queue for pages in the visible range ±1 lacking a
  fresh preview; runs every 16 ms but waits while interacting or during a pen burst; canvas
  `ceil(w·s) × ceil(h·s)` with `s = min(0.25, 320/w, 440/h)`; PDF pages rendered directly by pdf.js at
  that size; strokes drawn in ≤ 3 ms slices; JPEG q 0.65; result also written into the IndexedDB page
  record's `preview` only (never its strokes); session/structure/generation guards discard stale work.

### 6.10 Remote live ink overlay (render rules only)
Full redraw uses `drawSmoothStroke`/`drawSinglePointStroke` (`app-v5.js:3748-3775`); incremental appends
are straight `lineTo` polylines with `setupContext` styling (for a remote highlighter: source-over,
alpha 0.35) (`app-v5.js:3777-3807`). Previews expire after 15 s without updates.

---------------------------------------------------------------------------------------------

## 7. Page lifecycle & visible-page tracking (§ 4.8, § 4.10)

* `addPage(bg=null, animate)` (`app-v5.js:2791-2826`): under the local structure lock; append a page
  (§ 1.2), DOM row, `scheduleSave(true)`, status "Page added", optional calendar panel.
* `updateVisiblePages(force)` (`app-v5.js:3574-3731`), scheduled per rAF (`app-v5.js:3733-3742`):
  * while writing: deferred to pen-up; while otherwise interacting: retried every 140 ms;
  * directly visible pages = pages intersecting the viewport (x tolerance 25 % of width), sorted by
    distance of page centre to viewport centre, first 3; candidate range = those ±1;
  * active set (max **3**) in priority: selection page, `activePageIndex`, directly visible, then by
    distance;
  * directly visible pages get canvases immediately (deferred paint if data not loaded) and data loads;
  * others are activated through a priority queue processed in idle callbacks (`timeout 120`, or 90 ms
    timers while interacting) (`app-v5.js:3051-3139`);
  * pages leaving the active set are released (`releasePageCanvas`, `app-v5.js:3141-3161`): preview
    applied, selection cleared if on it, all canvases and the selection box removed, data unloaded.
* `unloadPageData` (`app-v5.js:24485-24574`): flush queued stroke ops; render a preview if missing or
  dirty; revoke PDF blob; save the page record; then `strokes=null; images=null; unloaded=true`.
* `ensurePageDataLoaded` (`app-v5.js:24358-24483`): load from IndexedDB (legacy cache / PDF recovery /
  blank fallback), merge pending stroke ops, assign ids, redraw, clear preview after 100 ms; a page that
  reports strokes but has no data shows "Page N could not be loaded".

---------------------------------------------------------------------------------------------

## 8. Undo / redo (§ 4.11b, `app-v5.js:30803-31289`)

* Stack: `maxHistory = 10` actions and ≤ 25 000 points total (`MAX_HISTORY_MEMORY`, oldest dropped,
  newest always kept; `app-v5.js:1096-1155`). New action truncates redo. Not persisted.
* Actions record `pageId` (resolved by id at undo time) and item ids; restoration matches by id, falling
  back to stored index.
* Types and undo semantics: `add-stroke` (remove), `add-image` (remove), `erase-strokes` (re-insert at
  recorded indices), `erase-area` (remove `after` ids, re-add `before` — appended at the end, § 11 Q6),
  `transform-selection` (replace by id with `before`), `transform-strokes` (legacy), `delete-images`,
  `delete-selection` (re-insert), `duplicate-selection` (remove duplicates), `color-change` (replace),
  `cross-page-move` (move back by ids, y −= yOffset), `replace-page` (page snapshot, other spec).
  Every undo/redo stamps/tombstones as a new causal write, redraws, saves; pages not active are unloaded
  again afterwards.
* Buttons `#btn-undo`/`#btn-redo` disabled when read-only or nothing to undo/redo
  (`updateHistoryButtons`, `app-v5.js:31278-31289`).

---------------------------------------------------------------------------------------------

## 9. DOM, classes, states and CSS (must stay identical)

### 9.1 Editor skeleton (`index.html:9521-9744`)

`#editor-view(.hidden|.read-only)` > `#readonly-banner(.hidden)` + `#app` > `header`
(`.logo#btn-drive-home` [SVG logo, `.brand-name` "inhouse notes", `#doc-title.doc-title`
contenteditable], `.header-actions` [`#presence-avatars`, `#btn-drive-share.hidden`, `#btn-fullscreen`,
`#btn-edit-pages`, `.mode-toggle-group` > `button.mode-toggle-status` (`#save-indicator.save-indicator`,
`#save-status-label`, `#save-timestamp`) + `#mode-toggle-btn.mode-toggle-btn` (`#mode-toggle-label`
"Read"/"Edit", `.mode-toggle-pill > .mode-toggle-knob`)], `#drive-profile-editor` menu incl.
`#theme-toggle-editor`, `#touch-draw-toggle`, `#doc-stats`, `[data-app-version]`), `#presence-popup`,
`#canvas-viewport > #canvas-container`, `#toolbar`, `#eraser-menu`, `#status-bar` (`.status-dot`,
`#status-text`; present but never shown by JS — `statusBar` is unused, CSS opacity 0).

### 9.2 Toolbar markup (`index.html:9639-9715`)

```
div#toolbar
  div#toolbar-handle[title="Drag to move"]  (SVG: six 1.5r dots at (9|15, 6|12|18))
  div.tool-divider
  button.tool-btn[data-tool=pen][data-color=#4D4D4D][data-width=2][title="Black Pen"]
  button.tool-btn.active[data-tool=pen][data-color=#002FD9][data-width=2][title="Blue Pen"]
  button.tool-btn[data-tool=pen][data-color=#E81010][data-width=2][title="Red Pen"]
  div.tool-divider
  button.tool-btn[data-tool=highlighter][data-color=#ffde00][data-width=16][title=Highlighter]
  div.tool-divider
  button.tool-btn#btn-eraser[data-tool=eraser][title=Eraser]
  button.tool-btn[data-tool=lasso][title="Lasso Select"]
  div.tool-divider
  button.tool-btn#btn-undo[title="Undo (Ctrl+Z)"][disabled]
  button.tool-btn#btn-redo[title="Redo (Ctrl+Y)"][disabled]
```
Pen icon: pencil path filled/stroked in the pen colour with a white 1px highlight line; highlighter:
yellow marker with `#1a1a1a` outline; eraser: two-tone (`#f5f5f5` + `#ffb6c1`) block; lasso: outline
loop; undo/redo arrows (stroke-width 2.5). Copy the SVGs verbatim.

### 9.3 Toolbar CSS (`index.html:2553-2790`, `@property --toolbar-rotate` `174-178`)

* `#toolbar`: `position:fixed; left:20px (10px ≤768px); top:50%`; `transform: translateY(var(--toolbar-translate,-50%)) scale(var(--toolbar-scale,1)) rotate(var(--toolbar-rotate,0deg))`, `transform-origin: top left`;
  surface `rgba(255,255,255,.92)` + `backdrop-filter: blur(8px)`, ink `#1a1a1a`, muted `#666`, border
  `1px rgba(0,0,0,.06)`, radius 16, shadow `0 4px 30px rgba(0,0,0,.12)` (`--shadow-medium`), padding 10,
  column flex gap 6, z-index 1000, `transition: border-radius .2s, opacity .25s`; **stays light in dark
  theme** (no dark overrides).
* `.horizontal`: row, nowrap, centred; handle becomes 24×44 and its icon rotates 90°; dividers become
  `1px × 26.4px`, margin `0 4px`. `.snapping`: transition of left/top/right/bottom 0.2 s.
* `.tool-btn`: 44×44, radius 12, transparent, hover `rgba(0,0,0,.05)` (none on touch devices),
  `.active` bg `#ffde00` (`!important` on touch devices, `index.html:7766-7773`), `:disabled` opacity .3,
  icons 20×20 counter-rotated by `--toolbar-rotate`.
* `.tool-divider`: `1px × 26.4px` (0.6·44), `rgba(0,0,0,.1)`, margin `4px 0`.
* Animations: `.toolbar-flash` → `toolbar-fade` 0.45 s opacity .35→1 on entering edit mode;
  `.toolbar-tilt-forward/back` (0.36 s `cubic-bezier(.22,.61,.36,1)`, `--toolbar-rotate` 0→±18°→0) when the
  orientation changes after a snap.
* Read-only: `#editor-view.read-only #toolbar {opacity:0; pointer-events:none}` (`index.html:1266-1269`);
  all toolbar buttons get `disabled`.

### 9.4 Toolbar drag & snap (`setupToolbarDrag`, `app-v5.js:17093-17313`)

Drag only from `#toolbar-handle` (pointer capture); position constrained to `left ∈ [m, innerWidth−w−m]`,
`top ∈ [headerOffset+m, innerHeight−h−m−mobileInset]` (m = 10 while dragging). On release `snapToolbar`
(m = 15, threshold 80 px): nearest of left/right/bottom edges within 80 px → bottom: `.horizontal`, centred
horizontally, `top = innerHeight − h − 15`; left: vertical at `left = 15`; right: vertical with
`right = 15`; else constrain only. `--toolbar-scale = max(0.1, min(1, availableH/h, availableW/w))`.
Position is not persisted; `clampToolbarPosition` (m = 12) runs when the editor is shown
(`app-v5.js:6219-6250`); resize re-clamps. `headerOffset` = header + visible read-only banner height;
`mobileInset` = space taken by the fixed bottom mode-toggle bar at ≤720 px (`app-v5.js:6191-6217`).

### 9.5 Eraser menu (`index.html:9717-9737`, CSS `2790-2870`)

`div#eraser-menu.eraser-menu[aria-hidden]` > `.eraser-menu-title` "Eraser size" +
`.eraser-menu-options` > four `button.tool-btn.size-btn[data-eraser-size=4|12|22|32]` (titles Tip/Small/
Medium/Large, `span.size-dot.xs|sm|md|lg` = 3/6/10/14 px dots, ink colour, opacity .7; 12 initially
`.active`) + `label.eraser-toggle` "Stroke eraser" + `input#eraser-stroke-toggle[type=checkbox]`.
CSS: fixed, hidden unless `.visible` (flex column gap 10, padding 12, radius 12, same light surface &
blur, z-index 1500, min-width 180); title 0.7 rem uppercase, letter-spacing .08em; buttons 40×40
radius 10; switch 38×20 pill `rgba(0,0,0,.15)` → `#1a1a1a` when checked, 14 px white knob moving 18 px.

### 9.6 Selection box markup (`app-v5.js:2932-2964`) and CSS (`index.html:2317-2516`)

```
div.selection-box.hidden[data-page]
  div.selection-rotate-line                 (1×10, rgba(0,47,217,.7), top −18px, centred)
  div.rotate-handle[data-handle=rotate]     (10×10 circle, top −28px, centred, cursor grab)
  div.selection-actions                     (absolute flex gap 4, z 10; shown only with .has-selection)
    button.selection-color-btn[title="Change color"]   (palette-dots icon; contains:)
      div.selection-color-menu               (above button, flex when .open, 3 swatches)
        div.selection-color-swatch[data-swatch-color] ×3   (24×24 circles, hover scale 1.2)
    button.selection-duplicate[title="Duplicate selection"]   (blue icon)
    button.selection-trash[title="Delete selection"]          (red icon)
  div.selection-handle.edge.handle-n|s|e|w[data-handle]  (14×14, radius 3)
  div.selection-handle.handle-ne|nw|se|sw[data-handle]   (14×14 circles)
```
Box: `border:1px dashed rgba(0,47,217,.55); border-radius 8px; background rgba(0,47,217,.03);
touch-action none`. Handles: `rgba(255,255,255,.9)` fill, `1px rgba(0,47,217,.7)` border, opacity .9,
offset `calc(-1px − 7px)` outside the edges, resize cursors. Action buttons 34×34, radius 10, white,
`1px rgba(0,0,0,.1)` border, shadow `0 6px 16px rgba(0,0,0,.12)`, 16 px icons; hover tints
red/blue/grey. Read-only: `pointer-events:none` (`index.html:1284-1286`).

### 9.7 Other editor CSS facts

* `html, body {touch-action:none; overflow:hidden}` (`index.html:223-231`).
* Design tokens (`index.html:185-212`): `--accent-yellow #ffde00`, `--accent-blue #002FD9`,
  `--accent-red #E81010`, `--accent-black #4D4D4D`, `--tool-size 44px`, `--toolbar-padding 10px`,
  `--handle-width 44px`, `--handle-height 24px`, `--icon-size 20px`, `--toolbar-tilt 18deg`,
  `--selection-handle-size 14px`, `--rotate-handle-size 10px`.
* Pages are white in both themes; the viewport background changes (`#2c2c2c` / `#121212`).
* Mode toggle (`index.html:1128-1190`): 111 px wide grid button; pill `rgba(0,0,0,.15)`, orange
  (`#E07A3C`) with knob translated 18 px when `.is-editing`. ≤720 px the `.mode-toggle-group` becomes a
  fixed bottom-centre floating bar (`index.html:383-420`), which the toolbar/eraser menu avoid.

### 9.8 Read/Edit mode (`setReadOnlyMode`, `app-v5.js:2117-2210`)

Toggles `.read-only` on `#editor-view` and `body`, mode toggle classes/ARIA/label, title editability,
disables import/scan/undo/redo/reset/toolbar/eraser controls. Entering read-only also ends gestures,
closes the eraser menu/pages panel/scanner/modals, clears the selection and overscroll. Existing
documents open in **Read** mode; new documents switch to Edit (`app-v5.js:11790`, `11862`); `init`
forces read-only (`app-v5.js:2288`).

---------------------------------------------------------------------------------------------

## 10. Performance-critical paths, constraints and recorded pitfalls

### 10.1 Per pointer event (pen, 120–240 Hz)
Only: id/type checks, coalesced-sample iteration (no array copies, squared distances), point pushes,
and at most one rAF request. **No** canvas work, no layout reads (geometry cached at down), no device
detection, no save/IDB/network work, no collaboration send. Duplicate batches produce zero frames and
zero preview sends (e2e test). Eraser: bbox-prefiltered hit tests (Rust for ≥ 256-point strokes),
damage-region repaint batched per rAF.

### 10.2 Per frame during contact
Draw only the new spline segments on the page bitmap + the newest segment on the cursor overlay
(clear previous tip rect only). Never replay stored strokes, never repaint the page, never resize
canvases, never recompute visible pages, never build thumbnails. Background repaints poll every 40 ms
and yield; staged repaints pause while the pen writes (e2e asserts 0 stored-path `stroke()` calls and 0
page writes while a pen is held, even stationary for 650 ms).

### 10.3 At pen-up
Draw remaining segments (≤ 1 segment changes), push the stroke, enqueue the journal op (flushed on a
microtask unless writing continues), history, stamp; heavy work (page checkpoints, offline copy, Drive
PDF) waits for the 180 ms idle window and, within a burst, 1200 ms quiet. Pen-up must not clone the full
stroke more than necessary (v5.17.7).

### 10.4 Gestures
Geometry (`getBoundingClientRect`, layout bounds) read once per gesture; transform applied in the
current rAF; no canvas resizes or page activation mid-gesture (e2e: 0 resizes and ≤ 3 layout reads over
wheel/pinch sequences). Interaction start interrupts workers/uploads once; held pointers keep work paused.

### 10.5 Pitfalls recorded in comments and release notes (do not regress)
* One path per stroke, not one `beginPath/stroke` per point (v5.11.20).
* Repaint dense pages in cancellable slices; a slice must never overwrite ink added meanwhile
  (generation + stroke-count check) (v5.11.35, test "a stroke added during a staged repaint survives").
* Keep the previous bitmap visible during resolution changes and coalesce identical paints (v5.16.51).
* Erasing must be visible before lift and must preserve template/images/neighbouring ink (v5.11.39).
* `desynchronized` canvas contexts and direct raw-event painting caused invisible ink (v5.16.53 → reverted
  v5.16.54). Use standard contexts and rAF batching.
* Live ink history: midpoint-quadratic + 6-point provisional tail (old) → lasso-layer live ink (v5.16.55)
  → raw polyline through samples (v5.16.57) → definitive segments on the page bitmap during contact
  (v5.16.58) → centripetal Catmull–Rom identical live/repaint (v5.17.20) → no predicted tip (v5.17.21).
* Pen release recovery: hover-without-up ends the stroke without bridging; a new pen down or new pointer
  id is a new stroke; document-level release closes strokes outside the canvas; capture keeps strokes
  across page edges (v5.15.10, v5.16.41).
* Explicit x/y logical-to-bitmap scales during drawing (fractional backing sizes 1.25/2.25/3.5 tested)
  (v5.16.58).
* Android tablets without "Mobile" in the UA must use the touch canvas budget (v5.16.55).
* Hide empty overlays (they were composited every pen frame); thumbnails on CPU canvases to avoid GPU
  readbacks (v5.17.22).
* Pinch → pan transition must reset velocity/time to avoid inertia teleports; drop >100 px deltas.
* Baking strokes into a raster is **disabled** for data-loss reasons (`shouldBakeStrokes → false`,
  `app-v5.js:959-962`); do not reintroduce destructive baking.

---------------------------------------------------------------------------------------------

## 11. Quirks to reproduce or consciously change (current behaviour)

* **Q1** Selection transforms, cross-page moves, duplicate clamping and image insertion use A4 constants
  (794×1123) instead of the page's real size (`app-v5.js:5583-5605`, `12997-13066`, `5424-5434`,
  `12641-12650`). On A5/PDF pages the selection moves faster/slower than the pointer and images are
  placed/clamped as if the page were A4.
* **Q2** Cross-page drag shifts y by exactly ±1123 regardless of real page heights, gap or side panels.
* **Q3** Layout metrics centre narrower pages (`left = 30 + (maxW−w)/2`) but the DOM left-aligns page
  rows (`align-items:flex-start` on the container); and the container padding is 20 px at ≤768 px while
  JS assumes 30 px. Only affects visibility tests/indicator offsets slightly.
* **Q4** A single-point pen stroke is shown as a dot live but is not drawn by any later repaint
  (pens need ≥ 2 points); single-point highlighters persist as dots.
* **Q5** Highlighter live preview (polyline, overlay resolution) differs slightly from its committed
  rendering (quadratic midpoints); the committed render uses unsimplified points, later repaints use the
  simplified ones.
* **Q6** Undoing an area erase re-appends the original strokes at the end of the z-order.
* **Q7** Erasers sample positions; a fast swipe can skip a thin stroke between samples spaced
  ≥ `max(0.8, 0.18·eraserWidth)`. Stroke-eraser hit uses centre-line distance only.
* **Q8** Mouse drawing requires "Draw with finger" to be enabled.
* **Q9** Recolouring a selection also recolours selected highlighter strokes to a pen colour.
* **Q10** `Ctrl+Z/Y` only with lowercase key and Ctrl (no Cmd).
* Dead code not to port: `drawHighlighterCaps`, `ensureHighlighterMask`, `addRoundedRectPath`, `deleteSelectedImages`,
  `getCanvasPos`, `setZoom`, `centerViewport`, `bakeOldStrokes` body, `#status-bar` logic,
  `.color-dot` CSS.

---------------------------------------------------------------------------------------------

## 12. Tests that pin this behaviour

E2E (Playwright, `/?e2e=1` exposes `window.__IHN_TEST_API__`, `app-v5.js:35286-35795`; the v6 build must
provide an equivalent hook):

* `tests/e2e/editor-pen-input.spec.mjs`
  * empty coalesced lists + raw samples + release tail keep a curved 140–210-point pen; no paint before
    the frame, ≤ 3 strokes in a frame; foreign pointer ids and `pointerleave` while captured ignored; ink
    identical after checkpoint+reload.
  * stationary pen on 40/3000-stroke pages: 0 stored-path paints, 0 page writes, snapshot pending until lift.
  * lost releases (hover, new down, new pointer id, outside up) never bridge letters (two strokes, exact x).
  * offline checkpoint yields on 3000-stroke pages and reuses the cached preview; cancellation stops copies.
  * writing bursts: 6 strokes durable between contacts, 0 op-log writes during contact, page snapshots deferred.
  * ink visible before/after reopen, new ink writable, `desynchronized === false`.
  * pen segments definitive before lift: page paints during contact, ≤ 1 extra at lift, 0 lasso-layer paints,
    0 shifted pixels outside ±10 px of the last sample, overlays cleared.
  * SM-X700 UA is `tablet` and ≤ 16 MiPx at zoom 4.
  * 500 duplicate coalesced batches: 0 frames, previews, device detections, `Array.from`, `Math.hypot`.
* `tests/e2e/pen-path-fidelity.spec.mjs` — sharp corner preserved live & stored, points unchanged;
  live position matches backing at scales 1.25/2.25/3.5 (< 0.6 px error, stale transform ignored);
  circle samples: live vs repaint coverage differ < 1 %, ink on the curve (>100 alpha) not on the chord (<30).
* `tests/e2e/editor-repaint-continuity.spec.mjs` — ink stays visible right after `resizePageCanvas` and
  during a delayed repaint; duplicate repaint requests reuse the pending job, `assetsReady` and new ink invalidate it.
* `tests/e2e/editor-performance.spec.mjs` — both erasers clear dense-page ink while the pen is still captured
  (area keeps the edge, stroke removes the whole stroke); remote erase previews repaint; worker ink
  renderer matches main-thread pen/highlighter/eraser pixels (≤ 1 diff); synchronous dense redraw < 30 ms;
  stroke added during staged repaint survives; partial erasing of scanned-size pages copies < 15 000
  points and undo preserves others; background scan repaints never replay stored pens during contact;
  interactions do not discard PDF prep / in-flight uploads.
* `tests/e2e/editor-rust-hit.spec.mjs` — Rust hit test == JS geometry, cache invalidated on edit and
  eviction; eraser calls Rust once on long ink and keeps neighbours.
* `tests/e2e/gesture-priority.spec.mjs` — touch/wheel/keyboard abort active PDF worker and upload;
  wheel/pinch frames: 0 canvas resizes, ≤ 3 layout reads; held touch keeps work paused; previews wait for
  gestures; blur releases held pointers.
* `tests/e2e/fast-scroll.spec.mjs` — thumb drags the editor (aria-valuenow 100 → panY < −500), Home key
  → 0; Home/folder containers; hidden when content fits.
* `tests/e2e/scroll-previews.spec.mjs` — offscreen PDF/ink pages get ≤ 320×440 JPEG previews without full
  canvases; stale previews never attach to another document.
* `tests/e2e/pen-background-priority.spec.mjs` — PDF/checkpoint/upload work pauses or aborts on pen
  contact and resumes after; native sync stops once per burst.
* `tests/e2e/pdf-nondestructive-erase.spec.mjs` — ink stays a removable layer on PDFs (never baked).
* `tests/pen-writing-idle-v5.test.mjs` (node:vm on real source slices) — isolated strokes never wait;
  bursts wait 1200 ms; exit/hidden never wait; offline copy once per burst; deferred view work runs once
  at lift; visible-page tracking reads no layout while writing; empty overlay hidden with 3 style writes
  for 32 calls.
* `tests/history-memory.test.mjs` — history trimming keeps exact actions/undo index, newest kept, grouped
  entries scored once.
* `tests/rust-core.test.mjs` — WASM loads without imports; checkpoint codec bit-exact; PDF ink keeps every
  point/width/colour (plus scanner kernels).
* `rust-core/src/editor.rs` unit test — segment endpoints, zero-length segments, exact radius boundary.

---------------------------------------------------------------------------------------------

## 13. Rust/WASM kernels used by the editor

* `editor_stroke_hit(ptr, count, x, y, radius) -> i32` (`rust-core/src/editor.rs:18-24`): exact polyline
  point-to-segment test over f64 x/y pairs (`windows(4).step_by(2)`), `≤ radius²` inclusive; returns −1 on
  invalid input. JS adapter `strokeHitTest` (`rust-core-v5.js:372-410`): only for 256…262 144 points,
  finite inputs ≤ 1e9; packs coordinates once into WASM memory cached per points-array in an LRU of
  ≤ 64 entries / 4 MiB, invalidated when `generation = "<sessionToken>:<pageMutationGeneration>"` changes;
  `null` → JS fallback. Called from `strokeHitsPoint` (`app-v5.js:13328-13330`).
* Loader: `wasm/inhouse_notes_core.wasm`, ABI `core_abi_version() === 1`, 4 s fetch timeout, no imports
  (`rust-core-v5.js:1-22`).
* Related but outside the live editor: `pdf_ink_path` (PDF vector export, polyline `m/l`, every sample,
  `rust-core/src/pdf_ink.rs`), checkpoint XOR codec for binary ink pages (`rust-core/src/checkpoint.rs`).
  v6 note: if Rust owns the ink model, these must stay lossless (IEEE-754 bits incl. signed zero and the
  pressure-presence lane).

---------------------------------------------------------------------------------------------

## 14. Ten invariants (summary)

1. Stored ink = `{id, tool, color, width, points[{x,y,p}]}` in logical page px; pen points are never
   simplified, smoothed or rounded; pressure is stored but never rendered.
2. Pen rendering = one round-capped path per stroke: centripetal Catmull–Rom (α = ½) through every sample,
   straight below 2 px, identical live, on repaint, in thumbnails and the worker.
3. Live pen: definitive segments go to the page bitmap each rAF; only the newest segment lives on the
   cursor overlay; lift changes pixels only near the last sample; no prediction.
4. Highlighter = multiply, alpha 0.22 @ width + 0.10 @ 0.62·width, quadratic-midpoint path, min width 4.
5. Page bitmap scale = max(2, quantized 0.25 of min(2·max(1,zoom), 6|4.5, pixel cap 28|16 MiPx)); no DPR;
   resize keeps the old bitmap; re-raster only after 90 ms idle.
6. While the pen writes nothing replays stored strokes, resizes canvases, activates pages or saves; full
   repaints of > 128 strokes / > 2048-point strokes are sliced (≤ 4 ms) and atomically blitted.
7. Input: pen always draws; touch/mouse draw only with "Draw with finger", touch also 2 s after pen;
   fingers pan/pinch (zoom 0.5–3, axis lock, >100 px teleport drop, fling 0.935/16.67 ms); pen never pans.
8. Erasers act on vector strokes only: stroke eraser deletes whole strokes whose centre-line is within
   eraserWidth/2; area eraser splits strokes into new-id fragments; damage-region repaint before lift.
9. Selection = lasso (any point inside), axis-aligned box, move/scale-about-centre/rotate from originals,
   cross-page at ±50 px with ±1123 shift, duplicate +18/+18, palette-only recolour; A4 constants quirk.
10. Max 3 active page canvases (selection page, active page, nearest visible first); others are previews;
    undo depth 10 / 25 000 points; DOM ids/classes, toolbar markup and CSS values in § 9 are the UI contract.
