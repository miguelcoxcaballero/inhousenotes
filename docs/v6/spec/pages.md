# v6 spec — Pages, templates, agenda/diary, calendar & side panels

Exact behavioural specification of Inhouse Notes **v5.17.22** for: page records and page
structure (add / insert photo / reorder / delete / reset), page templates (bullet, agenda, diary),
photo ("cover") pages, the Manage-Pages sidebar, the slide-to-confirm modals, agenda/diary detection,
Google Calendar integration, page side-panels (calendar cards) and their event editor, the 5-second
calendar auto-sync, "Add calendar to pages", academic-year agenda generation, the document title and
new-document creation.

Source checkout: commit `2bff515`. Citation prefixes:

| Prefix | File |
|---|---|
| `A:` | `app-v5.js` |
| `H:` | `index.html` (markup + CSS) |
| `B:` | `bullet-template.js` |
| `S:` | `security-core-v5.js` |
| `T:` | `tests/…` |

All coordinates are CSS px unless stated. "Logical page units" = the page's `pageWidth × pageHeight`
coordinate space (A4 = 794×1123). Strings in `'…'` are user-visible and must be reproduced exactly
(including language mix: a few strings are Spanish).

Per `docs/v6/REQUIREMENTS.md`, markup/CSS/assets are kept byte-for-byte and canvas drawing must use
the same Canvas-2D operations in the same order. Where this spec quotes CSS, it gives the **effective
cascade result** (several rules are overridden later in `index.html`); the v6 build should keep the
original stylesheet rather than re-deriving it.

---

## 1. Data model

### 1.1 Page record (`state.pages[i]`)

Created by `createBlankPageData` (A:19737-19758), `addPage` (A:2798-2812), `buildAcademicAgenda`
(A:8986-9007), `createNewDriveDocumentWithFolder` (A:11813-11825), reset (A:21295-21307), PDF import/open
(`importPDFData`, A:32453-32490), revert-to-original (A:14347-14357), storage restore (A:24039-24055), timeline (A:14092-14110).

| Field | Type | Semantics |
|---|---|---|
| `pageId` | string | Stable identity. New: `'pg_' + Date.now().toString(36) + '_' + Math.random().toString(36).slice(2, 11)` (A:743-745). Legacy/Drive-derived: `pg_legacy_<simpleHash(docId + ':' + (index+1))>` (A:747-751, A:357-366). Timeline "revert to original PDF" pages: `original-page-<n>` (`revertToOriginalPdf`, A:14348). Identity survives reorders. |
| `strokes` | array \| `null` | Ink (owned by ink spec). `null` while unloaded (A:24042). |
| `images` | array \| `null` | Placed images. Legacy calendar cards have `id` starting `cal_` and `calendarImportId: 'google-calendar'` (A:18276-18279). |
| `deletedStrokeIds`, `deletedStrokeStamps` | array / object | Causal tombstones (collab spec). |
| `backgroundSource` | `'template' \| 'pdf' \| 'custom'` | Which background layer is drawn. Inferred when missing: `pdfPageIndex ? 'pdf' : (backgroundImage ? 'custom' : 'template')` (A:22430). |
| `templateKind` | `'default' \| 'agenda' \| 'diary'` | Template drawn for `'template'` pages. Anything else normalises to `'default'` (A:307-310). PDF/photo pages keep a (meaningless) `templateKind`, normally `'default'`. |
| `backgroundImage` | data-URL \| blob-URL \| `null` | Photo (`custom`) background as data URL; PDF pages hold a transient rendered blob/data URL that is never persisted (A:22429). |
| `pdfPageIndex` | integer (1-based) \| `null` | Page number inside the backing PDF (`activePdfDocument.getPage(pdfPageIndex)`, A:3319-3332). `0`/`null` = none. |
| `pageWidth`, `pageHeight` | integer ≥ 64 | Logical size. `normalizePageDimension(v, fallback)` = non-finite → fallback, else `max(64, round(v))` (A:2392-2395). |
| `sidePanel` | `{title, html, dateKeys}` \| `null` | Calendar card under the page (§10). Only persisted/rendered in agenda-calendar documents (§8). |
| `legacyCoverStrokes` | array \| `null` | Legacy white-cover strokes (PDF spec). |
| `bakedBackground` | data URL \| `null` | Transient raster cache; never holds durable ink. |
| `preview` | data URL \| `null` | Low-res preview (`PAGE_PREVIEW_SCALE = 0.18`, A:336) shown while unloaded. |
| `strokeCount` | number | Cached visible-stroke count (eraser strokes excluded, A:21136-21148). |
| `unloaded`, `pendingUnload`, `needsRedraw`, `scrollPreviewDirty` | booleans | Runtime lifecycle flags (not persisted semantically). |

Page sizes (A:293-305):

| Constant | Value | Use |
|---|---|---|
| `A4_WIDTH × A4_HEIGHT` | 794 × 1123 | default template and fallback size |
| `A5_WIDTH × A5_HEIGHT` | 559 × 794 | agenda and diary templates |

`getTemplateKindDefaults(kind)` → A5 for `agenda`/`diary`, else A4 (A:300-305).
`getPageDimensions(pageOrIndex)` → normalised `{width, height}` with A4 fallback (A:2397-2404).

### 1.2 Page size labels (A:19866-19911)

- `formatPageMillimetres(px)`: `mm = max(1, px) * 25.4 / 96`; if `|mm - round(mm)| < 0.25` → integer
  string, else `mm.toFixed(1)` with trailing `.0` stripped.
- `formatPageAspectRatio(w, h)`: `landscape = w >= h`; `r = (long/short).toFixed(3)` with trailing
  zeros/dot stripped (`/\.?0+$/`); landscape → `'<r> : 1'`, portrait → `'1 : <r>'`.
- `getStandardPageName(wMm, hMm)`: within ±1.5 mm (either orientation) of 210×297 → `'A4'`,
  148×210 → `'A5'`, 215.9×279.4 → `'US Letter'`, else `''`.
- `getPageSizeInfo(w, h)` → `{standardName, dimensionsLabel: '<w> × <h> mm', ratioLabel}`.
  Examples: A4 → `A4 · 210 × 297 mm · 1 : 1.414`; A5 (559×794) → `A5 · 148 × 210 mm · 1 : 1.42`.
- `getDocumentBasePageDimensions()` (A:19829-19855): the most frequent `w×h` among pages **excluding
  page 1 when there are ≥ 2 pages** (page 1 is treated as a possible cover); ties → earliest. Empty →
  page 1's size or A4.

### 1.3 Document-level fields

| Field | Semantics |
|---|---|
| `state.exportName` | Logical document name (no `.pdf`). Collab register `doc:exportName` (A:1463). |
| `state.driveFileName` | Drive file name incl. `.pdf`; preferred for the title (§7.1). |
| `state.driveFolderKind` | `'agenda' \| 'diary' \| null` — quick-access folder classification (§8). Persisted in the Drive session (`DRIVE_SESSION_KEY`, A:22061) and the open cache (`details.folderKind`, A:11171). |
| `state.calendarPageConfig` | `null` or `{mode: 'day'\|'week', startDateKey: 'YYYY-MM-DD'\|null, startPage: number (0-based), nextDateKey: 'YYYY-MM-DD'\|null}`. Collab register `doc:calendarPageConfig` (A:1464). Key order differs by producer: academic agenda writes `{mode, startPage, startDateKey, nextDateKey}` (A:9005-9006); "Add to pages" writes `{mode, startDateKey, startPage, nextDateKey}` (A:18412-18417). |
| `state.collabStructure` | Causal page-order metadata (collab spec). Page add/move/delete/replace call `noteCollabPageAdded/Moved/Deleted/StructureReplacement` (A:1614-1655). |
| `state.collabFields` | LWW registers. Per page: `page:<pageId>:background` → `{backgroundSource, backgroundImageHash}`, `page:<pageId>:template` → normalised kind, `page:<pageId>:size` → `{pageWidth, pageHeight}`, `page:<pageId>:sidePanel` → `null` or `{title, dateKeys}` (**html is excluded** — it is a device-local rendering cache) (A:1468-1512). |

### 1.4 Persistence surfaces (fields owned by this area)

- **IndexedDB page body** `sanitizePageForStorage` (A:22413-22438): `pageId, strokes, images,
  deletedStrokeIds, deletedStrokeStamps, backgroundImage (null for pdf and blob: URLs), backgroundSource,
  templateKind (normalised), pdfPageIndex, legacyCoverStrokes, pageWidth, pageHeight,
  sidePanel: shouldRenderPageCalendarPanels() ? (sidePanel || null) : null`.
- **Meta payload** `buildMetaPayload` (A:23747-23757): `{sessionId, pages: buildPageMeta[], version: 3,
  savedAt, calendarPageConfig: canUseAgendaCalendarForCurrentDocument() ? config : null,
  collabStructure, collabFields}`; `buildPageMeta` (A:22563-22591) stores `pageId, backgroundImage,
  backgroundSource, templateKind, pdfPageIndex, pageWidth, pageHeight, strokeCount, preview, sidePanel`
  (sidePanel gated as above). On restore the config is rebuilt field by field (A:24060-24066).
- **Drive PDF** (export spec owns the writer): each page entry in `STROKES_Z` contains
  `backgroundSource, templateKind, pdfPageIndex: (pdf || backgroundImage) ? i+1 : null, pageWidth,
  pageHeight, sidePanel (gated)` (A:34178-34194). Document keywords append
  `;IH_CAL:<base64(utf8(JSON.stringify(calendarPageConfig)))>` only when a config exists **and**
  `canUseAgendaCalendarForCurrentDocument()` (A:34386-34392), then `;IH_STRUCT:` and `;IH_FIELDS:`.
  Parsing (A:32209-32233 and A:28849-28871): section ends at next `;`; `> 64 KiB` (`CALENDAR_KEYWORD_SECTION_MAX_CHARS`,
  A:1659) → error; accepted only if `mode` is a string; normalised to the 4 fields; then overridden by
  the `doc:calendarPageConfig` field register when `IH_FIELDS` exists (A:32252-32256).
  Consequence: a **photo page** saved to Drive re-opens as `backgroundSource: 'pdf'` with its
  `pdfPageIndex` (its pixels live in the PDF page).
- **Timeline snapshots** include `templateKind, sidePanel, pageWidth/Height` per page and
  `calendarPageConfig` per entry (A:33273-33290, A:33302); restoring an entry restores the config and
  records a field change (A:14270-14273).

**Invariant (data loss guard):** `sidePanel`, `calendarPageConfig` and `IH_CAL` are only written when
the document is agenda-calendar-capable (§8). Opening an agenda PDF outside an `Agenda` folder and
saving it drops its side panels and config.

---

## 2. Templates

### 2.1 Kinds and selection

`TEMPLATE_KINDS = {DEFAULT: 'default', AGENDA: 'agenda', DIARY: 'diary'}` (A:162). There are **no**
lined/grid/dotted/blank template kinds and no template picker UI. The kind is set only by:

| Origin | Kind | Size |
|---|---|---|
| New PDF (folder picker, §7.2) | `default` | A4 |
| Quick access **Diary** (§7.3) | `diary` | A5 |
| Quick access **Agenda** (§7.3/§7.4) | `agenda` (53-ish weekly pages) | A5 |
| Overscroll "add page" (§3.3) | inherits `normalizeTemplateKind(lastPage.templateKind)`; size = **kind default**, not the previous page's size | per kind |
| Reset document (§6.3, dormant) | `default` (field omitted) | A4 |
| Native scanned page (scanner spec) | `default` + ink strokes/image | A4 portrait/landscape (`getScannedPageLogicalSize`, A:19631-19635) |
| Photo page | `default` (irrelevant: `backgroundSource: 'custom'`) | §5 |

### 2.2 Default kind = bullet-journal template (`B:` exact ops)

`window.InhouseBulletTemplate.render(ctx, width, height, options = {})` (B:42-118); the editor calls it
through `_renderTemplateToCanvas(w, h)` on a fresh `w×h` canvas (A:164-169). Throws `TypeError('A valid
canvas is required for the bullet-journal template.')` on invalid input. Exact sequence, all inside
`ctx.save()/restore()`:

1. Reset state: `setTransform(1,0,0,1,0,0)`, `globalAlpha=1`, `globalCompositeOperation='source-over'`,
   `filter='none'`, `shadowColor='transparent'`, `shadowBlur=0`, `lineCap='butt'`, `lineJoin='miter'`,
   `setLineDash([])`.
2. Unless `options.background === false`: `fillStyle='white'; fillRect(0,0,width,height)`.
3. `u = width / 21` (A4 width = 21 cm; height is not used for geometry). `strokeStyle = '#f0db4c'`,
   `lineWidth = 0.06u`, `cornerR = 0.03u`.
4. Frame: `beginPath(); roundRect(1.5u, 1.43u, 18u, 27u, cornerR); stroke()` (B:68-70).
5. Unless `options.dots === false`: dot stamp (B:18-40, cached per `u`, max 4 entries): `dotR = 0.035u`,
   `blurSigma = 0.02u`, `stampR = dotR + blurSigma`, `stampSize = ceil(2·stampR) + 2`, canvas
   `stampSize²`, `center = stampSize/2`; radial gradient (center, 0 → center, stampR) with stops
   `0 '#a8a8a8'`, `max(0,(dotR−blurSigma)/stampR) '#a8a8a8'`, `1 'rgba(168,168,168,0)'`; `arc(center,
   center, stampR)`, `fill()`. Grid: for `col` 0..34, `cx = (2.0 + 0.5·col)u`; for `row` 0..50,
   `cy = (1.93 + 0.5·row)u`; `drawImage(stamp, cx − center, cy − center)` (35 × 51 = 1785 dots,
   column-major order, sub-pixel positions not rounded).
6. Footer boxes (`lineY = 27.43u`, `lineH = 0.5u`): `roundRect(2.0u, lineY, 8.0u, lineH, cornerR)`
   stroke; `roundRect(11.0u, lineY, 8.0u, lineH, cornerR)` stroke; then **one** path of vertical ticks
   `x = 2.5…9.5 step 0.5` and `x = 11.5…18.5 step 0.5` (`+1e-9` tolerance), each `moveTo(xu, lineY)
   lineTo(xu, lineY+lineH)`, single `stroke()` (B:83-100).
7. Colour markers: `circleY = 27.68u`, `r = 0.125u`, colours `['#ff0000','#000000','#0000ff','#6eff12']`
   at `x = 10.125u, 10.375u, 10.625u, 10.875u`; for each: `fillStyle=c; beginPath(); arc(x, circleY, r,
   0, 2π); fill(); stroke()` (stroke = yellow ring, `lineWidth 0.06u`) (B:102-111).
8. Returns `ctx.canvas`.

The same module is used by the scanner preview/worker (scanner spec) and must stay byte-identical
(`sha256 6a0b9bcf…0088`).

### 2.3 Agenda and Diary kinds = embedded SVG rasterised

