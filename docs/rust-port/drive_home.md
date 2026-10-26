# drive_home porting plan (planning only; no code changed yet)

Area: app-v5.js sections 4.15 (L9457-9655), 4.15b helpers (L8477-8710, L8791-9456), 4.15b home tabs/manila folders (L9656-10897),
4.16 Drive file open (L10898-12043 real content), 4.21a sharing API (L8711-8790), 4.21b presence (L27889-27926).
Rust module: rust-core/src/drive_home.rs (submodules inside it). Adapter: rust-core-v5.js between `stream:drive_home` markers.
Already ported: rust-core/src/folder.rs (folder_motion, folder_fade, FolderRows) used by readDriveFolderGridPlans and
playDriveFolderTransition. The area is ~90% IO/DOM glue (Drive REST + DOM cards); the pure share is small, so units are small.

## 1. Classification (JS bytes)
| Block | Lines | Bytes | Class |
|---|---|---|---|
| ensureDriveFolder/ensureNamedDriveFolder/createDriveFolder/escapeDriveQueryValue, findDrivePdfInFolder, new-folder modal | 8477-8606 | ~6K | query strings portable; fetch/modal glue |
| isQuickAccessDriveFile, getDriveQuickAccessPeriod, buildAcademicAgenda | 8635-8674 | ~2.5K | date arithmetic portable; page objects/labels glue |
| openQuickAccessDriveDocument | 8675-8710 | ~2K | glue |
| 4.21a permissions API (fetch/add/setLink/update/revoke/ownerInfo) | 8711-8790 | 4.3K | glue + one decision (setDriveLinkPermission) portable |
| formatDriveDate, formatDriveShortDate, describeDriveFile | 8791,9039-9059 | ~1.5K | uses toLocale*: glue (stays JS) |
| normalizeDriveThumbnail, getDriveSheetRatio | 8797-8817 | ~1K | pure |
| sampleDrivePreviewTone (pixel bin/luminance part) | 8818-8869 | 2.3K | canvas read = glue; binning + luminance pure |
| drainDrivePreviewToneQueue/applyDrivePreviewTone/createDrivePreview | 8870-8981 | ~8K | glue |
| isSharedDriveDoc, formatDriveCardFileName | 8982-9006 | ~1K | pure |
| applyDriveTitleWithSharedIcon, createDriveCardInfo, bindDriveCardActivation, createDriveFileCard/FolderCard/StarredCard, createDriveItemMenu, menus, color popup, breadcrumbs | 9007-9367 | ~35K | DOM glue |
| updateDriveStarStatus/updateDriveFolderColor | 9278-9313 | ~1.5K | IO glue |
| driveItemFingerprint + diffRenderDriveList | 9368-9456 | 4.5K | keyed reconcile plan portable; DOM moves glue |
| renderDriveRecents/Starred/Shared, createDeferredDriveFolderCard, renderDriveFolderContents | 9457-9655 | ~6K | glue |
| invalidate/readDriveFolderGridPlans, hydrateVisibleDriveFolderCards | 9539-9617 | 5K | CSS-string parsing + range math portable; getComputedStyle glue; folder_rows already ported |
| Folder colour maths (resolveDriveFolderColor, driveRelativeLuminance, driveHexToHsl, driveHslToHex, warmDriveFolderHue, displayDriveFolderColor, shadeDriveFolderColor, getDriveFolderViewTint, value part of applyDriveFolderPalette) | 9664-9807 | ~7K | pure; style.setProperty stays JS |
| createDriveFolderArt, folder view positioning/snapshot/ghost, playDriveFolderTransition (WAAPI), open/close/navigate | 9808-10385 | ~30K | DOM/animation glue (motion maths already in folder.rs) |
| setDriveHomeTab, refreshDriveRecents/Starred/Shared/FolderContents/Files/Quiet | 10386-10627 | ~13K | query building + filters portable; fetch glue |
| updateDriveStatusCounts, skeleton builders | 10508-10572 | ~3K | status text portable (tiny), rest DOM |
| setupMobileHomePullToRefresh | 10629-10779 | 6.9K | event glue; setPull easing 5 lines |
| isDriveHomeAutoRefreshBlocked, refreshDriveHomeSnapshot, auto-refresh timer | 10780-10826 | ~2K | glue (reads live DOM/flags) |
| applyCachedDriveOpenMetadata, revalidateCachedDriveOpen (remoteChanged test) | 10827-10897 | 4K | decision portable, rest state/IO glue |
| openDriveFile, showDriveOpenError, resumeDetachedExitUploadAfterOpen | 10898-11264 | 19.5K | orchestration IO glue |
| updateViewerModeUI, makeDriveCopy, create menu, uploadDeviceFile, createNewDriveDocument* | 11265-11581 | ~28K | glue |
| getDrivePathLabelFromStack/getDriveFolderMeta/resolveDrivePathForFile | 11582-11640 | 2.5K | path-chain walk (cycle/limit) portable given metas; fetch glue |
| Folder picker, trash modal, bin (refreshDriveBin, restore, trash) | 11641-12043 | ~21K | DOM/IO glue |
| presence clearPresence | 27889-27926 | 1.9K | payload (null-map of keys) portable; fetch glue |
| L12044-13298 (image paste/insert, selection transforms, eraser, stroke hit-testing, highlighter drawing) | 55K | OWNERSHIP FLAG: sits under the 4.16 heading in the file but is editor/ink content, not Drive file open. I will NOT port it; the ink/editor owners should claim it. |

