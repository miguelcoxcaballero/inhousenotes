# Inhouse Notes v6 spec — Application shell

Source of truth: web app **v5.17.22** (`index.html:93`), Android shell **1.0.28 / versionCode 29**
(`.github/scripts/build_android_apk.py:20-21`). v6.0.0 must reproduce everything below with identical
UI, strings, storage formats, URLs and native-bridge contracts unless a line is explicitly marked
*quirk* (then the quirk itself is the current behaviour and must be kept or consciously changed).

Citations are `file:line` (or `file:start-end`) against the v5.17.22 checkout. "app" = `app-v5.js`.

Scope of this document: boot and run modes, Google OAuth, theme, fonts, status/loading surfaces,
global event wiring, settings surfaces, the shared updater UI and A/B channels, version/release
rules, the build/deploy pipeline and the e2e test surface. Editor, Drive home, save/sync, PDF,
collaboration, calendar and scanner internals are specified elsewhere; they are referenced here only
where the shell routes into them.

> **Frozen**: `vendor/inhouse-services-updater/android/PhotosUpdateInstaller.kt`,
> `vendor/inhouse-services-updater/android/UpdateFileProvider.java`,
> `vendor/inhouse-services-updater/register_android.py` and their registration must never be changed
> by the v6 work (`vendor/inhouse-services-updater/FROZEN_INSTALLER.md`, `AGENTS.md`). v6 only talks
> to them through the existing JS bridge described in §14.

---

## 1. Static document (`index.html`)

### 1.1 Head, in order
| Line | Content |
|---|---|
| `index.html:1` | HTML comment documenting the shared-updater policy (before the doctype). |
| `index.html:2-78` | Master table-of-contents comment. |
| `index.html:79-80` | `<!DOCTYPE html>`, `<html lang="en">` (`<head>` at 82). |
| `index.html:83` | `<meta charset="UTF-8">` |
| `index.html:84` | **CSP** (verbatim, must be kept): `default-src 'self'; base-uri 'none'; object-src 'none'; script-src 'self' 'wasm-unsafe-eval' https://accounts.google.com https://cdnjs.cloudflare.com; script-src-attr 'none'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src 'self' data: https://fonts.gstatic.com https://unpkg.com; img-src 'self' data: blob: https://*.googleusercontent.com https://www.googleapis.com https://drive.google.com; connect-src 'self' https://*.googleusercontent.com https://raw.githubusercontent.com https://api.github.com blob: https://accounts.google.com https://oauth2.googleapis.com https://www.googleapis.com https://drive.google.com https://drive.usercontent.google.com https://fonts.googleapis.com https://fonts.gstatic.com https://cdnjs.cloudflare.com https://unpkg.com stun: turn:; worker-src 'self' blob: https://cdnjs.cloudflare.com; child-src 'self' blob: https://accounts.google.com https://drive.google.com; frame-src 'self' https://accounts.google.com https://drive.google.com; form-action 'self' https://accounts.google.com; manifest-src 'self'; media-src 'self' blob: data:; upgrade-insecure-requests`. `'wasm-unsafe-eval'` is already allowed (needed by v6 WASM). No inline executable scripts and no inline event attributes are allowed (`script-src-attr 'none'`). |
| `index.html:85` | `<meta name="referrer" content="strict-origin-when-cross-origin">` |
| `index.html:86` | viewport `width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no` |
| `index.html:87` | `<meta name="color-scheme" content="light dark">` |
| `index.html:88` | `<meta name="theme-color" id="app-theme-color" content="#f5f5f0">` (boot rewrites it, §6). |
| `index.html:89-91` | icons: `icons/inhouse-notes-32.png` (32×32), `icons/inhouse-notes-192.png` (192×192), apple-touch `icons/inhouse-notes-180.png`. |
| `index.html:92` | `<title>inhouse notes</title>` (lower case). |
| `index.html:93` | `<script type="application/x-inhouse-config" id="inhouse-app-version">const APP_VERSION = '5.17.22';</script>` — inert (non-executable type); **single source of truth** for the visible version (§15). |
| `index.html:94` | `<script type="application/json" id="inhouse-native-assets">{…}</script>` — map *absolute path → sha256 hex* of every native-bundled asset, generated (§16.2). |
| `index.html:95` | `<script src="boot-v5.js?v=5.17.22&amp;native=<sha256>">` — synchronous, render-blocking (must run before first paint). |
| `index.html:96` | `<link rel="stylesheet" href="offline-v5.css?v=…&amp;native=…">` |
| `index.html:97` | `<link rel="stylesheet" href="vendor/fonts/ui.css?native=…">` |
| `index.html:98` | `<script src="https://accounts.google.com/gsi/client" async defer data-auto_prompt="false">` (Google Identity Services). |
| `index.html:99-9191` | The single `<style>` block (CSS index at `index.html:100-150`). |
| `index.html:9192` | `<link rel="stylesheet" href="shared/inhouse-services-updater/updater.css?v=1.2.3&amp;native=…">` (last in head). |

### 1.2 Body top-level structure (`index.html:9195-10455`)
Top-level children in order: `#welcome-view` (9234), `#drive-home.hidden[data-home-tab="home"]` (9263),
`#editor-view.hidden` (9521), modals `#modal-delete-page` (9751), `#modal-reset-document` (9764),
`#modal-export-pdf` (9782), `#modal-cover-image` (9798), `#drive-create-menu[role=menu][hidden]`
(9853), `input#drive-upload-input[type=file][hidden]` (9857), `#modal-folder-picker` (9860),
`#modal-new-folder` (9885), `#modal-confirm-slider` (9900), `#modal-trash-file` (9921),
`#modal-drive-bin` (9942), `#panel-overlay` (9958), `#pages-panel[role=dialog]` (9967),
`#scanner-editor-overlay[aria-hidden=true]` containing
`iframe#scanner-editor-frame[title="Inhouse Scanner"][allow=camera][referrerpolicy=same-origin]`
(10019-10021), `#modal-calendar` (10028), `#modal-calendar-day` (10073), `#modal-download-app`
(10092), `#modal-calendar-pages` (10119), `#sp-event-modal` (10170), `#loading-overlay` (10259),
`input#pdf-input[type=file][accept=.pdf]` (10276), `input#cover-image-input[type=file][accept=image/*]`
(10277), `#timeline-panel[aria-hidden=true]` (10284), `#share-modal-overlay.modal-overlay` (10320), then
the deferred scripts (10431-10453). Runtime-created top-level nodes: `#add-page-indicator` inside
`#canvas-container` (`app:2112-2115`), `#document-fast-scroll` appended to `body`
(`app:35797-35862`), the updater dialog `#inhouse-services-update` (§14), the offline-settings
`<dialog id="offline-settings">` (`offline-library-v5.js:549-571`), hidden offline-worker iframe
(`offline-library-v5.js:534-547`).

### 1.3 Script loading order (all `defer`, executed in document order after parse)
`index.html:10431-10453`:
1. `vendor/jspdf/jspdf.umd.min.js` (SRI sha384, `crossorigin=anonymous`)
2. `vendor/pdfjs/pdf.min.js` (SRI) — PDF.js **3.11.174** (`app:87`)
3. `vendor/pdf-lib/pdf-lib.min.js` (SRI)
4. `rust-core-v5.js` → `InhouseRust`
5. `collaboration-core-v5.js`
6. `peer-transport-v5.js` → `InhousePeerTransport`
7. `live-collaboration-v5.js`
8. `runtime-core-v5.js` → `InhouseRuntimeCore`
9. `security-core-v5.js` → `InhouseSecurityCore`
10. `timeline-core-v5.js` → `InhouseTimelineCore`
11. `document-processing-v5.js` → `InhouseDocumentProcessing`
12. `shared/inhouse-services-updater/updater.js?v=1.2.3` → `InhouseServicesUpdater`
13. `bullet-template.js` → `InhouseBulletTemplate`
14. `offline-store-v5.js` → `InhouseOfflineStore`
15. `offline-sync-v5.js` → `InhouseOfflineSync`
16. `offline-library-v5.js` → `InhouseOfflineLibrary`
17. `offline-media-v5.js` → `InhouseOfflineMedia`
18. `offline-app-v5.js` → `InhouseOfflineApp`
19. `ink-palette.js` → `InhouseInkPalette`, `InhouseInkDefaults`
20. `drive-ink-journal-v5.js` → `InhouseDriveInkJournal`
21. `drive-live-preview-v5.js` → `InhouseDriveLivePreview`
22. `drive-ink-app-v5.js` → `InhouseDriveInk`
23. `app-v5.js` (orchestrator, last).

Every first-party script/stylesheet carries `?v=<APP_VERSION>` (vendor libs and `ui.css` carry none;
the shared updater carries its own `?v=1.2.3`) and `native=<sha256 of the file>` (§16.2). Exactly three `integrity="sha384-…"` attributes exist
(`tests/release-v5-smoke.mjs:153`). PDF libraries must come after `</style>`
(`tests/release-v5-smoke.mjs:180-183`).

### 1.4 Global-scope contract (important for v6)
All scripts are **classic scripts sharing one global scope**. Modules publish frozen APIs on
`window` (list above). `app-v5.js` top-level `function` declarations are window properties and
top-level `let/const` live in the global lexical scope; other scripts and the e2e suite rely on this:
- `offline-library-v5.js:362,386` calls `root.hasHotUserInteraction`, `root.getDocumentSaveIdleWindow`;
  the offline-worker iframe calls `window.parent.hasHotUserInteraction` / `getDocumentWorkInteractionState`
  (`app:31903-31905, 31949-31956`) and `parent.InhouseOfflineApp` (`offline-app-v5.js:320-321`).
- `live-collaboration-v5.js` calls `isDocumentLiveWorkPaused()` (`app:6030-6033`).
- e2e specs call and even **reassign** app globals inside `page.evaluate` (§17.3).
v6 must expose equivalent named hooks (a JS facade over WASM) or rewrite those specs.
Window globals written by app: `INHOUSE_NOTES_APP`, `InhouseUpdater`, `__IHN_TEST_API__`,
`__INHOUSE_BOOT_SIGNED_IN__`, `_templateBgUrlCache`, `_templateBgScaleCache`, `_templateBackgroundUrl`,
`_templateBackgroundScale`, `handleInhouseNativeOAuth`, `handleInhousePdfExportResult`,
`pendingSharedDriveId`. boot writes `InhouseAssetUrl`, `InhouseBackground`, `__INHOUSE_BOOT_SIGNED_IN__`.
updater writes `handleInhouseUpdateResult`.

---

## 2. Theme tokens and base CSS

`:root` (`index.html:185-212`): `--bg-primary #f5f5f0`, `--bg-secondary #ffffff`, `--text-primary #1a1a1a`,
`--text-secondary #666666`, `--accent-yellow #ffde00`, `--accent-blue #002FD9`, `--accent-red #E81010`,
`--accent-black #4D4D4D`, `--accent-orange #E07A3C`, `--accent-green #34c759`,
`--shadow-soft 0 2px 20px rgba(0,0,0,.08)`, `--shadow-medium 0 4px 30px rgba(0,0,0,.12)`,
`--radius-sm 8px`, `--radius-md 12px`, `--radius-lg 16px`, `--tool-size 44px`, `--toolbar-padding 10px`,
`--handle-width 44px`, `--handle-height 24px`, `--icon-size 20px`, `--header-btn-size 44px`,
`--banner-height 64px`, `--toolbar-tilt 18deg`, `--selection-handle-size 14px`,
`--selection-handle-offset calc(-1px - (var(--selection-handle-size) / 2))`, `--rotate-handle-size 10px`;
`@property --toolbar-rotate` (`<angle>`, inherits, 0deg) at `index.html:174-178`.

`[data-theme="dark"]` (`index.html:214-221`): `--bg-primary #151515`, `--bg-secondary #1c1c1c`,
`--text-primary #f5f5f5`, `--text-secondary #a2a2a2`, `--shadow-soft 0 2px 20px rgba(0,0,0,.35)`,
`--shadow-medium 0 4px 30px rgba(0,0,0,.45)`. 136 `[data-theme="dark"]` selectors exist across the
stylesheet; component specs own them.

Mobile (`@media (max-width:720px)`): `--header-btn-size 34px` (`index.html:284-288`, repeated as
final override at `index.html:7378-7382`).

`html, body` (`index.html:223-231`): `height:100%; overflow:hidden; font-family:'DM Sans', -apple-system,
BlinkMacSystemFont, sans-serif; background:var(--bg-primary); color:var(--text-primary); touch-action:none`.
Global reset `* {margin:0;padding:0;box-sizing:border-box;-webkit-tap-highlight-color:transparent}`,
`.hidden {display:none !important}` (`index.html:151-160`).