- Source: `_AGENDA_SVG_B64` / `_DIARY_SVG_B64` (A:172-173) are base64 of `Agenda.svg` / `Diary.svg`
  **byte-for-byte** (verified: sha256 `3a68d0a8…18f1` and `a3696fa4…a1df`). `AGENDA_TEMPLATE_SVG`
  (A:402) is an unused plain-text copy.
- Image: `new Image()`, `src = 'data:image/svg+xml;base64,' + b64`, cached per kind (A:176-197).
  `img.onload` → `_tplCache.clear()`, `clearTemplateBackgroundUrlCache()`,
  `warmTemplateBackgroundUrlsForDoc()`, re-set `wrapper.style.backgroundImage` of every
  `.page-wrapper` whose page is `backgroundSource === 'template'`, then
  `scheduleVisiblePageUpdate(true)`.
- Raster (A:199-208): new `w×h` canvas, `fillStyle='#ffffff'; fillRect(0,0,w,h)`; if
  `img.complete && img.naturalWidth > 0` → `drawImage(img, 0, 0, w, h)`. Before the SVG decodes the
  template is plain white (and that white canvas is cached until `onload` clears the cache).
- SVG viewBox `0 0 559.3700787 793.7007874` (A5 at 72 dpi-ish) stretched non-uniformly to the target.
  Contents (viewBox units; all strokes `stroke-width 0.7165354`, `stroke-linecap butt`,
  `stroke-linejoin round`; text is outlined glyph paths in black):
  - **Diary** (`Diary.svg`, 76 paths): white page; black rule `y 98.19, x 34.41→524.96`; 36 grey
    (`#666666`) rules `x ≈52.25→507.12` at `y = 150.45 + 16.49·k`, k = 0…35 (last `727.63`). No text.
  - **Agenda** (`Agenda.svg`, 128 paths; Spanish weekly layout): title glyphs **"Semana _/_ - _/_"** (x 126.8→435.1,
    y 53.6→90.1); black rule `y 101.44, x 35.16→525.71`; left column day blocks **"Lunes _/_",
    "Martes _/_", "Miércoles _/_", "Jueves _/_"** with black header rules `x 33.66→360.59` at
    `y 149.65, 288.59, 427.54, 566.48` and 5 grey rules each (`x 45.56→348.71`, at header + 31.0,
    47.5, 64.0, 80.5, 97.0); right column: rounded box **"Objetivos"** (fill `#d9ead3`, black stroke,
    corner r ≈ 10.61, x 373.68→524.17, y 125.22→263.11) with a yellow `#ffff00` highlight behind the
    label (x 416.18→481.76, y 130.15→147.36), header rule y 149.65 and 5 grey rules
    (`x 387.04→510.85`); **"Viernes _/_", "Sábado _/_", "Domingo _/_"** with header rules
    `x 373.70→524.19` at y 288.59/427.54/566.48 and 5 grey rules each (`x 387.07→510.88`); footer
    black rule `y 722.29, x 35.16→525.71`; footer glyphs **"__-__"** (x 260.0→300.8, y 741.8→748.4).
  - v6 must ship these exact SVG bytes and rasterise them with the browser (`drawImage` of the SVG
    image) — any re-implementation of glyphs would not be pixel-identical.

### 2.4 Template caches and render paths

| Path | Code | Behaviour |
|---|---|---|
| Core cache | `drawTemplateBackground(ctx, w, h, scale, kind)` (A:224-237) | Key `"<kind>\|<w>x<h>"` in `_tplCache` (Map, **max 6 entries**, FIFO eviction of the oldest key). Renders via §2.2/§2.3 at exactly `w×h` and then `ctx.drawImage(cached, 0, 0)` if `ctx` given. `scale` is ignored. |
| Live page canvas | `drawPageBackgroundLayer` (A:13679-13695) | For `backgroundSource === 'template'` without `backgroundImage`: `targetW = round(pageWidth × max(1, renderScale))`, `targetH = round(pageHeight × …)`; `ctx.save(); setTransform(1,0,0,1,0,0); drawTemplateBackground(ctx, targetW, targetH, 1, kind); restore()` — i.e. the template is rendered **in backing pixels** (bullet `u = backingWidth / 21`), so zoom increases real detail. `T:e2e/scanner-native-template.spec.mjs` asserts the footer/blank-grid pixels equal `InhouseBulletTemplate.render` at the canvas backing size. |
| CSS background URL | `ensureTemplateBackgroundUrl(kind)` (A:2478-2506), `getTargetTemplateBackgroundScale` (A:2466-2473) | Per kind JPEG data URL (`toDataURL('image/jpeg', 0.86)`) of the template at the kind's **default** size × scale. Scale: PC base 3.6 cap 4.8, touch base 3 cap 4.2; `scale = max(base, min(cap, base × (1 + (max(1, zoom) − 1) × 0.35)))`. Re-rendered only when the target scale exceeds the cached one by > 0.01. Stored in `window._templateBgUrlCache/_templateBgScaleCache`. |
| CSS on canvas | `applyPageBackground` (A:3302-3343) | Template pages: `canvas.style.backgroundImage = url("<jpeg>")`, `backgroundSize: '100% 100%'`, `no-repeat`, `center`, `backgroundColor: transparent`. Photo/PDF image: `backgroundSize` `'100% 100%'` for pdf, `'cover'` for custom. |
| Wrapper placeholder | `renderAllPages` (A:13947-13960) | Before canvases exist: `page.preview` as `cover`, else (non-pdf) template JPEG URL `100% 100%`. |
| Pages-panel thumbnail | `generateThumbnail` (A:21067-21116) | `fillStyle='white'` fill, then `drawTemplateBackground(ctx, thumbW, thumbH, 1, kind)` at the thumbnail **backing** size (2× CSS). |
| Scroll previews | `renderPagePreview` (A:23631-23663) → `drawTemplateBackgroundThumbnail` (A:246-269) | Renders the full-size template once, then a CPU-downscaled copy (`_tplThumbCache`, key `kind\|WxH\|TWxTH`, max 6, `willReadFrequently`, materialised with `getImageData(0,0,1,1)`), drawn `drawImage(copy, 0, 0, w, h)`. |
| PDF export | A:34087-34120 | Template embedded as JPEG quality **0.85** at `round(pageWidthPx × overlayScale)` (export spec). |

`warmTemplateBackgroundUrlsForDoc()` (A:2515-2525) pre-renders URLs for `default` plus every kind used
by template pages.

---

## 3. Page DOM, layout and structure operations

### 3.1 Page DOM (A:13919-13990, A:2830-2885)

```html
<div id="canvas-container">                 <!-- flex column, align-items:flex-start, padding 30px, gap 30px -->
  <div class="page-row" data-page-row="{i}"> <!-- flex column, align-items:center, gap 14px, flex-shrink 0 -->
    <div class="page-wrapper" data-page="{i}" style="width:{w}px;height:{h}px">
      <!-- canvases inserted lazily before .page-number: canvas.page-canvas, canvas.lasso-overlay,
           canvas.cursor-overlay, canvas.remote-live-overlay, div.selection-box (editor spec) -->
      <div class="page-number">{i+1}</div>
    </div>
    <!-- optional: div.page-side-panel (§10.3), only in agenda-calendar documents -->
  </div>
  …
  <div id="add-page-indicator">+</div>       <!-- always re-appended last (A:2112-2115, A:13985) -->
</div>
```

CSS (effective):
- `#canvas-container` (H:1467-1478): `position:absolute; top:0; left:0; transform-origin:0 0;
  will-change:transform; display:flex; flex-direction:column; align-items:flex-start; padding:30px;
  gap:30px` (`padding:20px` at `max-width:768px`, H:4245-4247).
- `.page-row` (H:1656-1661 + H:3432-3438): `display:flex; flex-direction:column; align-items:center;
  gap:14px; flex-shrink:0`.
- `.page-wrapper` (H:2270-2277): `position:relative; box-shadow:0 4px 20px rgba(0,0,0,0.4);
  border-radius:2px; overflow:hidden; flex-shrink:0; background:white`. `.page-wrapper.new-page` →
  `animation: pageFadeIn 0.5s ease` (opacity 0→1, H:2279-2296); class added on overscroll-created pages
  and removed after 600 ms (A:2843-2846).
- `.page-number` (H:2537-2545): `position:absolute; bottom:10px; right:15px; font-size:0.75rem;
  color:var(--text-secondary); pointer-events:none; opacity:0.6`.

Layout model (A:16656-16711, A:2423-2428): pages stack vertically starting at `y = PAGE_PADDING (30)`;
each page occupies `height + (sidePanel && calendarEnabled ? 184 + 14 : 0)` plus `PAGE_GAP (30)`
between pages; the metrics assume pages horizontally centred on the widest page (`left = 30 + (maxW −
w)/2`), **but the DOM left-aligns every row** (`align-items:flex-start` on `#canvas-container`): in a
mixed-width document a 559 px page sits at x = 30 while the metrics say 147.5 (measured in Chromium).
`goToPage`/visibility use the metrics; drawing hit-tests use the canvas element itself.
`PAGE_CALENDAR_PANEL_HEIGHT = 184`, `PAGE_CALENDAR_PANEL_GAP = 14` (A:329-330) must match the CSS
(`.page-side-panel` height 184 + row gap 14). `cachedPageLayoutCalendarEnabled` flips with
`updateCalendarFeatureAvailability()` and invalidates bounds (A:5837-5840).

### 3.2 Structure-mutation lock (shared by every structural edit)

`acquireLocalPageStructureMutation(timeoutMs = 12000, expectedContext)` (A:20233-20297):
- If a prepared Drive PDF build exists for the same document, abort it (structural edits invalidate
  the export page map).
- Wait (poll 32 ms, or await the specific promise) while any of: another local structure token, a
  remote merge token, `hasSmoothInteraction()`, active page-store writes, stroke-op flush in
  progress/promise, collab page-order mutation, pending collab page-store rewrite.
- Document changed (`sessionToken`/`driveFileId`) → return `null` silently. Timeout → status
  `'Still syncing page order. Try again in a moment.'` (error, preserveState) → `null`.
- Returns token `{cancelled, sessionToken, fileId}`; `assertLocalPageStructureContext(token)` throws
  `'Local page-structure document changed'` when stale; `releaseLocalPageStructureMutation(token)`.
- `ensureAllPagesLoadedForStructureChange` loads every page body (`keepLoaded`), yielding every 4
  pages; any failure throws `Could not load page N`.

Every structural operation (add, insert photo, insert scan, move, delete, reset, add calendar) runs
under this token. Page indices are re-resolved from `pageId` after waiting (anchors survive concurrent
reorders).

### 3.3 Add page (overscroll at the bottom) — `addPage` (A:2791-2827)

There is **no "add/insert/duplicate page" button** (`duplicate` exists only for lasso selections).
New blank pages are created only by pulling past the end of the document in edit mode:

- Constants (A:340-343): `OVERSCROLL_THRESHOLD 360`, `OVERSCROLL_FRICTION 0.24`, `OVERSCROLL_MARGIN 100`,
  `CREATE_PAGE_MIN_BOTTOM_RATIO 0.45`.
- `applyVerticalPan(dy)` (A:16942-17007): bottom limit `minPanY = viewportH − 100 − contentHeight·zoom`.
  Dragging further accumulates `overscrollY` (cap `max(1080, required×1.1)` where
  `required = max(0, (viewportH × 0.55 − 100) / 0.24)`), `panY = minPanY − overscrollY × 0.24`.
  `canCreate = (panY + contentHeight·zoom) < viewportH × 0.45`; `pendingPageCreate = overscrollY ≥ 360 && canCreate`.
  An idle 600 ms resets overscroll to 0 (snaps back).
- Indicator `#add-page-indicator` (A:16800-16828): `left:50%; top: (contentHeight − 30 + 20)px;
  transform: translateX(-50%)`; opacity `0` when read-only/no pages/no overscroll, else
  `min(1, 0.2 + 0.8·min(1, overscrollY/360))`, ×0.6 when `!canCreate`. CSS (H:2518-2535): 56×56 circle,
  `border:2px dashed rgba(255,222,0,0.95)`, `color` same, `font-size:28px; font-weight:600`,
  `background:rgba(20,20,20,0.25)`, `box-shadow:0 6px 18px rgba(0,0,0,0.25)`,
  `transition:opacity 0.5s ease`, `pointer-events:none`; read-only forces `opacity:0 !important` (H:1280).
- `finalizePendingPageCreation()` on gesture end (A:17009-17033): if still pending and `canCreate` →
  `addPage(null, true)`, reset overscroll, `animatePanTo(previous minPanY)` (`transform 0.35s ease`
  on `#canvas-container`, removed after 360 ms).
- `addPage(backgroundImage = null, animate)` under the lock: new page
  `{pageId, strokes:[], backgroundImage, images:[], backgroundSource: backgroundImage ? 'custom' :
  'template', templateKind: inherited, pdfPageIndex:null, pageWidth/Height: kind defaults,
  needsRedraw:true, preview:null, strokeCount:0, unloaded:false}` appended; `noteCollabPageAdded`;
  `createPageElement(i, animate)`; `scheduleVisiblePageUpdate(true)`; `scheduleSave(true, i)`; status
  `'Page added'`. If `calendarPageConfig` exists, edit mode and agenda-capable →
  `spAutoAttachCalendarPanel(i)` (§11.2). Undo history is **not** cleared.

### 3.4 Insert photo page — `insertPhotoPageAt` (A:20374-20451)

Called by the photo modal (§5). Under the lock: anchor by `pageId` of the page to insert after;
load all pages; clear selection; `clearStrokeOpsStore()` (failure → throw `'Could not reset pending
page operations'`); reset dirty/access/sync tracking; create
`createBlankPageData({backgroundImage: dataUrl, backgroundSource: 'custom', pageWidth, pageHeight})`
(templateKind `default`); insert at `clamp(insertAfter + 1, 0, length)`; `noteCollabPageAdded`;
`activePageIndex = insertAt`; **undo history cleared** (`history = []`, `historyIndex = -1`);
`reindexPagesAfterInsert` shifts IndexedDB rows high→low in one transaction (A:20559-20611);
`renderAllPages()`, `renderPagesList()`, `markAllPagesDirty('full')`, `scheduleSave(true)`,
`goToPage(insertAt)`.

`insertScannedPagesAt` (A:20453-20557) is the same procedure for 1..24 scanned pages (scanner spec):
native ink scans become `template/default` pages with editable strokes (or a full-page image item),
other scans become `custom` photo pages.

### 3.5 Reorder (drag in the Pages sidebar) — `initPageListDrag` / `movePage` (A:20771-20984)

Binding is installed once per list element (`WeakSet`), so reopening the panel never stacks handlers
(`T:e2e/pages-panel-performance.spec.mjs` "only one gesture per drag").

