# Inhouse Notes 6.0.0 — Intended differences from 5.18.0

The parity suite (TESTING §3) fails on any difference from 5.18.0 that is not listed here. Every entry has an id that
parity states reference. **Status of every entry: proposed — needs the user's approval before release**
(REQUIREMENTS §1 and §6). Rule used to decide: keep every intended look and behaviour; do not reproduce evident bugs;
keep intentional-looking oddities (listed in §5 as decisions the user may change).

Legend — *Visible*: **yes** (pixels or flows differ in some state), **flow** (only behaviour over time differs),
**no** (internal; listed so the user knows).

---

## 1. Removed features shown as "Soon" (unchanged from 5.18.0)

5.18.0 already removed these and shows "Soon"; v6 keeps exactly that, with the same markup, badge and toast.

| Id | Entry point | 5.18.0 and v6 behaviour |
|---|---|---|
| S-1 | `#btn-scan-page` "Scan" in the Manage-Pages panel | Soon badge; tap shows the `#soon-toast` "Soon" for 1.8 s. No camera, no scanner assets. Photo pages from the gallery still work. |
| S-2 | Profile menu `Diary` / `Agenda` (`#btn-drive-menu-diary`, `#btn-drive-menu-agenda`) | Soon badges; tap shows the toast. No agenda/diary/academic-year creation. |
| S-3 | Google Calendar | No calendar scope, API calls, side panels, month/day modals, event editor or 5 s sync. Existing agenda/diary pages keep their SVG page background; side-panel data is kept but not shown. Diary/Agenda PDFs are ordinary documents. |

---

## 2. v5 bugs fixed (REQUIREMENTS §6)

### 2.1 Home, links, shell

| Id | 5.18.0 | v6 | Why | Visible | Affected parity states |
|---|---|---|---|---|---|
| B-H1 | REQUIREMENTS §6 lists the public-link download bug of 5.17.22 (`fetchWithDeadline` consumer in the timeout slot, so signed-out recipients always got the Drive preview iframe). **5.18.0 already fixed it** (`app-v5.js:7533` passes the 45 s timeout before the consumer). | v6 reproduces 5.18.0: download (`drive.usercontent.google.com`, then `drive.google.com/uc`, credentials omitted, `%PDF-` check), open read-only with the 5.18.0 strings and polling; the iframe preview stays the fallback when the download is refused. | Listed so the requirement is visibly closed. | no (same as 5.18.0) | share/public-link viewer (must match 5.18.0) |
| B-H2 | Create-menu `ArrowUp` moves focus forward like `ArrowDown` (spec/home.md §12.6). | `ArrowUp` moves to the previous item. | Evident keyboard bug. | flow | none (keyboard only) |
| B-H3 | Toggling Night mode at runtime leaves `meta[name=theme-color]` and the `html` inline background at the boot colour until reload (Android status bar stays the old colour) (spec/shell.md §19.2). | The toggle also updates `#app-theme-color` and `html.style.backgroundColor` with the boot colours (`#151515` / `#f5f5f0`). | Evident inconsistency between boot and runtime theme. | **yes** (browser/Android chrome; page pixels only on overscroll) | profile-menu night toggle (meta tag in DOM diff) |
| B-H4 | Android PKCE `state` is not validated (spec/shell.md §8.5). | `handleInhouseNativeOAuth` requires `state=inhouse_notes_pkce`; otherwise `Sign-in failed. Try again.` | Security hardening; legitimate flows are unchanged. | no | — |

### 2.2 Editor (spec/editor.md §11)

