# v6 spec — Document formats, PDF pipelines, timeline and undo/redo

Status: behavioural specification of Inhouse Notes **v5.17.22** (`app-v5.js` and companions) for the
Rust/WebAssembly rebuild 6.0.0. Read-only extraction; every claim cites `file:line` in this checkout.
Unless stated otherwise `app-v5.js` is implied by a bare `:line`.

Conventions

- **MUST** = needed to open existing Drive documents or to interoperate with v5 devices
  (REQUIREMENTS.md §4 "Interoperates with devices still on v5 during the transition").
- **QUIRK** = odd but observable v5 behaviour; keep it unless the lead decides otherwise.
- **DEFECT?** = probable v5 bug found while reading. Reproduce with a v5 e2e before deciding; do not
  silently "fix" in a way that changes the bytes v5 readers depend on.
- "px" = logical page pixel (CSS px at 96 dpi). "pt" = PDF user unit (1/72 in). 1 px = 0.75 pt.

---

## 1. Overview: what a document is on disk

A notebook in Drive is **one PDF file** (`application/pdf`). It is simultaneously:

1. a human-viewable PDF (any reader shows pages with the ink burnt in), and
2. a container for the editable document, carried entirely in the PDF **Info dictionary `/Keywords`**
   string (vector strokes, images, page layout, collaboration registers, sync identity, timeline),
   plus optionally
3. an **embedded file attachment `IH_CLEAN_ORIGINAL.pdf`** holding the untouched source PDF so ink
   can be erased non-destructively and the document reverted.

There is no sidecar file, no Drive appProperties payload and no multi-key chunking of the metadata:
the whole editable state lives in a single `/Keywords` string (§3). (The `rebuild-v2/` prototype's
"sidecar schema 3" is **not** shipped — see §11.)

Two independent writers produce the PDF (§4):

| Path | Library | When | Original PDF pages | Ink |
|---|---|---|---|---|
| Text-preserving ("pdf-lib path") | pdf-lib (worker facade or main thread) | `canBuildCurrentPdfWithPdfLib()` true and no failure (:33874, :34540) | copied as vector pages (`copyPages`) | separate overlay XObject per page (PNG/raw raster or vector Form), recorded in `IH_OVERLAY_REFS` |
| Raster fallback ("jsPDF path") | jsPDF 2.5.1 in a worker (:33058) | otherwise, or pdf-lib failed twice | rendered by pdf.js into a JPEG | flattened into the same JPEG (destructive) |

---

## 2. Units, page geometry, identifiers

### 2.1 Page sizes
- A4 template page = **794 × 1123 px** (:294-295); A5 (agenda/diary templates) = **559 × 794 px** (:297-298, :300-305).
- `normalizePageDimension(v, fallback)`: non-finite → fallback, else `max(64, round(v))` (:2392-2395).
  All stored `pageWidth/pageHeight` are integers ≥ 64.
- Imported PDF pages take the pdf.js viewport size at scale 1 **in points, used as px**:
  `round(viewport.width)`, `round(viewport.height)` (:32106-32113, :32121-32133). An A4 PDF page
  therefore becomes 595 × 842 "px" in the model (not 794 × 1123). pdf.js viewports honour `/Rotate`
  and use the CropBox ("view"). **MUST** keep this convention: stored stroke coordinates of imported
  pages are in this space.

### 2.2 Coordinates
- Stroke points and image centres are in page px, origin top-left, y down (:4497-4505, :12639-12659).
- Export maps px → pt: template/custom pages are created at `px × 0.75` pt (:33957, :34057-34060);
  copied PDF pages keep their own size (`getSize()` = MediaBox) and ink is scaled by
  `pageWidthPt / logicalPageWidthPx` (:34050-34056, :34294-34303).
- **QUIRK/DEFECT?** pdf-lib `getSize()` returns the unrotated MediaBox while the logical size came
  from the rotated CropBox viewport; overlays on pages with `/Rotate ≠ 0`, a non-zero MediaBox origin
  or CropBox ≠ MediaBox are misplaced in the exported file. The editable data (px space) is unaffected,
  so v6 may place overlays correctly (CropBox origin + rotation) without breaking v5 readers.
- Raster path page size: `mm = max(10, round(px × 25.4 / 96 × 1000) / 1000)`; orientation
  `'landscape'` iff wMm > hMm (:34640-34644). **QUIRK**: imported PDF pages (595 px) come out at 75 %
  of their physical size in the raster path (157.4 mm wide for A4).

### 2.3 Identifiers (deterministic forms are **MUST** for multi-device convergence of legacy files)
- New stroke/image id: `Date.now().toString(36) + '-' + Math.random().toString(36).substr(2, 9)` (:736-738).
- New page id: `'pg_' + Date.now().toString(36) + '_' + Math.random().toString(36).slice(2, 11)` (:743-745).
- Legacy page without `pageId`, when a document id is known:
  `pg_legacy_${simpleHash(`${documentId}:${pageNumber1Based}`)}` (:747-751; used at :32452, :32499, :24093-24097, :776-785).
- Legacy stroke/image without `id`: `st_legacy_` / `im_legacy_` + `simpleHash(identity)` +
  `simpleHash([...identity].reverse().join(''))` with
  `identity = [documentId, pageId, 'stroke'|'image', floor(index), fingerprint].join(':')`; on import
  `fastStableIndex` is set so `fingerprint = ''` (:753-774, :788-818, :24168-24173). Without a document
  id or page id a random id is used.
- `simpleHash(str)`: Java string hash over UTF-16 code units, `h = ((h << 5) - h + c) | 0`, then
  `Math.abs(h).toString(36).slice(0, 8)` (:27687-27693).

---

## 3. The `/Keywords` container (byte level)

### 3.1 Grammar (as written by v5.17.22)

```
Keywords      = Strokes ";IH_NORM:1" [ ";IH_OVERLAY_REFS:" Refs ] [ ";IH_CAL:" B64J ]
                [ ";IH_STRUCT:" B64J ] [ ";IH_FIELDS:" B64J ] [ ";IH_SYNC:" B64J ]
                [ ";IH_VERSIONS:" B64Z ]
Strokes       = "STROKES_Z:" B64Z            ; normal
              / "STROKES_DATA:" B64J         ; only if compression threw (legacy/older files too)
Refs          = Ref *( "," Ref )             ; pdf-lib path only
Ref           = PageIndex0 "=" ( XObjectName / ObjectNumber )
B64Z          = base64( zlib( utf8( JSON ) ) )
B64J          = base64( utf8( JSON ) )       ; uncompressed
```

Order is fixed by the writers: pdf-lib path :34379-34431, raster path :34957-34996 (raster never
writes `IH_OVERLAY_REFS`). Sections are separated by `;`; base64 never contains `;`, `:` or `_`, so
values are delimited by the next `;` or end of string.

- **base64** = RFC 4648 standard alphabet with `=` padding, no line breaks (`btoa`). Readers use
  `atob` (forgiving: ASCII whitespace stripped, missing padding tolerated).
- **zlib** = `CompressionStream('deflate')`, i.e. RFC 1950 zlib-wrapped DEFLATE (2-byte header +
  Adler-32), **not** raw deflate. v5 readers use `DecompressionStream('deflate')`, which rejects raw
  deflate. **MUST**: v6 writes zlib-wrapped data; level is free (output need not be byte-identical).
  Producers: :32803-32831 (main), document-processing-worker-v5.js:109-117 (worker); readers
  :32833-32860, :7495-7520, worker :198-212.
- **utf8**: `STROKES_Z` JSON is encoded with `TextEncoder` (lone surrogates → U+FFFD) and decoded
  with lenient `TextDecoder` (:32806, :7518). `B64J` sections use
  `btoa(unescape(encodeURIComponent(json)))` (throws on lone surrogates → section omitted, e.g.
  :1660, :1690, :7445) and are decoded strictly with `decodeURIComponent(escape(atob(x)))`.