Start (`pointerdown` on `.page-drag-handle`): ignored if `e.button > 0`, `isPrimary === false`,
read-only, a drag already active, a local structure token or a remote merge token exists, or the
page has no `pageId`. Then `preventDefault/stopPropagation`; measure every `.page-item` once
(`top` in list-content coordinates, `height`); build ghost:

```html
<div class="page-reorder-preview" aria-hidden="true"><canvas (copy of the thumbnail)/><span>{n}</span></div>
<div class="page-reorder-indicator" aria-hidden="true"></div>   <!-- both appended to <body> -->
```

Item gets `.is-dragging` (opacity 0.4); list gets `.is-reordering`; `pageStructureDragInProgress = true`;
`list.setPointerCapture`. Document listeners: `pointermove` (passive:false, only same `pointerId`),
`pointerup`, `pointercancel`, `keydown` (Escape cancels), list `lostpointercapture` (cancel), window
`blur` and `resize` (cancel).

Per animation frame (`tick`):
- Cancel if item detached, document changed, read-only, or `#pages-panel` lost `.visible`.
- `dt = min(40, now − prev)` ms (0 on first frame). `edge = min(88, listRect.height / 3)`. When the
  pointer x is within `[left − 48, right + 48]`: above `top + edge` → `speed = −min(1, (top + edge − y)/edge)`;
  below `bottom − edge` → `speed = min(1, (y − bottom + edge)/edge)`.
  `list.scrollTop += sign(speed) · speed² · 1050 · dt / 1000` — continues while the pointer is
  stationary (pinned by `T:e2e/page-reorder.spec.mjs`).
- Ghost: `transform: translate3d(x + 14px, y − 48px, 0)`.
- Target (`updateTarget`): `within = x ∈ [left−48, right+48] && y ∈ [top−64, bottom+64]`; if not, hide
  the indicator and no destination. `contentY = clamp(y − top, 0, height) + scrollTop`; binary search
  the first row whose midpoint is below `contentY` (`before = true`), or after the last row
  (`before = false`). Destination is `null` when that row is the dragged page. Indicator:
  `width = listWidth − 20`, `transform: translate3d(listLeft + 10px, clamp(lineY, top, bottom − 3)px, 0)`
  with `lineY = top + row.top + (before ? 0 : row.height) − scrollTop`; hidden when no destination.

Finish: remove listeners/ghost/indicator/classes, release capture, flag off. On drop with a destination:
`movePage(fromIndex, row.index, {movingPageId, targetPageId: row.pageId, insertBeforeTarget: before})`;
on success (and not `prefers-reduced-motion`) the moved row animates
`[{opacity:.55, transform:'scale(.97)'}, {opacity:1, transform:'scale(1)'}]`, 180 ms `ease-out`;
on failure status `'Could not move the page. Try again.'` (error).

`movePage` (A:20918-20984), under the lock: re-resolve `fromIndex` by `movingPageId`; with a semantic
target `toIndex = targetIndex + (before ? 0 : 1)`, minus 1 if `from < to`; clamp; no-op if equal.
Checkpoint: mark pages with pending stroke ops dirty, `flushDirtyPages({force:true})`, then
`clearStrokeOpsStore()`; failure → `'Could not safely move the page. Try again.'` (error). Splice,
`noteCollabPageMoved(pageId, toIndex)`, remap `activePageIndex` (moved page follows; pages in between
shift by ±1), `reindexPagesAfterMove` (read range, rotate, rewrite in one transaction,
A:20986-21065), `renderAllPages()`, `markAllPagesDirty('full')`, `scheduleSave(true)`,
`renderPagesList()`. Undo history is not cleared.

### 3.6 Delete page — `openDeleteModal` / `confirmDeletePage` (A:21172-21260)

- Allowed only in edit mode with ≥ 2 pages (button otherwise `disabled aria-disabled="true"`).
- Opens the generic slide-to-confirm modal (§6.1) with title `'Delete page?'`, message
  `'This will delete page {n} and all its strokes. This cannot be undone.'`, slider `'Slide to delete'`.
  The static `#modal-delete-page` (H:9751-9761, "Delete" button wired to `confirmDeletePage`) is legacy
  and never shown.
- Confirm, under the lock: resolve by `pageIdToDelete`; load all pages; revoke a blob PDF background;
  checkpoint as in move (failure → `'Could not safely delete the page. Try again.'`); splice;
  `noteCollabPageDeleted`; `reindexLocalCachesAfterDelete`; `reindexPagesAfterDelete`;
  `renderAllPages()`; `markAllPagesDirty('full')`; `goToPage(min(deleteIndex, last))`;
  `scheduleSave(true)`; `renderPagesList()`; status `'Page deleted'`. Undo history not cleared.

### 3.7 `goToPage(i)` (A:21153-21165)

If the wrapper exists: `panY = −pageTop·zoom + viewportH/2 − pageH·zoom/2`,
`panX = viewportW/2 − pageW·zoom/2 − 30·zoom`; `updateTransform()`; `activePageIndex = i`;
`scheduleVisiblePageUpdate(true)`. Zoom is unchanged.

---

## 4. Manage-Pages panel ("Pages" sidebar)

### 4.1 Markup (H:9958-10022) — keep byte-for-byte

```html
<div class="panel-overlay" id="panel-overlay"></div>
<div id="pages-panel" role="dialog" aria-label="Pages">
  <div class="panel-header"><h2>Pages</h2>
    <button class="panel-close" id="panel-close" type="button" aria-label="Close pages"><svg ×/></button></div>
  <div class="pages-tools">
    <button class="pages-tool" id="btn-open-timeline" title="Version Timeline" aria-label="Open version timeline">…<span>Timeline</span></button>
    <button class="pages-tool" id="btn-open-calendar" title="Calendar" aria-label="Open calendar">…<span>Calendar</span></button>
    <button class="pages-tool" id="btn-set-cover" title="Add Photo Page" aria-label="Add Photo Page">…<span>Photo</span></button>
    <button class="pages-tool" id="btn-scan-page" title="Scan pages" aria-label="Scan pages">…<span>Scan</span></button>
    <button class="pages-tool" id="btn-import-pdf" title="Import PDF" aria-label="Import PDF">…<span>Import</span></button>
    <button class="pages-tool pages-tool-primary" id="btn-export-pdf" title="Export PDF" aria-label="Export PDF">…<span>Export</span></button>
  </div>
  <div class="pages-default-size" id="pages-default-size" aria-live="polite"></div>
  <div class="pages-list" id="pages-list"></div>
</div>
```

Opened by the header button `#btn-edit-pages` (`title/aria-label "Manage Pages"`, page-with-line icon,
H:9566-9573). `#btn-scan-page` must stay directly after `#btn-set-cover` (`T:release-v5-smoke.mjs:72`).
Timeline/Import/Export/Scan behaviour belong to other specs; this spec owns open/close, Calendar
visibility, Photo, the default-size line and the list.

### 4.2 Open / close

- `openPagesPanel()` (A:19479-19489): `updateCalendarFeatureAvailability()`;
  `renderPagesList({force:true})`; `#btn-set-cover.disabled = #btn-scan-page.disabled = isReadOnly`;
  set CSS var `--pages-top = max(0, round(editorHeader.getBoundingClientRect().bottom)) + 'px'` on the
  panel; add `.visible` to `#pages-panel` and `#panel-overlay`.
- `closePagesPanel()` (A:19491-19495): cancel an active drag, remove both `.visible`.
- Close triggers: `#panel-close`, click on `#panel-overlay` (transparent, covers the screen under the
  panel), choosing a page, entering read-only (A:2172), opening the scanner. Escape does **not** close it.
- The panel can be opened in read mode: Photo/Scan disabled, no drag handles, delete disabled.

### 4.3 Effective CSS (light; dark via tokens unless stated) — H:3106-3160, H:3256-3282, H:3972-4205, H:6178-6192, H:8950-9190, H:4848-4908

Tokens (H:185-221): light `--bg-primary #f5f5f0, --bg-secondary #ffffff, --text-primary #1a1a1a,
--text-secondary #666666, --accent-orange #E07A3C, --radius-sm 8px, --radius-md 12px, --radius-lg 16px,
--shadow-medium 0 4px 30px rgba(0,0,0,.12)`; dark `--bg-primary #151515, --bg-secondary #1c1c1c,
--text-primary #f5f5f5, --text-secondary #a2a2a2, --shadow-medium 0 4px 30px rgba(0,0,0,.45)`.
Global `* {margin:0; padding:0; box-sizing:border-box}`; `.hidden {display:none !important}`.

| Element | Effective style |
|---|---|
| `#pages-panel` | `position:fixed; left:0; right:auto; top:var(--pages-top,64px); bottom:0; width:var(--pages-w); --pages-w:176px` (`136px` at `max-width:720px`); `max-width:100vw; height:auto; border-radius:0; background:var(--bg-primary); border-right:1px solid rgba(0,0,0,0.08); box-shadow:8px 0 28px rgba(0,0,0,0.12); transform:translateX(-105%); visibility:hidden; transition:transform .2s ease, visibility 0s linear .2s; z-index:1500; display:flex; flex-direction:column`. The older right-slide and ≤720 px bottom-sheet rules are fully overridden. |
| `#pages-panel.visible` | `transform:none; visibility:visible; transition:transform .2s ease` |
| `#panel-overlay` | `position:fixed; inset:0; background:transparent; z-index:1400; opacity:0; visibility:hidden; transition:all .3s ease`; `.visible` → `opacity:1; visibility:visible` |
| `.panel-header` | `display:flex; align-items:center; justify-content:space-between; padding:10px 8px 6px 14px; border-bottom:0`; `h2 {font-size:.95rem; font-weight:700}` |
| `#panel-close` | `32×32; border:none; background:transparent; color:var(--text-primary); border-radius:12px; display:flex; center; transition:background .2s`; hover `rgba(0,0,0,.05)` (dark `rgba(255,255,255,.08)`); svg 20×20 |
| `.pages-tools` | `display:grid; grid-template-columns:repeat(2,1fr); gap:6px; padding:2px 10px 8px` |
| `.pages-tool` | `display:flex; flex-direction:column; align-items:center; gap:3px; min-width:0; padding:8px 2px 7px; border:1px solid rgba(0,0,0,.06); border-radius:12px; background:var(--bg-secondary); color:var(--text-primary); font:600 .68rem 'DM Sans', system-ui, sans-serif; cursor:pointer`; `svg 20×20`; `span {max-width:100%; overflow:hidden; text-overflow:ellipsis; white-space:nowrap}`; `:hover:not(:disabled)` border `rgba(224,122,60,.45)`; `:focus-visible` `outline:2px solid var(--accent-orange); outline-offset:2px`; `:disabled {opacity:.45; cursor:default}`; `.hidden {display:none}` |
| `#btn-export-pdf` (`.pages-tool-primary`) | `background:#1a1a1a; border-color:#1a1a1a; color:#fff`; dark `#f4f1ea / #f4f1ea / #1a1a1a` |
| `#btn-open-calendar` (ID rules win over `.pages-tool`) | `border:1px solid transparent` then `background:#f5c84c; border-color:#d9a932; color:#2e250d`; hover `background:#edbd3c` (border stays `#d9a932`); dark `#d9ad38 / #f0c24f / #1e1808` |
| `#btn-scan-page` | `#pages-panel #btn-scan-page` neutralises the accent tint: `border-color:rgba(0,0,0,.06); background:var(--bg-secondary)`; hover border `rgba(224,122,60,.45)` |
| `.pages-default-size` | `display:block; margin:0 10px 4px; padding:6px 4px; border:0; border-top:1px solid rgba(0,0,0,.07)` (also in dark: the dark border rule has lower specificity); `background:transparent; font-size:.66rem; line-height:1.35; text-align:center; color:var(--text-secondary)`; `span, strong {display:block}`; `strong {font-weight:600; color:var(--text-primary); font-size:.78rem; font-variant-numeric:tabular-nums; text-align:right}` |
| `#pages-list` | `flex:1; overflow-y:auto; padding:6px 10px 18px; overscroll-behavior:contain; scroll-behavior:auto; overflow-anchor:none; min-height:0`; `.is-reordering {cursor:grabbing; user-select:none; touch-action:none}` |
| `.page-item` | `position:relative; display:block; padding:6px 6px 2px; margin-bottom:10px; background:transparent; border-radius:10px; transition:background-color .15s, opacity .15s` (`none` under reduced motion); hover `rgba(0,0,0,.04)`; `.is-dragging {opacity:.4}` |
| `.page-item-main` | `display:flex; flex-direction:column; align-items:center; gap:4px; flex:1; min-width:0; cursor:pointer` |
| `.page-thumbnail-slot` | `width:auto; height:auto; display:flex; align-items:center; justify-content:center; flex-shrink:0` |
| `.page-thumbnail` | inline `width/height`; `background:white; border:1px solid rgba(0,0,0,.1); border-radius:5px; overflow:hidden; box-shadow:0 1px 2px rgba(0,0,0,.08), 0 4px 12px rgba(0,0,0,.08)`; `canvas {display:block; width:100%; height:100%}`; current page: `border-color:transparent; box-shadow:0 0 0 3px var(--accent-orange)` |
| `.page-info` | `width:100%; min-height:28px; display:flex; align-items:center; justify-content:center; flex:1; min-width:0`; `h3 {margin:0; font-size:.75rem; font-weight:600; color:var(--text-secondary); font-variant-numeric:tabular-nums}`; current `h3 {color:var(--accent-orange); font-weight:700}`; `p {display:none}` |
| `.page-drag-handle` | `position:absolute; left:4px; bottom:2px; width:28px; height:28px; display:flex; center; cursor:grab; color:var(--text-secondary); opacity:.45; border-radius:8px; transition:opacity .15s, background .15s; touch-action:none; user-select:none`; hover `opacity:1; background:rgba(0,0,0,.06)`; active `cursor:grabbing`; svg 16×16 |
| `.page-actions` | `position:absolute; right:2px; bottom:0; display:flex; gap:4px; z-index:1` |
| `.page-action-btn` | `30×30; border:none; background:transparent; border-radius:8px; display:flex; center; transition:all .2s; color:var(--text-secondary)`; hover `rgba(0,0,0,.08)` / text-primary; `.delete:hover` `rgba(255,59,48,.1)` / `#ff3b30`; `:disabled {opacity:.45; color:#9a9a9a; cursor:not-allowed; pointer-events:none}`; svg 16×16 |
| `.page-reorder-preview` | `position:fixed; left:0; top:0; width:82px; z-index:1600; pointer-events:none; background:var(--bg-primary); color:var(--text-primary); padding:4px; border-radius:8px; box-shadow:0 8px 24px rgba(0,0,0,.22); text-align:center; font-size:12px; will-change:transform`; canvas `display:block; width:100%; height:auto; border-radius:4px` |
| `.page-reorder-indicator` | `position:fixed; left:0; top:0; height:3px; border-radius:3px; background:var(--accent-orange); pointer-events:none; z-index:1601; will-change:transform` |