App-mode CSS: `html[data-inhouse-app="true"] #btn-drive-fullscreen, #btn-fullscreen {display:none!important}`
(`index.html:169-172`); `html[data-inhouse-app="true"] .btn-download-app {display:none!important}`
(`index.html:4661-4663`).

Boot gating CSS (`offline-v5.css:1-2`): while `html[data-boot-destination="drive"]` and not yet
`[data-app-ready="true"]`, `#welcome-view` is forced `display:none` and `#drive-home` `display:flex`.

---

## 3. Fonts

- `vendor/fonts/ui.css:1-49`: local `@font-face` for **Comfortaa** 400/500/600/700 (`ui-0..3.ttf`) and
  **DM Sans** 400/500/600 (`ui-4..6.ttf`), `font-display: swap`, each `src` URL carries
  `?native=<sha256>` rewritten by `scripts/build-native-assets.mjs:6-10`. Licences OFL
  (`vendor/fonts/README.md`).
- Usage: body text DM Sans (`index.html:227`); brand/logo text Comfortaa 600 (`index.html:252-261`,
  welcome logo `index.html:4399-4405`, loading brand `index.html:4279-4288`). Welcome button
  `font: 700 1rem 'DM Sans', system-ui, sans-serif` (`index.html:4440`).
- First-paint brand reveal: `#welcome-view .welcome-logo .brand-name` is `visibility:hidden` until
  `#welcome-view.fonts-ready` (`index.html:4399-4409`). boot adds `fonts-ready` when
  `document.fonts.load('600 2.25rem Comfortaa')` settles (resolve or reject), immediately if the API is
  missing, and unconditionally after 3000 ms (`boot-v5.js:81-84`).
- Updater UI uses its own stack `"Google Sans Text", Roboto, system-ui, -apple-system, "Segoe UI",
  sans-serif` (`shared/inhouse-services-updater/updater.css:13`). The scanner page uses Google Fonts
  CDN (`scanner/index.html:9-12`).

---

## 4. App modes and URL parameters

| Mode | Trigger | Effect |
|---|---|---|
| Web (default) | none | GIS popup auth (§8), APP download button visible. |
| Android app | `?inhouse_app=1|true` or `?app=1|true`, or UA matches `/\bInhouseNotesApp\/\d+(?:\.\d+)?\b/i`, or persisted `localStorage.inhouseNotesAppMode==='1'`; `?inhouse_app=0|false` (or `app=`) forces web **and removes** the persisted flag (`app:100-122`). | `html[data-inhouse-app="true"|"false"]` and `window.INHOUSE_NOTES_APP` (`app:124-126`); `#btn-drive-fullscreen`, `#btn-fullscreen` removed from DOM (`app:128-135`); APP button hidden by CSS; updater created (§14); PKCE sign-in (§8.5); PDF export via native bridge (`app:35231-35284`). |
| Offline worker | `?offline-worker` present (any value) (`app:71`) | Headless processor (§4.3). |
| E2E test | host ∈ {`127.0.0.1`,`localhost`,`[::1]`} **and** `?e2e=1` (`app:35286-35289`) | Installs `window.__IHN_TEST_API__` (§17). Nothing else changes. |
| OAuth callback | `?oauth_callback=1` (`boot-v5.js:28-45`) | Legacy implicit-flow bounce back into the APK (§8.6). Release check skipped (`boot-v5.js:133`). SW bypassed (`service-worker.js:29`). |
| Shared link | `?drive=<fileId>` (+ `resourcekey`/`rk`, `name`, `role`, `mode`, `public`) | §4.4. |
| Embedded scanner | `scanner/index.html?embed=1&session=<id>&v=<APP_VERSION>` | §4.5. |

### 4.1 Detection order at script evaluation (`app-v5.js` top level)
1. Read `APP_VERSION` from `#inhouse-app-version` with `/APP_VERSION\s*=\s*['"](\d+\.\d+\.\d+)['"]/`;
   missing → `throw new Error('The public app version is missing from index.html')` (`app:67-70`).
2. `OFFLINE_WORKER` flag (`app:71`).
3. Write `v${APP_VERSION}` into every `[data-app-version]` (`app:72-74`).
4. Require `InhouseRuntimeCore`, `InhouseSecurityCore`, `InhouseTimelineCore` else
   `throw new Error('Required Inhouse Notes runtime modules failed to load')` (`app:75-80`).
5. PDF.js worker URL `InhouseAssetUrl('vendor/pdfjs/pdf.worker.min.js')`, cmaps and standard fonts under
   `vendor/pdfjs/` (`app:87-92`).
6. App-mode detection, `applyInhouseAppMode`, updater creation (`app:94-158`).
7. Constants, caches; `setInterval(checkMemoryPressure, 15000)` (`app:1187-1231`: when
   `performance.memory` used/limit > 0.8, prune image cache to 40 %, history to 40 %, evict PDF
   backgrounds of non-active pages).
8. `state` (`app:1240-1321`), DOM refs (`app:1721-1858`), global pointer redispatch: capture-phase
   `pointerup`/`pointercancel` on `document` re-dispatch to a drawing canvas that lost the event
   (`app:1943-1949`); `window.blur` releases viewport gestures and sends `pointercancel` (pointerType
   `pen`) to each active drawing canvas (`app:1950-1956`).
9. Start: `appInitializationPromise = OFFLINE_WORKER ? InhouseOfflineApp.worker() : init()`;
   `installLocalE2ETestApi(appInitializationPromise)`; if not worker `installDocumentFastScroller()`
   (`app:35865-35868`).

### 4.2 Run-mode storage namespacing
In offline-worker mode these keys get a `background-` prefix so the hidden processor never touches the
visible editor's state: `notebook-view-v1` (`app:345`), `drive-session-v1` (`app:411`),
`drive-pending-exit-upload-v1` (`app:412`), `notebook-data-v3` (`app:430`), IndexedDB
`notebook-data-db-v1` (`app:432`). Shared (unprefixed) in both: theme, token, refresh token, scopes,
hint email, the original-PDF DB `inhouse-original-pdfs-v1` (`app:445`).

### 4.3 Offline worker mode
- Creators: (a) the visible app's offline library creates a hidden same-origin iframe
  `/?offline-worker=1&v=<version>` titled "Background document sync" (`offline-library-v5.js:534-547`);
  (b) Android `InhouseSyncWorker` loads `https://inhousenotes.com/?inhouse_app=1&offline-worker=1` in a
  headless WebView (UA suffix ` InhouseNotesApp/Background`) (`.github/android/background/InhouseSyncWorker.java:30-48`).
- `InhouseOfflineApp.worker()` (`offline-app-v5.js:310-384`): `initialize()`, require a known account,
  borrow the parent window's `backgroundAccess` for tokens when embedded same-origin (overrides
  `ensureDriveToken`), else `loadDriveTokenFromStorage()` and silent refresh (throws
  `'Sign in to renew background access'` if no token and no PKCE). Once per processor it checks
  `drive/v3/about?fields=user(emailAddress)` equals the account (`'Background account changed'`).
  Runs `InhouseOfflineSync.run(account,{background:true,…})`, skipping the parent's active document and a
  live editor lease; posts `{type:'ihn-offline-progress'}` / `{type:'ihn-offline-complete',result,reusable}`
  to the parent and calls `InhouseBackground.complete(JSON)` (Android). Embedded processors rerun on
  `{type:'ihn-offline-run'}` from the parent.
- Shell differences: `updateDriveAuthUI` and `refreshDriveFiles` are no-ops (`app:6606, 10908`); no
  fast scroller; boot release-check disabled (`boot-v5.js:136`); interaction checks consult the parent
  (`app:31903-31905, 31949-31956`); idle yielding prefers animation frames (`app:31916`).
- `InhouseBackground` (boot, `boot-v5.js:195-198`): only when the host injected the WebMessage channel
  `InhouseBackgroundChannel` → frozen `{schedule(), cancel(), complete(jsonString)}` posting
  `JSON.stringify({type, result})`. Android accepts it only from main frame `https://inhousenotes.com`
  (`InhouseBackgroundSync.java:18-33`); `schedule` enqueues unique work `inhouse-offline-outbox`
  (KEEP, network required, exponential backoff 30 s) and periodic `inhouse-offline-retry` (15 min);
  `cancel` cancels both (`InhouseBackgroundSync.java:34-47`). Worker success iff result
  `!busy && pending==0 && errors empty`, timeout 8 min else retry (`InhouseSyncWorker.java:41-53`).

### 4.4 Shared links
- **Format produced** by the share modal's copy-link (`app:15707-15714`):
  `${origin}${pathname}?drive=<id>&public=1&role=<reader|writer>&name=<file name>[&mode=edit][&resourcekey=<key>]`
  (`mode=edit` only for writer). Copied with `navigator.clipboard.writeText`; status
  `'Editor link copied. Recipients sign in to save changes to Drive.'` or
  `'Public viewer link copied — no sign-in required.'` (`app:15722-15727`).
- **Parsing** (twice: `showFastInitialView` `app:2223-2233` and `init` `app:2291-2308`):
  `drive` → `window.pendingSharedDriveId`; `resourcekey` or `rk` → resource key; `name` trimmed, max 240
  chars; role = `writer` iff `role=writer` or `mode=edit`, else `reader`. Parameters stay in the URL until
  opening succeeds (`app:2306-2307`).
- **Routing at boot**: pending share → editor if an access token is already in memory, else Drive Home
  as backdrop (`app:2246-2248, 2324-2331`); then if no token `tryOpenPendingSharedWithoutSignIn()`
  (`app:2349-2351`).
- **Anonymous open** (`app:8680-8795`): Home + `showLoading('Opening shared document...')`; fetch the
  public PDF; import through the normal PDF path; set read-only state (`driveCanEdit=false`,
  `driveAutosave=false`); start public polling and live collaboration; `showEditorView()`; writer links
  show the sign-in gate; status `'Shared PDF loaded. Sign in to edit and save.'` or
  `'Shared PDF loaded (View Only)'`. On failure → Drive preview iframe (`openPublicDrivePreview`,
  `app:8510-8534`) with status `'Sign in to activate this editor link.'` /
  `'Public document opened — no sign-in required.'`, and Home status
  `'Opening public view — no sign-in required.'`. One attempt per file id unless forced (`app:8797-8809`).
- **Sign-in gate** (`app:8454-8508`): `#readonly-banner[data-shared-editor-gate=true]` visible, message
  `<strong>Editor link</strong> Sign in to edit and save.`, `#btn-make-copy` hidden, `#btn-ask-edit`
  text `Sign in`. Cleared state: message `<strong>View Only</strong> You don't have permission to edit this file.`,
  button `Ask to Edit`.
- **Signed-in open** (`app:6673-6720`): when a token appears, GET
  `drive/v3/files/<id>?fields=id,name,resourceKey[&resourceKey=…]`, status
  `'Loading document info...'` (no auto close) then `'Opening document...'`, `openDriveFile({id,name,resourceKey})`;
  if metadata fails, open with `name = pendingSharedDriveName || 'Shared Document'`; on success clear
  pending share vars and the gate; on failure status `'Could not open shared document'` (error).

### 4.5 Embedded scanner (shell part only)
- Open (`app:19515-19533`): only when editable and not importing; closes pages panel and cover modal;
  session id `scan-<base36 time>-<random>` (`app:19501`); iframe src
  `scanner/index.html?embed=1&session=<id>&v=<APP_VERSION>`; overlay `.visible`, `aria-hidden=false`,
  `body.scanner-editor-open`; focus frame.
- Close (`app:19535-19551`): refused during import unless `{force:true}`; frame set to `about:blank`
  on next tick if still closed.
- Scanner page sets `window.__scannerEmbedMode` (`embed=1`) and `html[data-embed=true]` before paint
  (`scanner/index.html:18-21`).
- Bridge (`app:19504-19513, 19711-19735`): messages must have origin = own origin, source = the frame,
  `source:'inhouse-scanner'`, matching `sessionId`. `ihn-scanner-ready` → reply
  `{source:'inhouse-notes', type:'ihn-scanner-init', sessionId, documentName: exportName||'Untitled',
  theme: html[data-theme]||'light', maxPages: 24, openSource: true}`; `ihn-scanner-close` → close;
  `ihn-scanner-pages` → import. Listener installed in `setupEventListeners` (`app:16362`).

---

## 5. localStorage / sessionStorage inventory (shell-owned)