### 3.2 How the PDF stores the string
- pdf-lib main thread: `setKeywords([kw])` → `/Keywords <FEFF…>` UTF-16BE hex string
  (vendor/pdf-lib/pdf-lib.min.js `setKeywords` → `PDFHexString.fromText(keywords.join(' '))`).
- pdf-lib worker: if the joined text matches `/^[A-Za-z0-9+/=;:_,.\- ]*$/` it is written as a
  **literal** `PDFString` (no 4× hex expansion), otherwise via the Unicode setter
  (document-processing-worker-v5.js:308-319).
- jsPDF: `setProperties({ keywords: metaPrefix + metadata })` literal string (:33085-33090).
- Readers obtain it through pdf.js `getMetadata().info.Keywords` (Info dict only, not XMP) (:32180-32182,
  :7283-7285, :28828-28830). **MUST**: the v6 reader handles literal (with escapes), hex, UTF-16BE BOM
  and PDFDocEncoding strings of tens of MB, in the trailer Info of the newest xref section.
- Other Info entries written: `Title 'Cuaderno Digital'`, `Subject 'Notas con S-Pen'`,
  `Creator 'Cuaderno Digital App'` (:34439-34441, :33085-33089). pdf-lib also stamps Producer
  `pdf-lib (https://github.com/Hopding/pdf-lib)`, CreationDate, ModDate; jsPDF stamps its own producer.
  **QUIRK** v5 remote merge reads `Title` (minus `.pdf`) as a fallback document name when no
  `doc:exportName` field register exists (:28831, :29551-29553); keep Title constant.

### 3.3 Section parsing rules (reader), exact
| Section | Locate | Value end | Size guard | Decoder | On failure |
|---|---|---|---|---|---|
| `STROKES_Z:` | `startsWith` at offset 0 only (:7631) | first `;` (`stripPdfKeywordExtras` :7414-7418) | encoded ≤ 48 MiB chars, inflated ≤ 96 MiB (:7476-7477, :7495-7509, :7571-7573) | worker inflate + `JSON.parse` (:7487-7540); main-thread fallback only if Worker/DecompressionStream unavailable (:7611-7626) | throws (import catches unless strict) |
| `STROKES_DATA:` | `startsWith` at 0 (:7635) | first `;` | same | `decodeURIComponent(escape(atob))` | same; sets `needsMigrationResave` (:32199-32208) |
| `IH_NORM:1` | `includes(';IH_NORM:1')` (:32183) | — | — | flag | — |
| `IH_OVERLAY_REFS:` | `indexOf('IH_OVERLAY_REFS:')` (no `;` required) (:33617-33645) | next `;` | — | split `,`; `k=v`; `k=parseInt`; `v` all digits → `{refNum}` else `{name}` | entries without `=`/value skipped; empty → null |
| `IH_CAL:` | `indexOf(';IH_CAL:')` (:32209-32232) | next `;` | ≤ 64 KiB chars (:1655) | B64J → object; accepted only if `typeof mode === 'string'`; normalised to `{mode, startDateKey||null, startPage (number else 0), nextDateKey||null}` | warn; throws only if strict |
| `IH_STRUCT:` | `indexOf(';IH_STRUCT:')` (:1667-1685) | next `;` | ≤ 8 MiB chars (:1654) | B64J → `ihnNormalizeStructureMeta(parsed, pageIdsFromStrokes)` | null, or throw if `throwOnInvalid` |
| `IH_FIELDS:` | `indexOf(';IH_FIELDS:')` (:1697-1716) | next `;` | ≤ 8 MiB chars | B64J → `ihnNormalizeFieldMeta` | same |
| `IH_SYNC:` | `indexOf(';IH_SYNC:')` (:7449-7468) | next `;` | raw ≤ 16384 chars else ignored | B64J; valid only if `v === 1` and `contentHash` truthy | null |
| `IH_VERSIONS:` | `indexOf('IH_VERSIONS:')` (:33566-33582) | next `;` | ≤ 48 MiB chars, inflated ≤ 96 MiB (worker :199-209) | zlib inflate → `materializeArchive` → `normalizeWorkerTimeline` | null (timeline empty) |

Unknown sections (e.g. `;IH_TEST:123`) are ignored and survive nothing (v5 rewrites the whole string
on every save). Third-party Keywords that do not start with `STROKES_` are ignored and are **lost from
the saved file's Info dict on first save** (they survive inside `IH_CLEAN_ORIGINAL.pdf` when attached).

Strict mode: Drive opens, offline opens and public/shared opens pass `strictCollabMetadata: true`
(:11385-11393, :8559-8565, :8712-8718, offline-app-v5.js:131): an invalid `STROKES_*`, `IH_CAL`,
`IH_STRUCT` or `IH_FIELDS` aborts the open instead of opening an empty/partial document
(:32193-32205, :32229-32232, :32240-32246, :32256-32260). User file import is non-strict.

### 3.4 `STROKES_Z` payload: array of page entries
JSON **array**, one entry per PDF page, in page order (entry *i* ↔ PDF page *i + 1*). Writer
(pdf-lib :34183-34194; raster :34755-34773) emits keys in this order:

```json
{ "pageId": "pg_…",
  "strokes": [ Stroke… ],
  "images": [ Image… ],
  "deletedStrokeIds": [ "id", … ],
  "deletedStrokeStamps": { "id": { "clock": 0, "actor": "" }, … },
  "hasCustomBackground": true|false,
  "backgroundSource": "template"|"pdf"|"custom",
  "templateKind": "default"|"agenda"|"diary",
  "pdfPageIndex": 1-based export position | null,
  "pageWidth": int, "pageHeight": int,
  "sidePanel": { "title", "html", "dateKeys" } | null }
```

- `hasCustomBackground = backgroundSource === 'pdf' || !!backgroundImage`; `pdfPageIndex = i + 1`
  when that is true else `null` — the **position in the saved PDF**, never the source-PDF index
  (:34187-34190, :34764-34769 comment :34767-34768).
- `backgroundSource` default: pdf-lib path `pdfPageIndex ? 'pdf' : 'template'` (:34188); raster path
  `pdfPageIndex ? 'pdf' : (backgroundImage ? 'custom' : 'template')` (:34686-34687).
- **No `backgroundImage` is ever written**: photo-page pixels live only in the PDF page; the field
  register `page:<id>:background` carries `simpleHash(dataUrl)` (:1473-1484; test
  tests/e2e/pdf-photo-provenance.spec.mjs:84-91).
- `sidePanel` is written only when `shouldRenderPageCalendarPanels()` (agenda folder + agenda
  template/file name, :5826-5833), else `null`. It includes the raw `html` (sanitised on render).
- `images`: written as-is including `src` data URLs (inline photos travel inside the compressed JSON).
- **Stroke** = the in-memory stroke with points replaced: `{...stroke, points}` keeps the stroke's own
  key order (`id, tool, color, width, points[, syncStamp]`) and passes through unknown keys
  (:34143-34167, :34723-34745). Each point becomes `{x: round2(x), y: round2(y), p: round2(p)}` where
  `round2(v) = Math.round(v*100)/100`; `p` is omitted when `null/undefined` (JSON drops `undefined`).
- Duplicate filtering before writing: keep first occurrence of fingerprint `id:${id}` or, for id-less
  strokes, `JSON.stringify([tool,color,width,points])` (:34131-34141, :34707-34718).
- `tool ∈ {'pen','highlighter','eraser-stroke','eraser-area'}`; `eraser-*` strokes are legacy and
  never drawn by exports except `eraser-area` (see §4.4 quirk).
- `syncStamp = {clock, actor}` (:835-852). `deletedStrokeStamps` is normalised by
  `ihnNormalizeDeletionStamps` (sorted keys; legacy id arrays get `{clock:0, actor:''}`)
  (collaboration-core-v5.js:400-438).
- **Image** = `{id, src, x, y, width, height, rotation, [syncStamp]}`; `x,y` = centre;
  `rotation` radians (:12651-12659).

Reader normalisation on import (:32398-32505):
- An entry **without an `images` key** is legacy-raster: images were flattened into the page, so the
  page is forced to `backgroundSource 'pdf'` and `images = []` (:32408-32417).