### 4.4 List rendering — `renderPagesList(options)` (A:20655-20764)

- No-op unless `#pages-panel.visible` or `options.force` (edits while closed cost nothing;
  `T:e2e/pages-panel-performance.spec.mjs`). Cancels an active drag, keeps `scrollTop`, clears the list.
- `#pages-default-size` ← `<span>Default page</span><strong>{standardName · dimensionsLabel}</strong>`
  (empty parts dropped) with `title = standardName · dimensionsLabel · ratioLabel`, using
  `getDocumentBasePageDimensions()`.
- `canDeletePages = canReorder = !isReadOnly && pages.length > 1`.
- Per page `i` (`isPdf = backgroundSource === 'pdf' || Number.isFinite(pdfPageIndex)`;
  `isPhoto = backgroundSource === 'custom' && backgroundImage`; `pageType = isPdf ? 'PDF' : isPhoto ? 'Photo' : ''`;
  `box = innerWidth <= 720 ? 100 : 136`; thumbnail = `getContainedRect(w, h, box, round(box × 1.42))`,
  canvas backing = thumbnail × 2):

```html
<div class="page-item[ is-current]" data-page-index="{i}" data-page-id="{pageId}" style="cursor:pointer">
  [<div class="page-drag-handle" title="Drag to reorder"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
      <line x1="4" y1="8" x2="20" y2="8"/><line x1="4" y1="16" x2="20" y2="16"/></svg></div>]   <!-- only if canReorder -->
  <div class="page-item-main" data-action="open" title="Page {n} · {std} · {k} stroke(s) · {PDF|Photo}">
    <div class="page-thumbnail-slot"><div class="page-thumbnail" style="width:{tw}px;height:{th}px">
      <canvas id="thumb-{i}" width="{tw*2}" height="{th*2}"></canvas></div></div>
    <div class="page-info"><h3 aria-label="Page {n}">{n}</h3>
      <p class="page-size">{std · dims · ratio}</p><p>{k} stroke(s)[ · PDF|Photo]</p></div>
  </div>
  <div class="page-actions"><button class="page-action-btn delete" data-action="delete" data-page="{i}" title="Delete" [disabled aria-disabled="true"]>
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3 6 5 6 21 6"/>
    <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg></button></div>
</div>
```

  (`stroke` vs `strokes`: singular only when k = 1; empty title parts are dropped.)
- Thumbnail (`generateThumbnail`, A:21067-21116): white fill; unloaded with preview → `drawImage(preview,
  0, 0, tw2, th2)` only; `backgroundImage` → on load `drawImage(bg, 0, 0, tw2, th2)` (stretched) then
  strokes; PDF page without image → inset `max(2, round(min(w,h)·0.04))` rect `#f2f2f2` fill, `#d0d0d0`
  stroke, text `'PDF'` in `bold max(9, round(min·0.18))px sans-serif`, `#9a9a9a`, centred; else template
  (§2.4) then strokes. Strokes: `scale(tw2/pageW, th2/pageH)`, images then every stroke except
  `eraser-stroke` with ≥ 2 points (highlighter ≥ 1) via `drawSmoothStroke`.
- Clicking `.page-item-main` → `goToPage(i)` + `closePagesPanel()`. Delete button → `openDeleteModal(i)`
  (its `pointerdown` is stopped so it never starts a drag).
- After building: restore `scrollTop`; bind drag (§3.5) when `canReorder`.

---

## 5. Photo page modal ("cover image")

### 5.1 Markup (H:9798-9847) + hidden input `<input type="file" id="cover-image-input" accept="image/*">` (H:10277, `display:none`)

Title `'Add photo page'`; dropzone `#cover-upload-dropzone` (`tabindex="0" role="button" aria-label="Choose photo"`)
containing `#cover-preview-stage.is-empty` with `#cover-preview-empty` (image icon + `'Choose a photo'`)
and `#cover-preview-page > img#cover-preview-image`; controls: `#btn-cover-browse` (plus icon +
`span#cover-browse-label 'Choose photo'`) and radiogroup `aria-label="Page size"` with radios
`name="cover-layout-mode"` `fit` (`'Document'`, checked) and `photo` (`'Photo ratio'`);
`#cover-modal-meta` (`aria-live="polite"`); actions `#cover-modal-cancel 'Cancel'`,
`#cover-modal-confirm 'Add Page'` (initially `disabled`).

### 5.2 Behaviour (A:19913-20092, A:20097-20185, A:20613-20653, wiring A:16396-16475)

- `openCoverModal()` (Photo tool): edit mode only; select `fit`; not busy; clear selection; add
  `.visible`; next frame → `updateCoverModalPreview()` and focus the dropzone.
- `closeCoverModal()`: refused while applying; removes `.visible`, clears selection (revokes the object
  URL, resets the input). Triggers: Cancel, click on the overlay itself, global **Escape** (A:16142-16156).
- Choosing a file: dropzone click / Enter / Space or `#btn-cover-browse` → `coverImageInput.click()`
  (value reset first); drag-over adds `.dragover` (`dropEffect='copy'`), first `image/*` file of a drop
  is used; `change` uses `files[0]`.
- `setCoverModalFile(file)`: non-image → status `'Choose an image file'` (error). Label `'Loading…'`;
  decode (token-guarded); selection `{file, previewUrl: createObjectURL(file), width, height}`; failure →
  `'Could not read that photo'` (error).
- Layout `getCoverResultLayout(imgW, imgH, mode)` with `doc = getDocumentBasePageDimensions()`,
  `fitted = getContainedRect(imgW, imgH, doc.w, doc.h)`:
  - `fit` ("Document"): page = `doc` size; image = `fitted` (centred, white margins).
  - `photo` ("Photo ratio"): page = `normalizePageDimension(fitted.w/h)`; image contained in that page.
- Preview (`updateCoverModalPreview`): available box = stage client size (fallback 460×320); empty →
  `.is-empty`, page box = document size contained, `img` src removed and 0×0, meta = document size info,
  label `'Choose photo'`, aria `'Choose photo'`; selected → page box = result page contained,
  image = layout image × `previewScale`, meta = result size info (`'A4 · 210 × 297 mm · 1 : 1.414'`),
  label `'Replace photo'`, aria `'Replace {file.name}'`. Re-run on radio change and window resize.
  Confirm disabled while busy or without selection.
- Confirm (`confirmCoverModal`): insert after `activePageIndex` (fallback last); busy (disables
  browse/cancel/input/radios, `aria-disabled` on dropzone); loading `'Adding photo page...'`;
  `buildCoverImageAsset` then `insertPhotoPageAt` (§3.4); success → status `'Photo page added'`, close;
  failure → `'Failed to add photo page'` (error); document switched mid-way → silent abort.
- `buildCoverImageAsset(file, mode)` (A:20117-20185): `renderScale = min(3, 3840 / max(pageW, pageH),
  decodedW / imageW, decodedH / imageH)` (`COVER_RENDER_SCALE 3`, `COVER_RENDER_MAX_SIDE 3840`), non-finite
  → 1, floor 0.05. **Lossless PNG path**: `file.type === 'image/png'`, size ≤ 4 MiB, max side ≤ 3840,
  pixels ≤ 8 Mi and the image exactly fills the page → store the original file as a data URL.
  Otherwise a canvas `round(page × scale)` (`alpha:false`), `#FFFFFF` fill, `imageSmoothingQuality
  'high'`, image drawn centred at `round(image × scale)`, encoded `image/jpeg` quality **0.94**
  (`toDataURL` fallback). Returns `{dataUrl, pageWidth, pageHeight}` (logical page size).
  Pinned by `T:e2e/photo-page-persistence.spec.mjs`.

### 5.3 Effective CSS (H:3709-3962)

`.modal-cover {width:min(540px, calc(100vw - 24px)); max-width:540px; max-height:min(92svh,720px);
padding:20px; overflow-y:auto; overscroll-behavior:contain}`; `h2 {margin-bottom:14px}`;
`.cover-modal-layout {flex column; gap:12px}`; `.cover-upload-dropzone {position:relative; flex center;
width:100%; height:clamp(220px,44svh,360px); min-height:180px; padding:16px; border-radius:8px;
border:1px solid rgba(0,0,0,.08); background:var(--bg-primary); cursor:pointer; overflow:hidden;
transition:border-color .18s ease, background .18s ease; outline:none}`; hover/`.dragover`/`:focus-visible`
`border-color:rgba(224,122,60,.72); background:rgba(224,122,60,.045)`; `:focus-visible` adds
`box-shadow:0 0 0 3px rgba(224,122,60,.16)`; `.cover-preview-stage {relative; flex center; 100%×100%}`;
`.cover-preview-empty {absolute inset 0; flex column center; gap:10px; padding:20px;
color:var(--text-secondary); font-size:.88rem; font-weight:500}` (svg 34×34, `var(--accent-orange)`);
`.cover-preview-page {flex center; background:#fff; border:1px solid rgba(0,0,0,.08); border-radius:4px;
box-shadow:0 10px 26px rgba(0,0,0,.13); overflow:hidden; transition:width .2s cubic-bezier(.22,1,.36,1),
height .2s cubic-bezier(.22,1,.36,1)}` (opacity .16 when empty; image hidden when empty);
`.cover-modal-controls {grid; columns minmax(0,1fr) auto; gap:10px}`; `#btn-cover-browse` inline-flex,
svg 17×17, label ellipsis; `.cover-layout-options {relative; grid 1fr 1fr; padding:3px; border:1px solid
rgba(0,0,0,.08); border-radius:8px; background:var(--bg-primary)}`; radios visually hidden;
`.cover-layout-label {min-height:34px; padding:8px 11px; border-radius:6px; color:var(--text-secondary);
font-size:.82rem; font-weight:600; line-height:1; center; nowrap; transition:background .16s, color .16s,
box-shadow .16s}`; checked → `background:var(--bg-secondary); color:var(--text-primary);
box-shadow:0 1px 4px rgba(0,0,0,.1)`; `.cover-modal-meta {min-height:18px; color:var(--text-secondary);
font-size:.78rem; tabular-nums; line-height:1.35; center}`. Dark overrides at H:3900-3931. ≤720 px:
overlay `align-items:flex-end`, modal full width bottom sheet `border-radius:16px 16px 0 0`, safe-area
padding, dropzone `clamp(210px,46svh,360px)`; ≤480 px: controls single column; height ≤560 px: dropzone
`clamp(150px,36svh,210px)`.

---

## 6. Slide-to-confirm modals

### 6.1 Generic `showConfirmSlider({title, message, sliderText})` (A:21371-21492; markup H:9900-9918)

```html
<div class="modal-overlay" id="modal-confirm-slider"><div class="modal modal-danger">
  <h2 id="confirm-slider-title">Are you sure?</h2><p id="confirm-slider-message"></p>
  <div class="confirm-slider" id="confirm-slider" aria-label="Slide to confirm">
    <div class="confirm-slider-fill" id="confirm-slider-fill"></div>
    <div class="confirm-slider-text" id="confirm-slider-text">Slide to confirm</div>
    <div class="confirm-slider-handle" id="confirm-slider-handle" aria-hidden="true"><svg 18×18 chevron "9 6 15 12 9 18"/></div>
  </div>
  <div class="modal-actions"><button class="btn btn-secondary" id="confirm-slider-cancel">Cancel</button></div>
</div></div>
```

- Returns `Promise<boolean>`; a pending earlier promise resolves `false`. Sets texts (defaults
  `'Are you sure?'`, `''`, `'Slide to confirm'`), resets the slider, adds `.visible`, measures next frame.
- Metrics: `padding 4`, `handleWidth = handle rect width || 44`, `maxX = max(0, sliderWidth − handleWidth − 8)`.
  `setPos(x, animate)`: clamp to `[0, maxX]`, `.dragging` toggled `= !animate`, handle
  `transform: translateX(xpx)`, fill `width = x + handleWidth/2 + 4`.
- `pointerdown` (primary button only): drag with pointer capture; `dragOffset = clamp(clientX −
  handleLeft, 0, handleWidth)`; position `clientX − sliderLeft − 4 − dragOffset`. Release (`pointerup`,
  `pointercancel`, `lostpointercapture`): `x ≥ maxX − 2` → `.complete`, snap to `maxX`, after **120 ms**
  resolve `true`; else animate back to 0. Cancel button or click on the overlay backdrop → `false`.
  Resolving hides the overlay and resets the slider.

### 6.2 Effective CSS (H:2950-3093, H:4207-4221, H:6564-6587)

`.modal-overlay {position:fixed; inset:0; height:100svh; background:rgba(0,0,0,.5); display:flex; center;
z-index:3000; opacity:0; visibility:hidden; pointer-events:none; transition:opacity .2s ease}`
(the later "Legacy Modals Overrides" block wins over the first `.4 / 2000 / all .3s` rule);
`.visible {opacity:1; visibility:visible; pointer-events:auto}`. `.modal {background:var(--bg-secondary);
border-radius:16px; padding:24px; max-width:400px; width:90%; box-shadow:var(--shadow-medium);
transform:scale(.95); transition:transform .3s ease}`; visible → `scale(1)`; `.modal h2 {font-size:1.25rem;
margin-bottom:16px}`. `.confirm-slider {relative; height:52px; border-radius:16px;
background:rgba(0,0,0,.05); border:1px solid rgba(0,0,0,.08); overflow:hidden; touch-action:none;
user-select:none}`; `-fill {absolute; left 0; top 0; bottom 0; width:0%; background:rgba(224,122,60,.18);
transition:width .2s ease}`; `-text {absolute; centred via translate(-50%,-50%); font-size:.95rem;
color:var(--text-secondary); pointer-events:none; transition:color .2s}`; `-handle {absolute; top:4px;
left:4px; 44×44; border-radius:14px; background:var(--accent-orange); color:white; flex center;
box-shadow:0 8px 18px rgba(0,0,0,.18); transition:transform .2s ease}`; `.dragging` → no transitions;
`.complete` → fill `rgba(224,122,60,.9)`, text `white`. `.modal-actions {display:flex;
gap:clamp(4px,1.2vw,8px); margin-top:20px}` with `.btn {flex:1}`. Buttons: `.btn {display:flex; center;
gap:6px; padding:10px 16px; border:none; border-radius:12px; font:500 .875rem inherit; transition:all .2s}`,
`.btn-secondary {background:rgba(0,0,0,.05)}` (hover `.1`; dark `rgba(255,255,255,.08)`),
`.btn-primary {background:var(--text-primary); color:white}` (hover `#333`; dark `#f5f5f5/#111`),
`.btn:disabled {opacity:.6; cursor:not-allowed}`. Measured at 1280×900: modal 400×242, slider 352×52.

### 6.3 Reset document (dormant) — `openResetModal` / `confirmResetDocument` (A:21262-21326; H:9764-9778)