| Key | Format | Writer → reader |
|---|---|---|
| `inhouse-theme` | `'dark'|'light'`; absent = follow system | written only by user toggle (`app:5873-5879`); read by boot (`boot-v5.js:15`) and `initTheme` (`app:5893-5921`). Never cleared. |
| `inhouseNotesAppMode` | `'1'` | `app:105-121`. |
| `drive-token-v1` | JSON `{token, expiry(ms epoch), scopeVersion:'drive-calendar-v2', grantedScopes}` | `saveDriveTokenToStorage` (`app:21727-21737`), cleared `app:21750-21756`. |
| `drive-remember-v1` | `'1'` | on any token save or sign-in start (`app:6369, 6507, 21731`); removed on sign-out (`app:6979`). |
| `drive-refresh-token-v1` | Google refresh token (PKCE only) | `app:6489-6491`; removed on 400/401 refresh or sign-out (`app:6458-6461, 6984`). |
| `drive-pkce-verifier-v1` | base64url(32 random bytes) | `app:6372`; removed after exchange/sign-out (`app:6433, 6985`). |
| `google-scope-version-v1` | `'drive-calendar-v2'` | `app:6493, 6563, 21732`. |
| `google-granted-scopes-v1` | space-separated scopes | `app:21733`. |
| `drive-consent-granted` | `'1'` | `app:6492, 6562`; decides `prompt=consent`. |
| `drive-hint-email` | account email | `app:7056`; login_hint and offline account identity. |
| `drive-profile-v1` | JSON `{name,email,photo}` | `app:7057`; restored at boot (`app:6993-7000`). |
| `notebook-view-mode-v1` | `'drive'|'editor'` | written by view switches (`app:6136, 6253`); read at `app:2310` but **unused** (*quirk*). |
| `notebook-view-v1` | JSON `{panX,panY,zoom}` (all numbers) | 250 ms debounced (`app:21675-21708`). |
| `drive-session-v1` | JSON `{fileId, offlineLocalId, fileName, folderId, parentIds, folderKind, exportName, autosave, canEdit, ownedByMe, ownerName, ownerEmail, ownerPhoto, resourceKey, lastDriveSaveAt, headRevisionId, revisionVersion, lastKnownModified}` | `app:21788-21822`, restored with legacy defaults at `app:22049-22101`. |
| `touchDrawEnabled` | `'true'|'false'` | editor menu toggle (`app:15972-15980`). |
| `inhouse-services-updater:notes` (+`:installation`, `:installation-seen`) | §14.3 | updater. |
| session `ihn-release-reload` | published version string | boot release check (`boot-v5.js:137-151`). |
| session `ihn-home-tab` | home tab id | restored in `setupEventListeners` (`app:16234-16237`). |
| session `ihn-network-offline` | `'1'|'0'` | offline library connectivity (`offline-library-v5.js:58-63`). |

Other keys (`notebook-data-v3`, `drive-pending-exit-upload-v1`, `presence-client-id-v1` `app:27710`) and
IndexedDBs belong to the storage/collab specs.

---

## 6. Boot (`boot-v5.js`, runs synchronously in `<head>`)

Execution order (`boot-v5.js:195-214`):
1. **InhouseAssetUrl(path)** (`boot-v5.js:4-10`): parse `#inhouse-native-assets` JSON (errors ignored);
   resolve `path` against `location.href`; if same origin and a digest exists for the pathname, set
   `native=<digest>`; return href.
2. **InhouseBackground** facade if `InhouseBackgroundChannel.postMessage` exists (§4.3).
3. **applySavedTheme()** (`boot-v5.js:12-26`): dark iff stored `'dark'`, or nothing stored and
   `prefers-color-scheme: dark`; storage exception → system preference. Sets `html[data-theme]`,
   `html.style.backgroundColor` = `#151515` (dark) / `#f5f5f0` (light), and `#app-theme-color[content]`
   to the same colour.
4. **handleOAuthCallback()** (§8.6).
5. **selectBootDestination()** (`boot-v5.js:47-74`): `hasToken` = stored `drive-token-v1` with token,
   expiry, `scopeVersion==='drive-calendar-v2'` (also the separate scope-version key), granted scopes
   containing `https://www.googleapis.com/auth/calendar.events` and `now < expiry-60000`; `hasRefresh` =
   scope version current + calendar scope in `google-granted-scopes-v1` + refresh token present;
   `returning` = `drive-remember-v1==='1'` or a `drive-hint-email`. `window.__INHOUSE_BOOT_SIGNED_IN__ =
   hasToken||hasRefresh||returning` (false on any exception). `html[data-boot-destination]` =
   `drive` | `welcome`.
6. **watchPublishedVersion()** (§15.3).
7. **revealInitialView()** as soon as `#drive-folder-files` exists (MutationObserver on
   `documentElement`, disconnected at `DOMContentLoaded`) (`boot-v5.js:203-214`). It
   (`boot-v5.js:76-106`): hides `#welcome-view` when signed in; schedules the fonts-ready reveal (§3);
   when signed in, unhides `#drive-home` and fills skeletons — 6× card skeleton into `#drive-recents`,
   `#drive-starred`, `#drive-shared`; 4× folder skeleton into `#drive-folders`; 8× card skeleton into
   `#drive-folder-files`; `#drive-status` gets `<span class="drive-status-skeleton" aria-hidden="true">`
   and `aria-label="Loading your Drive"`. Card skeleton markup:
   `<div class="drive-card-skeleton" aria-hidden="true"><div class="skeleton-bar title"></div><div class="skeleton-preview"></div></div>`;
   folder: `<div class="drive-folder-card-skeleton" aria-hidden="true"><div class="skeleton-icon"></div><div class="skeleton-bar"></div></div>`.

---

## 7. Initialisation and view switching

### 7.1 `showFastInitialView()` (`app:2222-2254`)
Parses share params (§4.4). `hasStoredSession` = valid stored token (expiry−60 s) OR refresh token OR
`__INHOUSE_BOOT_SIGNED_IN__===true`. Route: share → editor if token in memory else Home; else
token/stored session → Home; else Welcome.

### 7.2 `init()` (`app:2256-2369`) — exact order
1. `pendingViewState = loadViewState()`; `initTheme()`; `loadDriveSession()`; `restoreCachedDriveProfile()`;
   `showFastInitialView()`.
2. `offlineLibraryReady = InhouseOfflineApp.initialize()`; on rejection `InhouseOfflineLibrary.disable()` and
   `console.warn('Offline library unavailable:', error)`.
3. `initDriveAuth()` (must precede `loadFromStorage`, `tests/release-v5-smoke.mjs:186-190`);
   `setupMobileHomePullToRefresh()`; `html[data-app-ready="true"]`.
4. `await Promise.all([loadFromStorage(), offlineLibraryReady])`; `await InhouseOfflineApp.reconcileStartup()`;
   `void InhouseOfflineLibrary.resume()` (warn `'Offline resume pending:'`); `bakeLegacyEraserAreaStrokes()`.
5. If the editor is visible at boot: add a page if empty, else `renderAllPages()`.
6. `setupEventListeners()` (§11); `syncEraserMenuState()`; `updateDocTitle()`; 60 s
   `setInterval(updateSaveTimestamp)`; `setReadOnlyMode(true,{force:true})` — every session starts in
   **Read** mode.
7. Re-read share params (resets `sharedLinkSignInPrompted`, `lastPendingSharedPublicAttemptId`).
8. `isSignedInAtBoot` = token in memory, valid stored token, refresh token, `__INHOUSE_BOOT_SIGNED_IN__`,
   or `InhouseOfflineLibrary.ready()`. Route: share → editor/Home; not signed in → Welcome; else **always
   Drive Home on cold boot**, even if the last session was in a document (`app:2335-2339`).
9. Wire `#btn-welcome-login` → `signInDrive()`; `installWelcomeSheet()`; `updateViewerModeUI()` if a
   Drive file is set; `tryOpenPendingSharedWithoutSignIn()` if share and no token.
10. If editor visible at boot: `whenViewportReady` (rAF up to 60 frames until the viewport has size and a
    `.page-row` exists, `app:2374-2389`) → `scrollToTodayCalendarPage()` else `applyViewState()` else
    `centerViewOnTop()`; start collab polling if file+token.

### 7.3 Views (`app:6119-6254`)
- `showDriveHome()`: force-close scanner; hide welcome & editor, show `#drive-home`; close timeline,
  save-debug popup, presence popup, profile menus; clear public inline preview and sign-in gate;
  `closeDocumentScopedWork()`; `clearDriveSyncTimers()`; persist `'drive'`; start Home auto refresh
  (30 s, `app:376, 11147-11152`); `updateCalendarFeatureAvailability()`.
- `showWelcomeView()`: show welcome, hide `#welcome-sheet` and backdrop, hide Home and editor, same
  closures, `closeDocumentScopedWork()`, stop auto refresh.
- `showEditorView()`: hide welcome and Home, show editor; `documentClosedToHome=false`;
  `restoreDocumentInkSession()`; stop Home auto refresh; start calendar side-panel sync if visible;
  next frame `clampToolbarPosition()` + forced visible-page update; persist `'editor'`.
- `closeDocumentScopedWork()` (`app:6106-6117`): stop collab polling, stop side-panel sync, cancel the
  background-suspend timer, `documentClosedToHome = !!state.driveFileId`, `suspendDocumentInkSession()`.
- Background suspension: hidden for **60 000 ms** (`DOCUMENT_BACKGROUND_SUSPEND_MS`, `app:6070`) with
  the editor active → `stopLiveCollaboration()` + ink suspension, log `'Document live work suspended in
  background'` (`app:6072-6091`); becoming visible resumes (`app:6094-6100`).
- `isDocumentEditorActive()` = file id set && editor visible && not suspended (`app:6020-6023`).
  `isDocumentLiveWorkPaused()` = suspended || (closed to Home && no Drive/offline open in progress)
  (`app:6030-6033`).
- Welcome sheet (`app:6150-6170`): `#btn-welcome-start` opens `#welcome-sheet` + backdrop and focuses
  `#btn-welcome-login`; backdrop click or Escape inside the sheet closes and refocuses the start button.

### 7.4 Welcome markup and strings (`index.html:9234-9260`)
Art `icons/welcome-pages.svg` (360×330, `decoding=async`); logo SVG (orange `#FF7F1F` page, `#FFB984`
fold, white chevron) + `inhouse notes`; `<h1 class="welcome-title">Write by hand.<br>Find it anytime.</h1>`;
lead `Real notebooks in your own handwriting, saved to your Google Drive and ready offline.`; button
`#btn-welcome-start` (`aria-haspopup=dialog`, `aria-controls=welcome-sheet`) with Google "G" SVG and
`Continue with Google`. Sheet `role=dialog aria-modal=true aria-labelledby=welcome-sheet-title`: grab
bar, `When you sign in`, three items — **Your notebooks are PDFs in your Google Drive** / *Open them with
any PDF app, or move them like any other file.*; **The same notebooks on your phone and computer** /
*Changes sync on their own.*; **You can sign out anytime** / *Your notebooks stay in your Drive.*; note
`Google will ask for access to Drive and Calendar. Calendar is only used for the events on your pages.`;
`#btn-welcome-login` `Continue with Google`. Styling `index.html:4348-4580` (fixed full screen,
z-index 1300, dark button `#1a1a1a` → white in dark theme, 14px radius, 54px min height; sheet 22px top
radius, max width 520px, backdrop `rgba(20,14,8,.42)`; ≥900px two-column layout).

---

## 8. Google OAuth

### 8.1 Constants (`app:381-427`)
- Web client `435784295430-cmug30o42f1vu4ijgor9sjb0ro4oo37o.apps.googleusercontent.com`.
- Android client `435784295430-9ta7t92s33r25bamvrnbmgt65iihqpfo.apps.googleusercontent.com`; PKCE
  redirect `com.local.inhousenotes:/oauth2redirect`. `useAndroidPkceFlow() = isInhouseApp && !!ANDROID_OAUTH_CLIENT_ID`
  (always true in the APK today).
- Scopes `https://www.googleapis.com/auth/drive https://www.googleapis.com/auth/calendar.events`;
  scope version tag `drive-calendar-v2`. A token **without the calendar scope is rejected everywhere**
  (`app:6482-6487, 6549-6555, 6848-6853, 21769-21773`).
- Optional public Drive API key from `window.INHOUSE_PUBLIC_DRIVE_API_KEY` or
  `<meta name="inhouse-public-drive-api-key">` (`app:385-389`); none is present in production.

### 8.2 `initDriveAuth()` (`app:6288-6334`)
1. `processDriveOAuthRedirect()` (hash/search with `access_token=`/`error=`) and
   `loadDriveTokenFromStorage()` (drops tokens with wrong scope version, no calendar scope or within
   60 s of expiry; schedules refresh).