- `backgroundSource` missing → `hasCustomBackground ? 'pdf' : 'template'` (:32415-32416).
- Any non-template page (`pdf` or `custom`) becomes `backgroundSource: 'pdf'`,
  `pdfPageIndex = i` (its own position in this PDF), `backgroundImage: null` (:32418-32421, :32454-32472).
  **QUIRK**: after a reopen a photo page is a `'pdf'` page whose background is the saved PDF page.
- Missing size: page 1 reads its PDF page size; later pages use page 1's size then are refined in
  idle batches of 4 (:32426-32437, :32651-32681).
- Entries beyond the PDF page count are ignored; PDF pages without an entry become plain `'pdf'`
  pages with no strokes (:32483-32503).
- Without `IH_NORM:1`: duplicate stroke ids on a page are re-issued with fresh random ids, then
  duplicates removed (same fingerprint rule); either sets `needsMigrationResave` which forces a Drive
  save (:32338-32385, :26858, :27078-27082).
- Missing ids → deterministic legacy ids (§2.3) via `migrateLegacyPayload(..., {ensureLegacyIds:true,
  documentId: idbCacheKey})` (:32519-32530, :24118-24260).

### 3.5 Other sections (payload JSON)
- `IH_CAL` = `JSON.stringify(state.calendarPageConfig)`; written only if the config exists **and**
  `canUseAgendaCalendarForCurrentDocument()` (:34387-34394, :34970-34976). Shape
  `{mode, startDateKey, startPage, nextDateKey}` (:18412-18417).
- `IH_STRUCT` = `ihnCloneStructureMeta(...)`: `{v:1, clock, entries:{pageId:{rank, orderStamp:{clock,actor},
  createdStamp:{clock,actor}}}, tombstones:{pageId:{clock,actor}}}` (:1343-1345,
  collaboration-core-v5.js:92-133). Owned by the collaboration spec.
- `IH_FIELDS` = `{v:1, clock, fields:{key:{value, stamp:{clock,actor}}}}` with keys
  `doc:exportName`, `doc:calendarPageConfig`, `page:<pageId>:background|template|size|sidePanel`
  (:1463-1471, :1473-1553, collaboration-core-v5.js:464-486). When present it **overrides**
  `exportName` and `calendarPageConfig` from Title/IH_CAL on import (:32244-32255).
- `IH_SYNC` = `{v:1, writer: presenceClientId, contentHash, contentVersion}` where
  `contentHash = ihnCanonicalDocumentHash(pages, structure, calendar, exportName, fields)` (16 hex chars,
  collaboration-core-v5.js:19-31, :637-649) computed over the **written** (rounded) page entries
  (:34399-34411 via worker `prepareMetadata` document-processing-worker-v5.js:179-192; raster
  :34982-34986 → :7425-7447). Importer stores it as `driveConfirmedContentHash` (:32266-32267).
- `IH_VERSIONS` → §6.

### 3.6 Writer constraints for v6 (so v5 devices keep working) — all **MUST**
1. Exactly one PDF page per `STROKES_Z` entry, in order; `pdfPageIndex = position` for non-template pages.
2. Keep writing `images` (even empty) on every entry, else v5 treats the page as legacy-raster.
3. Write `;IH_NORM:1` only if ids are unique per page and duplicates removed.
4. Non-template pages must be visually correct **with ink removed** after one of: stripping the
   XObjects named in `IH_OVERLAY_REFS`, or using the attached `IH_CLEAN_ORIGINAL.pdf` (§4.6, §5.3).
   Do **not** write ink as `/Ink` annotations for v5-visible files: v5 renders annotation appearances
   into the background (pdf.js default annotation mode, :3440-3441) and would show every stroke twice
   with an un-erasable copy.
5. zlib-wrapped deflate, standard base64, sections in the §3.1 order, `IH_VERSIONS` archive schema 3
   (§6.2) — v5 rejects other archive versions (timeline-core-v5.js:306).
6. Sizes within v5 limits: STROKES/IH_VERSIONS encoded ≤ 48 MiB and inflated ≤ 96 MiB; IH_STRUCT/IH_FIELDS
   ≤ 8 MiB; IH_CAL ≤ 64 KiB; IH_SYNC ≤ 16384 chars; total PDF ≤ 64 MiB (security-core-v5.js:5) and
   ≤ 2000 pages (security-core-v5.js:6).

---

## 4. Export (serialization) pipelines

### 4.1 Entry point and decision tree — `buildPdfBlob(options)` (:34521-35049)
1. `ensureVersionHistoryRestored()` so a lazily-undecoded timeline is not dropped (:34522-34525, :15198-15211).
2. If a clean-original recovery is pending, start it and wait (never export from the baked display
   source) (:34529-34540).
3. If `canBuildCurrentPdfWithPdfLib()` (pdf-lib loaded, ≥1 page, and either clean original bytes exist
   or every page is `template` or has a `backgroundImage`) (:33874-33882): try the pdf-lib path up to
   **2 attempts**, the second with `forceMainThread: true`; `AbortError` propagates (:34552-34565).
4. Otherwise: if any page has `backgroundSource 'pdf'`, call `markDocumentLegacyBakedAfterRasterFallback()`
   — seeds white cover strokes for every PDF page with strokes, sets `hasLegacyBakedOverlay`, status
   `'Saved, but this PDF may not erase cleanly — reopen to retry reversible mode'` (error) (:34507-34519,
   :34572-34574) — then run the raster path.

Before building, both paths flush dirty pages (:33988-33993, :34646-34651) and load every page's data
(`loadPageDataForExport`: live page, else IndexedDB, else legacy cache; a page whose `strokeCount > 0`
cannot be loaded aborts with `Missing page data for export (page N)`) (:32751-32787).

### 4.2 Quality profiles
| Caller | options | pdf-lib overlay scale (px→device) | raster page scale | raster JPEG q (<20 / ≥20 pages) |
|---|---|---|---|---|
| Drive create/save/prepare | `DRIVE_UPLOAD_PDF_OPTIONS = {highQuality:false, optimizeForUpload:true}` (:377-380, :26436-26440, :11876, :25597) | **2** (`opt`) | 1.22 / 1.1 | 0.62 / 0.55 |
| Local export (no Drive file) | `{highQuality:true}` (:35156) | **min(3.5, CANVAS_SCALE×1.75) = 3.5** (`hq`) | `CANVAS_SCALE` = 2 (:311) | 0.85 / 0.75 |
| No options (tests, default) | `{}` | 2.5 (`std`) | 1.5 | 0.75 / 0.70 |

Source: :33948-33953, :34654-34663. Template backgrounds in the pdf-lib path are JPEG q **0.85** at
`round(pagePx × overlayScale)` (:34090-34119). Cache profile strings: `pdflib:${scale}:${opt|hq|std}:${png|raw|vector}`
(:33953) and `${hq|drv-opt|std}:${scale}:q${q}` (:34661-34663). A Drive-connected document's
"Export PDF" never builds a second PDF: it forces local+Drive save, then downloads the confirmed Drive
revision byte-for-byte (§4.7).

### 4.3 pdf-lib text-preserving path — `buildPdfBlobWithPdfLib` (:33890-34505)
Backend: the worker facade `InhouseDocumentProcessing.pdfLib()` unless `forceMainThread` or no Worker
(:33894-33900; facade document-processing-v5.js:34-103; worker ops document-processing-worker-v5.js:247-323).

Per document:
1. Load (and cache across saves, keyed by the clean-bytes reference and backend) the clean original
   with `{ignoreEncryption:true, updateMetadata:false}` (:33966-33986).
2. `outDoc = PDFDocument.create()`; batch `copyPages` of every state page with
   `backgroundSource === 'pdf'` and `1 ≤ pdfPageIndex ≤ originalPageCount` (source index
   `pdfPageIndex − 1`) (:33994-34013). Copying keeps page content, resources and `/Annots` but drops
   document-level structures (AcroForm dict, outlines, struct tree, page labels, XMP, other
   attachments) — **QUIRK**; the full original survives only in the attachment.