`#modal-reset-document` (`.modal.modal-warning`, `'Start new document?'`, text `'This action will clear
all pages and strokes and start a fresh document. This cannot be undone.'`, slider `#reset-slider`
`'Slide to start over'`, no Cancel button; backdrop click closes) is wired to `#btn-reset-document`,
which **does not exist in the markup** — unreachable from the UI. Its own slider (A:21326-21669)
completes at `≥ 0.9 × maxX`, then after 120 ms resets the document to one A4 `default` page, clears
history and the page store, records `noteCollabStructureReplacement`, status `'New document'`. The
identical `#trash-slider` code path (Drive bin) is outside this spec. Keep the markup; reproduce the
dormant function only if a caller is added.

---

## 7. Document title and document creation

### 7.1 Document title (A:5737-5759, A:5923-6008, wiring A:16062-16098; markup H:9541-9542)

- Element `<span class="doc-title" id="doc-title" role="textbox" aria-label="Document title"
  contenteditable="true" spellcheck="false">Untitled</span>` inside the editor header logo block.
  The HTML `<title>` stays `inhouse notes` (H:92); it never shows the document name.
- `updateDocTitle()`: skipped while editing; text = `getBaseDocTitle(driveFileName || exportName ||
  'Untitled')` where `getBaseDocTitle` trims and strips a trailing `.pdf` (case-insensitive), empty →
  `'Untitled'`. Also toggles `#btn-drive-share` `.hidden` (shown iff `driveFileId`) and calls
  `updateCalendarFeatureAvailability()`.
- Editing: `focus` in read mode → `blur()`; in edit mode `beginDocTitleEdit()` (`.editing`, remember
  original). Click in read mode switches to edit mode (`setReadOnlyMode(false, {user:true})`) and
  focuses next frame. `pointerdown`/`click` stop propagation; `keydown` stops propagation, `Enter` →
  blur (commit), `Escape` → cancel + blur. `blur` commits.
- `finishDocTitleEdit(commit)`: `normalizeDocTitleInput(v)` = collapse whitespace, trim, strip `.pdf`,
  `sanitizeFilename` (`[\\/:*?"<>|]+` → `_`, A:31782-31785), empty → `'Untitled'`. Cancel restores. If
  changed: `exportName = next`, `noteCollabDocumentFieldChanged('exportName')`; with a Drive file →
  `renameDriveFile` = `PATCH https://www.googleapis.com/drive/v3/files/{id}` body `{name: next + '.pdf'}`
  then `driveFileName = response.name`; failure → status `'Rename failed'` (logical name kept); without
  a Drive file → `driveFileName = next + '.pdf'`. Then `saveDriveSession()`, `scheduleSave(false)`.
- CSS (H:6537-6560; mobile H:7417-7430): `.doc-title {display:inline-flex; align-items:center;
  font-size:.95rem; font-weight:500; color:var(--text-secondary); margin-left:12px; padding:4px 8px;
  border-radius:8px; max-width:320px; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;
  cursor:text}`; `.editing, :focus {background:rgba(0,0,0,.05); color:var(--text-primary);
  outline:2px solid rgba(0,47,217,.3); outline-offset:2px; max-width:420px; text-overflow:clip}`;
  read-only `cursor:default` (H:1288); ≤720 px: `margin-left:6px; padding:3px 6px; font-size:.86rem;
  max-width:clamp(72px,26vw,128px); flex:1 1 auto`, editing `max-width:min(52vw,180px)`.
  Remote merges that change the name update the title (A:29589-29591).

### 7.2 New PDF (Home "+" → "Create PDF")

`#drive-create-pdf` → `createNewDriveDocument()` → `openFolderPicker('new')` (A:11734, A:11777-11779):
title `'New PDF'`, subtitle `'Choose a folder and name your file.'`, name input prefilled with
`normalizeDocTitleInput(state.exportName || 'Untitled')` (i.e. the last document's name), `.pdf`
suffix shown, confirm `'Create'` (disabled while the trimmed name is empty). Confirm →
`createNewDriveDocumentWithFolder(folderId, name)` (A:12178-12179).

`createNewDriveDocumentWithFolder(folderId, base, {templateKind, driveQuickAccessKind,
academicStartYear})` (A:11781-11915):
1. Refuse if a Drive/offline open is in progress; `InhouseOfflineLibrary.capture()` failure → status
   `'Keep this document open: its offline copy could not be saved.'` (error) and abort.
2. `safeBase = normalizeDocTitleInput(base)`; `fileName = safeBase + '.pdf'`; kind normalised;
   `beginDocumentSession()`; read-only; `showEditorView()`; zoom 1, pan 0; clear selection/PDF state;
   clear the page store and caches.
3. `state.pages = [one blank page of the kind at the kind's default size]` (no `pageId` yet; assigned
   lazily); `driveFolderKind = normalize(driveQuickAccessKind) || kind from folder stack`;
   `driveFolderId/ParentIds = [folderId]`; `calendarPageConfig = null`; if `agenda` and an integer
   `academicStartYear` → pages and config from `buildAcademicAgenda` (§7.4).
4. Reset version history, pending version hint and undo; `renderAllPages()`; `centerViewOnTop()`
   (`panX = (viewportW − maxW·zoom)/2 − 30·zoom`, `panY = 20`, A:2752-2773); `exportName = safeBase`;
   `driveFileId = null`; `driveFileName = fileName`; autosave off; owner = signed-in profile;
   `updateDocTitle()`; `saveDriveSession()`; **edit mode**; `scheduleSave(true, 0)`.
5. Loading `'Creating PDF...'`. Offline library ready → `InhouseOfflineApp.newDocument()`, status
   `'Created on this device'`. Else ensure token, build the PDF (`buildPdfBlob`, upload options),
   `createDriveFileInFolder`, record id/revision/version/modified time, status `'PDF created'`;
   failure → `'Failed to create PDF'`. Finally autosave on, hide loading.

### 7.3 Quick access Diary / Agenda (A:394-401, A:8811-9043; markup H:9313-9316; CSS H:4848-4893)

Buttons in the Home profile menu: `#btn-drive-menu-diary` `'Diary'` (light `background:#c9bfd8;
border:1px solid #b7abca; color:#362b45`, hover `#bcaece`; dark `#b39ec7/#9d89b1/#21182d`, hover
`#a78fbe`) and `#btn-drive-menu-agenda` `'Agenda'` (light `#2f5a3f/#274a34/#f4f8f2`, hover `#274d36`;
dark `#2b5b3b/#3b7a51/#f4f8f2`, hover `#356b46`), both `.btn.btn-secondary` inside
`.drive-profile-actions` (flex, gap 8px, each `flex:1`).

- Config: `DRIVE_QUICK_ACCESS_DOCS = {diary: {label:'Diary', folderName:'Diary', filePrefix:'Diary'},
  agenda: {label:'Agenda', folderName:'Agenda', filePrefix:'Agenda'}}`;
  `DRIVE_QUICK_ACCESS_DOC_RE = /^(?:Diary|Agenda)\d{4}(?:-\d{4})?(?:\.pdf)?$/i` — matching files are
  **excluded from Home "Recents"** (A:10753-10755).
- `getDriveQuickAccessPeriod(kind, now)` (A:8979-8984): diary → `{label: 'YYYY', fileBaseName:
  'DiaryYYYY'}`; agenda → academic year `startYear = month ≥ September ? year : year − 1`,
  `{label: 'S–S+1'` (en dash), `fileBaseName: 'AgendaS-S+1'` (hyphen), `startYear}`.
- `openQuickAccessDriveDocument(kind)` (A:9009-9043): guarded by `driveOpenInProgress` /
  `driveQuickAccessInProgress`; both buttons disabled while running; close profile menus; loading
  `'Checking {label} {period.label}...'`; ensure token unless offline;
  `ensureNamedDriveFolder(folderName)` (no parent constraint: `files.list q="name='Agenda' and
  mimeType='application/vnd.google-apps.folder' and trashed=false"`, `orderBy=createdTime`,
  `pageSize=1`, `fields=files(id,name)`; missing → `POST files?fields=id,name` creates it in My Drive;
  offline library short-circuits to cached/local folders);
  `findDrivePdfInFolder(folderId, base)` (`q="(name='base.pdf' or name='base') and
  mimeType='application/pdf' and trashed=false and '<folder>' in parents"`, `orderBy=modifiedTime desc`,
  `pageSize=5`, `fields=files(id,name,thumbnailLink,resourceKey,modifiedTime)`; offline cache first).
  Found → `openDriveFile({...file, parents:[folderId], driveQuickAccessKind: kind})`; else
  `createNewDriveDocumentWithFolder(folderId, base, {templateKind: agenda|diary,
  driveQuickAccessKind: kind, academicStartYear: period.startYear})` and refresh Home lists.
  Failure → `'Failed to open {label}'` (error). Pinned by `T:e2e/academic-agenda.spec.mjs` (rollover in
  September, find-then-create-then-reopen, offline reuse).

### 7.4 Academic-year agenda — `buildAcademicAgenda(startYear)` (A:8986-9007)

- `first = new Date(startYear, 8, 1, 12)` then back to the Monday on/before (`−(getDay()+6)%7` days);
  `last = new Date(startYear + 1, 7, 31, 12)`; one page per week while `cursor <= last`, cursor += 7 days
  (`addCalendarDays`, local-time `setDate`; noon avoids DST edge cases).
- Each page: `{pageId: generatePageId(), strokes:[], images:[], backgroundImage:null, backgroundSource:
  'template', templateKind:'agenda', pdfPageIndex:null, pageWidth:559, pageHeight:794,
  needsRedraw:true, preview:null, strokeCount:0, unloaded:false, sidePanel:{title: 'Week of ' +
  cursor.toLocaleDateString(), dateKeys: [Mon..Sun 'YYYY-MM-DD'], html: buildSidePanelHtml(7 sections,
  label toLocaleDateString([], {weekday:'short', month:'short', day:'numeric'}), events [])}}`.
  Title uses the **default numeric locale date** (en-US: `'Week of 8/31/2026'`), unlike the
  `'Week of Aug 31'` form of §12/§11.2.
- `calendarPageConfig = {mode:'week', startPage:0, startDateKey: key(first), nextDateKey: key(cursor
  after the last page)}`.
- Example 2026: 53 pages, `startDateKey '2026-08-31'`, `nextDateKey '2027-09-06'`; every first key is
  a Monday; keys unique; includes `2026-09-01` and `2027-08-31`; 2027 course includes `2028-02-29`
  (`T:e2e/academic-agenda.spec.mjs`).

---

## 8. Agenda / Diary detection and calendar gating (A:5761-5860)

- `state.driveFolderKind` comes from: explicit `file.driveQuickAccessKind|driveFolderKind|quickAccessKind`;
  else the Home folder breadcrumb stack (nearest crumb among the file's parents whose name equals
  `Diary`/`Agenda`, compared case-insensitively after stripping `.pdf`); else by fetching each parent's
  metadata (`GET files/{id}?fields=id,name,parents`, cached). On open the provisional kind is applied
  immediately and the authoritative lookup later; a change re-runs `updateCalendarFeatureAvailability()`,
  `renderAllPages()` when it becomes `agenda`, and `saveDriveSession()` (A:11488-11538).
- `usesAgendaTemplate()` = any page with `templateKind === 'agenda'`.
- `isAgendaQuickAccessFileName(name)` = normalised name starts with `agenda`.
- **`canUseAgendaCalendarForCurrentDocument()` = `driveFolderKind === 'agenda'` AND
  (`usesAgendaTemplate()` OR agenda-prefixed `driveFileName || exportName`)**.
  `shouldRenderPageCalendarPanels()` is the same predicate. Diary documents never get calendar features.
- `updateCalendarFeatureAvailability()` (called on panel open, title update, read-only toggle, editor
  show, open): invalidates layout if the predicate flipped; `#btn-open-calendar` gets `.hidden` and
  `disabled` when not allowed; `#calendar-add-pages` `.hidden` when not allowed and `disabled` when not
  allowed or read-only; when not allowed it closes the month, day and add-to-pages modals and removes
  every `.page-side-panel` from the DOM.

---

## 9. Google Calendar integration

### 9.1 Auth and scope

- One Google token covers Drive and Calendar: `GOOGLE_API_SCOPE = 'https://www.googleapis.com/auth/drive
  https://www.googleapis.com/auth/calendar.events'` (A:390-392), `include_granted_scopes = true`.
  Scope version `GOOGLE_SCOPE_VERSION = 'drive-calendar-v2'` stored under `google-scope-version-v1`
  (A:415-417). Any token response lacking `calendar.events` is rejected with `'Calendar access was not
  granted by Google.'` (A:6481-6487, A:6847-6853). Owned by the auth spec; repeated here because the
  calendar depends on it.
- Calendar used: always `primary`. No calendar list, no time-zone handling beyond the browser's local
  time.

### 9.2 `calendarFetch(url, options, retried)` (A:19312-19369)

- `token = await ensureDriveToken(false)`; header `Authorization: Bearer <token>`; timeout
  `options.timeout` (min 1000) default **30 000 ms** via `AbortController`; external `options.signal`
  forwarded.
- `401` (first time) → drop the token (`driveAccessToken = null`, expiry 0), `ensureDriveToken(false)`,
  retry once.
- Non-OK: read text; parse `error.message` and `error.errors[0].reason || error.status`.
  `403` matching `/insufficient|forbidden/i` → remove `google-scope-version-v1`, clear token storage,
  throw `'Calendar access was not granted by Google. Sign in again from the app login and accept Calendar
  access.'`; `403` matching `/not been used|disabled|not enabled|accessNotConfigured/i` → `'Google
  Calendar API is not enabled for this app project.'`; otherwise `'Calendar API error {status}:
  {detail}'`. Own timeout → `'Calendar request timed out'`.

### 9.3 Event listing and cache (A:17743-17748, A:19371-19473)

- Module state: `calendarVisibleMonth` (1st of month), `calendarEvents` (visible month list),
  `calendarSelectedDateKey`, `calendarEventsCache: Map<'YYYY-MM', items[]>`,
  `calendarEventsInFlight: Map<'YYYY-MM', Promise>`, `calendarLoadingMonthKey`. Cache is per page
  load (not persisted) and never evicted or cleared — it survives switching documents and signing out
  within the same page session (no `calendarEventsCache.clear()` exists).
- Month window `getCalendarGridRange(month)` (A:17863-17871): `start` = local midnight of the Monday
  on/before the 1st (`(getDay()+6)%7` days back), `end = start + 42 days` (6 full weeks).
- `fetchCalendarEventsForMonth(month, {force})`:
  `GET https://www.googleapis.com/calendar/v3/calendars/primary/events?timeMin={start.toISOString()}
  &timeMax={end.toISOString()}&singleEvents=true&orderBy=startTime&maxResults=2500`
  (URLSearchParams order exactly as listed; no `pageToken` follow-up, no `fields` filter). Stores
  `data.items || []` under the month key. Cached and not forced → cached value. Concurrent requests for
  the same key (forced or not) join the in-flight promise. On failure the previous cached value is
  restored and the error rethrown (panels never go blank).
- Event fields consumed: `id, summary, start.date|dateTime, end.date|dateTime, location, description,
  colorId, recurrence, reminders.{useDefault, overrides[{method, minutes}]}, updated`.
- `prefetchNeighborCalendarMonths()` (only while the month modal is visible): months −3,−2,−1,+1,+2,+3
  not cached/in flight, errors only logged.
- `loadCalendarEventsForVisibleMonth({force})` (only while visible): cached & not forced → render +
  prefetch; else `calendarLoadingMonthKey = key`, show `#calendar-loading`, hide/clear `#calendar-error`,
  render (skeleton), fetch; on success (if still the visible month) render + prefetch; on error set
  `calendarEvents = []`, render, show `#calendar-error` with `err.message || 'Could not load calendar
  events.'`; finally hide loading.

### 9.4 Event → days, ordering, formatting (A:17873-18005)

- Keys are local `'YYYY-MM-DD'` (`formatCalendarDateKey`). `parseGoogleEventDate(v)`: `YYYY-MM-DD` →
  local midnight; otherwise `new Date(v)`.
- `getEventDayKeys(event)`: all-day → every day from `start.date` up to but excluding `end.date`
  (missing/invalid end → one day; `end ≤ start` → the start day only). Timed → every local day from the
  start's day while `day < end` instant (an event ending exactly at 00:00 does not add the next day;
  missing end = start + 1 ms); at least the start day.