Pure computation total is ~35-40 KB of JS; with query building and decisions ~50 KB.

## 2. Units (ordered by speedup, then risk)

### U1 Folder colour palette (high speedup, low risk) ~7 KB JS
Scope: resolveDriveFolderColor, driveRelativeLuminance, driveHexToHsl, driveHslToHex, warmDriveFolderHue, displayDriveFolderColor, shadeDriveFolderColor, getDriveFolderViewTint and the value computation of applyDriveFolderPalette (L9664-9807). JS keeps style.setProperty/dataset and isDriveNightTheme (night flag passed in).
Why high: runs for every folder card render and on every theme repaint (repaintDriveFolders over all arts) and during folder-open tint.
Boundary: callJson `drive_home.palette` input `{night, exact, items:[hex...]}` -> array of objects with 8 CSS strings (color, cover, inside, tab, ink, hl, hlStrong, line); `drive_home.view_tint` `{night,color}` -> {bg,inside,tab}. Batched so repaint is one call. Parity: JS Math.round (half away toward +inf) vs Rust f64::round (half away from zero): values are non-negative so identical; verify channel()/hex formatting and the `(h*60+360)%360` modulo.
Tests: differential node test vs legacy (random hex x night/day, thousands) in tests/drive_home_palette.test.mjs; e2e folder-navigation.spec.mjs, home-action-menu.spec.mjs.

### U2 List reconcile plan + card text (high, low-medium) ~7 KB JS
Scope: driveItemFingerprint, the keyed matching core of diffRenderDriveList (L9368-9456), formatDriveCardFileName, isSharedDriveDoc, updateDriveStatusCounts text, getDrivePathLabelFromStack, normalizeDriveThumbnail, getDriveSheetRatio (L8797-8817, 8982-9006).
Rust computes from `existing:[{id,fp}]` (in DOM order) and `desired:[{id,fp}]` an op list: keep(index), moveBefore, build(index, replaceExisting?), remove(existing index), sweep-untagged. JS executes DOM ops with the same side effects (refreshFile, decorate, sub text, arrival animation). Runs on every 5 s poll and each keystroke in search, with hundreds of items; also removes JSON-string fingerprint building in JS.
Boundary: JSON (ids/fingerprints are strings); op list as compact array of [code, a, b] ints. Fingerprint computed in Rust (`id|name|starred|color|mime`), returned as strings so cards still store dataset.driveItemFp.
Tests: differential reconcile test (simulate DOM as array in node against legacy function using a fake container) tests/drive_home_diff.test.mjs; e2e folder-navigation, home-save-navigation, offline-library*.spec.mjs.