Per page *i* (:34025-34347):
3. Page creation: copied page → `addPage(copied)`, size from `getSize()`; otherwise
   `addPage([px×0.75, px×0.75])`, white `drawRectangle` full page, then either the page
   `backgroundImage` (data/blob URL decoded by magic bytes → `embedPng`/`embedJpg`; failure throws
   `Could not embed the image on page N`) drawn full-bleed, or the template JPEG for template pages
   (:34041-34120, :33747-33797). A `'pdf'` page with neither an original page nor a rendered
   background gets a blank white page.
4. Page entry for `STROKES_Z` (§3.4), compacted strokes cached per page object/generation (:34124-34196).
5. Overlay bounds: union of every drawable stroke's point box padded by `width×0.52 + 2`
   (highlighter width `max(4, width||12)`, pen `max(1,width||1)`) and every image's rotated box +2,
   clamped to the page, snapped outward to the device-pixel grid at `overlayScale`; no content → no
   overlay (:33799-33872, :34209-34217).
6. Overlay rendering (images first, then strokes; `eraser-stroke` and under-length strokes skipped):
   - worker + `pdfRaster` (worker document) + `vectorInk` (not `forceRasterInk`) + **no images** +
     every stroke `tool === 'pen'` with `#rrggbb` colour + total value count `≤ 8192` + crop raster
     pixels `≥ 32 × value count` → **vector Form XObject** from Rust `pdf_ink_path`
     (document-processing-worker-v5.js:214-221; rust-core-v5.js:440-472; rust-core/src/pdf_ink.rs).
   - else worker OffscreenCanvas raster: when `pdfRaster`, raw RGB + alpha planes are deflated and
     embedded directly as `/Image /DeviceRGB /BitsPerComponent 8 /FlateDecode` with `/SMask`
     `/DeviceGray /Decode [0 1]` (alpha omitted when fully opaque) (worker :125-158, :222-239);
     otherwise PNG.
   - else main-thread canvas PNG (:34238-34286).
7. Placement: `drawImage` at `x = bounds.x×sx`, `y = pageHeightPt − (bounds.y+bounds.height)×sy`,
   `w = bounds.width×sx`, `h = bounds.height×sy`, `sx = pageWidthPt / logicalWidthPx` (:34291-34303).
8. Record the overlay XObject name (`Image-<n>` for rasters, `Ink-<n>` for vector; pdf-lib picks
   `<tag>-<floor(random×10^10)>`), or the object number if the name is unknown, into `overlayRefs`
   as `{pageIdx: i (0-based), name|refNum}` (:34304-34346; worker :285-305).

Per document again:
9. Keywords assembly (§3.1) including timeline capture (§6.3) (:34351-34435); set Keywords, Title,
   Subject, Creator (:34436-34445).
10. Attach `IH_CLEAN_ORIGINAL.pdf` (§4.6).
11. `save({useObjectStreams: true, addDefaultPage: false})` (:34466-34469). Blob type
   `application/pdf`; `blob.inhouseContentHash`, `blob.inhouseOpenCacheAnalysis = {keywords, embeddedStrokes:null}`
   (:34473-34480).
12. Success self-heals: clears `hasLegacyBakedOverlay` and every page's `legacyCoverStrokes` (:34486-34494).

### 4.4 Vector ink content stream (bit-exact, rust-core/src/pdf_ink.rs:4-52)
Input per stroke: `width, r, g, b (0..1 = hex/255), count, x0, y0, …` (rust-core-v5.js:460-467). Output
(each number `{:.6}`), Form `BBox [0 0 bounds.w bounds.h]`, `/Resources <<>>`, FlateDecode (zlib):

```
q
1 0 0 -1 {-bounds.x} {bounds.y+bounds.height} cm
1 J 1 j
{width} w
{r} {g} {b} RG
{x0} {y0} m
{x1} {y1} l
…
S
…(next stroke)
Q
```

Validation → `null` (raster fallback): non-finite or |v| > 1e7 coordinates, width ≤ 0 or > 1e5, colour
outside 0..1, count < 2 or > 1e6 or fractional, truncated input, input > 3e6 values, output > 128 MiB
(pdf_ink.rs:56-74). **QUIRK**: vector ink is a polyline through every sample; on-screen and raster ink
use a centripetal Catmull-Rom spline (:13606-13637, worker :84-107). Pressure `p` is ignored.

**QUIRK (highlighter)**: highlighter uses `multiply` compositing with alpha 0.22 then 0.10
(:13534-13564). In overlays it multiplies against transparency (= normal alpha over the PDF page);
in the raster path it multiplies against the rendered background. `eraser-area` strokes (legacy) are
drawn with `destination-out`: in overlays they erase only ink, in the raster path they also erase the
background pixels (:34876-34882 draws all non-`eraser-stroke` strokes; worker :73-75).

### 4.5 jsPDF raster path (:34576-35049)
1. Per page: reuse the cached JPEG if `computePageHash(page)|exp:WxH|profile` matches
   (:33012-33032, :34694-34706); hash covers stroke/image fingerprints, deletions, background source,
   pdfPageIndex, `backgroundImage` length + hash of first 160 chars, size and stroke count.
2. Render to a canvas of `round(px × exportScale)`: optionally the live page canvas (only when same
   size and no CSS background, :32789-32801); else white fill, then background:
   `backgroundImage` (custom; decode failure throws `Could not decode the image on page N`) stretched
   full page; or pdf.js render of `activePdfDocument.getPage(pdfPageIndex)` at
   `scale = max(cw/vw, ch/vh)`, `intent:'print'`, stretched; or the template (:34808-34863).
3. Then `drawLegacyCoverStrokes` (white, width ×1.35 pen / ×1.05 highlighter) (:13754-13773),
   images, strokes (:34867-34918); JPEG at the profile quality (:34923-34934).
4. Metadata as §3 (no overlay refs); worker assembles: first page `new jsPDF({orientation, unit:'mm',
   format:[wMm,hMm], compress:true})`, later `addPage([wMm,hMm], orientation)`,
   `addImage(jpeg,'JPEG',0,0,wMm,hMm)`; zero pages → A4 portrait; `setProperties`; `output('arraybuffer')`
   (:33058-33123); 120 s timeout (:33042).
5. If pdf-lib and clean bytes exist, reload the jsPDF output (`updateMetadata:false`), attach
   `IH_CLEAN_ORIGINAL.pdf` unconditionally and save with object streams; failure keeps the unattached
   PDF (:35018-35039).

### 4.6 `IH_CLEAN_ORIGINAL.pdf` attachment
- pdf-lib `attach(bytes, 'IH_CLEAN_ORIGINAL.pdf', {mimeType:'application/pdf',
  description:'Clean original PDF used by Inhouse Notes for full revert'})` (:34455-34460, :35026-35029).
- pdf-lib path attaches only when `needsCleanOriginalAttachmentForSave()`: clean bytes exist and the
  page structure differs from the original (count differs, or some page not `'pdf'`, or
  `pdfPageIndex !== i+1`) (:14527-14540). When the structure is identical, `IH_OVERLAY_REFS` stripping
  reproduces the original pages, so no attachment is written. Raster path always attaches when bytes exist.
- Reader: pdf.js `getAttachments()`; entry keyed `IH_CLEAN_ORIGINAL.pdf` or with that `filename`;
  rejected above 64 MiB (:33585-33612).

### 4.7 Export to the user (file, Android, Drive)
- Menu → modal prefilled with `state.exportName || 'cuaderno'`; trailing `.pdf` removed,
  `sanitizeFilename` (replace runs of `\/:*?"<>|` with `_`, trim, fallback `cuaderno`), stored as the
  new `exportName` (collab field) unless read-only, then `exportPDF(name + '.pdf')` (:31782-31822).
- `exportPDF`: single-flight (`'A PDF export is already in progress'`); Drive document →
  `exportDrivePdf`, else `buildPdfBlob({highQuality:true, status:false})`; status `'PDF exported'`
  (:35139-35166).