- `dedupeEventsById(list)`: by `id`, keep the greatest `Date.parse(updated)` (ties keep the first);
  events without id pass through (appended after the deduped ones).
- `sortEventsByStart`: by `start.dateTime || start.date` time (all-day = local midnight, so it sorts
  before timed events of that day); stable sort.
- `buildCalendarEventMap(events)` → `Map<dayKey, sorted deduped events>`.
- `formatCalendarEventTime(e)` → `'All day'` | `toLocaleTimeString([], {hour:'2-digit',
  minute:'2-digit'})` (en-US `'09:00 AM'`) | `''`. `formatCalendarEventRange(e)` → `'All day'` |
  `'{start} - {end}'` | `'{start}'`.

### 9.5 Month modal (`#modal-calendar`, H:10028-10070; A:17754-17783, A:17831-17861, A:18056-18132)

Opened by `#btn-open-calendar` (Pages sidebar). Not allowed (§8) → refresh availability + status
`'Calendar is only available in Drive agenda documents.'`. Open: visible month = current month,
selected = today, apply cached events, add `.visible`, render, load, prefetch; next frame scroll the
`.calendar-day.today` cell into view (`block/inline: 'nearest'`).

Markup: `.modal.calendar-modal` > `.calendar-modal-head` (`<h2>Calendar</h2>` + `#calendar-modal-close.panel-close`
"Close calendar"), `.calendar-toolbar` with two `.calendar-toolbar-group`s: [`#calendar-prev`
(`.btn.btn-secondary.btn-icon-square`, "Previous month", chevron-left), `#calendar-title.calendar-title`,
`#calendar-next` ("Next month")] and [`#calendar-today` `'Today'`, `#calendar-refresh` `'Refresh'`,
`#calendar-add-pages.btn-primary` `'Add to pages'`]; `#calendar-loading.calendar-loading.hidden`
(3 `span.calendar-loading-line`), `#calendar-error.calendar-error.hidden`,
`#calendar-empty.calendar-empty.hidden` `'No events in this month.'`, `.calendar-body >
#calendar-grid.calendar-grid[aria-live=polite]`.

Grid render (`renderCalendarMonth`): title `toLocaleDateString([], {month:'long', year:'numeric'})`
(`'October 2026'`); 7 header cells `<div class="calendar-weekday">Mon…Sun</div>` (text uppercased by
CSS); then 42 `<button type="button" class="calendar-day[ muted][ today][ selected][ loading]"
aria-label="{weekday long, day numeric, month long}">` each with `<div class="calendar-day-number">{d}</div>`
and either 3 skeleton spans (`calendar-skeleton-line long|medium|short`, when the visible month is
loading and not cached; in-month cells only) or up to 3 `<span class="calendar-event" title="{summary}">
<span class="calendar-event-time">{time}</span><span class="calendar-event-title">{summary||'(No title)'}</span></span>`
plus `<div class="calendar-more">+{n−3} more</div>`. `#calendar-empty` hidden iff the visible month has
events (it also shows during a cold load).

Interactions: prev/next → `shiftCalendarMonth(±1)`; Today → visible = current month, selected = today;
Refresh → `loadCalendarEventsForVisibleMonth({force:true})`; day click: ignored on loading cells;
other-month day → select + switch month (no day modal); in-month → select, re-render, open the day
modal. Close button or backdrop click → `closeCalendarModal()` (also closes the day modal). Escape does
not close it.

### 9.6 Day modal (`#modal-calendar-day`, H:10073-10089; A:18007-18054)

Head: `<h2 id="calendar-day-title">` = `toLocaleDateString([], {weekday:'long', day:'numeric',
month:'long'})`, `#calendar-day-subtitle.calendar-day-panel-subtitle` = `'{n} event(s)'` or `'No events'`,
close `#calendar-day-modal-close` ("Close day events"). Body `#calendar-day-events`: empty →
`<div class="calendar-day-empty">No events for this day.</div>`; else per event
`.calendar-day-event` > `.calendar-day-event-time` (range) + `.calendar-day-event-title`
(`summary || '(No title)'`) + optional `.calendar-day-event-meta` = `[location, description with
whitespace collapsed].join(' · ')`. Read-only listing (no edit from here). Backdrop click closes.

### 9.7 Calendar modal CSS (effective; H:3302-3707, mobile H:6194-6216)

`.calendar-modal {width:min(94vw,920px); max-width:920px; max-height:min(92svh,760px); display:flex;
flex-direction:column; gap:16px; overflow:hidden}` (+ `.modal` base: padding 24, radius 16,
`--bg-secondary`); `.calendar-modal-head {flex; align center; space-between; gap:12px}`, `h2 {margin:0}`;
`.calendar-toolbar {flex; align center; space-between; gap:12px; flex-wrap:wrap}`;
`.calendar-toolbar-group {flex; align center; gap:8px}`; `.btn-icon-square {width/height:
var(--header-btn-size) (44px; 34px ≤720); padding:0}`; `.calendar-title {min-width:180px; center;
font-weight:700; font-size:1rem}`; `.calendar-loading {grid 1fr .6fr .8fr; gap:8px}`;
`.calendar-loading-line, .calendar-day.loading .calendar-skeleton-line {height:10px; border-radius:999px;
background:linear-gradient(90deg, rgba(0,0,0,.06) 0%, rgba(0,0,0,.12) 45%, rgba(0,0,0,.06) 90%);
background-size:220% 100%; animation:drive-skeleton-shimmer 1.2s linear 25 forwards}` (keyframes
`background-position -240px 0 → 240px 0`; dark stops `.06/.16/.06` white; reduced motion → no
animation, H:6417-6430); `.calendar-error {font-size:.86rem; color:var(--accent-red) #E81010}`;
`.calendar-empty {font-size:.86rem; color:var(--text-secondary)}`; `.calendar-grid {display:grid;
grid-template-columns:repeat(7,minmax(0,1fr)); grid-template-rows:34px repeat(6,minmax(0,1fr));
border:1px solid rgba(0,0,0,.08); border-radius:12px; overflow:hidden; background:rgba(0,0,0,.04);
height:clamp(420px,54vh,520px); min-height:0}` (dark border `.1` white, bg `.06` white; ≤720 px
`height:360px`); `.calendar-weekday, .calendar-day {background:var(--bg-secondary); min-width:0}`;
`.calendar-weekday {padding:10px 8px; center; font-size:.72rem; font-weight:700;
color:var(--text-secondary); text-transform:uppercase}` (≤720 `padding:8px 4px; .66rem`);
`.calendar-day {padding:8px (5px ≤720); border:none; border-top/left:1px solid rgba(0,0,0,.06) (dark
.08 white); first column no left border; overflow:hidden; cursor:pointer; color:var(--text-primary);
font:inherit; text-align:left}`; focus-visible `outline:2px solid rgba(0,47,217,.45);
outline-offset:-3px`; `.muted {opacity:.48}`; `.today {background:rgba(34,197,94,.12)}` (dark `.18`),
today number = 22×22 circle `#22c55e`, white, `.76rem`, inline-flex centred; `.selected
{background:rgba(0,47,217,.1); box-shadow:inset 0 0 0 2px rgba(0,47,217,.38)}`; `.loading {cursor:default}`
(number opacity .45; skeleton lines block, margin-top 7px, widths 92/78/58%); `.calendar-day-number
{font-size:.78rem; font-weight:700; margin-bottom:6px}`; `.calendar-event {display:block; width:100%;
border:none; border-radius:6px; padding:3px 5px (2px 3px ≤720); margin-top:3px;
background:rgba(0,47,217,.1); color:var(--text-primary); font-size:.64rem (.58rem ≤720);
line-height:1.12; text-align:left; overflow:hidden}`; `.calendar-event-time {display:block;
color:var(--text-secondary); font-size:.58rem; nowrap}` (hidden ≤720); `.calendar-event-title {block;
ellipsis; nowrap}`; `.calendar-more {margin-top:4px; font-size:.62rem; color:var(--text-secondary)}`.
Day modal: `.calendar-day-modal {width:min(92vw,520px); max-width:520px; max-height:min(86svh,640px);
flex column; gap:0; overflow:hidden}`; `.calendar-day-events {overflow:auto; padding:10px; flex column;
gap:8px; max-height:min(62svh,480px); min-height:0}`; `.calendar-day-panel-subtitle {margin-top:2px;
color:var(--text-secondary); font-size:.78rem}`; `.calendar-day-event {border:1px solid rgba(0,0,0,.07);
border-radius:10px; padding:9px 10px; background:rgba(0,0,0,.025)}` (dark `.09`/`.04` white);
`-time {.75rem; text-secondary; margin-bottom:3px}`; `-title {700 .88rem; line-height 1.25;
overflow-wrap:anywhere}`; `-meta {margin-top:5px; .75rem; text-secondary; line-height 1.3}`;
`.calendar-day-empty {padding:14px; .86rem; text-secondary}`.

---

## 10. Page side-panels (calendar cards)

### 10.1 Data