2. **App mode**: `updateDriveAuthUI()`; token → `refreshDriveFiles()`; else with PKCE and a stored
   refresh token (current scope version) → silent refresh then list (warn `'Silent token refresh failed:'`).
   Returns (GIS is never initialised in the APK).
3. **Web**: first time a token exists → `updateDriveAuthUI()` + `refreshDriveFiles()` once. If
   `google.accounts.oauth2` is missing → `scheduleDriveAuthRetry()`: retry every 200 ms for the first
   30 s, then every 2 s; the GIS `<script>` `load` event retries immediately (`app:6266-6287`).
   Otherwise create the token client `{client_id, scope, include_granted_scopes:true, callback:noop,
   error_callback: warn 'Google popup auth failed:'}`, `updateDriveAuthUI()`, and if
   `drive-remember-v1==='1'` and listing not started → silent `ensureDriveToken(false)` then list.

### 8.3 Token acquisition (`app:6759-6878`)
- `ensureDriveToken(interactive)`: rejects `'Signed out'` when signed out and not interactive;
  interactive clears signed-out/reconnect flags; returns the cached token if `now < expiry-60000`;
  de-duplicates concurrent requests (a pending silent request is upgraded to one interactive request
  only if it fails); tracks `driveTokenRequestGeneration` so sign-out cancels in-flight requests
  (`'Drive sign-in request was cancelled'`).
- `requestDriveToken` (web GIS): timeout 60 s interactive / 9 s silent (`'Silent token refresh timed
  out'`); options `{scope, include_granted_scopes:true}` + `prompt:'consent'` (interactive) or
  `prompt:'none'` + `login_hint` = profile email or `drive-hint-email` (silent). Success stores the token
  (`expiry = now + expires_in*1000`), schedules refresh, updates UI. Silent failures call
  `markDriveAuthNeedsReconnect`.
- `requestDriveToken` (PKCE): stored refresh token (+ current scope version) → refresh; else
  interactive → start PKCE sign-in; silent → reconnect state; reject `'No refresh token; user must sign in'`.
- Scheduled refresh: 5 min before expiry, silent `ensureDriveToken(false)` (`app:6724-6736`).
- `markDriveAuthNeedsReconnect(reason)` (`app:6738-6757`): unless signed out — clear token & storage,
  stop presence ping, defer a pending Drive save, `updateDriveAuthUI()`, status
  `'Drive sign-in needed to keep syncing'` (phase `paused`), save-debug `Drive auth paused: <reason>`.
- `driveFetch` (`app:7799-7890`): deadline (default 60 s) covers token acquisition; adds
  `Authorization: Bearer`; buffers JSON/error bodies; on 401 (first time) clears the token and retries
  once, a failed retry → reconnect; errors whose message matches
  `/auth|token|sign.?in|refresh|No refresh token|Silent token/i` → reconnect; timeouts →
  `'Drive request timed out'`; aborts → `'Drive request aborted'`; non-OK →
  `Error('Drive API error <status>: <body>')` with `code`/`status`. Streamed media bodies keep an idle
  timeout (`min(timeout, bodyIdleTimeout||15000)`) (`app:7749-7797`).

### 8.4 Web sign-in
`signInDrive()` (`app:6881-6893`): clear flags; app → `startDriveRedirectSignIn()`; web →
`ensureDriveToken(true)` (GIS popup) then `refreshDriveFiles()`; failure → `#drive-status`
`'Sign-in failed. Try again.'`.

### 8.5 Android PKCE flow
1. `startAndroidPkceSignIn()` (`app:6367-6405`): remember flag; verifier = base64url(32 random bytes)
   stored; challenge = base64url(SHA-256(verifier)); URL
   `https://accounts.google.com/o/oauth2/v2/auth?client_id=<android>&redirect_uri=com.local.inhousenotes:/oauth2redirect&response_type=code&scope=<scopes>&access_type=offline&include_granted_scopes=true&code_challenge=<c>&code_challenge_method=S256&state=inhouse_notes_pkce[&login_hint=<hint>][&prompt=consent]`
   (`prompt=consent` unless `drive-consent-granted==='1'` and scope version current). Opened via
   `InhouseNative.openAuthUrl(url)` (Chrome Custom Tab, `FLAG_ACTIVITY_NO_HISTORY`,
   `html_to_apk_builder.py:1801-1809`); status `'Complete sign-in in the Google window.'`; without the
   bridge → `location.assign(url)` (warn).
2. Android receives `com.local.inhousenotes:/oauth2redirect?code=…` (intent filter scheme = package id,
   `html_to_apk_builder.py:1507-1536`) and evaluates
   `window.handleInhouseNativeOAuth(<query or fragment>)` (`html_to_apk_builder.py:1631-1665`).
3. `handleInhouseNativeOAuth(payload)` (`app:6581-6601`): strip leading `#`/`?`; with `code` and PKCE →
   `exchangeAuthCodeForTokens` then list (error → `'Sign-in failed. Try again.'`), returns `true`;
   otherwise treat as implicit params. `state` is **not** validated (*quirk*).
4. Exchange/refresh (`app:6407-6472`): POST `https://oauth2.googleapis.com/token`
   (`application/x-www-form-urlencoded`) via `fetchWithDeadline(…, 20000)`; bodies
   `client_id, code, code_verifier, grant_type=authorization_code, redirect_uri` /
   `client_id, grant_type=refresh_token, refresh_token`. No client secret. Refresh 400/401 deletes the
   refresh token. `applyTokenResponse` (`app:6474-6496`): require `access_token`, expiry
   `now + max(60, expires_in||3600)*1000`, require calendar scope (`'Calendar access was not granted by
   Google.'`), store token, refresh token, `drive-consent-granted`, scope version; schedule refresh.

### 8.6 Legacy implicit redirect flow (kept for old shells)
- `startDriveRedirectSignIn()` non-PKCE branch (`app:6506-6536`): `response_type=token`,
  `redirect_uri=${origin}/?oauth_callback=1`, `state=inhouse_notes_app`, same hint/consent rules; in
  app → `openAuthUrl`, else `location.assign`.
- boot `handleOAuthCallback()` (`boot-v5.js:28-45`): if search contains `oauth_callback=1`: with
  `access_token=` in hash → `location.replace('inhousenotes://oauth2callback' + hash)`; with `error=` →
  `inhousenotes://oauth2callback#error=<e>`; after 500 ms → `location.replace('/?inhouse_app=1' + hash)`.
- `completeDriveSignInFromParams` (`app:6538-6570`): error → `#drive-status` `Sign-in failed: <error>`;
  missing calendar scope → `'Calendar access was not granted. Sign in again and accept Calendar access.'`;
  else store token (expiry `max(60, expires_in||3600)`), consent + scope version, schedule refresh,
  `history.replaceState` removing the hash.
- Android `inhousenotes://oauth2callback#access_token=…` additionally `loadUrl('https://inhousenotes.com/?inhouse_app=1#…')`.

### 8.7 `oauth-callback/index.html` (static page, shared with Inhouse Read)
Shows `Completing Google sign-in…`. Decodes `state` (`<base64url JSON>.<sig>`): a Read flow iff payload
origin is `https://miguelcoxcaballero.github.io` with a nonce. Errors post `{error,state}` to the opener
and show `Google sign-in failed: <e>`; no opener → `Google sign-in could not be completed. Return to the
app and try again.`. Notes flow posts `{code,state}` to `https://inhousenotes.com` and closes
(**no current Notes listener consumes it**, *quirk*). Read flow posts `{type:'ihr-oauth-code'}`, waits
for `{type:'ihr-oauth-exchange', code, verifier, redirectUri, state}` from the opener, exchanges with the
web client id and posts `{type:'ihr-oauth-token', tokens:{accessToken, refreshToken, expiresAt}}`
(`oauth-callback/index.html:10-73`). Must be deployed unchanged.

### 8.8 `updateDriveAuthUI()` (`app:6605-6722`)
Inputs: `signedIn` (token in memory), `reconnecting` (needs reconnect and not signed out), local
offline library ready, boot flag, pending share.
- `#btn-drive-login`: visible (`inline-flex`) iff not connected-or-recoverable, or reconnecting while
  online; text `Reconnect` / `Sign in`.
- `#drive-profile`, `#drive-profile-editor`: hidden iff not connected-or-recoverable.
- Disabled: search/new/new-folder unless signed in or local library; Bin unless signed in; Back
  additionally when stack ≤ 1.
- `#drive-status` text (priority): `Offline · changes stay on this device` → `Drive connection paused.
  Sign in again to continue syncing.` → `Loading your PDFs...` → (share) `Opening public preview...` /
  `Opening shared document...` → `Connecting to Drive…` (local library) → `Sign in to browse your Drive.`
- Not connected: hide empty states, close menus; pending share → gate or Home + anonymous open.
  Connected with dirty autosave → schedule Drive sync after 500 ms.
- Connected while welcome visible → `showDriveHome()`.
- Signed in: load profile, restore PDF backgrounds, resume pending exit upload (next tick), start collab
  polling if a file is open, open pending share (§4.4).

### 8.9 `signOutDrive()` (`app:6895-6991`) — exact effects
1. `await InhouseOfflineLibrary.capture()`; failure → error status
   `'Keep this document open: its offline copy could not be saved.'` and abort sign-out.
2. Offline store `settings/__session__/active = {enabled:false}`; clear `background-auth` for the
   account; `InhouseOfflineLibrary.logout()`; `__INHOUSE_BOOT_SIGNED_IN__=false`.
3. Flags: signed out, no reconnect, generation++ (cancels token requests); stop Home refresh, collab
   polling, sync timers, paused-save watchdog, deferred retry; abort Drive save controller and local save;
   reset all save/presence flags and revisions, hydration state.
4. `google.accounts.oauth2.revoke(token)` when GIS is loaded; clear token, timers.
5. Reset all `state.drive*` fields, `calendarPageConfig`; clear Drive caches, folder stack to
   `[{id:'root', name:'My Drive'}]`, hide folder view, empty all home lists/breadcrumbs, status
   `Sign in to browse your Drive.`; clear profile and shared-link state and gate.
6. Remove storage: token, `drive-remember-v1`, `drive-hint-email`, `drive-profile-v1`
   (+`InhouseOfflineMedia.clear()`), `drive-consent-granted`, scope version, granted scopes, refresh
   token, PKCE verifier, Drive session.
7. `updateDriveAuthUI()`, `updateCalendarFeatureAvailability()`, status `'Signed out'`,
   `showWelcomeView()`. Theme and app-mode flags are kept.

### 8.10 Profile
- `DEFAULT_AVATAR` (`app:282-291`): data-URI SVG 64×64, grey gradient circle (`#f2f2f2→#d9d9d9`) with
  `#b3b3b3` head and shoulders. Every avatar `<img>` falls back to it on `error` (`app:16100-16119`).
- `restoreCachedDriveProfile()` uses `drive-profile-v1` only when its email equals `drive-hint-email`
  (case-insensitive), marking it `fallback:true` (`app:6993-7000`).
- `loadDriveProfile()` (`app:7037-7066`): skipped offline or when already loaded for this token; GET
  `drive/v3/about?fields=user(displayName,emailAddress,photoLink)`; profile `{name: displayName||'Signed
  in', email, photo: photoLink||DEFAULT_AVATAR, token}`; persist hint email + `drive-profile-v1`;
  `InhouseOfflineLibrary.useAccount(email)`; `InhouseOfflineApp.authorizeBackground()`; render.
- `updateDriveProfileUI()` (`app:7002-7035`): fills both menus (avatars `#drive-profile-avatar[-menu][-editor]`,
  name default `Signed in`, email); photos are proxied through `InhouseOfflineMedia.url(email,'avatar',…)`
  for offline use.
- `authorizeBackground()` writes `settings/__session__/active={accountId,enabled:true}` and
  `settings/<account>/background-auth={token, expiry, refreshToken (PKCE only), clientId (PKCE only)}`
  (`offline-app-v5.js:216-223`). `backgroundAccess(expectedAccount, rejectedToken)` throws
  `'Background account changed or signed out'` unless the account and hint email still match before and
  after `ensureDriveToken(false)` (`offline-app-v5.js:275-288`).

---

## 9. Theme behaviour (app)

- `initTheme()` (`app:5893-5921`): stored `dark|light` wins, else system; subscribes to
  `prefers-color-scheme` changes and follows them **only while nothing is stored**.
- `applyTheme(mode, persist)` (`app:5869-5881`): sets `html[data-theme]`, repaints Drive folders, writes
  `inhouse-theme` when `persist`, syncs both checkboxes (`#theme-toggle`, `#theme-toggle-editor`).