- `exportDrivePdf`: needs a token; if editable: flush ops/pages, local save, wait ≤ 60 s for local
  idle, force upload until `driveUploadedContentVersion ≥ target` (≤ 300 s), then download the stable
  head revision and deliver it (:35050-35137). Loading texts are Spanish:
  `'Guardando últimos cambios en Drive...'`, `'Descargando PDF de Drive...'`, `'Generando PDF...'`.
- Delivery (:35231-35290): Android bridge v2 (`beginPdfExport(name)`, base64 chunks of 240 000 chars
  via `appendPdfExportChunk`, `finishPdfExport()`, then await `window.handleInhousePdfExportResult({ok,message})`
  ≤ 30 s); v1 bridge without the wait; legacy `savePdf(name, base64)`; app `navigator.share`; else an
  `<a download>` click (+ `window.open` in the app), URL revoked after 60 s (app) / 1.2 s (web).

---

## 5. Import (deserialization) pipelines — `importPDFData` (:32003-32732)

### 5.1 Sources
| Source | Call | Notes |
|---|---|---|
| Editor "Import PDF" (`<input accept=".pdf">`) | `importPDF` → blob URL + `sourceBlob` (:32734-32745, :16321-16326) | **Replaces the open document's pages**; does not clear `driveFileId`, so an open Drive file is overwritten on its next save (QUIRK). Non-strict. |
| Drive open | `openDriveFile` → cached blob or download (:11260-11560) | strict; `driveFileId` option; may pass `cachedAnalysis {keywords, embeddedStrokes}` to skip metadata parsing |
| Offline library open | offline-app-v5.js:131 | strict |
| Public/shared link | :8559-8565, :8712-8718 | strict; blob fetched first |
| Arbitrary URL | `isUrl` + `url` | only `blob:`, same-origin, or https `www.googleapis.com`, `drive.google.com`, `drive.usercontent.google.com` (security-core-v5.js:18-22, :103-117) |

There is no drag-and-drop PDF import (drop accepts images only, :12690-12710).

### 5.2 Safety limits (security-core-v5.js)
Blob ≤ 64 MiB and type empty/`application/pdf`/`application/octet-stream` (:84-98); pdf.js params
forced `isEvalSupported:false`, `maxImageSize ≤ 40 000 000` px (:119-146); page count 1..2000 else
`'PDF page count exceeds the safe limit'` (:148-159). Error text for the size pre-check:
`'This PDF is larger than the 64 MB safety limit.'` (:32005-32008). pdf.js 3.11.174 with packed
CMaps and standard fonts, `useSystemFonts:true` (:87-92, :32094-32099).

### 5.3 Steps
1. New document session; `calendarPageConfig = null`; loading `'Analyzing PDF...'` (:32017-32026).
2. Capture original bytes (≤ 64 MiB) eagerly for ArrayBuffer input, lazily with one retry for
   blob/URL input (:32036-32086).
3. Load with pdf.js; page-1 size from cache analysis or page 1 (:32101-32134); `resetPdfState()`;
   background scale policy (§7); blob URLs of files > 50 MiB are revoked immediately (:32163-32171).
4. Parse Keywords (§3.3). Status `'PDF with strokes detected...'`.
5. Timeline is **not** decoded now: `pendingVersionHistoryRestore` is armed and run on first Timeline
   open or before any export (:32269-32278, :15198-15230).
6. Clean-original policy (:32279-32334):
   - No embedded strokes and bytes captured → those bytes **are** the clean original; cached in IDB
     under the Drive file id.
   - Embedded strokes with ≥1 page needing a PDF background → recovery order **IDB cache →
     `IH_CLEAN_ORIGINAL.pdf` → strip `IH_OVERLAY_REFS` from the saved bytes** (IDB checked at once,
     the rest deferred to idle maintenance, :32565-32631).
   - `restorePdfBackgroundsIfNeeded` (reopen of an already-open doc) uses a different order:
     **attachment → overlay strip → IDB** (:7294-7339). QUIRK: keep both orders.
   - Stripping = delete each listed name (or the XObject whose ref objectNumber matches) from the
     page `Resources/XObject`; content-stream `Do` operators remain; save with object streams; 0
     removals → failure (:33647-33721).
7. Display source: if any page needing a PDF background has no drawable vectors
   (`pageEntryHasDrawableVectors`: a non-eraser stroke with ≥2 points, ≥1 for highlighter, or ≥1 image)
   the **saved PDF stays the display source** and `hasLegacyBakedOverlay = true` (:7642-7669, :32326-32334).
   Otherwise the display swaps to the clean original once recovered (`swapDisplayDocToCleanOriginal`
   drops cached backgrounds and clears legacy covers) (:7137-7196, :32592-32610).
8. Legacy-baked mode (no clean original recoverable): white `legacyCoverStrokes` (clone of the page's
   strokes) are painted under the ink so erasing a stroke also hides its burnt-in copy; seeded on
   page 1 at open and lazily for other loaded pages (:32439-32451, :32615-32627, :13735-13752).
9. Build pages (§3.4 reader rules); reset undo history (:32388-32396); `migrateLegacyPayload`
   (first page eager, others deferred one page per idle turn, :24118-24260); render; user imports
   (no `driveFileId`) schedule a local save (:32533-32537).
10. Status `'PDF imported: N pages, M strokes restored'` or `'PDF imported: N pages (as background)'`;
    errors → `'Error importing PDF'` (:32541-32546, :32717-32725).
11. Idle maintenance (first run after 1.8 s, re-armed every 0.7 s while the user is active or a save
    runs) performs clean-original recovery, lazy original-byte capture and page-size refinement
    (:32694-32711). Exports wait on it (§4.1 step 2).