`page.sidePanel = {title: string, html: string, dateKeys: string[]}` — one key (day mode) or 7
consecutive keys (week mode, Mon-based in generated agendas). `title` is user-editable; `html` is a
derived rendering cache rebuilt from the event cache (§11.1) and sanitised on every render; collab and
the canonical document hash only use `{title, dateKeys}` (`T:collaboration-core-v5.test.mjs`
"canonical document hash ignores … derived panel HTML", "side-panel removal and update obey the same
LWW rule").

### 10.2 HTML vocabulary — `buildSidePanelHtml(sections)` (A:18450-18480)

`sections = [{dateKey, label, events}]`; concatenated without whitespace; text through
`escapeHtmlSafe` (`& < > "`):

```html
<div class="sp-day-section[ today]" data-day-key="{key}">
  <div class="sp-day-header"><span class="sp-day-label">{label}</span>
    <button type="button" class="sp-add-btn" data-sp-add="{key}" title="Add event" aria-label="Add event">+</button></div>
  <div class="sp-events-list">
    <div class="sp-no-events">No events</div>            <!-- when no events -->
    <button type="button" class="sp-event" data-sp-event="{id}" data-sp-day="{key}" style="--sp-event-color:{hex}">
      [<span class="sp-event-time">{formatCalendarEventTime}</span>]<span class="sp-event-title">{summary||'(No title)'}</span></button>
  </div></div>
```

`today` = section key equals today's key at build time (re-evaluated on every render).
Event colour `getEventColor(e)` (A:18432-18444): `GOOGLE_EVENT_COLORS = {1:'#7986cb', 2:'#33b679',
3:'#8e24aa', 4:'#e67c73', 5:'#f6c026', 6:'#f5511d', 7:'#039be5', 8:'#616161', 9:'#3f51b5',
10:'#0b8043', 11:'#d60000'}`, default `'#039be5'`.

Sanitiser `securityCore.sanitizeCalendarPanelHtml(html, document)` (S:8-79), applied before any
`innerHTML` (`T:release-v5-smoke.mjs` asserts `body.innerHTML = panelData.html` never appears):
input > 512 KiB → `''`; parse via `<template>`; drop elements other than `DIV/SPAN/BUTTON`; keep only
attributes `class, data-day-key, data-sp-add, data-sp-event, data-sp-day, style, type, title,
aria-label`; keep only classes `sp-day-section today sp-day-header sp-day-label sp-add-btn
sp-events-list sp-no-events sp-event sp-event-time sp-event-title`; `style` reduced to
`--sp-event-color` with a 6-hex value (re-serialised by `style.setProperty`, i.e. `--sp-event-color:
#039be5;`); data/title/aria values truncated to 512 chars; buttons forced `type="button"`, others lose
`type`; comments removed; duplicate `.sp-event[data-sp-event]` ids within a section removed (first
kept); output > 512 KiB → `''`. When sanitising changes the html it is written back to
`page.sidePanel.html` (A:18577-18584, A:18510-18511).

### 10.3 DOM element — `buildSidePanelElement` / `renderPageSidePanel` (A:18489-18623)

Rendered only when `shouldRenderPageCalendarPanels()`; appended to `.page-row[data-page-row=i]` after
the wrapper (by `renderAllPages`, `createPageElement`, and `renderPageSidePanel`):

```html
<div class="page-side-panel[ is-week]" data-panel-page="{i}" style="width:{pageWidth}px">
  <div class="page-side-panel-header"><div class="page-side-panel-title" contenteditable="true">{title || 'Calendar'}</div></div>
  <div class="page-side-panel-body">{sanitised html}</div>
</div>
```

- `.is-week` when `dateKeys.length > 1`. The panel sits **below** the page (column layout), same width
  as the page (559 px for agenda pages).
- `renderPageSidePanel(i, data)`: invalidates page bounds; `data` null or panels not allowed → remove
  the existing panel. If a panel with the **same dateKeys sequence** exists → in-place update
  (`spUpdateSidePanelInPlace`, A:18625-18650): toggle `.is-week`; update the title text unless the
  title is focused; replace body `innerHTML` only if different, preserving `scrollTop`; re-apply
  `.today`. Otherwise replace the element (entry animation replays). Always
  `refreshPageSidePanelLayout()` (next frame: `updateAddPageIndicator()` + `scheduleVisiblePageUpdate(true)`).
- Title `input` (no read-only guard — the title stays editable even in read mode): `sidePanel.title =
  textContent`, `noteCollabPageFieldChanged(page, 'sidePanel')`, `markPageDirty(i, 'full')`,
  `scheduleSave(false)` (pinned by `T:release-v5-smoke.mjs:937`). Empty title shows CSS placeholder
  `'Calendar'`.
- Body `click` (ignored in read-only): `.sp-add-btn` → event editor `create` for its day; `.sp-event`
  → editor `edit` with `data-sp-event`/`data-sp-day`.
- Wheel inside `.page-side-panel-body` with |dy| > |dx| and no Ctrl/Meta scrolls the panel when it can
  scroll in that direction; otherwise the canvas pans (A:17316-17327). Pointer-down on the panel still
  starts canvas panning.

### 10.4 Side-panel CSS (always white/orange, identical in dark mode; H:1663-1995)

`.page-side-panel {width:100% (inline width wins); height/min-height/max-height:184px;
background:#ffffff; border:1px solid rgba(224,122,60,.22); border-radius:12px; box-shadow:0 6px 22px
rgba(224,122,60,.12), 0 1px 3px rgba(0,0,0,.06); display:flex; flex-direction:column; overflow:hidden;
font-family:'DM Sans','Inter',system-ui,sans-serif; font-size:11px; color:#2a1d14; flex-shrink:0;
animation:sidePanelIn .22s cubic-bezier(.22,1,.36,1); overscroll-behavior:contain}`;
`@keyframes sidePanelIn {from {opacity:0; transform:translateX(-10px)} to {opacity:1; transform:translateX(0)}}`.
Header `{flex; align center; space-between; padding:10px 12px; background:linear-gradient(180deg,#fff7f0
0%,#ffffff 100%); border-bottom:1px solid rgba(224,122,60,.22); gap:6px; flex-shrink:0}`.
Title `{font-size:12px; font-weight:700; letter-spacing:.02em; text-transform:uppercase; color:#e07a3c;
flex:1; outline:none; min-width:0; nowrap; overflow:hidden; text-overflow:ellipsis; cursor:text}`,
`:focus {white-space:normal; overflow:visible; text-overflow:clip; color:#c75c1f}`,
`:empty::before {content:'Calendar'; color:rgba(224,122,60,.5); font-style:italic}`.
Body `{flex:1; overflow-y:auto; padding:6px 7px 7px; outline:none; line-height:1.25; background:#fff;
color:#2a1d14; scrollbar-width:thin; scrollbar-color:rgba(224,122,60,.32) transparent;
overscroll-behavior:contain}`, webkit scrollbar 5px, thumb `rgba(224,122,60,.32)` radius 3px;
week body `{display:grid; grid-template-columns:repeat(7,minmax(0,1fr)); gap:5px; overflow-x:hidden}`.
Section `{margin-bottom:5px (0 last); border-radius:6px; padding:4px 6px 5px; background:#fffaf5;
border:1px solid rgba(224,122,60,.14)}`, week `{min-width:0; margin-bottom:0; padding:4px 4px 5px}`;
today `{background:rgba(34,197,94,.08); border-color:rgba(34,197,94,.45)}`, today label `#16a34a`, today
header border `rgba(34,197,94,.32)`, today add button `background:rgba(34,197,94,.14); color:#16a34a`,
hover `#22c55e`/white. Day header `{position:relative; flex; align center; space-between; gap:5px
(3px week); margin-bottom:3px; padding-bottom:2px; padding-right:18px (15px week); border-bottom:1px
solid rgba(224,122,60,.2)}`. Label `{font-size:9px (7.5px week); font-weight:700; uppercase;
letter-spacing:.04em; color:#e07a3c; flex:1; min-width:0; overflow:hidden; ellipsis; nowrap}`.
Add button `{position:absolute; right:0; top:50%; transform:translateY(calc(-50% - 1px)); 14×14;
border-radius:4px; border:none; background:rgba(224,122,60,.12); color:#e07a3c; flex center;
font-size:11px; line-height:1; padding:0; transition:background .14s, color .14s}`, hover
`#e07a3c`/white. Read-only (`body.read-only`): add button hidden, header `padding-right:0`, events
`cursor:default` and no hover change. Events list `{flex column; gap:2px}`. Event `{display:flex;
flex-direction:row; align-items:baseline; flex-wrap:wrap; gap:4px; font-size:10px; padding:2px 5px;
border-radius:4px; background:#fff; border:1px solid rgba(0,0,0,.07); border-left:3px solid
var(--sp-event-color,#e07a3c); color:#2a1d14; cursor:pointer; text-align:left; width:100%;
font-family:inherit; line-height:1.2; transition:background .14s, border-color .14s, box-shadow .14s}`,
hover `{background:#fafafa; box-shadow:0 1px 3px rgba(0,0,0,.07)}`; week event `{flex-direction:column;
align-items:flex-start; gap:1px; padding:2px 3px; font-size:8px; border-left-width:2px}`. Time
`{font-size:9px (7px week); font-weight:700; color:var(--sp-event-color,#e07a3c); letter-spacing:.02em;
flex-shrink:0}`; title `{font-size:10px (8px, line-height 1.15 week); color:#2a1d14; word-break:break-word;
white-space:normal; line-height:1.2; flex:1; min-width:0}`; no-events `{font-size:9.5px;
color:rgba(42,29,20,.45); font-style:italic; padding:1px 3px}`. `.page-side-panel-remove` styles exist
(H:1737-1763) but no element uses them.

### 10.5 Event editor (`#sp-event-modal`, H:10170-10260; A:18676-19310)

Markup: `.modal.sp-event-modal` > `.sp-event-modal-head` (`h2#sp-event-modal-title`, close
`#sp-event-modal-close.panel-close` aria "Close"); `.sp-event-form`: `Title` text `#sp-event-summary`
(placeholder `'Event title'`); `label.sp-event-check` checkbox `#sp-event-allday` `'All day'`;
`.sp-event-times` = `Date` `#sp-event-date` (date), `Start` `#sp-event-start` (time), `End`
`#sp-event-end` (time); `Color` `#sp-event-color-picker.sp-color-picker[role=radiogroup][aria-label=
"Event color"]`; `.sp-event-row-double` = `Repeat` `#sp-event-recurrence` (`none 'Does not repeat'`,
`daily 'Every day'`, `weekly 'Every week'`, `monthly 'Every month'`, `yearly 'Every year'`,
`weekdays 'Weekdays (Mon–Fri)'`) and `Reminder` `#sp-event-reminder` (`none 'No reminder'`, `0 'At the
time'`, `5/10/15/30 'N min before'`, `60 '1 hour before'`, `120 '2 hours before'`, `1440 '1 day
before'`); `Location` `#sp-event-location` and `Description` `textarea#sp-event-description rows=3`
(placeholders `'Optional'`); `#sp-event-error[hidden]`; actions `.sp-event-actions` = `#sp-event-delete
.btn.sp-event-delete 'Delete'` + `.sp-event-right` (`#sp-event-cancel 'Cancel'`, `#sp-event-save
.btn-primary 'Save'`).

Colour picker (built once at load): first a default swatch `button.sp-color-swatch.default-swatch.has-color`
(`--default-event-color:#039be5`, `data-color-id=""`, `title 'Color predeterminado'`, `role=radio`),
then one per `GOOGLE_EVENT_COLORS` entry in key order 1…11 (`background:{hex}`, `title={hex}`).
`spSelectColor(id)` toggles `.selected` and `aria-checked`.

Open `spOpenEventModal({mode, pageIndex, dayKey, eventId})`: requires §8 (else refresh availability)
and edit mode (else status `'Switch to edit mode to modify events'`). Edit: look up the event in all
cached months then `calendarEvents`; missing → fetch that month (non-forced) and retry. Populate:
title `'Edit event'`; summary/location/description; all-day → checkbox on, date = `start.date`,
times empty; timed → date = local key of start, start/end `HH:MM` (24 h, zero-padded, end empty if
none); colour = `colorId || ''`; repeat = `spParseRecurrenceRules(recurrence)` (`RRULE:` → `FREQ`
+`BYDAY=MO,TU,WE,TH,FR` → `weekdays`, else daily/weekly/monthly/yearly/none; singleEvents instances
normally lack `recurrence` → `none`); reminder = `spParseEventReminders` (`useDefault` without
overrides → `'30'`; else first popup override (or first override) minutes if in
`0,5,10,15,30,60,120,1440`, else `none`); Delete visible. Create: title `'New event'`, empty texts,
not all-day, date = `dayKey || today`, `09:00`–`10:00`, default colour, `none`, `none`, Delete hidden.
Then start/end `disabled` iff all-day (also on checkbox `change`), add `.visible`, focus the title
after 50 ms. Close: close button, Cancel, backdrop click → hide, clear state and error.

Payload `spBuildEventPayload()` (A:18834-18893): summary trimmed, empty → inline error `'Title is
required'`; date must match `YYYY-MM-DD` else `'Pick a valid date'`. `{summary}` + `location` /
`description` when non-empty. All-day → `start:{date: D}`, `end:{date: D+1}`. Timed → local
`Date(y, m−1, d, h, min)`; end defaults to start time; end ≤ start → start + 1 h; both sent as
`{dateTime: toISOString()}` (UTC, no `timeZone`). Always `colorId: selected || null`,
`recurrence: rule ? [rule] : null` with rules `RRULE:FREQ=DAILY|WEEKLY|MONTHLY|YEARLY` or
`RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR`, and `reminders`: `none` → `{useDefault:false, overrides:[]}`,
minutes → `{useDefault:false, overrides:[{method:'popup', minutes}]}`.

Save (A:19029-19068): Save+Delete disabled during the request; edit → `PATCH
https://www.googleapis.com/calendar/v3/calendars/primary/events/{encodeURIComponent(id)}` JSON body,
status `'Event updated'`; create → `POST …/calendars/primary/events`, status `'Event created'`; then
close and `spRefreshPanelsForKeys(affected)` where affected = new date + the old event's start day.
Delete (A:19070-19093): native `confirm('Delete this event?')`; `DELETE …/events/{id}`; status
`'Event deleted'`; refresh the event's start day and `dayKey`. Errors → inline `#sp-event-error` with
the error message (fallbacks `'Could not save event'` / `'Could not delete event'`).
`spRefreshPanelsForKeys(keys)` force-refetches each affected month (failures logged) then
`spRebuildAllPanels()`.

Editor CSS (always white/orange; H:2001-2263): `.modal.sp-event-modal {background:#fff; color:#2a1d14;
max-width:480px; width:min(94vw,480px); padding:20px 22px; border:1px solid rgba(224,122,60,.22);
box-shadow:0 18px 50px rgba(0,0,0,.22); max-height:92svh; overflow-y:auto}` (≤480: `padding:16px 14px;
width:96vw`); head `{flex; space-between; margin-bottom:14px}`, `h2 {1.05rem; 700; #e07a3c; margin:0;
uppercase; letter-spacing:.03em}`; close 28×28, `color:rgba(224,122,60,.7)`, radius 6px, hover
`rgba(224,122,60,.12)`/`#c75c1f`, svg 16×16; form `{flex column; gap:12px}`; field `{flex column; gap:4px}`,
label `{.72rem; 700; uppercase; letter-spacing:.04em; #c75c1f}`; inputs/select/textarea `{font:inherit;
.92rem; color:#2a1d14; background:#fffaf5; border:1px solid rgba(224,122,60,.28); border-radius:8px;
padding:8px 10px; outline:none; transition:border-color .14s, background .14s; width:100%; min-width:0}`,
focus `border-color:#e07a3c; background:#fff`; textarea `resize:vertical; min-height:60px`;
times grid `1.2fr 1fr 1fr; gap:8px` (≤480: 2 columns, date full row); row-double `1fr 1fr; gap:10px`
(≤480: 1 column); picker `{flex; wrap; gap:8px}`; swatch `{26×26; radius 50%; border:2px solid
transparent; padding:0; transition:transform .12s, border-color .12s, box-shadow .12s}`, hover
`scale(1.1)`, selected `{border-color:#2a1d14; box-shadow:0 0 0 2px #fff inset}`, default with colour
`background:var(--default-event-color,#039be5)`; checkbox row `{inline-flex; gap:8px; .88rem;
#2a1d14; user-select:none}`, input `accent-color:#e07a3c; 16×16`; error `{color:#c0392b; .84rem;
background:rgba(192,57,43,.08); border:1px solid rgba(192,57,43,.25); border-radius:6px; padding:6px 8px}`;
actions `{flex; space-between; gap:8px; margin-top:18px}`, right group gap 8px; Delete
`{background:transparent; color:#c0392b; border:1px solid rgba(192,57,43,.35)}` hover
`rgba(192,57,43,.08)`, `[hidden]` → `display:none`; primary `#e07a3c`/white hover `#c75c1f`; secondary
`#fff`/`#2a1d14` border `rgba(224,122,60,.32)` hover `#fff4ea`.

---

## 11. Calendar auto-sync, auto-attach, reconciliation, "today" navigation

### 11.1 5-second sync (A:18959-19008, A:19253-19302)

- `spStartAutoSync()`: one `setInterval(…, 5000)`; each tick returns early when signed out / no token,
  `document.visibilityState === 'hidden'`, editor not active, panels not allowed, or no page has a
  non-empty `sidePanel.dateKeys`; otherwise `spSyncAllPanels()`. Runs in **both** read and edit mode.
- Started at the end of `setupEventListeners()` (A:16599), on `showEditorView()` when visible
  (A:6247), on `visibilitychange → visible` when the editor is active (A:16594). Stopped on
  `visibilitychange → hidden` (A:16562), other non-visible states (A:16596) and by
  `closeDocumentScopedWork()` when going Home (A:6108). Pinned by `T:e2e/document-lifecycle.spec.mjs`
  ("Home stops every document-scoped loop", "hidden editor pauses … calendar sync, then resumes").
- `spSyncAllPanels()`: union of all panels' dateKeys → set of months → force-refetch every month in
  parallel (failures keep the old cache) → **always** `spRebuildAllPanels()`. An academic agenda spans
  ~13 months, i.e. ~13 Calendar requests every 5 s while visible (behavioural fact of v5).
- `spRebuildAllPanels()`: build one day map from **all** cached months (multi-day spreading, dedupe,
  sort); for each page with a panel whose every dateKey's month is cached: sections from its keys
  (label `weekday 'short'` for weeks, `'long'` for single days, `month:'short', day:'numeric'`, default
  locale), `html = buildSidePanelHtml(sections)` written to `sidePanel.html` if changed (no dirty mark,
  no collab change), then `renderPageSidePanel` (in-place). Panels whose months are not cached keep
  their stored html.

### 11.2 Auto-attach on new page — `spAutoAttachCalendarPanel(i)` (A:19095-19150)

Only when allowed, a config exists and the page has no panel. `span = week ? 7 : 1`; occupied = all
other pages' dateKeys; start at `config.nextDateKey`, advance by `span` while any candidate key is
occupied. Week: 7 sections from that date (`weekday 'short'`), title `'Week of ' +
toLocaleDateString([], {month:'short', day:'numeric'})`; day: one section (`weekday 'long'`), title
`toLocaleDateString([], {weekday:'long', month:'long', day:'numeric'})`. `config.nextDateKey =
start + span`; record the config field; set `sidePanel` (record field), render, `markPageDirty(i,
'meta')`, `scheduleSave(false)`; then fetch that month and `spRefreshPanelsForKeys(dateKeys)`.

### 11.3 `reconcileConcurrentCalendarPanels()` (A:19152-19251)

Called (edit mode only) at the end of every remote page merge (A:30627-30637). Walks pages in order
with a cursor starting at `config.startDateKey || nextDateKey`: a page whose keys overlap keys already
used by an earlier page is re-assigned the next free `span`-block at/after the cursor (labels and
titles built with **`'en-US'`** locale), persisted to IndexedDB and recorded as a field change; the
cursor moves past each page's last date; finally `nextDateKey` = max(cursor, existing next) adjusted
past occupied blocks. Returns whether anything changed (forces a merge publish).

### 11.4 Land on today — `findTodayCalendarPageIndex` / `scrollToTodayCalendarPage` (A:16839-16940)

Only when panels are allowed. Pass 1: the first page whose dateKeys include today. Pass 2 (today
outside all panels): today before all → first calendar page; after all → last calendar page; in a gap
→ the first page with a key after today. Fallback: from config, `startPage + floor(days/7)` (week) or
`+ days` (day), clamped. Scroll: `zoom = clamp(min(viewportH / (pageH + 60), viewportW / (pageW + 60)),
0.5, 3)`; centre the page vertically (`panY = −centre·zoom + viewportH/2`), `panX = (viewportW −
pageW·zoom)/2 − 30·zoom`; next frame correct any residual > 1 px and `scheduleVisiblePageUpdate(true)`.
`activePageIndex` is intentionally not changed. Used at boot when the editor is visible (before
restoring the saved view, A:2357-2363) and after every PDF open/import (A:32711-32713).

---

## 12. "Add calendar to pages" (A:17785-17829, A:18302-18425; H:10119-10166)

Opened from `#calendar-add-pages` in the month modal (disabled in read mode). Not allowed (§8) →
`'Calendar can only be added to Drive agenda documents.'`.

Markup: `.modal.calendar-pages-modal` (`width:min(92vw,560px); max-width:560px;
max-height:min(90svh,680px); overflow:auto`) > head `<h2>Add Calendar to Pages</h2>` + close
`#calendar-pages-close` ("Close calendar pages setup"); `.calendar-pages-form` (`grid; gap:14px;
margin-top:14px`) with two `.calendar-pages-row` (`grid 1fr 1fr; gap:12px`; 1 column ≤720) of
`.calendar-pages-field` (`flex column; gap:6px`; label `.78rem 700 text-secondary`; input/select
`height:40px; border:1px solid rgba(0,0,0,.12) (dark .14 white); border-radius:10px;
background:var(--bg-primary); padding:0 10px; .9rem`): `Agenda layout` `#calendar-pages-mode`
(`day 'One page per day'`, `week 'One page per week'`), `Start page` `#calendar-pages-start-page`
(number, min 1), `First day / week starts` `#calendar-pages-start-date` (date), `Text size`
`#calendar-pages-font` (number 8–18, default 11 — **unused**); `.calendar-pages-note#calendar-pages-summary`
(`.8rem; line-height 1.35; text-secondary`, initial text `'Events will be placed on the right side of
each target page.'`); actions Cancel / `#calendar-pages-apply 'Add to Pages'`.

Open: start date = `config.startDateKey || calendarSelectedDateKey || today`; start page `max =
pages.length`, value `activePageIndex + 1`; mode = `config.mode || 'day'`; summary; `.visible`.
Summary (on every `input`/`change` of mode/start page/start date): `count = max(1, pages − startPage + 1)`,
`last = start + (week ? 7·count − 1 : count − 1)` days →
`'{count} page(s) from p. {startPage} to end, covering {start.toLocaleDateString()} – {last.toLocaleDateString()}.'`.
Close: close button, Cancel, backdrop.

**Apply — current v5.17.22 behaviour:** `applyCalendarEventsToPages()` reads
`calendarPagesClear?.checked` (A:18320), an identifier that is declared nowhere (no
`#calendar-pages-clear` element either). In edit mode with an allowed document this throws
`ReferenceError: calendarPagesClear is not defined` before anything happens: the async handler rejects
(unhandled), the modal stays open, no pages change (verified in Chromium against this checkout). In
read mode it shows `'Switch to edit mode to add calendar events'` first. The intended algorithm (code
after that line), for reference / a deliberate fix:
1. `mode`, `startDate`, `startPage` (0-based); out of range → `'Target pages are outside this document'`
   (error). Target = pageIds from `startPage` to the end. Close the modal; loading `'Adding calendar
   events...'`.
2. Fetch every month spanning `[startDate, startDate + (week ? 7n : n) − 1]` (`getCalendarEventsForDateRange`,
   A:18146-18172: per-month cached fetch, spread by day, keep keys in range, dedupe, sort).
3. Under the structure lock, load all pages; for target page k: optionally remove legacy calendar image
   cards (`removeCalendarImportedImages`, with tombstones); week → 7 sections from `start + 7k`
   (`weekday 'short'`), title `'Week of {Mon d}'`; day → 1 section (`weekday 'long'`), title
   `'{Weekday}, {Month} {d}'` (`toLocaleDateString([], {weekday:'long', month:'long', day:'numeric'})`);
   `sidePanel = {title, html: buildSidePanelHtml(sections), dateKeys}`; record field; render; mark
   dirty `'full'` if cards were removed else `'meta'`; yield every 4 pages.
4. `calendarPageConfig = {mode, startDateKey, startPage: current index of the first target, nextDateKey:
   start + (week ? 7n : n)}`; record field; `scheduleSave(true)`; status `'Calendar added to {n}
   page(s)'`; errors → `'Could not add calendar events'` (error). Always hide loading.

---

## 13. Dormant, legacy and quirky behaviour (decide explicitly in v6)

| Item | Where | Status in v5.17.22 |
|---|---|---|
| "Add to pages" apply throws `ReferenceError: calendarPagesClear is not defined` | A:18320 | **Broken**: nothing is applied. Identical-behaviour v6 = button does nothing; a fix would treat the missing checkbox as unchecked (`clearExistingCards = false`). Needs a user decision. |
| Reset document modal | `#modal-reset-document` H:9764, A:21262-21326 | Unreachable (`#btn-reset-document` absent). |
| Static delete modal | `#modal-delete-page` H:9751 | Never shown (slider modal used). Wiring exists (A:16477-16482). |
| Legacy calendar image cards | `renderAgendaEventsCard`, `makeAgendaCalendarImage` A:18190-18282 | Unused; old documents may still contain `cal_*` images (ordinary images). |
| `removePageSidePanel` | A:18653-18674 | Unused (no remove button; would show Spanish `'Calendario eliminado'`). CSS `.page-side-panel-remove` unused. |
| `#calendar-pages-font` | H:10151 | Read nowhere. |
| `AGENDA_TEMPLATE_SVG` | A:402 | Unused duplicate of `Agenda.svg`. |
| Unused CSS | `.panel-timeline-row`, `.panel-cover-row`, `.pages-panel-actions`, `.calendar-pages-check` | Keep (byte-identical stylesheet). |
| Side-panel title editable in read mode | A:18505-18515 | No guard; edits persist. |
| Locale mix | academic titles numeric (`'Week of 8/31/2026'`), others `'Week of Aug 31'`; labels default locale; reconcile uses `'en-US'` | Reproduce. |
| Calendar empty text during cold load | A:18130-18132 | `'No events in this month.'` shows together with the skeleton. |
| Escape | A:16142-16156 | Closes the photo modal only (not Pages, calendar or event modals). |
| Mixed-width alignment | §3.1 | Layout metrics centre narrower pages, DOM left-aligns them. |
| Panels only under agenda folder | §8 | Agenda PDFs opened from other folders hide panels and drop them on save. |

---

## 14. Tests pinning this behaviour

- `T:e2e/academic-agenda.spec.mjs` "academic year rolls over in September, not January; diary remains annual" — period names and quick-access regex.
- `T:e2e/academic-agenda.spec.mjs` "weekly agenda covers the entire course with unique dates including leap day" — 52–54 Monday-based pages, unique ids/keys, Sep 1 … Aug 31.
- `T:e2e/academic-agenda.spec.mjs` "quick access creates this course and then reopens it without touching the old agenda" — find → create(startYear) → find → open.
- `T:e2e/academic-agenda.spec.mjs` "new agenda initializes all weekly pages before the first save" — 53 pages, kind `agenda`, name `Agenda2026-2027.pdf`, config `{week, 0, 2026-08-31, 2027-09-06}`.
- `T:e2e/academic-agenda.spec.mjs` "pending local agenda is reused online before Drive upload finishes" — offline library folder/file short-circuit.
- `T:e2e/academic-agenda.spec.mjs` "course agenda survives offline close and reopen with all dated pages".
- `T:e2e/page-reorder.spec.mjs` "stationary pointer scrolls down continuously and drops into the visible position without losing ink".
- `T:e2e/page-reorder.spec.mjs` "dragging a lower page upwards scrolls without more pointer moves and Escape cancels".
- `T:e2e/page-reorder.spec.mjs` "touch drag ignores other pointers and closing the panel stops scrolling and releases the gesture".
- `T:e2e/pages-panel-performance.spec.mjs` "hidden page edits defer thumbnail work until the panel opens".
- `T:e2e/pages-panel-performance.spec.mjs` "reopening the pages panel starts only one gesture per drag".
- `T:e2e/document-lifecycle.spec.mjs` "Home stops every document-scoped loop and nothing restarts it" — `spSyncInterval` cleared.
- `T:e2e/document-lifecycle.spec.mjs` "a hidden editor pauses Drive polling and calendar sync, then resumes when visible".
- `T:e2e/drive-ink-cache-invalidation.spec.mjs` "incoming {rename|calendar|sidePanel} conservatively invalidates every page"; "an ordinary stroke in an unchanged calendar keeps unrelated page caches".
- `T:e2e/app-resilience.spec.mjs` "untrusted shared HTML and hostile PDF sources fail closed" — calendar-panel sanitiser.
- `T:e2e/photo-page-persistence.spec.mjs` (4 tests) — photo page raster fallback, unreadable photo rejection, high-resolution detail + lossless bounded PNG, single encode/no upscaling.
- `T:e2e/scanner-native-template.spec.mjs` "scanned ink uses the native editor template through insertion, local reload and a PDF round-trip" — page-canvas template pixels equal `InhouseBulletTemplate.render`.
- `T:e2e/scanner-template.spec.mjs` (4 tests) — bullet template geometry, opaque colour rings, caller state preserved, pixel parity of scanner vs editor renderer, worker parity.
- `T:e2e/live-scanner.spec.mjs` "retaking releases the previous camera and only confirmation inserts the scanned page" — inserted page `templateKind 'default'`.
- `T:e2e/collaboration-chaos.spec.mjs` "three concurrent replicas converge for strokes, fields, page moves and deletes".
- `T:collaboration-core-v5.test.mjs` structure tests (lines 33, 56, 70, 86, 102, 171, 186, 203) — page add/move/delete/replace merge semantics used by §3.
- `T:collaboration-core-v5.test.mjs` "canonical document hash ignores arrival order and derived panel HTML".
- `T:collaboration-core-v5.test.mjs` "independent document fields survive concurrent edits" / "production page fields preserve independent edits and atomic compound values" — `doc:calendarPageConfig`, `page:*:template|size|background`.
- `T:collaboration-core-v5.test.mjs` "side-panel removal and update obey the same LWW rule".
- `T:timeline-core-v5.test.mjs` — timeline entries carry `templateKind`, `sidePanel`, `calendarPageConfig`.
- `T:release-v5-smoke.mjs:72` — `#btn-scan-page` directly after `#btn-set-cover`.
- `T:release-v5-smoke.mjs:248` — timeline snapshots include `calendarPageConfig:`.
- `T:release-v5-smoke.mjs:419` — `templateBackgroundImageCache` exists in PDF export.
- `T:release-v5-smoke.mjs:445` — clearing legacy calendar cards records tombstones.
- `T:release-v5-smoke.mjs:628` — remote merge calls `reconcileConcurrentCalendarPanels({`.
- `T:release-v5-smoke.mjs:711-720` — remote reorder keeps the active page by `pageId`.
- `T:release-v5-smoke.mjs:937` — side-panel title edits mark the page `'full'` dirty.
- `T:release-v5-smoke.mjs:1098-1109` — sanitiser shape and no raw `body.innerHTML = panelData.html`.
- `T:release-v5-smoke.mjs:1129-1135` — `CALENDAR_KEYWORD_SECTION_MAX_CHARS = 64 KiB` enforced in both IH_CAL parsers.

---

## 15. Key invariants (summary)

1. Three template kinds only (`default` bullet A4 794×1123, `agenda`/`diary` SVG A5 559×794); unknown → `default`; new overscroll pages inherit the last page's kind at the kind's default size.
2. Bullet template = exact Canvas-2D sequence of `bullet-template.js` with `u = width/21`, rendered at backing resolution; agenda/diary = byte-identical SVGs drawn with `drawImage` over white; template caches max 6 entries.
3. Page identity is `pageId`; every structural edit (add, photo, scan, move, delete, calendar apply) runs under the single structure lock and re-resolves indices by id; photo/scan insertion clears undo history, move/delete do not.
4. Pages sidebar: 176 px (136 px ≤720) docked left under the header, transparent overlay closes it, thumbnails 136/100 px boxes ×1.42 at 2× backing, drag handle reorder with edge auto-scroll `speed²·1050 px/s`, `edge = min(88, h/3)`.
5. Calendar features exist only when `driveFolderKind === 'agenda'` and the document uses the agenda template or is named `Agenda…`; side panels / `calendarPageConfig` / `IH_CAL` are neither rendered nor persisted otherwise.
6. Agenda quick access = folder `Agenda`, file `AgendaS-S+1.pdf` with S rolling over in September; 52–54 Monday-based weekly pages with week side panels and config `{mode:'week', startPage:0, …}`.
7. Side panel = 184 px white/orange card **below** the page (row gap 14), width = page width; html is a sanitised, device-local cache rebuilt from cached events; only `{title, dateKeys}` is merged/hashed (LWW).
8. Calendar API: `primary` events list over the 42-day Monday-based grid window, `singleEvents=true&orderBy=startTime&maxResults=2500`, per-month cache with in-flight coalescing and failure fallback; 401 retry once; 30 s timeout; create/patch/delete through the side-panel editor with explicit `colorId`/`recurrence`/`reminders`.
9. Panels auto-sync every 5 s (visible editor, signed in, any panel) by force-refetching every covered month and rebuilding in place (scroll and focused title preserved); stopped on hide/Home.
10. "Add to pages" is currently non-functional (ReferenceError on `calendarPagesClear`); reset-document and static delete modals are dormant — v6 must decide explicitly rather than silently "fix".