- Toggles: label `Night mode` in both profile menus (`index.html:9309-9312, 9601-9604`); `change` →
  `applyTheme(checked?'dark':'light', true)` (`app:15960-15971`). Style `.theme-toggle`
  (`index.html:2871-2911`): 38×20 pill, knob 14px, checked background `var(--text-primary)`, knob
  translate 18px.
- *Quirk*: only boot writes `html.style.backgroundColor` and `meta#app-theme-color`; toggling at runtime
  leaves them at the boot colour until reload.
- Android boot colour: `@color/ihn_boot_background` `#F5F5F0` / night `#151515` for window, status bar,
  nav bar, splash and the WebView background (`html_to_apk_builder.py:2393-2443, 1601-1604`).
- Updater dialog follows `[data-theme=dark|light]`, else system (`updater.css:20-24`). Scanner receives
  the theme in `ihn-scanner-init`.

---

## 10. Status, loading and toasts

- **No toast component exists.** `showStatus(text, options)` (`app:31660-31719`) only drives the editor
  save indicator: `options` may be a boolean (`saving`) or `{saving, error, preserveState, savedAt,
  phase}`; save-related texts (`/save|sync|drive|pdf|upload|build|pending|paused|queued/i`) or any
  phase/saving/error/savedAt also update the save-debug state; unless `preserveState`, the
  `#save-indicator` class becomes one of `paused|building|uploading|syncing|saving|saved|error`
  (`SAVE_STATES`, `app:31290`); `title` and `aria-label` = text; `savedAt` updates `state.lastSavedAt`;
  plain `Saved`/`Synced` confirms after 350 ms; building/syncing/saving reconcile after 1800 ms;
  finally `refreshSaveIndicatorState()`. Messages therefore appear only as tooltip/ARIA and in the save
  details popup (owned by the save spec). The editor-only `#status-bar` (`index.html:9739-9742`,
  `.status-dot` + `#status-text` `Saved`) is never made `.visible` (*quirk*, dead UI; CSS at
  `index.html:1345-1347` then `2913-2941`).
- **Home status line** `#drive-status` (`index.html:9343`) carries auth/listing messages (§8.8).
- **Loading overlay** `#loading-overlay[role=status][aria-live=polite]` (`index.html:10259-10270`):
  hidden brand block, `.spinner`, `#loading-text` default `Loading PDF...`.
  - `showLoading(text)` (`app:31734-31744`): cancel fade-out, remove `home-save-transition`, set text,
    `aria-busy=true`, add `.visible`.
  - `showHomeSaveLoading(text='Saving changes to Drive...')` (`app:31746-31755`): same plus
    `.home-save-transition` (opaque `var(--bg-primary)` background, brand "inhouse notes" visible,
    38px orange spinner, secondary text) — used by Home with `'Saving on this device…'` (`app:16001`).
  - `hideLoading()` (`app:31757-31780`): if visible add `.fading-out` for 360 ms; remove `.visible`;
    `aria-busy=false`; drop `home-save-transition` immediately or after 180 ms.
  - CSS (`index.html:4251-4338`): fixed, `rgba(0,0,0,.7)`, z-index 3000, `transition: all .3s`;
    spinner 48px white ring 1 s linear; spinner animation is `none` unless `.visible` or `.fading-out`.
- **Save timestamp** `#save-timestamp` (`app:22148-22162`) with `formatTime` (`app:22125-22146`):
  falsy → `--:--`; 1–59 min ago → `N min`; otherwise `toLocaleTimeString(undefined,{hour:'2-digit',
  minute:'2-digit', hour12:false})` cached per clock minute. Titles `Last saved <t>` (Drive time) /
  `Last edit <t>` / `Not saved yet`.
- **Fullscreen** (web only) (`app:16602-16649`): toggles `documentElement.requestFullscreen` (webkit/ms
  fallbacks); buttons get title/aria `Fullscreen`/`Exit fullscreen`, `.active`, and swap icons
  `FULLSCREEN_ICON_ENTER/EXIT` (`app:270-281`); on change dispatch a `resize` after 50 ms.
- **APP download modal** (web only) `#modal-download-app` (`index.html:10092-10117`): close button
  `aria-label="Cerrar"`, Android icon `#3ddc84`, title `Download the inhouse notes Android app`, link
  `#download-app-link` → `…/releases/download/android-v1.0.11/inhouse-notes-release-v1.0.11.apk`
  (`download` attr, `rel=noopener`, text `Download APK`) — *quirk*: pinned to 1.0.11. Opens from
  `#btn-download-app` (`APP`), closes on ×, backdrop, or 100 ms after the link click (`app:15832-15851`).
- **Fast scroller** `#document-fast-scroll` (`app:35797-35862`, CSS `offline-v5.css:24-28`): role
  `scrollbar`, label `Fast scroll`, drives Home/folder scroll or editor pan; keys ArrowUp/Down ±4 %,
  PageUp/Down ±20 %, Home/End; hidden when content fits (`max<2` or travel `<30`).

---

## 11. Global event wiring (`setupEventListeners`, `app:15742-16600`) — shell-relevant entries

| Lines | Listener |
|---|---|
| 15746-15765 | `pointerup/pointercancel/touchend` blur the pressed button (`button,.btn,.tool-btn,.drive-profile-btn,.presence-popup-close`) next frame for touch/pen. |
| 15766-15771 | capture `pointerdown/pointermove` remember last client point. |
| 15772-15784 | `paste` images into the document when editable. |
| 15824-15829 | `#btn-drive-login` → `signInDrive`; create menu install. |
| 15832-15851 | APP download modal. |
| 15852-15885 | Profile menu buttons: Diary/Agenda → quick-access open; Refresh → close menus + `refreshDriveFiles()`; Sign out → close menus + `signOutDrive()` (both menus). |
| 15931-15942 | Avatar buttons toggle their menu (`stopPropagation`); opening one closes the other; editor menu refreshes `#doc-stats` (`N page(s) · N stroke(s)[ · size]`, `app:7068-7111`). |
| 15960-15980 | Night mode toggles; `Draw with finger` toggle (`touchDrawEnabled`). |
| 15981-16061 | `#btn-drive-home` (click unless on the title; Enter/Space) → **goHome**: `forceFlushingForExit`; stop pen/gesture state; if anything pending show `showHomeSaveLoading('Saving on this device…')`, interrupt background work, `queueSave()`, `persistExitLocalCheckpoint()`; require a durable offline copy when Drive is pending; flush live collaboration (650 ms, not awaited); clear presence; `showDriveHome()`; status `Saved on this device · waiting to sync` or `All changes saved.`; kick outbox; start lifecycle exit upload when online; `refreshDriveFiles()`. Failure → error `Could not finish saving. The document is still open; please try Home again.` Never waits for Drive auth or uploads (`tests/release-v5-smoke.mjs:450-458`). |
| 16120-16141 | document `click` closes bin/card/color menus, presence popup, and both profile menus unless the click is inside a menu or its button. |
| 16142-16157 | `Escape`: navigate folder up if folder view open and no overlay; close cover/folder-picker/trash/bin/bin-menu/profile menus/presence. |
| 16159-16198 | capture `pointerdown/touchstart/wheel/keydown` (not composing) → `interruptProcessingForUserInput()`; `pointermove/touchmove` (pressed or touch) throttled to 60 ms, else `noteRecentUserActivity()`; ignored inside `.save-debug-backdrop, .mode-toggle-status, #save-indicator`. |
| 16199-16237 | Back button, home tabs, `sessionStorage['ihn-home-tab']` restore. |
| 16263-16271 | `#drive-search` input → `refreshDriveFiles()` debounced 250 ms. |
| 16292-16296 | `#mode-toggle-btn` → toggle read/edit. |
| 16306-16318 | `resize`: reposition eraser menu / presence popup / cover preview, reset slider, preserve view. |
| 16321-16337 | Export/import/fullscreen/reset buttons. |
| 16350-16359 | `Ctrl+Z` undo, `Ctrl+Y` redo (editable only). |
| 16362 | `message` → scanner bridge. |
| 16526-16529 | `fullscreenchange` (+webkit/ms) → button state; initial call. |
| 16531-16538 | `beforeunload`: clear presence, `flushOnClose()`; if an exit upload is running or a pending-exit marker exists → `preventDefault()` + `returnValue=''` (leave-page prompt). |
| 16540-16542 | `pagehide` → `flushOnClose()` (`app:26268-26299`: marker, view state, metadata backup, lifecycle checkpoint/upload). |
| 16544-16550 | `offline`/`online` → refresh save indicator; online resumes pending exit upload. |
| 16552-16598 | `visibilitychange`: hidden → `flushOnClose()` then `releaseMemoryOnHide()` (halve image cache, halve history budget, evict inactive PDF backgrounds, `app:1158-1184`), stop side-panel sync, pause public polling or collab polling + schedule 60 s suspension; visible → resume, restart collab polling + immediate poll + fast catch-up if editor active, resume exit upload, or restart public polling; restart side-panel sync. |
| 16599 | `spStartAutoSync()`. |

Other module-level listeners owned by the shell: boot `visibilitychange`/`pageshow`/`load` (§15.3);
updater `focus`/`visibilitychange` + 15 min interval (§14); offline library `offline`/`online`/
`visibilitychange`/`message`/SW `message` (`offline-library-v5.js:574-594`); offline app editor-lease
MutationObserver and `visibilitychange` (`offline-app-v5.js:262-273`).

---

## 12. Settings surfaces

Home profile menu `#drive-profile-menu[role=menu]` (`index.html:9296-9325`), in order: meta (avatar
44px, name, email); `Night mode` toggle; row `Diary` | `Agenda`; row `Refresh` | `Sign out`;
`Offline documents` (`[data-offline-settings]`); `Actualizaciones · A/B` (`[data-inhouse-update-settings]`,
`hidden` until the updater is created — Spanish label, *quirk*); `[data-app-version]` (centred, 11px,
`#bbb`, `padding-top:4px`).
Editor profile menu `#drive-profile-menu-editor` (`index.html:9588-9622`): meta; `Night mode`;
`Draw with finger` (`#touch-draw-toggle`); `#doc-stats` (12px `#888`, top border); row `Refresh` |
`Sign out`; `Offline documents`; `Actualizaciones · A/B`; `[data-app-version]`.
Menu styling `index.html:4755-4870` (white card, 16px radius, 14px padding, min-width 240px, z 2000;
dark `#1f1f1f`; Diary button `#c9bfd8/#362b45`, Agenda `#2f5a3f/#f4f8f2`).
`Offline documents` opens `<dialog id="offline-settings">` (retention days 1–3650, outbox status,
merge recoveries; `offline-library-v5.js:549-571`; spec owned by the offline spec).

---

## 13. Security core (`security-core-v5.js`) — `window.InhouseSecurityCore` (frozen)
- Limits: calendar HTML 512 KiB, PDF 64 MiB, 2000 pages, 40 M image pixels (`:4-7`).
- `sanitizeCalendarPanelHtml(html, doc)` (`:24-79`): template parse; allowed tags `DIV, SPAN, BUTTON`
  (others removed with contents); allowed attributes `class, data-day-key, data-sp-add, data-sp-event,
  data-sp-day, style, type, title, aria-label`; allowed classes `sp-day-section, today, sp-day-header,
  sp-day-label, sp-add-btn, sp-events-list, sp-no-events, sp-event, sp-event-time, sp-event-title`;
  `style` reduced to `--sp-event-color: #rrggbb` only; data/title/aria values truncated to 512; buttons
  forced `type=button`; comments removed; duplicate `data-sp-event` per day removed; oversize → `''`.
- `assertSafePdfBlob(blob)` (`:88-99`), `isTrustedPdfUrl(url, href)` (`:101-117`: `blob:` ok; http(s)
  same origin ok; https `www.googleapis.com`, `drive.google.com`, `drive.usercontent.google.com` ok),
  `hardenPdfDocumentParams` (`:119-146`: `isEvalSupported=false`, `maxImageSize ≤ 40e6`, size checks,
  error texts `'Blocked untrusted PDF URL'`, `'PDF data exceeds the safe size limit'`, `'PDF source must
  contain data or a URL'`, `'Unsupported PDF source'`), `loadPdfDocument(pdfjs, source, opts)` (`:148-160`,
  destroys task on page-count violation). App wrapper `loadSecurePdfDocument` (`app:82-86`).

Runtime core used by the shell: `fetchWithDeadline(url, opts, timeoutMs≥1000 default 30000,
consume)` throws `TimeoutError` `'Network request timed out'` and preserves caller aborts;
`settleWithTimeout`, `runWithRetries` (`runtime-core-v5.js:4-93`).

---

## 14. Shared updater UI and A/B channels