| Id | 5.18.0 | v6 | Why | Visible | Affected parity states |
|---|---|---|---|---|---|
| B-E1 | Selection move/scale, duplicate clamping and image placement/clamping use A4 constants (794×1123) whatever the page size (Q1). On A5 agenda/diary pages and imported PDF pages (595×842) the selection moves faster/slower than the pointer and images are placed as if on A4. | The page's real size is used. Identical on A4 template pages. | Evident bug. | **yes** (non-A4 pages) | editor-tools selection/images on A5 and PDF pages |
| B-E2 | Cross-page drag shifts y by exactly ±1123 regardless of real page heights and gaps (Q2). | Shift = real distance between the two pages' origins in page px. | Evident bug (strokes land at the wrong height on non-A4 pages). | **yes** (non-A4 pages) | editor-tools cross-page drag on A5/PDF |
| B-E3 | A single-tap pen dot is shown live but disappears at the next repaint (pens need ≥ 2 points) (Q4). | The dot is kept and redrawn with the live dot rule (filled circle radius `max(0.7, width/2)`), in repaints, thumbnails and export. | Evident data-visible bug (user's dots vanish). | **yes** | canvas scenes with dots (i, full stops) after repaint |
| B-E4 | Undoing an area erase re-appends the restored strokes on top of the z-order (Q6). | Restored strokes return to their original z position. | Evident bug (highlighters end up over ink). | **yes** (after undo) | editor-tools undo after area erase |
| B-E5 | Layout metrics centre narrower pages (`left = 30 + (maxW − w)/2`) and assume 30 px padding, while the DOM left-aligns rows and uses 20 px padding at ≤768 px (Q3). | Metrics follow the DOM (left-aligned, 20/30 px). The DOM and CSS are unchanged. | Evident mismatch; affects which page is considered visible and the add-page indicator offset. | **yes** (indicator offset on phones with mixed page widths) | editor-view pull-to-add on phone with mixed sizes |
| B-E6 | Erasers test sample positions only; a fast swipe skips thin strokes between samples spaced ≥ `max(0.8, 0.18·eraserWidth)` (Q7). | Erasers test the swept segment between consecutive samples (capsule of radius eraserWidth/2). | Evident bug (erasing misses ink). | flow | none at paused instants; e2e 04 |

### 2.3 Documents and export (spec/documents.md)

| Id | 5.18.0 | v6 | Why | Visible | Affected states |
|---|---|---|---|---|---|
| B-D1 | Exported PDFs draw ink as page rasters (PNG/raw) or, for sparse pen-only pages, as polylines through every sample; a jsPDF raster fallback can flatten a whole document; highlighters multiply against transparency; imported PDFs also embed a second clean copy of the source (`IH_CLEAN_ORIGINAL.pdf`) when the structure changed. | Ink is one vector layer per page (a separate content stream over the unchanged source page, so no second copy of the source is stored): pens as the same Bézier spline as on screen, highlighters with `/Multiply` blend at alpha 0.22 + 0.10, images as image objects. No raster fallback: if pdf-lib fails, the save reports an error and retries (ops are already durable through sync, so nothing is lost or flattened). | Smaller files, sharper ink in other PDF apps, no destructive saves, much less code (no jsPDF, no overlay rasterizer). | **yes, outside the app** (other PDF readers, Drive thumbnails); in-app screens identical | none in the app; e2e 08 checks the file |
| B-D2 | After deleting, reordering or inserting pages, `pdfPageIndex` is the export position but is resolved against the clean original, so a later export can copy the wrong original page (DEFECT, §5.3). | Each PDF page keeps its provenance (source asset + page index); exports copy the right page. | Evident data bug. | **yes** (only in the affected documents) | open: PDF doc after reorder (fixture) |
| B-D3 | Overlays on PDF pages with `/Rotate ≠ 0`, a non-zero MediaBox origin or CropBox ≠ MediaBox are misplaced in exported files (QUIRK/DEFECT, §2.2). | Ink is placed with CropBox origin and rotation. Editable coordinates are unchanged. | Evident export bug. | **yes, outside the app** | — |
| B-D4 | Raster-path exports of imported PDF pages come out at 75 % of physical size (QUIRK, §2.2). | Gone with the raster path (pages keep their size). | Evident bug. | **yes, outside the app** | — |

### 2.4 Collaboration and security (spec/collaboration.md §12, §16)

| Id | 5.18.0 | v6 | Why | Visible |
|---|---|---|---|---|
| B-C1 | The live signalling key property `ihn_live_key_v1` matches the `ihn_live_` preview prefix and is fetched as a preview file: a 404 every 3 s. | The 5.x signalling/preview mechanism is not used (SYNC.md). | Evident bug; also wasted requests and battery. | no |
| B-C2 | The signalling key is stored in public file `properties`, readable by viewers. | No signalling key exists (B-Y4). | Security. | no |
| B-C3 | Peers are never checked for edit rights; a viewer's snapshot is merged additively and re-uploaded by editors. | Only installations whose relay files are sealed by an editor-only property write can connect or contribute ops; viewers never send ops. | Security (REQUIREMENTS §6). | no |
| B-C4 | A journal op for an unknown page throws and blocks that source until the page arrives (hazard 7); history-prefix conflicts retry forever (hazard 8). | Ops are order-independent by design (ARCHITECTURE §4.2); no blocking states. | Evident bugs. | flow (faster convergence) |

---

## 3. Consequences of the new design (not bugs; the user should know)

| Id | 5.18.0 | v6 | Why | Visible | Affected parity states / expected-diff rule |
|---|---|---|---|---|---|
| N-1 | Every saved edit eventually rewrites the PDF in Drive (debounced 0.04–3 s, often several uploads per minute while writing, by every editor). | Edits sync instantly as operations; the PDF in Drive is rebuilt by one device (the scribe) after 10 s idle, by a device leaving alone, and on user Export (SYNC §14). Other PDF apps and Drive's own preview/thumbnail/`modifiedTime` see changes after that delay; Drive keeps fewer revisions. | REQUIREMENTS §2, network and battery. | flow | Sweep states that cross time (`open doc → write → Home`: Recents order, card "edited" text, Drive thumbnail) are captured only after **both** apps settle (5.18.0 upload done; v6 export done), so they must not differ. No other state may differ. |
| N-2 | The save indicator's `saved` means "PDF uploaded to Drive"; Home's pending badge means "outbox upload pending". | `saved` and "no badge" mean "every own edit is sealed in the Drive relay" (ARCHITECTURE §6.6); `building`/`uploading` appear while an export runs. Same classes, colours and badge texts. The save-details popup lines that described PDF build phases describe the export; the error report's `<phase>` may be `relay`. W3.1/W5.5 append the exact changed strings here before merging. | Follows N-1. | **yes** (save-details popup text, error report phase) | `export/save details popup`: only the listed lines differ. Save-indicator and badge states: none (captured after settling). |
| N-3 | A 5.x device can open and edit any document. | Documents saved by v6 carry a fence: a still-open 5.x tab cannot open them (it shows its "could not open" error) until it reloads into 6.0.0. Anything a 5.x device uploads later is merged, never overwritten. | REQUIREMENTS §4. | flow (stale 5.x tabs only) | — |
| N-4 | Version history travels inside the PDF (`IH_VERSIONS`, 3.5 MB budget; entries over budget silently stay local). | Each editor captures its own versions (same moments, coalescing and caps); versions travel with the document's operations, so every collaborator sees the same timeline; the export carries the newest within 3.5 MB; imported 5.x history is kept. | Collaborative by design. | no (same UI) | — |
| N-5 | The web service worker uploads the outbox on `sync`/`periodicsync`; a hidden `?offline-worker=1` iframe uploads while the app is open. | No service-worker background run and no iframe. Edits are sealed in the Drive relay as they happen and on `pagehide` (keepalive); a pending PDF export completes in the Android WorkManager headless page (same `InhouseBackground.complete({busy,pending,errors})` contract) or in the next session of any editor. | A service worker cannot start the PDF worker; less code; nothing is lost (edits are already in Drive). | no | — |
| N-6 | When IndexedDB is blocked, the document metadata is backed up in `localStorage` (`notebook-data-v3`). | When IndexedDB is unavailable the session keeps the document in memory, sends edits to Drive at once, and shows 5.18.0's local-save error state until they are sealed; leaving waits for the seal. No localStorage backup. | Removes a legacy layer; Drive becomes the durable copy within seconds. | **yes** (only when storage is blocked) | — |
| N-7 | 5.x local databases and caches persist forever. | After the one-time migration, once every migrated edit is sealed in Drive, no 5.x tab is open and no 5.x tab wrote since, the 5.x databases and `inhouse-media-*` caches are deleted. | REQUIREMENTS §4; frees device storage. | no | — |
| N-8 | `notebook-view-mode-v1` is written on every view switch but never read. | Not written. | Dead state. | no | — |
| N-9 | pdf.js, pdf-lib and jsPDF load at every start. | pdf.js and pdf-lib load on first need; jsPDF is gone. | Lighter boot. | no | — |
| N-10 | On WebViews older than Chrome 96 the app runs (JS). | v6 needs WebAssembly reference types (Chrome/WebView ≥ 96) and shows "update Android System WebView" otherwise. | Platform requirement of Rust 1.90's wasm target. | **yes** (very old WebViews only) | — |
| N-11 | Page rows and timeline rows carry 5.x string ids in `data-page-id` / `data-version-id`. | They carry v6 ids (hex of the 128-bit id). | Ids are 128-bit in the model; not visible. | no | DOM diagnostics normalise these attributes by order of first appearance (TESTING §3.2). |
| N-12 | Creating a document offline always works (`local-` id, `Created on this device`). | Works the same while the device holds reserved Drive ids (20, refilled whenever online); after more than ~10 offline creations without any connection, creation shows 5.18.0's `Failed to create PDF`. | A document gets its final Drive id at creation, removing all re-keying code. | flow (rare) | — |

### 3.1 Sync (SYNC.md)

| Id | 5.18.0 | 6.0.0 | Why | Visible |
|---|---|---|---|---|
| B-Y1 | Same-account devices without a P2P route saw in-progress ink through Drive "live preview" files (100–400 ms frames). | In-progress ink only over P2P; without P2P the finished stroke arrives through the relay (< 4 s P95). | Preview files cost a Drive write every 100 ms per writer and a read every 100–300 ms per reader. | flow |
| B-Y2 | Journal files lived in the private `inhouse notes` folder (same account only). Removing someone from a document removed their access to everything. | Relay files live in a link-readable, non-discoverable `Inhouse Notes sync` folder whose file ids are published only to the document's readers. The mutable relay file moves to a new id daily and whenever the document's permissions change. **Residual:** a removed person who saved file ids can still read, until the next PDF export deletes them, the archive and asset files they could already see, plus at most one more relay write per device made before that device noticed the removal; group-membership changes are bounded to one day. | Cross-account collaborators and viewers read edits without copying permissions. | no (security trade-off for the user to approve) |
| B-Y3 | No extra folder. | A folder `Inhouse Notes sync` appears in the user's My Drive root (in Google Drive's own UI; hidden inside the app). Deleting it is harmless (it is recreated; edits since the last export are re-sent). | Needed by B-Y2. | **yes, outside the app** |
| B-Y4 | The P2P signalling key was readable by viewers and peers were not checked for edit rights. | No key exists; signalling offers/answers travel in editor-sealed relay files; only proven editors connect. A viewer of the document can read the editors' connection offers (which contain their IP addresses) for at most 60 s while a connection is being set up; it cannot use them to connect. | Security finding fixed with the least code; encrypting offers would add a key-exchange layer only to hide IP addresses from the document's own readers (open question in BUILD-PLAN). | no |
| B-Y5 | Signed-in viewers saw changes only through PDF re-downloads (1.8–20 s). Presence online window 13–61 s with sticky misses. | Signed-in viewers get changes through the relay (< 4 s P95). Present = record changed within 120 s (measured on the viewer's own clock) or an open channel; leaving devices disappear at once. Signed-out public links are unchanged (5.18.0 poll). | Network documents. | flow |
| B-Y6 | Every editor uploaded the PDF, so Drive's "modified by" was usually the last writer. | Automatic exports are written by the scribe (smallest installation id present), so "modified by" in Drive may name another collaborator. | One export instead of one per editor. | **yes, outside the app** |
| B-Y7 | Unlimited tabs. | A browser profile can edit with up to 256 tabs at once; a 257th tab opens documents in 5.18.0's read-only viewer state. | Per-tab actor slots. | no (practically unreachable) |
| B-Y8 | Cross-account changes without P2P needed a whole-PDF save + poll (0.6–20 s). | Relay-only changes arrive in < 4 s P95 (typ. 1–3 s); writes are capped at one per 2 s per device to stay within Drive's per-file limits. | Network documents within Drive's limits. | flow |

## 4. Kept on purpose (5.18.0 oddities reproduced exactly)

Reproduced because they look intentional or are part of the byte-identical UI: every session starts in Read mode;
mouse drawing needs "Draw with finger"; `Ctrl+Z`/`Ctrl+Y` only with Ctrl and lowercase keys; recolouring a selection
also recolours selected highlighters with the pen palette (Q9); the highlighter's live preview differs slightly from
its committed rendering (Q5); the dead `#status-bar` markup and unused CSS stay in the copied stylesheet; Spanish
strings (`Actualizaciones · A/B`, `Abre la app Android para instalar.`, `Cerrar`, `Actividad del documento`, the export
loading texts); `installedVersion:'1.0.2'` updater fallback; the PDF Info `Title 'Cuaderno Digital'`; `public=1` in links
is decorative and `mode=edit` grants the writer flow; Escape closes only the photo modal in the editor; the "Inhouse
Notes" folder lookup matches a folder of that name anywhere in Drive; Import PDF replaces the open document's pages.

---

## 5. Candidates for the user's decision (kept as 5.18.0 unless the user says otherwise)

| Id | 5.18.0 behaviour | Possible change |
|---|---|---|
| Q-1 | `#drive-status` is hidden by CSS, but an error/upload message forces it visible permanently (spec/home.md §12.2). | Hide it again when the message clears. |
| Q-2 | Starred ignores the search term; cached Recents/Starred/Shared also ignore it (§12.3). | Filter all lists by the search term. |
| Q-3 | Lists never page: > 24 recents/shared, > 12 starred, > 100 per folder are truncated (§12.4). | Load more on scroll. |
| Q-4 | The new-PDF picker pre-fills the previous document's name; copy pre-fills `Copy of X.pdf` with a visible `.pdf` (§12.5). | Empty name / no `.pdf` suffix. |
| Q-5 | The web "Download APK" link is pinned to Android 1.0.11 (spec/shell.md §19.4). | Point to the latest release (the catalog's B or A latest). |
| Q-6 | Home auto-refresh (30 s) refreshes only Recents and the open folder (§12.10). | Also Starred/Shared. |

Approval checklist for release: the user approves §2, §3 (including §3.1) and any §5 changes; W6.1's parity report
lists every state that differs and the id that explains it.