### U3 Folder grid plan and virtual hydration (medium-high, medium) ~5 KB JS
Scope: pure parts of readDriveFolderGridPlans and hydrateVisibleDriveFolderCards (L9539-9617): count `px` tracks from gridTemplateColumns/Rows strings, cache key, boxWidth/inset math, scale, visible range union with overscan, and the fallback-rect filtering. Builds on existing folder.rs FolderRows (already ported, keep). JS only calls getComputedStyle/getBoundingClientRect and passes the raw strings plus rect numbers.
Boundary: callJson `drive_home.grid_plan` {strings, rect numbers, viewport, overscan} -> {key, columns, rows (Float64 typed array through existing folder_rows_new), inset, boxWidth, scale}; scroll-time call keeps using the existing typed FolderRows range. Hot path: scroll handler during folder view.
Tests: legacy differential on CSS strings (cases: subpixel, `repeat`, empty); e2e folder-navigation.spec.mjs (animation, hydration), drive-preparation.spec.mjs.

### U4 Preview tone from sampled pixels (medium, low) ~2.5 KB JS
Scope: the pixel analysis inside sampleDrivePreviewTone (L8836-8863): 24x16 RGBA -> 3-bit-per-channel dominant bin (first-max tie rule), mean rgb, luminance, ink choice. Crop-rect math (ratio, sourceWidth/Height, left) also moves. Canvas creation/drawImage/getImageData stay JS (browser only).
Boundary: typed array: Uint8ClampedArray 1536 bytes -> exported C fn with out Uint32/Uint8 buffer {r,g,b,darkInk}; crop via callJson or f64 out buffer of 4 values. JS builds the strings.
Tests: differential on random/real-looking 24x16 buffers incl. tie cases; e2e home-action-menu.spec.mjs (label tone) .