### 14.1 Integration (`app:136-158`)
In this section `updater.js`/`updater.css` = `shared/inhouse-services-updater/updater.{js,css}`.
Only when `isInhouseApp && window.InhouseServicesUpdater`:
```js
const updates = InhouseServicesUpdater.create({
  appId: 'notes', appName: 'Inhouse Notes', repository: 'miguelcoxcaballero/inhousenotes',
  installedVersion: '1.0.2',
  install: release => {
    if (InhouseNative?.installInhouseUpdate) InhouseNative.installInhouseUpdate(release.apkUrl, release.apkSha256);
    else if (InhouseNative?.installAppUpdate) InhouseNative.installAppUpdate(release.apkUrl);
    else throw new Error('Abre la app Android para instalar.');
  }
});
```
Every `[data-inhouse-update-settings]` is unhidden and opens `updates.settings()`;
`window.InhouseUpdater = updates`; `updates.start()`. `installedVersion:'1.0.2'` is only a fallback
— the real version comes from `InhouseNative.getAppVersion()` (*quirk*). Script/CSS are byte-identical
to the vendored canonical copy `vendor/inhouse-services-updater/web/updater.{js,css}` (lock
`inhouse-updater.lock.json`, revision `d0d2afdd…`, version 1.2.4 while the JS says `VERSION='1.2.3'`).
v6 must load the **same files unmodified** (they are synced from the canonical repo, never edited
locally).

### 14.2 Catalog (`updater.js:15-101`)
- Fetch `https://raw.githubusercontent.com/<repo>/main/updates/<A|B>.json?check=<now>` (`cache:'no-store'`,
  12 s abort). If not newer (or failed), fallback `https://api.github.com/repos/<repo>/contents/updates/<ch>.json?ref=main`
  (base64 `content`), taken if its latest versionCode ≥ the first. Channels never fall back to each other.
- Validation: `schemaVersion===1`, `channel` matches, `releases` array; each release `version` `x.y.z`,
  safe-int `versionCode ≥1`, `apkUrl` https `github.com/<repo>/releases/download/…` ending `.apk` without
  credentials, `apkSha256` 64 hex, `apkSizeBytes ≥1`, `changelog` non-empty ≤120 chars; sorted by
  versionCode desc, no duplicates; `latest` must equal the top release (code and hash) or be null only
  when empty.
- `newer(r, installed, code)` = `compare(r.version, installed) ≥ 0 && (code>0 ? r.versionCode>code :
  compare>0)`.
- Current catalogs: `updates/A.json` latest **1.0.28 (29)** "Test the in-app updater." with 16 releases (1.0.28 … 1.0.11; no 1.0.20/1.0.22);
  `updates/B.json` latest **1.0.16 (17)** with 1.0.16 and 1.0.13. `android-update.json` mirrors A's latest
  with `required:true`, `publishedAppVersion:'5.16.29'`.

### 14.3 State
- Key `inhouse-services-updater:notes`; read from `InhouseNative.getUpdateState()` when present, else
  localStorage; normalised `{channel: (channel==='A' && developmentConsent===1) ? 'A' : 'B',
  developmentConsent, snooze:{A?,B?}}` — **B is the default and legacy implicit A migrates to B**
  (`updater.js:65-72`). Saved to native `saveUpdateState(json)` and localStorage.
- `…:installation` = `{version, versionCode, apkUrl, awaitingResult, reported?}`; `…:installation-seen` =
  `<nativeId>:<status>`.
- Snooze: `Later` (and any dismissal of an offer: Esc, backdrop, drag) sets `snooze[channel] = now +
  14 days` (`TWO_WEEKS`), suppressing **automatic** prompts only.

### 14.4 Lifecycle
`start()`: set `window.handleInhouseUpdateResult = result`; on `focus` and immediately and every
15 min run `automatic()` = `refreshInstallation() || check()`; on `visibilitychange` (not hidden)
`refreshInstallation()` (`updater.js:533-541`). `check({manual})` skips while installing or when snoozed
(automatic); de-duplicates in-flight checks; shows an offer when newer (automatic only if not snoozed and
not the same `channel:versionCode:snooze` id as last time); manual checks report status.

### 14.5 Screens (all English) — container `div#inhouse-services-update.isu-overlay[role=dialog][aria-modal=true][aria-labelledby=isu-title]`, `section.isu-card[tabindex=-1]` + `.isu-handle`, z-index 2147483647
- **Offer**: `Update available`; subtitle `<installed> → <b>version</b> · <size>`; `What’s new` +
  latest changelog; `Details` disclosure (`aria-expanded`) revealing `Also in this update` (≤3 earlier
  releases, then `+N earlier version(s)`), `Released` (date `en`, `{day:'numeric', month:'short',
  year:'numeric'}`), `Channel` `Version X` + `Change` link (`aria-label="Change channel"`); status line;
  recovery block: link `Download APK` → `https://github.com/<repo>/releases/tag/<tag>` + help `Choose the
  APK under Assets on GitHub.`; dock: `Later` | `Update` (filled primary) + progress (`Preparing
  download…`, `<progress max=100 aria-label="Download">`).