**DEFECT?** The clean original (IDB or attachment) is swapped in without checking that the current
page list still maps 1:1 onto it. After deleting/reordering/inserting pages and saving, entries carry
`pdfPageIndex = export position`, but those indices are then resolved against the clean original
(both for display and for the next export's `copyPages`). The photo-page test only covers pages
without ink (which keep the saved PDF as display source; tests/e2e/photo-page-persistence.spec.mjs:2-35).
v6 should keep per-page provenance (source page index in the clean original) and fall back to the
overlay-stripped saved PDF when the mapping is unknown; verify with a v5 e2e first.

### 5.4 Missing page data recovery
If a page's local data is lost, `recoverMissingPageDataFromActivePdf` re-reads the entry from the
active PDF's Keywords and rebuilds the page (legacy covers when no clean original) (:7685-7741).

### 5.5 What v5 does **not** import
- `/Ink` (or any) PDF annotations are **not** converted to strokes; they render as part of the
  background (pdf.js default annotation mode) and are preserved by `copyPages`.
- No flattened-ink recovery from rasterised pages (the scanner's ink separation is a camera feature,
  not PDF import).
- See §11 for the unshipped rebuild-v2 implementations.

---

## 6. Version timeline (§ 4.19 / 4.19b)

### 6.1 Entry model (normalised, :33296-33341)
```
{ id ≤160, originId ≤160, ts (ms, ≥0; invalid → now), author:{email ≤256, name ≤160 (fallback
  email | 'Unknown editor'), photo ≤1024}, summary ≤320 (default 'Saved'), kind ≤48 (default 'autosave'),
  isMilestone, schemaVersion: 3, parentId ≤160 | null, contentHash, deviceId ≤96,
  calendarPageConfig, exportName ≤220, pages: SnapshotPage[] (≥1, else entry dropped) }
```
- Milestone kinds: `baseline, restore, revert-original, pre-restore, manual` (:33306-33307).
  Synthetic `original` (id `__original__`) is UI-only, never stored (:14390, :14421-14487).
- `originId`: explicit, else the id with a legacy branch suffix `~[a-z0-9]+(_\d+)?` removed (:33314-33319).
- SnapshotPage (:33263-33293): `{pageId (default 'legacy-page-<n>'), strokes, images, deletedStrokeIds,
  deletedStrokeStamps, backgroundImage (custom pages with a non-blob URL only, else null),
  backgroundSource, templateKind, pdfPageIndex, pageWidth, pageHeight, sidePanel, legacyCoverStrokes}`.
- `contentHash = simpleHash(JSON.stringify({pages, calendarPageConfig, exportName})) + ':' +
  length.toString(36)` (:33203-33214; worker `timelineHash` is the same function,
  document-processing-worker-v5.js:193-197).
- New id: `v_${Date.now().toString(36)}_${random base36 8}` (:33154-33156). Author from the Google
  profile, name fallback `'You'` (:33158-33165). `deviceId = getPresenceClientId()`.

### 6.2 Archive format `IH_VERSIONS` (timeline-core-v5.js)
```json
{ "schema": "inhouse-timeline", "schemaVersion": 3, "checkpointInterval": 8,
  "entries": [
    { "type": "checkpoint", "meta": Meta, "hash": H, "snapshot": {pages, calendarPageConfig, exportName} },
    { "type": "delta", "meta": Meta, "baseHash": H_prev, "hash": H,
      "delta": { "order"?: [pageId…], "upserts"?: [[pageId, page]…], "removed"?: [pageId…],
                 "document"?: { "calendarPageConfig"?, "exportName"? } } } ] }
```
- `Meta` = own fields among `id, originId, ts, summary, kind, isMilestone, schemaVersion, parentId,
  contentHash, deviceId` (in that order), then `author`, with `schemaVersion` forced to 3 (:9-12, :84-98).
- Checkpoint when first entry, `index % 8 === 0`, the entry is a milestone, or a delta cannot be built
  (missing/duplicate/over-long page ids) (:154-202, :106-118).
- `H = hashString(stableStringify(snapshot))`: stable stringify sorts object keys recursively, drops
  `__proto__/prototype/constructor`, JSON semantics (undefined omitted, non-finite → null); point
  objects `{x,y}` / `{p,x,y}` short-cut keeps the same order (:23-56). `hashString` = FNV-1a 32-bit
  over each UTF-16 unit as two bytes (low then high), output base36 padded to 7 (:58-69).
  Pinned: `'0pkv6lo'` for the fixture in tests/timeline-core-v5.test.mjs:132-136.
- Materialise: replay; delta requires `baseHash === previous hash`; after each record recompute
  `H` and require equality (fail closed) (:303-337). Limits: ≤ 200 entries, ≤ 2000 pages, page id ≤ 256,
  hash ≤ 64 chars (:4-8, :215-225).
- **Compatibility**: a JSON **array** is the legacy full-snapshot format and is accepted as-is (:304);
  an object with another `schema`/`schemaVersion` yields `null` → empty timeline (:306).

### 6.3 Capture (§ 4.19b, :33481-33532)
- Called by **every PDF build** (Drive save, idle pre-build, export): pdf-lib path with the parsed
  page entries and a worker-computed hash (:34416-34434); raster path with the parsed entries
  (:34987-35000). Local-only documents therefore get entries only when a PDF is built.
- Skip when not a milestone and `last.contentHash === contentHash`.
- Coalesce (replace the last entry, keeping its id) when not a milestone, last not a milestone,
  same author email+name, same `deviceId`, and `ts − last.ts < 5 min` (:33150, :33494-33510).
  **QUIRK**: the coalesced entry gets `parentId = last.id` (its own id) and its summary is computed
  against the replaced entry's pages.
- Summary (`summarizeVersionDelta`, :33216-33261): parts joined by `', '` from
  `+N page(s)`, `-N page(s)`, `Pages reordered`, `+N stroke(s)`, `-N stroke(s)`,
  `N image change(s)`, `N page setting(s) changed`; nothing → `'Saved'`. Hint summaries override.
- `pendingVersionHint {milestone, kind, summary}` marks the next capture (restore/revert/manual).
- Pruning (`pruneTimelineHistory`, :33343-33441): per `originId` keep the newest variant per device
  (`deviceId` or `legacy:<email|name|unknown>`; tie → milestone, then larger stable JSON); drop
  non-milestone duplicates with the same `author:contentHash` (keep newest); extra device variants
  get ids `<origin>~<simpleHash(deviceKey:contentHash)>`; sort by `ts, id`; milestones capped at 30
  (latest 30 + first `baseline`); total capped at **80** by dropping the oldest non-milestone.
- `ensureTimelineBaselineFromPages` (:33461-33479) exists but is never called (dead code).

### 6.4 Embedding budget and storage
- `encodeVersionHistoryForKeywords` → worker `timeline` task with budget **3 500 000** (:33152,
  :33556-33564, :33171-33195; worker :324-342):
  1. While uncompressed archive JSON length > budget and >1 entry: drop the oldest non-milestone
     except the last.
  2. `retained = id:ts:contentHash` of survivors (in-memory history is pruned to these).
  3. Compress; while base64 length > budget: drop oldest non-milestone (not last), else oldest
     non-baseline (not last), else index 0; return `''` if still over (section omitted).
  QUIRK: entries dropped in step 3 stay in memory/IDB but not in the PDF.
- Local copy: IndexedDB `notebook-data-db-v1` (prefix `background-` in the offline worker) v4,
  store `timeline-history` keyPath `key`, record `{key, historyArchiveJson, savedAt, schemaVersion:3}`;
  key `drive:<fileId>` or `local:<simpleHash(lowercase name:firstPageId)>`; legacy records with
  `historyArchive`/`history` are read (:432-438, :22309-22362, :22380-22411).
- Adoption (open, remote pull): merge in-memory ∪ IDB ∪ incoming via `mergeVersionHistories`
  (normalise both, prune once), persist, assign (:22380-22411, :33454-33459, :28914-28925).

### 6.5 Restore (:14223-14296) and helpers
1. Slider confirm (`'Restore this version?'` / `'Revert to the original PDF?'` or `…document?'`).
2. Page-structure lock (12 s). Create a **pre-restore milestone** from the live pages
   (`'Before restoring <locale date>'`); failure cancels with
   `'Restore cancelled: a safe recovery point could not be created.'` (:33534-33554).
3. Original entry → swap display to the clean original.
4. Hint `{milestone, kind: 'revert-original'|'restore', summary}`; apply `calendarPageConfig` and
   `exportName` from the entry (collab field changes).
5. `buildPagesFromSnapshot` (custom pages keep `backgroundImage` only from the snapshot) (:14090-14111).
6. `applyRestoredPages` (:14113-14189): tombstone every current stroke/image id absent from the target
   page (matched by pageId, else index) (:14191-14221); **re-stamp all restored items** (a restore is a
   new causal write); clear the IDB page store; reset undo; replace pages; structure replacement in
   `IH_STRUCT`; mark all dirty; local save; force an immediate Drive upload.
- Original entry: with clean bytes → one empty `'pdf'` page per original page
  (`pageId 'original-page-<n>'`, sizes from the clean PDF); without → current pages stripped of
  strokes/images keeping template and size (:14421-14487).
- `revertToOriginalPdf` (:14298-14381) is unreachable (dead code).

### 6.6 Timeline UI semantics (rendering owned by the UI spec)
- List = display history (original first) newest-first, grouped by month then 1-hour clusters; only
  the newest cluster of the newest month expanded by default; tags `Current`, `Restored`, `Reverted`,
  `Original` (:14927-14965, :14972-15196).
- Preview: per page white fill, template, custom background from the **live** page at the same
  index, PDF page fitted/centred from `activePdfDocument`, images, strokes, then diff highlights vs the
  previous entry (added/changed: `#f59e0b` stroke + `rgba(245,158,11,0.12)`; removed: `#ef4444`
  dashed `[8,5]` + `rgba(239,68,68,0.08)`; line width 2.5) (:14621-14665, :14668-14845).
- "Revert" disabled for the current (latest) entry (:14847-14858). Zoom 0.2–3 via ctrl+wheel and
  pinch (:15282-15313).

---

## 7. PDF backgrounds on screen (§ 4.16a, :3163-3460)
- Scale policy `getPdfBackgroundScale(pages, bytes)`: ≥500 pages → 1.2; ≥200 → 1.5; > 80 MB → 0.9;
  > 30 MB → 1.25; > 10 MB or ≥30 pages → 1.6; else `min(3, CANVAS_SCALE×1.5) = 3` (:3163-3174).
- Render: `getPage(pdfPageIndex)`, viewport at `max(0.6, scale)`, white fill, pdf.js `render` with
  intent `'display'` (or `'print'` in the hidden/offline worker), JPEG **0.92** blob URL (fallback
  `toDataURL` JPEG 0.85), applied as CSS `background-image` with `background-size: 100% 100%`
  (:3405-3488). Custom backgrounds use `background-size: cover` (:3307-3318).
- Rendering is deferred while the viewport is interacting or within 240 ms of user activity, queued
  and resumed in idle callbacks (:3345-3403). LRU of ≤ 6 rendered backgrounds excluding active pages;
  eviction clears `page.backgroundImage` (:3273-3300, :334); memory pressure > 80 % heap evicts all
  non-active (:1187-1228).
- No pdf.js document yet → `restorePdfBackgroundsIfNeeded` downloads the stable Drive revision and
  re-runs clean-original recovery; retried by a save sync after 30 s on failure (:7198-7412).

---

## 8. Photos and images
- **Inline images** (paste/drop/insert): `compressImageForDocument` scales to ≤ 3200 px longest side
  and ≤ 8 MP; PNG ≤ 4 MiB that needs no scaling is kept byte-for-byte, else one encode (PNG keeps alpha,
  others JPEG q 0.94 on white) (:12527-12558, :354-356). Placement fits 60 % A4 width × 45 % A4 height,
  centred on the drop point, +18 px per extra image, clamped to **A4** bounds regardless of page size
  (QUIRK) (:12636-12659). History action `add-image`.
- **Photo pages** (Manage pages → cover/photo): `buildCoverImageAsset` → JPEG q 0.94 page-sized
  composite (white letterbox in `original` mode, page resized to the photo in `photo` mode), render
  scale ≤ 3 and ≤ 3840 px; small bounded PNGs kept losslessly (:20117-20181, :19918-19944, :403-405).
  Page `backgroundSource 'custom'`, `backgroundImage = dataUrl`; undo history cleared (:20374-20440).
- Persistence: inline images go into `STROKES_Z` (with `src`); custom backgrounds are only the PDF page
  pixels + field hash (§3.4). On reopen custom pages become `'pdf'` pages.

---

## 9. File names, MIME types, local caches
- Drive document name: `${sanitizeFilename(exportName || 'Untitled')}.pdf` (:26432); title edits:
  whitespace collapsed, `.pdf` stripped, sanitised, fallback `'Untitled'`; Drive rename PATCHes `name`
  with `.pdf` appended (:5923-5944, :5979-5998). Display title strips `.pdf` (:5737-5742).
  Agenda/Diary quick-access names match `/^(Diary|Agenda)\d{4}(?:-\d{4})?(?:\.pdf)?$/i` (:395-401).
- MIME: documents `application/pdf`; folders `application/vnd.google-apps.folder`; Home lists only
  those two (:10742-10814). Non-PDF device uploads keep `file.type || 'application/octet-stream'` (:11764).
- Drive create: resumable POST with `{name, mimeType:'application/pdf', parents}`; updates are
  conditional (`If-Match` etag) media PUT ≤ 4 MiB, else resumable in 1 MiB chunks (:26423-26548).
  (Sync semantics: see the Drive sync spec.)
- IndexedDB `inhouse-original-pdfs-v1` v2: store `originals` (out-of-line key = Drive file id,
  value = clean original `ArrayBuffer`) and `drive-open-cache` (keyPath `fileId`, index `cachedAt`)
  (:445-551). v6 **MUST** read `originals` so reopened documents keep non-destructive erasing offline.

---

## 10. Undo / redo (§ 4.11b, :30803-31288)

### 10.1 Model
- `state.history` (array of actions), `state.historyIndex` (−1 = nothing to undo), **`maxHistory: 10`**
  actions (:1301-1303). Not persisted; per editor session.
- Memory cap `MAX_HISTORY_MEMORY = 25 000` points (:1094): oldest actions removed while the weight sum
  exceeds the target, always keeping the newest (:1135-1157). Weight counts `action.stroke.points`,
  `action.strokes[*].stroke.points`, `action.before[*].stroke.points` (array) or
  `action.before.strokes[*].stroke.points`, `action.after[*].stroke.points` (array) only (:1095-1126) —
  **QUIRK**: raw-stroke arrays (`erase-area`) weigh 0. Targets halve when the tab hides (:1158-1170),
  0.4× on > 80 % heap every 15 s (:1187-1228), full trim after each Drive save (:27076).
- `saveToHistory(action)`: ignored when read-only; fills `pageId` (and `fromPageId/toPageId`);
  truncates the redo tail; pushes; if length > 10 shift (index unchanged) else index++; memory trim
  (:30948-30981).
- Cleared on: document open (:11843-11845), PDF import (:32394-32396), timeline restore (:14150-14151),
  photo page insert (:20434-20435), scanned pages insert (:20541-20542), document reset
  (:21310-21311), page delete (:23446-23447), remote structural merge (:29908-29912).

### 10.2 Actions (record shapes and inverse)
| type | recorded by | undo | redo |
|---|---|---|---|
| `add-stroke` `{stroke}` | pen-up (:4698-4707) | remove by id; tombstone | re-append at end; untrack tombstone (re-stamps) |
| `erase-strokes` `{strokes:[{index,stroke}]}` | stroke eraser (:4610-4621, :13405-13427) | reinsert at recorded indices (ascending, clamped); untrack | remove by id; tombstone |
| `erase-area` `{before: removed strokes, after: new fragments}` | area eraser (:4636-4646) | `applyHistoryStrokeSnapshotDelta(after → before)` | `(before → after)` |
| `add-image` `{index, image}` | (:12663-12668) | remove by id; tombstone | reinsert at index; untrack |
| `delete-images` `{images:[…]}` | (:5333-5337) | restore; untrack | remove; tombstone |
| `delete-selection` `{strokes, images}` | (:5394-5399) | restore both; untrack; clear selection | remove; tombstone |
| `duplicate-selection` `{strokes, images}` | (:5486-5491) | remove; tombstone | restore; untrack |
| `color-change` `{before, after}` | (:5532-5537) | replace by id with `before`; re-stamp | with `after` |
| `transform-selection` `{before:{strokes,images}, after}` | (:13117-13122) | replace both; re-stamp | with `after` |
| `cross-page-move` `{fromPageId, toPageId, strokeIds, imageIds, yOffset, …}` | (:12796-12808) | move ids back, `y −= yOffset`; re-stamp; tombstone on target, untrack on source | forward, `y += yOffset` |
| `transform-strokes`, `replace-page` | no producer in 5.17.22 | supported (:31024-31041) | supported |

`applyHistoryStrokeSnapshotDelta(source, target)` (:30900-30946): ids whose fingerprint differs or that
exist on one side only are "touched"; touched strokes absent from target are removed (tombstoned);
target strokes are written in place or **appended** (QUIRK: an undone area-erase brings strokes back
on top of the z-order).

### 10.3 Execution rules
- Blocked when read-only, during a local/remote page-structure mutation (:30984, :31130).
- The page is resolved by `pageId` first (index fallback); unknown page → step consumed, no-op
  (:30817-30826, :30989-30995). The page is loaded (keepLoaded) before applying.
- After applying: update buttons, `scheduleSave(false, pageIndex)`, unload the page if not active
  (:31121-31126). Buttons disabled when read-only or nothing to undo/redo (:31278-31288).
- Triggers: toolbar `#btn-undo` / `#btn-redo`; keyboard **Ctrl+z** / **Ctrl+y** exactly (`e.ctrlKey`,
  lowercase key; no Cmd, no Ctrl+Shift+Z) (:16339-16358).
- Collaboration: every undo/redo write is a new causal write (re-stamped items, fresh deletion stamps)
  so it competes correctly with remote edits (:5265-5301, :845-852).

---

## 11. Not shipped in v5: `rebuild-v2/` prototype (reference only)
`rebuild-v2/STATUS.md` states the legacy app remains the shipped version; none of this runs in v5.17.22.
- `/Ink` import (rebuild-v2/src/import/pdfImport.ts:132-161): pdf.js annotations with
  `annotationType 15` or `subtype 'Ink'`, colour from `C` (0..1 or 0..255 → `#rrggbb`), opacity < 0.75
  → highlighter, width `max(0.7, BS.W||1)`, each `inkList` → one stroke via `convertToViewportPoint`,
  `p = 0.5`, single points duplicated; only on pages with no legacy strokes.
- Flattened-ink recovery (flattenedInk.ts:39-124): render at ≤ 2200 px longest side (scale 1–2),
  36-bin hue histogram of chromatic pixels, candidate mask within 28° of the dominant hue, coverage ≤ 8 %,
  ≥ 18 core pixels, component filters (area, fill ratio > 0.46 rejection, light neutral ring ≥ 72 %),
  downsample, Zhang–Suen thinning, skeleton tracing, RDP 0.55, ≤ 4000 strokes; ink pixels in-painted
  from neutral neighbours; cleaned page re-wrapped as a one-page PDF asset.
- `/Ink` export (rebuild-v2/src/export/pdf.ts:185-218).
- **Do not port its legacy reader**: `legacyPayloadFromPdfKeywords` returns the parsed v5 **array** but
  callers read `payload.pages[...]` (pdfImport.ts:83, persist/migrate.ts:74-92), so v5 strokes are
  silently ignored.
If v6 adds these features they are new behaviour: gate them so v5-readable output still follows §3.6
(e.g. convert `/Ink` to strokes only while removing those annotations from the background source).

---

## 12. Round-trip guarantees (v5 → v6 → v5)
1. Page order, `pageId`s, stroke/image ids, tool, colour, width, image geometry and `src`, tombstones,
   `templateKind`, sizes and agenda `sidePanel` survive unchanged.
2. Point coordinates and pressure are quantised to 0.01 on the first save; further saves are idempotent.
   In-memory strokes are not rounded until the document is reopened.
3. The clean original bytes are preserved byte-for-byte (IDB or attachment) and remain usable for revert.
4. pdf-lib path preserves original page content (text, vectors, fonts, annotations); raster path does not.
5. `materializeArchive(createArchive(h))` deep-equals `h` for JSON history (timeline tests).
6. Not guaranteed (do not test bytes): zlib/PDF byte layout, XObject random suffixes, dates,
   jsPDF/pdf-lib producer strings. Parity tests compare decoded Keywords sections and rendered pixels.

---

## 13. Tests pinning this behaviour (one line each)

Existing (keep, port to v6):
- tests/timeline-core-v5.test.mjs:66 — archive round-trips edits/reorders/adds/deletes; checkpoints at 0, milestones, every 8.
- tests/timeline-core-v5.test.mjs:85 — legacy full-snapshot arrays readable without mutation.
- tests/timeline-core-v5.test.mjs:93 — tampered delta / broken base hash fail closed.
- tests/timeline-core-v5.test.mjs:105 — >200 entries rejected; foreign schema → null.
- tests/timeline-core-v5.test.mjs:114 — direct JSON encoder byte-equals `JSON.stringify(createArchive)`.
- tests/timeline-core-v5.test.mjs:132 — hash `'0pkv6lo'` fixture incl. surrogate pair and odd point keys.
- tests/history-memory.test.mjs:39 — history compaction keeps exact legacy weights and undo index.
- tests/history-memory.test.mjs:60 — newest action retained even when alone over the memory limit.
- tests/rust-core.test.mjs:161 — vector PDF ink keeps every point/width/colour; rejects highlighter, NaN, dense, tiny crops.
- tests/rust-core.test.mjs:13 — lossless RGB/alpha split for raw overlay rasters.
- tests/pdf-clean-original-cap.test.mjs:23 — clean-original capture cap equals `MAX_PDF_BYTES` (64 MiB).
- tests/e2e/pdf-nondestructive-erase.spec.mjs:24 — clean original available → never legacy-baked.
- tests/e2e/pdf-nondestructive-erase.spec.mjs:33 — transient pdf-lib failure retried, not rasterised.
- tests/e2e/pdf-nondestructive-erase.spec.mjs:54 — persistent failure marks legacy covers, still attaches clean original, self-heals.
- tests/e2e/save-cache.spec.mjs:2 — STROKES_Z inflates to JSON containing new ink; compact cache reuse/invalidation.
- tests/e2e/save-cache.spec.mjs:53 — `prepareMetadata` equals `compress` bytes and `contentHash`; rename changes hash.
- tests/e2e/editor-performance.spec.mjs:46 — raw-raster overlay pixels equal PNG path; literal Keywords readable by pdf.js.
- tests/e2e/photo-page-persistence.spec.mjs:2 — photo page survives save, reload and PDF reopen (with/without original).
- tests/e2e/photo-page-persistence.spec.mjs:62 — unreadable photo rejects export instead of saving a blank page.
- tests/e2e/pdf-photo-provenance.spec.mjs:84 — custom page entry has `pdfPageIndex`, no inline image, field hash.
- tests/e2e/processing-lifecycle.spec.mjs:56 — overlapping exports keep their source PDF alive.
- tests/release-v5-smoke.mjs:772 — IH_FIELDS encodes/decodes with a following `;IH_SYNC:` section.

New for v6 (write before porting):
- Keywords grammar: v6 output parses with the v5 reader functions (run app-v5.js decoders in a VM) for every section.
- zlib not raw: v6 `STROKES_Z`/`IH_VERSIONS` inflate with `DecompressionStream('deflate')`.
- Section order and `;`-delimiting exactly as §3.1; unknown `;IH_X:` sections ignored by the v6 reader.
- `STROKES_DATA` fixture opens and schedules a migration resave producing `STROKES_Z` + `IH_NORM:1`.
- Entry without `images` key opens as legacy-raster `'pdf'` page with no images.
- Missing `IH_NORM` → duplicate-id repair + dedupe; present → untouched.
- Legacy ids: id-less strokes/pages get the exact `st_legacy_…`/`pg_legacy_…` values for a fixed Drive id.
- Size guards: 48 MiB encoded / 96 MiB inflated rejection; IH_CAL > 64 KiB, IH_STRUCT > 8 MiB, IH_SYNC > 16384 ignored/rejected as §3.3.
- Strict open: corrupt IH_FIELDS aborts a Drive open but not a user file import.
- Overlay refs: v6-saved PDF stripped by v5 `stripOverlaysFromPdfBytes` equals the clean page render.
- Attachment rule: written iff structure differs (pdf-lib path); always on raster fallback.
- Profiles: overlay scales 2 / 2.5 / 3.5 and raster scale/quality table of §4.2.
- Vector ink content stream byte-equals the Rust fixture in §4.4 for a given stroke and bounds.
- Point rounding to 0.01 and idempotence across two saves.
- Raster page size in mm and orientation for 794×1123 and 595×842 pages.
- Timeline capture: identical content skipped; 5-minute same-device coalescing; milestone never coalesced.
- Timeline pruning: 80 entries, 30 milestones + baseline, branch ids `~hash` for concurrent variants.
- Timeline budget: payload omitted when > 3 500 000 chars after pruning; in-memory prune by `retained`.
- Restore: pre-restore milestone created first; tombstones for removed items; items re-stamped.
- Undo: 10-action cap, 25 000-point memory cap, cleared on each event of §10.1, Ctrl+z/Ctrl+y only.
- Undo of area erase re-appends strokes at the end of the stroke list.
- Clean-original page mapping after delete/reorder (§5.3 DEFECT?) — expected behaviour decided by the lead.