### U5 Drive query builder and result filters (low-medium speedup, low risk) ~9 KB JS
Scope: escapeDriveQueryValue, query/fields strings and post-filters in refreshDriveRecents/Starred/Shared/FolderContents, ensureDriveFolder/ensureNamedDriveFolder/findDrivePdfInFolder queries (L8477-8634, 10405-10507), isQuickAccessDriveFile, `.filter(...).slice(12)`, folder/file split of combined offline results. Note existing bug-for-bug: search escapes only `'` (not backslash); keep.
Boundary: callJson `drive_home.query` {kind, search, parentId?, name?} -> {url path string} (encodeURIComponent done in Rust with the exact JS unreserved set: A-Za-z0-9 -_.!~*'()); `drive_home.filter_list` {kind, files} -> files.
Tests: differential on random strings incl. unicode/quotes; e2e folder-navigation, drive-preparation, offline-library*.spec.mjs.

### U6 Permission, metadata and path decisions + presence payload (low speedup, low risk) ~7 KB JS
Scope: setDriveLinkPermission decision (L8726-8748) -> plan list [delete id | patch id role | create role | none]; fetchDriveOwnerInfo response normalisation (state updates stay JS); applyCachedDriveOpenMetadata value derivation and revalidateCachedDriveOpen `remoteChanged` rule (L10827-10897); resolveDrivePathForFile chain walk as a step function (given metas map -> next id/label/cycle/limit 20); clearPresence key set (L27889-27926; keys from getPresenceKey remain supplied by caller as prefix).
Boundary: JSON in, JSON plan out; JS executes fetches.
Tests: unit differential vs legacy; e2e drive-preparation.spec.mjs, home-save-navigation.spec.mjs.

### U7 Quick-access period and academic agenda calendar (medium for open latency, medium risk) ~3 KB JS
Scope: getDriveQuickAccessPeriod, and the date/week sequence of buildAcademicAgenda (L8645-8674): Monday-aligned first week, ~52 weeks x 7 local-date keys (formatCalendarDateKey format). Labels (toLocaleDateString), generatePageId, buildSidePanelHtml stay JS (locale/other-area functions).
Boundary: callJson `drive_home.agenda_weeks` {startYear} -> {firstKey, nextKey, weeks:[[dateKey x7]...]}; JS wraps pages. Pure civil-date arithmetic (no TZ: original uses noon-local to avoid DST; the same result is a plain Gregorian count, so TZ-independent).
Tests: differential for years 2000-2100; e2e drive-preparation.spec.mjs (quick access docs).

## 3. Order summary
U1, U2, U3 (highest user-visible smoothness: scroll, polling, theme/folder renders), U4, U5, U7, U6. 7 units, all under 40 KB; combined ~40 KB JS removed (~2% of app-v5.js) and roughly 1.5-2 KB JS adapter, ~25-35 KB Rust added.

## 4. Not portable (stays JS)
- All DOM card/menu/modal construction (createDrive*Card, menus, folder picker, bin list, skeletons, breadcrumbs): DOM.
- playDriveFolderTransition and folder open/close snapshot/ghost (WAAPI, getBoundingClientRect, clones): browser animation (motion maths already in folder.rs).
- Pull-to-refresh gesture (touch events; easing lines are trivial CSS-var writes).
- All Drive REST fetch orchestration (driveFetch, openDriveFile, copy/upload/trash/restore/move, permissions fetches): network I/O and token flow; only the decisions above move.
- canvas getImageData/drawImage for tone sampling and thumbnail Image probing queue.
- Locale date formatting (toLocaleDateString/TimeString in formatDriveShortDate, describeDriveFile, formatDriveDate): depends on browser Intl; faithful port impossible without ICU data.
- sessionStorage tab state, OfflineLibrary/IndexedDB integration, setInterval auto-refresh and its blocked-state predicate (reads live DOM/flags).
- L12044-13298 (image/selection/eraser/stroke geometry, highlighter drawing) lies under the 4.16 heading but is editor/ink scope; flagged for the integrator, not planned here.

## 5. Status
- **U1 Folder colour palette: DONE** (branch rust/drive_home).
  - Rust: `rust-core/src/drive_home/palette.rs` (10.2 KB incl. 9 unit tests) + dispatcher in `drive_home.rs`; methods
    `drive_home.resolve_color`, `drive_home.palette` (batched, flat output: 4 hex strings + dark-ink flag per item), `drive_home.view_tint`.
  - JS adapter: `rust-core-v5.js` stream:drive_home markers (~2.3 KB incl. a small Map cache keyed by colour/night/exact and the two fixed ink/highlight CSS tables).
  - JS removed from app-v5.js: resolveDriveFolderColor body, driveRelativeLuminance, driveHexToHsl, driveHslToHex, warmDriveFolderHue, displayDriveFolderColor,
    shadeDriveFolderColor, getDriveFolderViewTint, value maths of applyDriveFolderPalette. app-v5.js 1,926,082 -> 1,921,981 bytes (-4.1 KB net;
    about 5.5 KB of algorithm removed, 1.4 KB of glue added). `init()` now awaits `InhouseRust.ready` and throws if WASM is missing.
  - `repaintDriveFolders` is one batched call for all folders on screen.
  - Tests: `tests/drive_home_palette.test.mjs` (differential vs legacy `app-v5.js@a2dea10`: Drive colour fixtures, 6000 seeded random colours x day/night x exact/base,
    48.7k-colour RGB lattice (day and night), the 400 colours closest to the ink contrast switch found by an exhaustive 2^24 luminance sweep, view tint, odd strings for resolve) all byte-exact.
    Parity notes: Math.round re-implemented as floor(x+0.5) semantics; JS trim whitespace set replicated (NEL excluded, BOM included); pow(x,2.4) only feeds the
    ink-contrast boolean, and the near-tie sweep shows no flip.
  - `tests/release-v5-smoke.mjs`: replaced greps for the removed function declarations (getDriveFolderViewTint/displayDriveFolderColor/warmDriveFolderHue) by greps
    for the Rust calls and a negative check that the old functions are gone.
  - Speed (`node tests/drive_home_palette.bench.mjs`, node 22, shared box, indicative): theme repaint of 300 folders (24 distinct colours) JS 7.9 ms -> 0.04 ms (cache hits);
    300 never-seen colours JS 7.5-8.9 ms -> 2.4-2.5 ms (3-3.7x); 40 never-seen colours 1.27 -> 0.73 ms (1.75x); single uncached card 28 us -> 23 us (about equal);
    view tint 15 us -> 26 us (call overhead; once per folder open, no user impact).
- U2..U7: not started.