- **Settings** (`updater.js:303-331`): `Updates` + `Inhouse Notes <installed>`; `Channel`; group
  `aria-label="Update channel"` with items `Version B` / *Reviewed releases* and `Version A` /
  *Development builds, may be unstable* (`aria-pressed`); divider; `Check for updates` + status; credits
  `Made by Inhouse` (https://inhouse.miguelcoxcaballero.com/) and `GitHub` (repo URL); `Done`.
- **Switch-to-A warning** (`updater.js:285-302`): warning icon, `Switch to version A?`, `Version A gets
  untested development updates. Expect bugs, crashes or data loss.`, `Cancel` | `I understand`; the latter
  reveals a slider (`input[type=range]#isu-confirm-a`, `aria-label="Slide to confirm version A"`, label
  `Slide to switch to A`). Only a completed slide (value 100 on `change`, drag release ≥92 snaps to 100,
  keyboard End) after `I understand` selects A; tapping the track never confirms. Selecting A sets
  `developmentConsent=1`; selecting B clears it; each selection re-opens settings and runs a manual check.
- **Result**: title (`Update installed` / `Update not installed` / `Update not confirmed`), message
  (`Inhouse Notes <version>` …), optional recovery, `Updates` | `Done`.
- Status texts: `Checking…`; `Could not check for updates.`; `No releases on version X.`; `Your version is
  newer than version X.`; `You’re up to date.`; `Confirm the installation in Android. If the app closes,
  open it again.`; `Allow installs from this app, then tap Continue.`; `Installation cancelled.`; `The
  update could not be installed.`; `Installation has not been confirmed. You can try again.`; progress
  `Downloading… N%` / `Verifying…`, value `X MB of Y MB` (`size`: ≥1 MiB → one decimal MB else KB).
- Interaction: Tab focus trap; Escape closes unless installing; backdrop click dismisses; drag handle/
  headline down (non-mouse) dismisses past `min(140px, 30%)` or fast flick; leave animation 220 ms;
  height morph 300 ms `cubic-bezier(.2,0,0,1)`; reduced motion disables animation.

### 14.6 Install protocol
`Update` → `installing=true`, buttons disabled, persist installation attempt, indeterminate progress,
call `install(release)` (errors → `result({status:'error', message})`). Native reports via
`window.handleInhouseUpdateResult(payload)` (string or object): `downloading` (percent/downloadedBytes/
totalBytes), `ready`|`installing` → `Open installer` + recovery visible, `permission_required` →
`Continue` + warn, `error|cancelled|unconfirmed` → `Try again` + recovery, `installed` → result screen.
`refreshInstallation()` (`updater.js:402-436`) reconciles native `getInstallState()` (`{id, status,
expectedVersionCode, …}`) and the persisted attempt on focus/visibility/start, so outcomes survive
process death; a confirmed higher installed versionCode reports `Update installed` once.
Native bridge methods used (APK side; added by the frozen `register_android.py:52-56` or the builder):
`getAppVersion()`, `getAppVersionCode()`, `getUpdateState()`, `saveUpdateState(json)`,
`getInstallState()`, `installInhouseUpdate(url, sha256)`, legacy `installAppUpdate(url)` (rewritten to
call `installInhouseUpdate`). v6 must keep calling exactly these names and argument orders.

### 14.7 Policy constraints (`AGENTS.md`, `UPDATER.md`)
B default; A requires English warning + acknowledgement + slide; automatic releases publish to A; B
only on explicit user request; A includes A and B, highest global versionCode wins; tiny changelog
(2–10 words, ≤120 chars); publish the verified APK before its catalog; never replace published
binaries; never edit updater copies locally; keep package `com.local.inhousenotes` and signing key;
versionCode increments globally.

---

## 15. Version and release rules

### 15.1 Version sources
- Visible web version: `const APP_VERSION = 'x.y.z';` in `index.html:93` (the only inline script; inert
  type). App derives it (`app:67-70`) and renders `v<APP_VERSION>` into the two `[data-app-version]`
  nodes (`index.html:9323, 9620`). The string `data-app-version` must occur exactly 3 times across
  index+app+boot+cores (`tests/release-v5-smoke.mjs:63`).
- Must be kept in sync by the release: every `?v=` query in `index.html` and `scanner/index.html`
  (`tests/release-v5-smoke.mjs:64-67, 93`), `service-worker.js:2` `VERSION`, `package.json` `version`
  (read by `tests/e2e/app-resilience.spec.mjs:3`), `RELEASE_NOTES_v<version>.md` whose first line is
  `# v<version>` and mentions Android (`tests/release-v5-smoke.mjs:1198-1199`).
- Android: `versionName`/`versionCode` in `build_android_apk.py:20-21`, `build-android.yml` file names,
  `verify_release_apk.py:94`, `update_services_manifest.py`; independent of the web version.
- Semver policy and mandatory bump on every push to `main`; post-push verification of GitHub `main` and
  uncached `https://inhousenotes.com/` (`AGENTS.md:5-42`). v6.0.0 is the intentional major.

### 15.2 Scanner page
`scanner/index.html` also carries `?v=` and `native=` queries and is part of the offline shell and
release verification.

### 15.3 Published-release watcher (`boot-v5.js:108-193`)
Skipped without `fetch`, on `oauth_callback`, or in the offline worker. First check 1200 ms after
`load`; afterwards on `visibilitychange→visible` if >60 s since the last check, and on `pageshow`
with `persisted`. A check (not while offline/in flight) GETs
`/service-worker.js?release-check=<now>` with `cache:'no-store'`, extracts
`const VERSION='x.y.z'`; if it is greater than the running version and the editor is **not** visible:
once per tab per published version (`sessionStorage['ihn-release-reload']`; no reload if sessionStorage
throws) → `registration.update()`, delete `/index.html` and `/scanner/index.html` from every
`inhouse-shell-*` cache except `inhouse-shell-<published>`, then `location.reload()`.

### 15.4 Service worker (`service-worker.js`)
Registered by the offline library (non-worker) with `updateViaCache:'none'`
(`offline-library-v5.js:577-579`). Cache `inhouse-shell-<VERSION>`; install = atomic `addAll` of
`self.INHOUSE_OFFLINE_ASSETS` then `skipWaiting()`; activate deletes other `inhouse-shell-*` and
`clients.claim()`. Fetch: `/__offline-media__/` images from `inhouse-media-<account>` when a session is
active; ignore non-GET, cross-origin, `/oauth-callback*`, `?oauth_callback`; only listed assets (`/` →
`/index.html`); a `?v=` different from VERSION bypasses; HTML = cached copy first, background revalidate
(`cache:'no-cache'`) and store only if the body contains `v=<VERSION>`; other assets cache-first.
Background `sync`/`periodicsync` tag `inhouse-outbox` runs the outbox with stored background auth.

---

## 16. Build and deploy pipeline

### 16.1 npm scripts (`package.json`)
`test:unit` (21 node test files + `node tests/release-v5-smoke.mjs`), `test:e2e` (`playwright test`),
`test`, `build:rust` (`scripts/build-rust-core.mjs`: `cargo build --release --target
wasm32-unknown-unknown --locked`, require zero host imports and `core_abi_version()===1`, write
`wasm/inhouse_notes_core.wasm`; toolchain 1.90.0 `rust-toolchain.toml`), `build:offline`
(`build-offline-assets.mjs` then `build-native-assets.mjs`).

### 16.2 Offline and native assets
- `scripts/build-offline-assets.mjs`: list = fixed files (index.html, boot, app, all v5 modules, wasm,
  updater css/js, offline css, welcome svg) + every file under `vendor/pdfjs`, `vendor/pdf-lib`,
  `vendor/jspdf`, `vendor/fonts`, `scanner` except `.md/.map/.py/.zip/.pdf` and LICENSE; sorted, unique,
  `/`-prefixed → `offline-assets.js` (`self.INHOUSE_OFFLINE_ASSETS=[…]`).
- `scripts/build-native-assets.mjs`: rewrite font URLs in `ui.css` with `?native=<sha256>`; for every
  offline asset except HTML and `offline-assets.js` record `{sha256, size, mime}` →
  `native-assets.json` (`schemaVersion:1`); in `index.html` and `scanner/index.html` rewrite every
  same-origin `src`/`href` that is in the manifest to carry `native=<sha256>` (replacing old values);
  re-insert the `#inhouse-native-assets` JSON map before the boot script in `index.html`.
- CI requires these outputs to be reproducible (`git diff --exit-code` on wasm, offline-assets.js,
  native-assets.json, index.html, scanner/index.html, ui.css — `production-checks.yml:33-38`).
- Android serving: `install_native_assets.py` copies every manifest file into
  `assets/inhouse-native/` after verifying size+sha256 (`'Stale native asset manifest: <path>; run npm
  run build:offline'`) and attaches `InhouseLocalAssets`; it intercepts non-main-frame GETs to
  `https://inhousenotes.com` whose `native` query equals the manifest digest and serves the APK copy with
  `Cache-Control: public, max-age=31536000, immutable` and `X-Inhouse-Asset-Source: apk`; anything else
  goes to the network (`.github/android/performance/InhouseLocalAssets.java:46-65`). A newer web release
  therefore reuses unchanged APK bytes and fetches changed ones.

### 16.3 GitHub Pages (`.github/workflows/deploy-pages.yml`)
On push to `main` or manual; `contents: write`; concurrency `pages-production`, no cancel; rsync the
repo into `$RUNNER_TEMP/pages-site` excluding `.git/ .github/ .gh-config/ 'android app/' docs/
node_modules/ rebuild-v2/ rust-core/ scripts/ test-results/ tests/ tmp/ *.apk gh-*.tmp AGENTS.md .claude/
CLAUDE.md package.json package-lock.json playwright.config.mjs README.md RELEASE_NOTES_*.md`; add
`.nojekyll`; assert `index.html, CNAME, app-v5.js, bullet-template.js, scanner/script.js` and four
scanner processing files exist; publish with `peaceiris/actions-gh-pages@v4` to `gh-pages`
(`force_orphan: true`); then `verify_live_release.py`: poll `https://inhousenotes.com/` (no-cache
headers, `?verify_release=<sha>&attempt=n`) up to 180 s until it serves the same `APP_VERSION` and
byte-identical copies of the listed runtime files, service worker, A/B catalogs, wasm and scanner files.
`CNAME` = `inhousenotes.com`. Root `.nojekyll` exists.

### 16.4 Production checks (`.github/workflows/production-checks.yml`)
PRs and pushes to `main`: Node 22, `npm ci`, Rust 1.90.0 + wasm target, `build:rust` + `build:offline`
reproducibility diff, Playwright Chromium, `test:unit`, `test:e2e`, then `rebuild-v2` `npm ci`/`npm
test`/`npm run build`; upload Playwright report on failure.

### 16.5 Android APK (`.github/workflows/build-android.yml`, manual only)
1. Checkout `github.sha` (full history), Node 22, Java 21 (temurin), Python 3.12, `python3-tk` + pillow.
2. `python -m unittest … test_android_refresh_rate.py`.
3. Restore keystore from `INHOUSE_ANDROID_KEYSTORE_BASE64`; build with `build_android_apk.py`.
4. Verify `inhouse-notes-release-v1.0.28.apk`; download previous `android-v1.0.27` and run
   `verify_release_apk.py` (same signing certificate; badging `com.local.inhousenotes` 29/1.0.28;
   permissions `REQUEST_INSTALL_PACKAGES`, `CAMERA`; camera feature not required; hardware acceleration;
   `InstallActivity`, `InstallReceiver`, `UpdateFileProvider` registered; dex symbols incl.
   `InhouseSyncWorker`, `InhouseBackgroundSync`, `InhouseLocalAssets`; >200 native assets with exact bytes).
5. Upload artifact; require a successful `android-session.yml` run of the updater revision in the lock.
6. Create prerelease `android-v1.0.28` targeting `GITHUB_SHA` (fails with a Spanish message if it already
   exists), title `Inhouse Notes Android 1.0.28`, `--latest=false`, notes text from the workflow.
7. Download the public APK, `cmp` and re-verify; then `update_services_manifest.py` (writes
   `android-update.json` and calls canonical `catalog.py write_release`, default channel A,
   `--user-requested-b` for B) and upload catalogs as an artifact (committed with the next web version).

`build_android_apk.py`: builder object from `android app/html_to_apk_builder.py`; copy only
`.github/android/app-loader.html` as `www/index.html`; Capacitor project (`appId com.local.inhousenotes`,
`androidScheme https`, `allowNavigation` inhousenotes.com, `*.inhousenotes.com`, accounts.google.com,
`*.google.com`, `*.googleusercontent.com`, `*.gstatic.com`); `cap add/sync android`; patches: manifest
(permissions, optional camera features, `hardwareAccelerated`, `androidx` FileProvider `.fileprovider`
with `file_paths.xml` `exports/` and `updates/`, intent filters `inhousenotes://oauth2callback` and
scheme `com.local.inhousenotes`), MainActivity (UA = default minus `; wv` and `Version/4.0 ` plus
` InhouseNotesApp/<versionName>`, DOM storage, cookies incl. third-party, JS bridge `InhouseNative`
with `getPdfExportApiVersion()=2`, `getAppVersion`, `installAppUpdate`, `openAuthUrl`, `savePdf`,
`beginPdfExport/appendPdfExportChunk/finishPdfExport`, OAuth intent handling), display refresh-rate
policy (≤120 Hz, same resolution), startup theme colours, gradle versions, androidx browser/core deps,
background sync (WorkManager 2.10.1, webkit 1.12.1), native assets, launcher icon
`android app/Inhouse Notes Logo.png`, signing; run frozen `verify.py` and `register_android.py`
(`trust_read=False`); `gradlew assembleRelease`.
`app-loader.html`: themed launch mark (`#f5f5f0`/`#151515`), two rAFs then
`location.replace("https://inhousenotes.com/?inhouse_app=1")` — **the APK always runs the live web
app**, so v6 ships to existing APKs through Pages and must keep every native contract in §4.3, §8.5,
§14.6 and the PDF export bridge (`handleInhousePdfExportResult`, chunk size 240000, `app:35200-35284`).

---

## 17. Test surface

### 17.1 Harness
`playwright.config.mjs`: `tests/e2e/*.spec.mjs`, Chromium desktop, 30 s timeout, expect 8 s, serial,
CI retries 1 / workers 1, `baseURL http://127.0.0.1:4173`, server `node tests/e2e/server.mjs`
(static root, `Cache-Control: no-store`, `/` → `/index.html`, path traversal 403, `/__slow?delay=ms`
JSON after ≤10 s). Specs load `/?e2e=1` (plus `inhouse_app=1` for updater tests) and stub Google/GitHub
with `page.route`.

### 17.2 `window.__IHN_TEST_API__` (`app:35286-35793`)
Installed only on localhost with `?e2e=1`; `Object.freeze`d; methods use `this` (call as
`__IHN_TEST_API__.m()`). v6 must provide the same names, arguments and return shapes.

| Method | Behaviour / return |
|---|---|
| `ready()` | Promise of the app initialisation (`init()` or `worker()`). |
| `seedDensePageForTest(count=2400, points=100)` | Reset to 1 page "Dense handwriting", fill page 0 with `count` pen strokes (`dense-<i>`, `#123456`, width 1.5, `points` points in a 30-column grid), edit mode, editor, render. |
| `saveDensePageForTest()` | Mark page 0 dirty (full) and return `savePageToIndexedDb(0, page)`. |
| `redrawDensePageForTest()` | Redraw page 0 synchronously; returns elapsed ms. |
| `prepareEraserFixtureForTest(tool, count=260, points=15)` | Dense seed, set tool, eraser width 16, add stroke `erase-target` (black, width 6, y=203, x 100–200), redraw. |
| `eraserPixelsForTest()` | `{center: RGBA at (153,203), edge: RGBA at (110,203), penActive}` from page-0 canvas at render scale. |
| `receiveErasePreviewForTest()` | Prepare live doc and feed a remote `live-erase` packet removing `erase-target` (`webrtc`). |
| `densePaintStatusForTest()` | `{pending repaint?, stroke count, RGBA at (20,20)}`. |
| `compareWorkerInkForTest(strokes)` | Max per-channel difference between main-thread `drawSmoothStroke` and worker `renderOverlay` PNG on 128×128. |
| `resetLocalDocument(pageCount=2, name='E2E document')` | Await ready; reset PDF state; no Drive file; name ≤120 chars; 1–12 blank pages; clear history/dirty maps; mark all dirty; return `snapshot()`. |
| `showEditorForTest()` | Edit mode, editor view, render pages and pages list; `snapshot()`. |
| `addScannedPagesForTest(pageCount=2)` | Insert 1–4 synthetic 24×34 JPEG A4 scanned pages after the active page; `{inserted, snapshot}`. |
| `openScannerForTest()` / `closeScannerForTest()` | Open embedded scanner → `{visible, src}`; force close → `true` if hidden. |
| `addSyntheticStroke(pageIndex=0, marker='stroke')` | Append a 2-point pen stroke id `e2e-<marker≤48>-<now>`; mark dirty; return id. |
| `prepareLiveDocument(fileId='e2e-live-document')` | Pretend a Drive file (editable, no autosave), ensure page ids; `snapshot()`. |
| `receiveRemoteLiveStrokePackets(packets)` | Feed packets as `'tab'` transport, flush commits; `snapshot()`. |
| `mergeRemotePagesForTest(pages)` | `applyRemotePages` with additive merge using local structure/fields; `{result, snapshot}`. |
| `checkpointBeforeLeaving()` | `persistExitLocalCheckpoint(null)`; `{elapsedMs, snapshot}`. |
| `loadPage(pageIndex=0)` | Force-load page data; `snapshot()`. |
| `snapshot()` | `{exportName, pages: sanitized pages, timelineLength, dirtyPages:[…]}`. |
| `formatTimeForTest(ts)` | `formatTime(ts)`. |
| `localSaveStatusForTest()` | `getLocalSaveStatus()`. |
| `importPdfBlobForTest(bytesArray)` | Import through `importPDFData` (real path), edit mode, editor, render; `snapshot()`. |
| `seedLiveReplicaForTest(marker)` | 3 pages `shared-page-<i>`, 32 base strokes + one marker stroke each; marker `first` deletes page 1's first stroke with tombstone; reset structure/fields. |
| `checkpointLiveReplicaForTest()` | Load all pages; snapshot + `contentVersion`, `cachedLiveHash`, canonical `hash`. |
| `pauseLiveConnectionForTest()` | Stop live collaboration and drop the token. |
| `deleteLiveStrokeForTest(index, id)` | Tombstone and remove a stroke. |
| `startLiveConnectionForTest(email)` | Live doc `e2e-live-handshake`, token `e2e-live-token` (+10 min), profile `{email,name:email}`, edit mode, start live collaboration. |
| `liveConnectionOverviewForTest()` | Live overview + per-peer RTC diagnostics (states, hashes, SDP m-lines, candidate counts). |
| `congestBulkAndPingForTest()` | Delay the bulk channel 2 s and send a health ping over the interactive channel. |
| `dropNextLiveSnapshotForTest()` | Swallow the next chunked snapshot transfer. |
| `publishPreviewForTest(strokeId, pageId, count=2)` / `previewPointCountForTest(strokeId)` | Publish a live stroke preview / count received preview points. |
| `interactionDuringPreparationForTest()` | Simulate a background PDF build, call `pauseSavesDuringInteraction()`, restore; `{aborted}`. |
| `openTimelineForTest()` / `hasPendingTimelineRestoreForTest()` | Open timeline and await restore → `{pending, visible}` / pending flag. |
| `startDeferredPageMigrationForTest(pageCount=4)` / `deferredMigrationStateForTest()` | Start a legacy migration with deferred writes (2–8 pages) / `{active, pendingPages}`. |
| `noteUserActivityForTest()` | `noteRecentUserActivity()`. |
| `primePdfPageForTest(bytesArray, pageSize={width:200,height:300})` | One PDF-background page backed by the given clean original (+ live pdf.js doc) and one stroke; `snapshot()`. |
| `buildPdfBlobForTest(options)` | Build the export PDF (`status:false`; `milestone`, `queued`, `includeBytes` options) → `{ok, size, hasLegacyBakedOverlay, legacyCoverPageCount, hasCleanOriginalAttachment, bytes?, history}` or `{ok:false, error, …}`. |
| `uploadBlobResumableForTest(byteLength=614400, simulateActiveSave=false)` | Resumable upload of a `%`-filled blob to the Drive upload URL with token `e2e-resumable-upload-token`, key `e2e-resumable-upload`. |

### 17.3 Implicit global test surface
Specs also call and reassign app globals inside `page.evaluate` (classic-script scope). Computed by
intersecting app top-level declarations with spec identifiers (may contain a few false positives).
- **Reassigned** (monkeypatched) by specs: `ensureDriveToken`, `requestDriveToken`, `driveFetch`,
  `driveIsOffline`, `detectDeviceType`, `openDriveFile`, `refreshDriveFiles`, `buildPdfBlob`,
  `prepareDrivePdfBlob`, `queueSave`, `queueDriveSave`, `scheduleDriveSync`, `pullRemoteChanges`,
  `applyRemotePages`, `updateDriveFile`, `ensureDriveFile`, `ensureNamedDriveFolder`,
  `findDrivePdfInFolder`, `savePageToIndexedDb`, `saveToIndexedDb`, `renderPdfPageToBackground`,
  `renderPagePreview`, `startCollabPolling`, `waitForCollabPullIdle`, `yieldProcessingForUi`, and
  state variables `driveAccessToken`, `driveTokenExpiry`, `driveSignedOut`, `driveAuthNeedsReconnect`,
  `driveUserProfile`, `driveDirty`, `driveSaveInProgress`, `driveSavePhase`, `driveContentVersion`,
  `driveUploadedContentVersion`, `documentSessionId`, `documentSessionActivated`, `isPenWriting`,
  `lastUserInteractionAt`, `preparedDrivePdf*`, `activeDriveSaveController`, `saveQueued`, … (e.g.
  `tests/e2e/offline-worker.spec.mjs:155` replaces `ensureDriveToken`).
- **Called** shell functions: `showDriveHome`, `stopDriveHomeAutoRefresh`, `restoreCachedDriveProfile`,
  `signInDrive`, `setReadOnlyMode`, `formatTime`, `isDocumentEditorActive`, `isDocumentLiveWorkPaused`,
  `markDriveAuthNeedsReconnect`, `refreshSaveIndicatorState`, `getSavePresentation`,
  `wakeDocumentBackgroundWork`, `waitForHotInteractionToSettle`, `beginDocumentSession`,
  `getDocumentSessionToken`, `init`, … plus module globals `InhouseDriveInk`, `InhouseOfflineLibrary`,
  `InhouseOfflineStore`, `InhouseRust`, `InhouseDriveInkJournal`, `InhouseDocumentProcessing`,
  `InhouseOfflineSync`, `InhouseDriveLivePreview`, `InhouseOfflineApp`, `InhouseUpdater`,
  `InhouseRuntimeCore`, `InhouseOfflineMedia`, `InhouseSecurityCore`, `InhouseInkPalette`,
  `InhouseBackground`, `InhouseInkDefaults`, `InhouseBulletTemplate`.
v6 options: keep a JS compatibility facade exposing these names (writable where reassigned), or port the
affected specs to an extended `__IHN_TEST_API__`. Either way the behaviours they pin must hold.

---

## 18. Tests pinning shell behaviour (one line each)

Static/unit (`npm run test:unit`):
- `tests/release-v5-smoke.mjs:40` — `index.html` contains `const APP_VERSION = '5.17.22';`.
- `tests/release-v5-smoke.mjs:62` — app derives `APP_VERSION` from the index block.
- `tests/release-v5-smoke.mjs:63` — `data-app-version` appears exactly 3 times (2 labels + binding).
- `tests/release-v5-smoke.mjs:64-67` — collaboration/live/app/timeline scripts carry `?v=5.17.22`.
- `tests/release-v5-smoke.mjs:68-70` — CSP present with `script-src-attr 'none'` and `frame-src 'self'`.
- `tests/release-v5-smoke.mjs:141-152` — Pages workflow name, permissions, concurrency, gh-pages action, `*.apk` exclusion, publish dir.
- `tests/release-v5-smoke.mjs:153` — exactly three SRI `sha384` integrity attributes.
- `tests/release-v5-smoke.mjs:180-183` — PDF libraries load after `</style>`.
- `tests/release-v5-smoke.mjs:184` — boot sets `documentElement.dataset.theme`.
- `tests/release-v5-smoke.mjs:185-190` — `showFastInitialView` exists; `initDriveAuth()` precedes `loadFromStorage` in `init`.
- `tests/release-v5-smoke.mjs:191-196` — pull-to-refresh is not width-gated and checks `navigator.maxTouchPoints`.
- `tests/release-v5-smoke.mjs:197-204` — builder has startup theme, `setBackgroundColor(bootColor)`, local launch shell, no remote `server.url`.
- `tests/release-v5-smoke.mjs:205-221` — Android 1.0.28/29 pins and workflow step ordering (verify → publish APK → verify public → catalog).
- `tests/release-v5-smoke.mjs:222-223` — app-loader full-screen centred launch CSS.
- `tests/release-v5-smoke.mjs:231-232` — app calls `InhouseServicesUpdater.create`; index loads updater.js.
- `tests/release-v5-smoke.mjs:236-240` — native update progress payload fields.
- `tests/release-v5-smoke.mjs:277-303` — share link role/mode/name/resourcekey, editor-link gate, no "Sign in to open shared document.".
- `tests/release-v5-smoke.mjs:448-458` — Home save transition overlay and goHome never awaits Drive auth/uploads.
- `tests/release-v5-smoke.mjs:461-470` — Home/Welcome call `closeDocumentScopedWork`; background suspension wiring.
- `tests/release-v5-smoke.mjs:509-512` — goHome starts the lifecycle exit upload; `beforeunload` sets `returnValue=''`.
- `tests/release-v5-smoke.mjs:520-540` — public preview / sign-in gate / read-only strings.
- `tests/release-v5-smoke.mjs:541` — no `setTimeout(initDriveAuth, 200)` polling.
- `tests/release-v5-smoke.mjs:550-572` — the only inline script is the inert version block; all runtime scripts parse.
- `tests/release-v5-smoke.mjs:1110-1118` — `ensureDriveToken` de-duplication and generation checks; token endpoint via `fetchWithDeadline`.
- `tests/release-v5-smoke.mjs:1195-1203` — `android-update.json` 1.0.28/29 matches `updates/A.json` latest; release notes header equals version.
- `tests/release-v5-smoke.mjs:1241-1252` — release watcher fetch/compare/once-per-version; SW `skipWaiting`, HTML cached-first with `no-cache` revalidation; fonts CSS linked.
- `tests/release-v5-smoke.mjs:1262-1266` — oauth-callback Read origin, opener checks and PKCE exchange.
- `tests/native-assets.test.mjs:8` — manifest, inline map and every `native=` URL match the exact bytes.
- `tests/native-assets.test.mjs:26` — Android installer bundles verified files and rejects stale bytes.
- `tests/runtime-core-v5.test.mjs:8,24,43,48` — `fetchWithDeadline` timeout/cancel semantics, `settleWithTimeout`, `runWithRetries`.
- `tests/test_android_refresh_rate.py:69,102,111` — refresh-rate policy selection, compile against Android API, hosts activate it.

E2E (`npm run test:e2e`):
- `tests/e2e/app-resilience.spec.mjs:5` — boots under CSP with no violations/errors, Welcome visible, both labels `v<package.json version>`, PDF libs loaded.
- `tests/e2e/app-resilience.spec.mjs:872` — blocked IndexedDB cannot trap startup (Welcome still shown).
- `tests/e2e/app-resilience.spec.mjs:1029` — calendar HTML sanitiser and PDF source hardening fail closed.
- `tests/e2e/app-resilience.spec.mjs:1084` — `fetchWithDeadline` bounds a slow request (<2 s for 1 s deadline).
- `tests/e2e/app-resilience.spec.mjs:242,292` — embedded scanner opens below Photo / directly into live camera.
- `tests/e2e/release-check.spec.mjs:12` — newer published release reloads exactly once.
- `tests/e2e/release-check.spec.mjs:23` — same published release never reloads.
- `tests/e2e/release-check.spec.mjs:37` — an open editor is never reloaded.
- `tests/e2e/release-check.spec.mjs:63` — cached shell opens before network revalidation; mismatched offline copy rejected.
- `tests/e2e/release-check.spec.mjs:95` — release check escapes an old SW shell exactly once.
- `tests/e2e/offline-library.spec.mjs:7` — expired remembered session paints Drive Home (boot destination `drive`), never the sign-in screen.
- `tests/e2e/offline-library-cached.spec.mjs:27,81` — offline Home keeps avatar and fonts; cached Home paints while auth/listing stall.
- `tests/e2e/services-updater.spec.mjs:3` — offer shows `1.0.18 → 1.0.19`, Update calls `installInhouseUpdate(url, sha)`, progress 50 %.
- `tests/e2e/services-updater.spec.mjs:28` — legacy A state migrates to B; A needs acknowledgement + slide; Later snoozes 14 days; empty B reports "No releases on version B."; snooze preserved.
- `tests/e2e/services-updater.spec.mjs:119` — cancelled native install read on focus enables "Try again"; card fits 360 px.
- `tests/e2e/services-updater.spec.mjs:161` — restart reports persisted "Update installed" once; settings show new version.
- `tests/e2e/services-updater.spec.mjs:194` — Update stays in the native bridge, no navigation to GitHub.
- `tests/e2e/offline-worker.spec.mjs:2` — background processor (8 modes incl. 401 and cold) merges and uploads.
- `tests/e2e/offline-worker.spec.mjs:149` — `backgroundAccess` rejects account changes during pending auth.
- `tests/e2e/document-lifecycle.spec.mjs:79` — Home stops every document-scoped loop and nothing restarts it.
- `tests/e2e/document-lifecycle.spec.mjs:113` — reopening from Home restarts collaboration.
- `tests/e2e/document-lifecycle.spec.mjs:127` — a document left mid-save still uploads from Home.
- `tests/e2e/document-lifecycle.spec.mjs:149` — hidden editor pauses polling/calendar sync and resumes.
- `tests/e2e/document-lifecycle.spec.mjs:182` — short app switch keeps P2P and ink journal warm.
- `tests/e2e/document-lifecycle.spec.mjs:193` — long background suspends live work; return restores it.
- `tests/e2e/document-lifecycle.spec.mjs:226` — Home cancels a pending suspension; reopening is fully live.
- `tests/e2e/home-save-navigation.spec.mjs:44,59` — Home never waits for stalled auth/uploads; a failed checkpoint keeps the document open.
- `tests/e2e/idle-animations.spec.mjs:24,60` — no idle animations on Home; spinners/placeholders stop by themselves.
- `tests/e2e/save-timestamp-format.spec.mjs:10,25,43` — `formatTime` minute cache, boundary and `N min` range.
- `tests/e2e/fast-scroll.spec.mjs:6,19` — fast-scroll thumb drives editor/Home, keyboard, hides when content fits.
- `tests/e2e/save-status.spec.mjs:135` — local save failure retried before reconnecting Drive ("Sign in needed").

Build-time guards: `vendor/inhouse-services-updater/verify.py` (frozen installer hashes, run by
`build_android_apk.py:92`), `production-checks.yml:33-38` (reproducible assets), `deploy-pages.yml:59-67`
(required files), `verify_live_release.py` (public bytes).

---

## 19. Quirks to preserve or consciously retire in v6
1. `#status-bar` exists but never becomes visible; no toast UI exists (§10).
2. Runtime theme toggle does not update `meta[name=theme-color]` / `html` inline background (§9).
3. `notebook-view-mode-v1` is written but not used for routing; cold boot always lands on Home (§7.2).
4. APP download link pinned to Android 1.0.11 (§10).
5. Updater settings button label is Spanish (`Actualizaciones · A/B`), install error Spanish (`Abre la
   app Android para instalar.`), close button `aria-label="Cerrar"`, presence popup title Spanish
   (`Actividad del documento`); the rest of the UI is English.
6. `installedVersion: '1.0.2'` fallback in the updater config.
7. PKCE `state` not validated; oauth-callback Notes branch posts a code nobody consumes.
8. `?offline-worker` is detected by presence, not value; the Android background UA
   `InhouseNotesApp/Background` does not match the app-UA regex (mode comes from `inhouse_app=1`).
9. Every session starts in Read mode (`setReadOnlyMode(true,{force:true})`, `app:2288`).
