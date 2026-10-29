# Home, Welcome & Drive document management — behavioural spec (v5.17.22 → v6.0.0)

Scope: everything a user sees and does **outside the editor canvas**: boot destination, Welcome/sign-in view,
Drive Home (tabs, lists, cards, menus, folders, folder zoom, pull-to-refresh, auto-refresh, search, thumbnails,
offline badges), profile menus (`[data-app-version]`), Drive dialogs (create menu, folder picker, new folder,
trash, bin, download-app, offline documents), Drive file open, quick-access Diary/Agenda, shared/public links,
the Share modal and its permission API. Auth token mechanics are only described where Home depends on them.

Sources (all paths relative to repo root; `app` = `app-v5.js`, `html` = `index.html`):
`app` §4.15/4.15b/4.16/4.16b/4.21a/4.26/4.27, `boot-v5.js`, `offline-library-v5.js` (`lib`),
`offline-app-v5.js` (`oapp`), `offline-media-v5.js`, `offline-store-v5.js`, `offline-v5.css`,
`rust-core/src/folder.rs`, `rust-core-v5.js`.

Conventions: “must” = observable behaviour to reproduce exactly. “Quirk” = current behaviour that looks
unintended but is what v5.17.22 does; v6 must reproduce it unless the lead decides otherwise (all quirks are
collected in §12).

---------------------------------------------------------------------------------------------------------

## 1. Top-level views and boot

Three mutually exclusive top-level views, all present in the static markup:

| View | Element | Visible when |
|---|---|---|
| Welcome | `#welcome-view` (html:9234) | not signed in / no recoverable session |
| Drive Home | `#drive-home.hidden[data-home-tab="home"]` (html:9263) | signed in or recoverable |
| Editor | `#editor-view.hidden` (html:9521) | a document is open (owned by the editor spec) |

`.hidden { display:none !important }` (html:157) is the universal hide class.

### 1.1 First paint (`boot-v5.js`, runs synchronously in `<head>`, html:95)
1. `applySavedTheme()` (boot:12): `localStorage['inhouse-theme']` = `'dark'|'light'`, else
   `prefers-color-scheme`. Sets `html[data-theme]`, `html.style.backgroundColor` (`#151515` dark /
   `#f5f5f0` light) and `<meta id="app-theme-color" content>` to the same colour.
2. `handleOAuthCallback()` (boot:28): only when `?oauth_callback=1` (Android implicit-flow bounce page).
3. `selectBootDestination()` (boot:47): `window.__INHOUSE_BOOT_SIGNED_IN__ = hasToken || hasRefresh || returning`
   where
   - `hasToken`: `localStorage['drive-token-v1']` JSON `{token,expiry,scopeVersion,grantedScopes}` with
     `localStorage['google-scope-version-v1']==='drive-calendar-v2'`, `scopeVersion==='drive-calendar-v2'`,
     `grantedScopes` containing `https://www.googleapis.com/auth/calendar.events`, `now < expiry-60000`;
   - `hasRefresh`: current scope version + calendar scope in `google-granted-scopes-v1` + `drive-refresh-token-v1`;
   - `returning`: `drive-remember-v1==='1'` **or** any `drive-hint-email`.
   Sets `html[data-boot-destination]` = `'drive'|'welcome'`.
4. `revealInitialView()` (boot:76), run as soon as `#drive-folder-files` exists in the DOM (MutationObserver)
   or at DOMContentLoaded:
   - Welcome gets `.fonts-ready` once `document.fonts.load('600 2.25rem Comfortaa')` settles, or after 3000 ms
     (until then `.welcome-logo .brand-name { visibility:hidden }`, html:4398-4404).
   - If signed-in-at-boot: hide welcome, show `#drive-home`, pre-fill skeletons: 6 card skeletons in
     `#drive-recents`, `#drive-starred`, `#drive-shared`; 4 folder skeletons in `#drive-folders`; 8 card
     skeletons in `#drive-folder-files`; `#drive-status` gets
     `<span class="drive-status-skeleton" aria-hidden="true">` and `aria-label="Loading your Drive"`.
     Skeleton markup (identical to app’s builders):
     `<div class="drive-card-skeleton" aria-hidden="true"><div class="skeleton-bar title"></div><div class="skeleton-preview"></div></div>`,
     `<div class="drive-folder-card-skeleton" aria-hidden="true"><div class="skeleton-icon"></div><div class="skeleton-bar"></div></div>`.
5. `offline-v5.css:1-2`: while `html[data-boot-destination="drive"]:not([data-app-ready="true"])`,
   `#welcome-view` is `display:none !important` and `#drive-home` is `display:flex !important`.
6. Published-release check (boot:130) — see release spec; it never reloads while `#editor-view` is visible.

### 1.2 App init (`init()`, app:2255; `showFastInitialView()`, app:2222)
Order (pinned by release smoke test: `initDriveAuth()` must precede `await Promise.all([loadFromStorage(), offlineLibraryReady])`):
1. `loadViewState()`, `initTheme()`, `loadDriveSession()` (app:22049), `restoreCachedDriveProfile()` (app:6993).
2. `showFastInitialView()`: parses share params (see §8.1). If a `?drive=` link is present → editor view if a
   token is already in memory, else Drive Home. Else if token or stored session (`drive-token-v1` valid,
   any `drive-refresh-token-v1`, or `__INHOUSE_BOOT_SIGNED_IN__===true`) → Drive Home; else Welcome.
3. `InhouseOfflineApp.initialize()` (async), `initDriveAuth()`, `setupMobileHomePullToRefresh()`,
   then `html[data-app-ready="true"]`.
4. After storage + offline library load: `setupEventListeners()` (restores Home tab, §3.5), share params
   re-parsed, then the same decision again with `isSignedInAtBoot` additionally true when the offline
   library is ready. **A cold boot never lands in the editor**: signed-in → Drive Home always
   (`loadViewMode()` is read into `lastView` but unused — quirk).
5. `#btn-welcome-login` click → `signInDrive()`; `installWelcomeSheet()`.
6. If a share link is pending and there is no token → `tryOpenPendingSharedWithoutSignIn()` (§8.3).

### 1.3 View switching (app §4.26)
- `showDriveHome()` (app:6119): `closeScannerEditor({force:true})`; hide welcome, show `#drive-home`, hide editor;
  close timeline panel (`#timeline-panel` remove `.visible`, `aria-hidden=true`), save-debug popup, presence
  popup, both profile menus; `clearPublicInlinePreview()`; `clearSharedEditorSignInGate()`;
  `closeDocumentScopedWork()` (stops collab polling, side-panel auto-sync, background-suspend timer, sets
  `documentClosedToHome = !!state.driveFileId`, suspends ink session); `clearDriveSyncTimers()`;
  `saveViewMode('drive')`; `startDriveHomeAutoRefresh()`; `updateCalendarFeatureAvailability()`.
  The closed document stays in memory (exit upload may still finish from it) but nothing document-scoped runs.
- `showWelcomeView()` (app:6172): show welcome, re-hide `#welcome-sheet` and its backdrop, hide Home and editor,
  close the same overlays, `closeDocumentScopedWork()`, `clearDriveSyncTimers()`, `stopDriveHomeAutoRefresh()`.
- `showEditorView()` (app:6237): hide welcome/Home, show editor, `documentClosedToHome=false`,
  `restoreDocumentInkSession()`, `stopDriveHomeAutoRefresh()`, restart side-panel auto-sync if visible,
  next frame `clampToolbarPosition()` + visible-page update, `saveViewMode('editor')`.
- `updateDriveAuthUI()` (app:6605) also switches: if connected/recoverable and Welcome is visible →
  `showDriveHome()`.

### 1.4 Auth-dependent Home state (`updateDriveAuthUI`, app:6605)
Inputs: `signedIn = !!driveAccessToken`; `reconnecting = driveAuthNeedsReconnect && !driveSignedOut`;
`localLibrary = InhouseOfflineLibrary.ready()`; `connectedOrRecoverable = signedIn || reconnecting ||
localLibrary || __INHOUSE_BOOT_SIGNED_IN__`; `hasPendingShared = !!window.pendingSharedDriveId`.
No-op in the hidden offline worker (`?offline-worker`).

| Control | Rule |
|---|---|
| `#btn-drive-login` | `display:inline-flex` if `!connectedOrRecoverable || (reconnecting && !offline)`, else `none`; text `Reconnect` if reconnecting else `Sign in` |
| `#drive-profile`, `#drive-profile-editor` | `.hidden` unless `connectedOrRecoverable` |
| `#drive-search`, `#btn-drive-new`, `#btn-drive-new-folder` | `disabled = !signedIn && !localLibrary` |
| `#btn-drive-bin` | `disabled = !signedIn` |
| `#btn-drive-back` | `disabled = (!signedIn && !localLibrary) || driveFolderStack.length <= 1` (button is `display:none` by CSS anyway) |
| `#drive-status` text | `Offline · changes stay on this device` (library && offline) → `Drive connection paused. Sign in again to continue syncing.` (reconnecting) → `Loading your PDFs...` (signedIn) → pending share: `Opening public preview...` if `sharedLinkSignInPrompted` else `Opening shared document...` → `Connecting to Drive…` (library) → `Sign in to browse your Drive.` |

If not connected: hide the three empty-state messages, close profile menus, and if a share is pending:
show the editor (`sharedEditorSignInGateActive`) + gate UI, else `showDriveHome()` +
`tryOpenPendingSharedWithoutSignIn()`. If connected and `state.driveAutosave && !isReadOnly && driveDirty`:
schedule a Drive sync after 500 ms. If signed in: `sharedLinkSignInPrompted=false`, `loadDriveProfile()`,
`restorePdfBackgroundsIfNeeded()`, `setTimeout(resumePendingExitUploadIfNeeded,0)`, start collab polling
if a doc id exists, and open a pending share via the API (§8.2).
`driveIsOffline()` = `InhouseOfflineLibrary.offline() || navigator.onLine === false` (app:6603).

---------------------------------------------------------------------------------------------------------

## 2. Welcome view

### 2.1 Markup (html:9234-9260)
```
#welcome-view
  .welcome-art > img[src=icons/welcome-pages.svg][alt=""][width=360][height=330][decoding=async]
  .welcome-panel
    .welcome-copy
      .welcome-logo > svg[viewBox="34 30 40 48"](logo, 3 paths) + span.brand-name "inhouse notes"
      h1.welcome-title  "Write by hand.<br>Find it anytime."
      p.welcome-lead    "Real notebooks in your own handwriting, saved to your Google Drive and ready offline."
    button.welcome-google#btn-welcome-start[type=button][aria-haspopup=dialog][aria-controls=welcome-sheet]
        svg.welcome-g (4-colour G, viewBox 0 0 48 48) + span "Continue with Google"
  .welcome-sheet-backdrop#welcome-sheet-backdrop[hidden]
  .welcome-sheet#welcome-sheet[role=dialog][aria-modal=true][aria-labelledby=welcome-sheet-title][hidden]
    .welcome-sheet-grab[aria-hidden]
    h2#welcome-sheet-title "When you sign in"
    ul.welcome-sheet-list > 3× li: span.welcome-sheet-icon>svg + span(strong + span)
       1 cloud-check icon: "Your notebooks are PDFs in your Google Drive" / "Open them with any PDF app, or move them like any other file."
       2 devices icon:     "The same notebooks on your phone and computer" / "Changes sync on their own."
       3 sign-out icon:    "You can sign out anytime" / "Your notebooks stay in your Drive."
    p.welcome-sheet-note "Google will ask for access to Drive and Calendar. Calendar is only used for the events on your pages."
    button.welcome-google#btn-welcome-login "Continue with Google"
```
Logo SVG (used identically in welcome, Home topbar, editor header, loading overlay):
`<path d="M42 30 H61 L74 43 V70 A8 8 0 0 1 66 78 H42 A8 8 0 0 1 34 70 V38 A8 8 0 0 1 42 30 Z" fill="#FF7F1F"/>`,
`<path d="M61 30 V39 Q61 43 65 43 H74 Z" fill="#FFB984"/>`,
`<path d="M45 62.5 L54 53.5 L63 62.5" stroke="#FFFFFF" stroke-width="5.2" stroke-linecap="round" stroke-linejoin="round"/>`.
G logo paths: `#EA4335`, `#4285F4`, `#FBBC05`, `#34A853` (html:9249, copy verbatim).

### 2.2 Behaviour (`installWelcomeSheet`, app:6150)
- `#btn-welcome-start` click → `sheet.hidden=false`, `backdrop.hidden=false`, focus `#btn-welcome-login`.
- Backdrop click or `Escape` inside the sheet → hide both, focus `#btn-welcome-start`.
- `#btn-welcome-login` click → `signInDrive()` (app:6881): web → `ensureDriveToken(true)` (GIS popup,
  `prompt:'consent'`) then `refreshDriveFiles()`; failure → `#drive-status` = `Sign-in failed. Try again.`
  Android app → redirect/PKCE flow (auth spec).

### 2.3 Styling (html:4340-4576, verbatim)
Fixed full-screen, `z-index:1300`, column flex, padding `max(24px,safe-top) 28px max(32px,safe-bottom)`,
`overflow-y:auto`. Art max 360px/42vh. Title `clamp(1.85rem,7.4vw,2.4rem)`/700/1.1/-0.02em. Google button:
max 420px, min-height 54px, radius 14px, `#1a1a1a` bg / white text (inverted in dark), G in 26px white circle.
Sheet: fixed bottom, `width:min(520px,100%)`, radius `22px 22px 0 0`, backdrop `rgba(20,14,8,.42)`;
grab bar 40×4. ≥900px: row layout, column-gap 64px, art 420px, panel 400px. No open animation (instant).

---------------------------------------------------------------------------------------------------------

## 3. Drive Home

### 3.1 DOM tree (html:9263-9518)
```
#drive-home.hidden[data-home-tab="home"]            (fixed inset 0, z 1200, column flex, overflow hidden)
  .drive-topbar
    .drive-brand > svg(logo) + span.brand-name "inhouse notes"
    .drive-actions
      button.btn.btn-secondary.btn-icon-square#btn-drive-fullscreen[title/aria-label=Fullscreen]  (removed in Android app)
      button.btn.btn-primary#btn-drive-new[aria-label="Add file"][aria-haspopup=menu][aria-expanded=false][aria-controls=drive-create-menu] > svg(+)
      button.btn.btn-secondary.btn-download-app#btn-download-app > svg(download) + span "APP"   (hidden in app)
      button.btn.btn-primary#btn-drive-login "Sign in"
      .drive-profile.hidden#drive-profile
        button.drive-profile-btn#btn-drive-profile[aria-label="Account menu"][aria-haspopup=menu][aria-expanded=false] > img.drive-profile-avatar#drive-profile-avatar
        .drive-profile-menu.hidden#drive-profile-menu[role=menu]   (see §3.3)
  .home-pull-refresh#home-pull-refresh[role=status][aria-label="Pull to refresh"] > span.home-pull-refresh-surface > span.home-pull-refresh-spinner
  .drive-main
    .drive-toolbar
      button#btn-drive-bin.btn.btn-secondary.btn-icon-square[title/aria-label=Bin] > svg(trash)
      input.drive-search#drive-search[type=text][placeholder="Search PDFs..."][autocomplete=off]
      .drive-status#drive-status "Sign in to browse your Drive."
    section.drive-section[data-home-section="home"]          Recents: h2 "Recents" + .drive-recents#drive-recents
    section.drive-section[data-home-section="home starred"]  .drive-section-header.drive-home-only(h2 "Starred" + button.drive-section-link[data-home-tab-link=starred] "See all" + chevron)
                                                           .drive-page-header.drive-tab-only(h2.drive-page-title "Starred", p "Notebooks and folders you marked as favorites.")
                                                           .drive-file-grid#drive-starred, .drive-empty.hidden#drive-starred-empty "No starred PDFs yet."
    section.drive-section[data-home-section="home shared"]   same pattern: "Shared" / "Shared with me" / "Notebooks other people shared with you." / #drive-shared / #drive-shared-empty "No shared docs."
    section.drive-section[data-home-section="files"]
      .drive-page-header: h2 "My Drive", p "Your folders and notebooks.", .drive-section-actions(#btn-drive-new-folder[title=New folder], #btn-drive-back[title="Up one folder"])
      h3.drive-subsection-title "Folders"   + .drive-folder-grid#drive-folders
      h3.drive-subsection-title "Notebooks" + .drive-file-grid#drive-folder-files
      .drive-empty.hidden#drive-folder-empty "No PDFs found in this folder."
  section.drive-folder-view.hidden#drive-folder-view[aria-label=Folder]   (§3.10)
  nav.home-nav#home-nav[aria-label=Sections] > .home-nav-capsule[role=tablist] > 4× button.home-nav-tab (§3.5)
```
`#drive-create-menu` and `#drive-upload-input` live outside `#drive-home` (html:9853-9858).

### 3.2 Topbar & FAB
- Topbar: padding 12px 20px, min-height `var(--banner-height)`=64px, `--bg-secondary`, bottom border
  `rgba(0,0,0,.06)`, `z-index:100`. ≤720px: padding 10px 14px, min-height 52px, smaller logo (17×20), buttons 34px.
- `#btn-drive-new` is a **floating action button** at every width (`offline-v5.css:30,38`; also html:7564-7586 at ≤720px): fixed,
  `right: safe-right+24px; bottom: safe-bottom+24px` (≤720px: `+16px` / `+84px` = 12+56+16 above the nav),
  52×52, radius 14px, white bg, `#111` icon 26px, shadow `0 3px 16px #0002`, `z-index:1300`. Opens
  `#drive-create-menu` (§6.1).
- `#btn-drive-fullscreen` → `toggleFullscreen()` (app:16630) on `document.documentElement`; icon/label swap
  `Fullscreen`/`Exit fullscreen` on `fullscreenchange` (+50 ms synthetic `resize`). Removed from the DOM in the
  Android app (`applyInhouseAppMode`, app:126) and hidden by `html[data-inhouse-app="true"]` CSS.
- `#btn-download-app` (web only; `html[data-inhouse-app="true"] .btn-download-app{display:none!important}`)
  opens `#modal-download-app` (§6.6).

### 3.3 Profile menus and `[data-app-version]`
Two menus with identical behaviour: Home `#drive-profile-menu` (html:9306-9325) and Editor
`#drive-profile-menu-editor` (html:9590-9620).

Home menu content, in order: `.drive-profile-meta` (img `#drive-profile-avatar-menu` 44px round,
`#drive-profile-name` default `Signed in`, `#drive-profile-email`), `label.theme-toggle[for=theme-toggle]`
“Night mode” + `input#theme-toggle[type=checkbox]`, `.drive-profile-actions` with `#btn-drive-menu-diary`
“Diary” and `#btn-drive-menu-agenda` “Agenda”, `.drive-profile-actions` with `#btn-drive-menu-refresh`
“Refresh” and `#btn-drive-menu-logout` “Sign out”, `button[data-offline-settings]` “Offline documents”,
`button[data-inhouse-update-settings][hidden]` “Actualizaciones · A/B”, and
`<div data-app-version style="text-align:center;font-size:11px;color:#bbb;padding-top:4px;">`.
Editor menu: same minus Diary/Agenda, plus `label.theme-toggle` “Draw with finger” (`#touch-draw-toggle`,
`localStorage.touchDrawEnabled='true'|'false'`) and `#doc-stats` (`N page(s) · M stroke(s)[ · size]`, refreshed
on open, app:7088), Refresh/Sign out with `-editor` ids.

Version label: app:66-73 reads `<script type="application/x-inhouse-config" id="inhouse-app-version">const
APP_VERSION = 'x.y.z';</script>` (html:93), throws if missing, and sets **every** `[data-app-version]` to
`v${APP_VERSION}`. v6 must keep this single source of truth (AGENTS.md); smoke test counts exactly 3
`data-app-version` occurrences (2 labels + 1 binding).

Behaviour:
- Avatar button click (stopPropagation) → `toggleDriveProfileMenu(menu, btn)` (app:7068): closes the other
  menu, toggles `.hidden`, `aria-expanded`. Opening the editor menu refreshes `#doc-stats`.
- Document click outside both menus and buttons closes both; `Escape` closes both.
- Theme toggle `change` → `applyTheme(checked?'dark':'light', persist=true)` → `html[data-theme]`,
  `localStorage['inhouse-theme']`, sync both checkboxes, `repaintDriveFolders()` (§3.8).
  OS theme changes are followed only while no theme is stored.
- Refresh → close menus + `refreshDriveFiles()`. Sign out → close menus + `signOutDrive()` (§3.16).
- Diary/Agenda → `openQuickAccessDriveDocument('diary'|'agenda')` (§7.3); both buttons are disabled while busy.
  Colours (html:4854-4892): Diary `#c9bfd8`/border `#b7abca`/text `#362b45` (dark `#b39ec7`/`#9d89b1`/`#21182d`);
  Agenda `#2f5a3f`/`#274a34`/`#f4f8f2` (dark `#2b5b3b`/`#3b7a51`).
- Offline documents → offline settings dialog (§6.7). Updates button unhidden only in the Android app when the
  shared updater is present (app:144-149).
- Profile data: `loadDriveProfile()` (app:7037) — skip if no token or offline, or cached profile already bound to
  this token. `GET https://www.googleapis.com/drive/v3/about?fields=user(displayName,emailAddress,photoLink)`.
  Stores `localStorage['drive-hint-email']=email` and `localStorage['drive-profile-v1']={name,email,photo}`,
  calls `InhouseOfflineLibrary.useAccount(email)` and `InhouseOfflineApp.authorizeBackground()`.
  `updateDriveProfileUI` (app:7002): name fallback `Signed in`, photo fallback `DEFAULT_AVATAR` (app:282, grey
  gradient person SVG data URI); photo is cached per account via `InhouseOfflineMedia.url(email,'avatar',photo,photo)`
  (works offline). `img` `error` → `DEFAULT_AVATAR`. On boot, `restoreCachedDriveProfile()` paints
  `drive-profile-v1` when its email equals `drive-hint-email` (marked `fallback:true`).
- Visual (html:4752-4843): 38px round avatar button (32px ≤720), menu absolute `right:0; top:calc(100%+10px)`,
  white (dark `#1f1f1f`), radius 16, padding 14, min-width 240 (max `min(92vw,280px)` ≤720), shadow
  `0 18px 40px rgba(0,0,0,.18)`, column gap 12, `z-index:2000`. Theme toggle: 38×20 pill, 14px knob, checked
  bg `--text-primary`, knob `translateX(18px)` 0.2 s (html:2871-2912).

### 3.4 Toolbar, search and status
- Search: debounced 250 ms after each `input` → `refreshDriveFiles()` (app:16263). The term (`value.trim()`)
  filters Recents, Shared and the current folder (both root Files tab and an open folder) via
  `name contains '<term>'` with only `'` escaped as `\'`; **Starred ignores search**. Offline/cached renders filter
  folder contents by case-insensitive substring (`lib.folder`, lib:183); cached Recents/Starred/Shared ignore it.
- Search box: flex 1, min 240/max 460px, height 42px, radius 10px, bg `rgba(0,0,0,.025)`; focus ring
  `0 0 0 3px rgba(224,122,60,.12)` + white bg. Disabled → opacity .7.
- `#drive-status` is **`display:none` by CSS** (html:5057); its text is still updated (screen-reader/state).
  `showDriveOpenError` and device-upload success set inline `display:block`, which is never reset (quirk).
  After a successful network refresh: `Recents: R | Starred: S | Shared: H | This folder: N PDF(s).`
  (`updateDriveStatusCounts`, app:10842; only when a token exists). Errors: `Unable to load Drive files.`,
  `Unable to refresh Drive files.` (auto refresh, non-silent only), `Unable to load this folder.`,
  `Loading folder...` (on navigation).
- `#btn-drive-bin` → `openDriveBinModal()` (§6.5).

### 3.5 Tabs and bottom navigation
- Tabs: `DRIVE_HOME_TABS = ['home','starred','shared','files']` (app:2042). `setDriveHomeTab(tab,{keepScroll})`
  (app:10720): unknown → `'home'`; finish any folder animation; if a folder view is open or the stack is deeper
  than root → `closeDriveFolderView({animate:false})`; set `#drive-home[data-home-tab]`; each `.home-nav-tab`
  toggles `.is-active` and `aria-selected`; `driveMain.scrollTop=0` unless `keepScroll`; persist
  `sessionStorage['ihn-home-tab']`. Restored once in `setupEventListeners` with `keepScroll:true` (app:16235).
- Section visibility (html:7838-7853): `.drive-section[data-home-section]` hidden unless its space-separated
  list contains the current tab. On `home`, `.drive-tab-only` is hidden; on other tabs `.drive-home-only` is hidden.
  Starred/Shared on the Home tab are horizontal rows; on their own tab they are grids.
- “See all” buttons (`[data-home-tab-link]`) → `setDriveHomeTab(link)`.
- Nav click on a tab: if it is the current tab **and** a folder view is open → `closeDriveFolderView()`
  (animated); else `setDriveHomeTab(tab)`.
- Nav markup (html:9469-9517): `button.home-nav-tab[role=tab][data-home-tab][title][style="--home-nav-flex:N"]`
  with N = 4 (Home), 7 (Starred), 6 (Shared), 5 (Files); inside `span.home-nav-pill` > `svg` (20px, filled) +
  `span` label. Icons: house, star (`M12 3.6l2.6 5.3 5.8.8-4.2 4.1 1 5.7-5.2-2.8-5.2 2.8 1-5.7-4.2-4.1 5.8-.8z`),
  two people, folder — copy paths verbatim.
- Nav style (html:8786-8880): absolute bottom, padding `8px (16+safe-right) (12+safe-bottom) (16+safe-left)`,
  `z-index:80`, pointer-events only on the capsule. Capsule: max-width 496px (560 ≤720), height 56, radius 28,
  padding 0 6px, bg `--home-nav-bg` (`#eeeeee` / dark `#1f1f1f`), shadow `--home-nav-shadow`. Tab:
  `flex: var(--home-nav-flex) 1 0px`, padding 6px 0; active tab `flex:0 0 auto`. Pill: 14px/500, gap 6,
  padding 0 8 (active 0 12), radius 28, colour `--home-nav-label` (`#4c4546`/`#cfc4c5`), active bg
  `--home-nav-selected` (`#ffdbc9`/`#5b4130`) and colour `--home-nav-on-selected` (`#5c4131`/`#ffdcc6`);
  icon shown **only on the active tab**; hover pill bg `rgba(127,127,127,.08)`; bg transition .2 s ease.

### 3.6 Lists: data sources, ordering, rendering

#### 3.6.1 Network refresh (`refreshDriveFiles`, app:10907)
- No-op in the offline worker. Offline + library ready → `InhouseOfflineApp.renderHome()` only.
- No token → `updateDriveAuthUI()` only.
- Else: status skeleton; `renderDriveBreadcrumbs()`; if library ready `renderHome()` (cached first paint) else
  `showDriveSkeletons()` (app:10886: 6 card skeletons in recents/starred/shared; root view: 4 folder + 8 card
  skeletons; nested view: 0 folder + 4 card skeletons in `#drive-view-*`; hide the 3 empty messages).
- `ensureDriveToken(false)`, then in parallel `refreshDriveRecents()`, `refreshDriveStarred()`,
  `refreshDriveShared()`, `refreshDriveFolderContents()`; then `updateDriveStatusCounts()`.
- Any failure → status `Unable to load Drive files.` (cards already on screen stay).
- `refreshDriveHomeQuiet()` (app:10942): same four requests (folder with `{silent:true}`), no skeletons, errors
  swallowed. Used after star/colour/move/trash/restore/upload and on reconnect.

#### 3.6.2 Requests (all through `driveFetch`, §10; `q`/`fields` are `encodeURIComponent`’d, `orderBy` is raw)
| List | q | orderBy | pageSize | fields | post-processing |
|---|---|---|---|---|---|
| Recents (app:10739) | `mimeType='application/pdf' and trashed=false[ and name contains '<s>']` | `modifiedTime desc` | 24 | `files(id,name,mimeType,modifiedTime,createdTime,thumbnailLink,iconLink,parents,starred,ownedByMe,resourceKey)` | drop quick-access files (`/^(?:Diary\|Agenda)\d{4}(?:-\d{4})?(?:\.pdf)?$/i`), keep first 12; `lib.observed(list,null,'recents')` |
| Starred (app:10760) | `starred=true and trashed=false and (mimeType='application/pdf' or mimeType='application/vnd.google-apps.folder')` | `modifiedTime desc` | 12 | `files(id,name,modifiedTime,createdTime,thumbnailLink,iconLink,mimeType,parents,starred,ownedByMe,resourceKey,folderColorRgb)` | `observed(...,'starred')` |
| Shared (app:10770) | `(mimeType='application/pdf' or mimeType='application/vnd.google-apps.folder') and sharedWithMe=true and trashed=false[ and name contains ...]` | `modifiedTime desc` | 24 | as Starred | keep `ownedByMe === false`; `observed(...,'shared')` |
| Folder: subfolders (app:10790) | `'<id>' in parents and trashed=false[ and name contains ...] and mimeType='application/vnd.google-apps.folder'` | `name` | 100 | `files(id,name,mimeType,starred,ownedByMe,folderColorRgb)` | — |
| Folder: PDFs | same base `and mimeType='application/pdf'` | `modifiedTime desc` | 100 | `files(id,name,mimeType,modifiedTime,createdTime,thumbnailLink,iconLink,parents,starred,ownedByMe,resourceKey)` | — |

**No paging**: `nextPageToken` is never requested; more than 100 items per folder are silently truncated.
Folder contents: `<id>` = `'root'` or the open folder; local ids (`local-…`) are first resolved with
`InhouseOfflineStore.resolveId` (unsynced local folder → render from the offline catalogue only). Results are
discarded if `driveCurrentFolderId`/`driveFolderNavigationId` changed meanwhile. With the library ready the
remote list is merged with pending local entries (`lib.combine`, lib:190) and re-sorted. While a folder
animation runs, the refresh is deferred (`driveFolderAnimation.refresh = options`).

#### 3.6.3 Offline catalogue (`offline-library-v5.js`) — sort and saved views
- `compareEntries` (lib:19): folders first; folders by `name.localeCompare`; files by `modifiedTime` desc; ties by
  `String(remoteId||id)` ascending.
- `compareRecents` (lib:206): `lastOpenedAt` (local document record) desc, then `modifiedTime` desc, then id.
- `renderDriveRecents` (app:9795): when the library is ready the list passes through `lib.recentItems(files)`
  (lib:212): union of the given list and all pending/`local-` PDFs, keyed by `remoteId||id`, sorted by
  `compareRecents`, first 12. So **Recents order = most recently opened on this device first**.
- `lib.home()` (lib:195, used for cached paint): Recents = saved view `recents` (ids → active entries via aliases)
  or all non-quick-access PDFs sorted by `compareRecents` (12) then `recentItems`; Starred = saved view or
  starred entries (12); Shared = saved view or `ownedByMe===false` (24).
- Saved views: IndexedDB `inhouse-offline-v1`, store `settings`, key `${account}\0home-views`, value
  `{recents:[ids],starred:[ids],shared:[ids]}` written by `observed(items,parent,view)` (lib:233; skipped when
  nothing changed). Entries are cached with `cacheEntries` (offline-store:144).

#### 3.6.4 DOM diffing (`diffRenderDriveList`, app:9718)
- Each card carries `data-drive-item-id` and `data-drive-item-fp`, fingerprint =
  `[id, name, starred?1:0, folderColorRgb||'', mimeType].join('|')` (app:9702; `modifiedTime` deliberately excluded).
- Unchanged fingerprint → reuse the element: update `card.driveFile`, call `preview.refreshFile(item)`,
  `lib.decorate`, update the `.drive-card-sub[data-file-sub]` text, move it into order.
- Changed/new → build with the list’s builder; in `#drive-view-folders`/`#drive-view-files` (non-deferred, no
  reduced motion) add `.drive-folder-item-arriving` (fade `opacity 0→1`, 180 ms ease-out) until `animationend`.
- Remove stale cards and any untagged child (skeletons).
- Renderers: Recents → `createDriveFileCard`; Starred → folder or file card; Shared → folder or file card;
  Starred/Shared toggle their `-empty` message by `list.length>0` (also visible on the Home tab).
  `renderDriveFolderContents(folders, files)` (app:9952) targets root grids (`#drive-folders`,
  `#drive-folder-files`, `#drive-folder-empty`) or, when a folder is open (`driveFolderStack.length>1`), the view
  grids (`#drive-view-folders`, `#drive-view-files`, `#drive-view-empty`) using deferred placeholders (§3.10.3).
  The empty message shows when both lists are empty. Headings “Folders”/“Notebooks” hide via
  `:has(+ grid:empty)` (html:7877-7880).

### 3.7 File card (A4 sheet)

#### 3.7.1 Structure (`createDriveFileCard`, app:9423)
```
div.drive-card[tabindex=0][data-drive-item-id][data-drive-item-fp][data-offline-available][data-offline-pinned]
    [style="--sheet-ratio:R"] [.offline-unavailable][aria-disabled=true][title="Connect to download this document"]
  div.drive-card-preview[.is-loading|.is-loaded][.is-landscape][data-label-tone=light|dark][style=--sheet-label-*]
    svg.drive-card-icon (file outline, until the image decodes)  |  img[alt="<name> preview"][decoding=async]
    div.drive-card-info
      div.drive-card-text
        div.drive-card-title[title=<full name>]  (text, or .is-shared: svg.drive-shared-icon + span.drive-card-title-text)
        div.drive-card-sub[data-file-sub=true]
      div.drive-card-menu > button.drive-card-more + div.drive-card-dropdown.hidden   (§3.9)
  span.drive-sheet-flag[aria-hidden=true]   (only if file.starred)
  button.document-sync-status[data-phase]   (offline badge, §3.15)
```
Activation (`bindDriveCardActivation`, app:9410): click anywhere except inside `.drive-card-menu`, or
`Enter`/`Space` when the card itself has focus → `openDriveFile(card.driveFile)`.

#### 3.7.2 Text
- Title (`applyDriveTitleWithSharedIcon`, app:9341): strip `.pdf` (case-insensitive); `formatDriveCardFileName`
  (app:9321): ≤38 chars unchanged; else if there is an extension → head + `... .ext` (`.pdf` lower-cased) when the head
  has >6 chars, else first `max(6,35)` chars + `...`. `title` attribute = full original name. Shared
  (`ownedByMe === false`) → `.is-shared` + people icon (paths at app:9363).
  CSS clamps to 2 lines (1 line for landscape).
- Sub (`describeDriveFile`, app:9387): `pending` → `Saved on this device`; shared → `Shared · <date>` or
  `Shared with you`; else `Edited <date>` or `PDF`. Date (`formatDriveShortDate`, app:9373) from
  `modifiedTime||createdTime`: today → `toLocaleTimeString({hour:'2-digit',minute:'2-digit'})`; this year →
  `{month:'short',day:'numeric'}`; else with `year:'numeric'` (browser locale).

#### 3.7.3 Thumbnail pipeline (`createDrivePreview`, app:9268)
1. Source = `file.localPreview` (data URL of page 1 written by the offline capture, oapp:88) or
   `normalizeDriveThumbnail(thumbnailLink)` (app:9131): `data:`/`blob:`/`/` untouched; replace the first `=s\d+`
   with `=s440`; if no `=s` remains append `&sz=w312-h440`.
2. `refreshFile(current)` only reloads when `modifiedTime` or `localPreview` changed (Drive thumbnail URLs rotate
   without edits). Adds `.is-loading` until the first success.
3. `InhouseOfflineMedia.url(account, 'preview:'+id, source, modifiedTime)` (offline-media-v5.js): per-account Cache
   Storage `inhouse-media-<account>`, key `/__offline-media__/<encoded id>` + revision stamp
   `<key>?revision`; refetch only when missing or revision changed and online; `fetch(source,{credentials:'omit'})`,
   fallback `{mode:'no-cors'}` (opaque → served through the service worker key URL if controlled, else source);
   6 s timeout; returns a `blob:` URL (memory LRU 128, old URLs revoked after 60 s). `data:` sources and missing
   account return the source unchanged.
4. Decode off-screen (`new Image().decode()`), wait for any folder motion to settle
   (`whenDriveFolderMotionSettled`), then: set `img.alt`, swap `src` (never remove the visible image), first time
   remove the icon, prepend the img, add `.is-loaded` (img fades in, opacity .16 s).
5. `getDriveSheetRatio(w,h)` (app:9142): ratio = clamp(w/h, 0.35, 2.2); `''` when `|ratio−0.7071|<0.012`, else
   `toFixed(4)`. Non-empty → `card.style --sheet-ratio`, else removed. Card in `#drive-view-files` →
   invalidate grid plans. `.is-landscape` when `w>h`.
6. Label tone (`applyDrivePreviewTone`, app:9236 / `sampleDrivePreviewTone`, app:9152): draw into a 24×16 canvas
   (white fill) the region x = left+4% … 92% wide, y = 66% … +30% of the top-centred cover crop of the sheet; quantise
   pixels to 3 bits/channel, take the most populated bin’s mean RGB; WCAG luminance;
   `darkInk = (L+0.05)/0.05 >= 1.05/(L+0.05)`. Sets on the preview `--sheet-label-rgb`, `--sheet-label-ink`
   (`#000`/`#fff`), `--sheet-label-ink-rgb`, `--sheet-label-shadow-rgb`, `data-label-tone` (`light`/`dark`).
   If the displayed image is tainted, an anonymous CORS probe (`crossOrigin='anonymous'`,
   `referrerPolicy='no-referrer'`) is queued: max 2 concurrent, max 24 queued, 2000 ms timeout; results cached per
   URL (64 entries, FIFO). Failure → keep CSS defaults.
7. Errors → keep the last good image; retry on the next refresh.
8. While opening a file, the editor shows a 50 %-opacity A4 placeholder with the thumbnail at `=s800`
   (app:11346-11358).

#### 3.7.4 Sheet styling (html:8000-8270, verbatim)
- Grid: `repeat(auto-fill, minmax(138px,1fr))`, gap `40px 28px` (≤720: 3 columns, gap `28px 16px`). Rows
  (Recents; Starred/Shared on Home tab): flex, gap 28 (18 ≤720), item basis 138px (112 ≤720), scroll-snap x
  proximity, hidden scrollbar, padding `22px 10px 12px`, margin `-22px -10px 0`.
- Card is transparent; `.drive-card-preview` is the paper: `aspect-ratio: var(--sheet-ratio)` (default .7071),
  width `min(100%, 141.42% * ratio)`, margins `--sheet-inset-block/inline` keep an A4 slot, radius 2px, bg `#fff`
  (also in dark, image `filter:brightness(.92)`), stacked-pages shadow
  `3px 3px 0 -1px var(--sheet-stack-1), 3px 3px 0 0 var(--sheet-stack-edge), 6px 6px 0 -2px var(--sheet-stack-2), 6px 6px 0 -1px var(--sheet-stack-edge), var(--sheet-shadow)`;
  hover/focus-visible `translateY(-4px)` 0.22 s `cubic-bezier(0.4,0,0.2,1)`; focus outline 2px orange, offset 6.
- Image `object-fit:cover; object-position:top center`. Icon 30 % width, `rgba(0,0,0,.16)`.
- Label overlay: `.drive-card-info` absolute at the sheet bottom, padding `18px 3px 7px 10px`, ink from tone vars;
  `::before` gradient from transparent to `rgba(label-rgb, fade .4/.22)` at 18px to `rgba(label-rgb, solid .5/.36)`,
  `backdrop-filter: blur(1.5px)` masked, transition .3 s. Title 0.84rem/600 (0.78 ≤720), sub 0.66rem; menu button
  40×40. Landscape: one line, sub hidden, padding `2px 3px 2px 8px`, 10px fade.
- Star ribbon `.drive-sheet-flag`: 13×24, `#ffde00`, `clip-path: polygon(0 0,100% 0,100% 100%,50% 80%,0 100%)`,
  top-right inside the sheet inset, moves with hover.

### 3.8 Folder card (manila folder)

#### 3.8.1 Structure (`createDriveFolderCard` app:9659, `createDriveFolderArt` app:10142)
```
div.drive-folder-card[tabindex=0][data-drive-item-id][data-drive-item-fp]
  div.drive-folder-art[data-folder-color=<base>][style=--folder-* vars, optional --folder-name-size]
    span.drive-folder-back[aria-hidden]
    span.drive-folder-sheets[aria-hidden]
    span.drive-folder-cover
      span.drive-folder-cover-face
        span.drive-folder-print > span.drive-card-title + span.drive-folder-rule[aria-hidden]
        div.drive-card-menu (folder menu)
      span.drive-folder-cover-inside[aria-hidden]
```
Click/Enter/Space → `openDriveFolderFromCard(folder, card)`. Offline decoration is never applied to folders.
Name size: if the longest whitespace-separated word >10 chars →
`--folder-name-size: max(6, 120/longestWord).toFixed(2) + 'cqi'` (default `11cqi`).

#### 3.8.2 Colour algorithm (app:9998-10140)
- `resolveDriveFolderColor(item)`: lower-cased `folderColorRgb`; not `#rrggbb` or equal `#8f8f8f` (Drive default)
  → `DRIVE_DEFAULT_FOLDER_COLOR = '#e3c58e'` (manila).
- HSL helpers: `driveHexToHsl` → `[h°, s%, l%]`; `driveHslToHex(h,s,l)` clamps s,l to [0,100] and uses
  `k(n)=(n+h/30)%12`, `a=s·min(l,1−l)`, channel `round(255·(l − a·max(−1, min(k−3, 9−k, 1))))` with n = 0,8,4.
- `warmDriveFolderHue(h,pull)`: if `42 < h < 72` → `h − (h−42)·pull`.
- `displayDriveFolderColor(base)` (theme-aware): `warm = s>8 && 28≤h<72`. Light: warm →
  `hsl(warm(h,.45), s, l)`, else base unchanged. Dark: warm → `hsl(warm(h,.85), min(s,70)·.78, max(53, min(58, l·.8)))`;
  else `hsl(h, s·.68, max(28, min(l,62)·.8))`.
- `shadeDriveFolderColor(c, amt)`: `hsl(warm ? h − amt·.25 : h, s, l − amt)`.
- `applyDriveFolderPalette(el, base, {exact})` sets on `el`:
  `--folder-color` = display colour; `--folder-cover` = `hsl(h,s,l+2)`; `--folder-inside` = `hsl(h, s·.85,
  min(94, l + (night?9:14)))`; `--folder-tab` = `shade(color, night?6:12)`; ink by contrast
  `darkInk = (L+0.05)/0.0614 >= 1.05/(L+0.05)` → `--folder-ink` `#1f1a14`/`#ffffff`, `--folder-hl`
  `rgba(0,0,0,.07)`/`rgba(255,255,255,.2)`, `--folder-hl-strong` `.12`/`.3`, `--folder-line` `rgba(0,0,0,.25)`/
  `rgba(255,255,255,.55)`; and `data-folder-color=base` (unless `exact`).
- `repaintDriveFolders()` on theme change: re-apply every `.drive-folder-art[data-folder-color]` and the open view.
- Open-folder tint `getDriveFolderViewTint(color)` (app:10112): light `h=warm(h0,.6)`, `s=min(s0,85)·.62`:
  bg `hsl(h,s,89)`, inside `hsl(h,s,94)`, tab `hsl(h,s·1.2,78)`; dark `h=warm(h0,.9)`, `s=min(s0·.3,18)`: bg
  `hsl(h,s,14)`, inside `hsl(h,s,18)`, tab `displayDriveFolderColor(color)`. `applyDriveFolderViewPalette`
  applies bg with `exact`, then overrides `--folder-inside`/`--folder-tab`, `data-folder-color=color`.
- 24-colour picker palette `DRIVE_FOLDER_COLORS` (app:9442): `#f83a22 #fa573c #d06b64 #ac725e #ff7537 #ffad46
  #fad165 #fbe983 #b3dc6c #7bd148 #16a765 #42d692 #92e1c0 #9fe1e7 #9fc5e8 #4986e7 #b99aff #a479e2 #cd74e6
  #f691b2 #cca6ac #cabdbf #c2c2c2 #8f8f8f`.

#### 3.8.3 Folder artwork CSS (html:8272-8568, verbatim incl. the base64 `--folder-grain` PNG at html:8313)
Footprint `aspect-ratio:210/297`, `perspective:1100px`, `perspective-origin:0 50%`,
`container-type:inline-size`, `--folder-px:1px` (every pixel measure is `calc(N * var(--folder-px))` so the ghost
can scale). Back panel from `top:6%`, radii `2 7 7 2`, grain 120px tiles, inset crease + drop shadows; tab =
`::after` at `right:7%`, width 44%, height `6.4cqi+1px`, `clip-path: polygon(0 100%,5% 42%,8% 14%,12% 2%,16% 0,
84% 0,88% 2%,92% 14%,95% 42%,100% 100%)`. Sheets `top 7.4% right 3% bottom 1.5% left 4%`, white. Cover from
`top:9.5%`, `transform-origin:0 50%`, `preserve-3d`, transition `transform .18s cubic-bezier(.3,.7,.2,1)`; on
fine-pointer hover `rotateY(-12deg)`. Face = crease gradients + grain + `--folder-cover`; inside face rotated 180°
with `--folder-inside`; both `backface-visibility:hidden`. Print: `bottom 8% right 8% left 14%`, gap 7px,
padding-bottom 34px, title Comfortaa 700, `font-size: var(--folder-name-size, 11cqi)`, 4-line clamp; rule 1px
currentColor .35. Menu bottom-right 4px, 40px button, 16px icon, ink colour.

### 3.9 Card “⋮” menu, colour popup, actions

- `createDriveItemMenu(item)` (app:9451): `div.drive-card-menu` > `button.drive-card-more[type=button][title/aria-label="More actions"]`
  (vertical 3-dot svg 16px) + `div.drive-card-dropdown.hidden` whose buttons are **rendered on each open**:
  1. `star` — `Add to favorites` / `Remove from favorites` (star svg filled `currentColor` when starred)
  2. files only: `offline` — `Save offline` (icon `offline`) / `Unpin offline copy` (icon `offlineOn`) per `lib.record(id).pinned`
  3. folders only: `color` — `Change color`
  4. `move` — `Move`
  5. `trash` — `Move to bin` (`class="danger"`, `#ff3b30`)
  Each: `<button type="button" data-action><svg viewBox="0 0 24 24" fill stroke="currentColor" stroke-width="1.9" stroke-linecap=round stroke-linejoin=round aria-hidden>{DRIVE_TOOL_ICONS[icon]}</svg><span>label</span></button>`
  (icons app:2054-2061). Exactly 4 buttons for both kinds (test).
- Open (`toggleDriveCardMenu`, app:9529): same menu toggles closed; otherwise close others, **move the dropdown to
  `<body>`** (the `.drive-main` carries a transform), unhide, `position:fixed`, `top=btn.bottom+4`,
  `right=innerWidth−btn.right`; next frame flip above (`top=max(8, btn.top−h−4)`) if it would pass
  `innerHeight−8−navHeight` (nav height counts only when Home is visible), clamp `left≥8`.
  Close returns it to its original parent (or removes it).
- Closing: document click outside (and not on a `.drive-card-more`), `Escape`, opening another, running an action.
- Actions:
  - star → `updateDriveStarStatus(item, !starred)` (app:9612): PATCH `files/{id}?fields=id,starred` `{starred}`;
    status `Added to favorites`/`Removed from favorites` or `Failed to update favorites`; `refreshDriveHomeQuiet()`.
  - offline → `lib.pin(item)` (lib:298): toggles `pinned`; downloading first if no local copy (offline →
    error `Connect once to download this document`); statuses `Saved for offline use` /
    `Automatic offline retention restored`; requests `navigator.storage.persist()`.
  - color → `openColorPopup(item, rect)` (app:9572): `div.drive-color-popup` on body, fixed at
    `top=rect.bottom+6, left=rect.left`, next frame clamped to the viewport (8px margins, flip above). 24
    `div.drive-color-swatch` (28px circles; grid 4 rows, column flow → 6 columns), current colour `.active`.
    Swatch click → `updateDriveFolderColor` (app:9630): PATCH `files/{id}?fields=id,folderColorRgb`
    `{folderColorRgb}`; `Folder color updated`/`Failed to update folder color`; quiet refresh. Outside click closes.
  - move → `openFolderPicker('move', item)` (§6.2). trash → `openTrashModal(item)` (§6.4).
- Dropdown style: white/`#1f1f1f`, radius 12, padding 5, min-width 168, `z-index:9999`; items 0.85rem, 8×10
  padding, 16px icons, hover `rgba(0,0,0,.06)`.

### 3.10 Opened folder view

#### 3.10.1 Markup (html:9427-9466)
```
section.drive-folder-view.hidden#drive-folder-view[aria-label="<name> folder"]
  .drive-folder-view-scroll#drive-folder-view-scroll > .drive-folder-view-inner
    .drive-folder-view-head
      button.drive-folder-view-btn#btn-folder-view-back[title/aria-label="Close folder"] (chevron left)
      .drive-folder-view-heading > h2#drive-folder-view-title + nav.drive-breadcrumbs#drive-breadcrumbs[aria-label="Folder path"]
      button.drive-folder-view-btn#btn-folder-view-new-folder[title/aria-label="New folder"]
    .drive-folder-grid#drive-view-folders
    .drive-sheet-grid > .drive-sheet-grid-items#drive-view-files (display:contents) + button.drive-new-sheet#btn-folder-view-new-pdf (+ "New PDF here")
    .drive-empty.hidden#drive-view-empty "This folder is empty."
```
- Positioned `absolute; top: var(--drive-folder-view-top)` (= topbar bottom relative to `#drive-home`, measured by
  `positionDriveFolderView`, app:10160, on open and resize), full width/height to the bottom, background
  `--folder-color` (tint bg), ink `--folder-ink`; a 14px `::before` strip at the left in `--folder-inside`
  (the folder’s inner flap). Scroll padding `0 40px (nav+40) 46px` (≤720 `0 16px (nav+84) 28px`). Title Comfortaa
  1.75rem/700 (1.35 ≤720). Head buttons 44px circles with `--folder-hl`. “New PDF here” = dashed A4 cell
  (`1.5px dashed --folder-line`, `--folder-hl` bg). Grid gaps as §3.7.4; folder grid `margin-bottom:56px` when not empty.
- `#drive-home.folder-open` while visible (no CSS rules attached).

#### 3.10.2 Navigation state
`driveFolderStack` (array of `{id,name,color,origin}`; root = `{id:'root',name:'My Drive'}`),
`driveCurrentFolderId`, `driveFolderViewOriginId` (id of the card that opened the view from the main area),
`driveFolderNavigationId` (incremented on every navigation; stale responses are dropped).
- `openDriveFolderFromCard(folder, card)` (app:10641): close card menu; finish any running animation;
  entry `{id, name||'Folder', color: resolveDriveFolderColor(folder), origin: captureDriveFolderOrigin(art)}`
  where `origin` = clone of the art, its rect, viewport size and the folder-view `scrollTop`. Opened from inside
  the view → push (ignored if already the top); otherwise stack = `[root, entry]` and origin id = folder.id.
  Then `positionDriveFolderView()` and `playDriveFolderOpen(art, entry, swap)` where swap =
  `revealDriveFolderView(entry)` + `prepareDriveFolderViewContent(entry)` + `renderDriveBreadcrumbs()` + status
  `Loading folder...` + `refreshDriveFolderContents()`.
- `revealDriveFolderView(entry)` (app:10616): apply tint palette, show, remove `.is-switching/.is-leaving`,
  `inert=false`, add `#drive-home.folder-open`; move focus to `#btn-folder-view-back` when focus was on body, in
  `.drive-main` or in the view (deferred to the end of the animation).
- `prepareDriveFolderViewContent(entry, scrollTop)` (app:10168): title, `aria-label`; **paint the cached directory
  immediately** (`lib.folder(id, search)`), or 4 card skeletons if nothing cached and online; restore scrollTop.
- `navigateDriveFolderUp()` (app:10690): depth ≤2 → `closeDriveFolderView()`; else go to parent.
- `navigateDriveFolderTo(index)` (app:10698): index ≤0 → close; else truncate the stack, set current, play the
  close transition from the closing entry’s saved origin (art `null`) and re-render the parent, restoring its
  `scrollTop` from `closing.origin.scrollTop`.
- `closeDriveFolderView({animate})` (app:10667): stack → root, origin cleared; animated unless `animate:false`,
  the view isn’t open or reduced motion; ghost lands on `findVisibleDriveFolderArt(originId)` (a visible
  `[data-drive-item-id=id] .drive-folder-art` inside `.drive-main`, ≥8px inside the viewport); focus returns to that
  card if focus was inside the view; `refreshDriveFolderContents({silent:true})` (root).
- Breadcrumbs (`renderDriveBreadcrumbs`, app:9666): one `button.drive-breadcrumb` per crumb (`.active` on the
  last), `span` “/” separators; root alone renders `My Drive/`; non-last crumbs navigate; also sets the title.
- Back controls: `#btn-folder-view-back` → up; Escape (no open overlay/menu/popup) → up; same-tab nav click → close;
  tab switch → close without animation.

#### 3.10.3 Deferred cards & viewport hydration (app:9831-9950)
Inside the open view, each card is first a placeholder: `div.drive-card|drive-folder-card.drive-card-deferred`
(`style="aspect-ratio:210/297"`, `tabindex=0`, `aria-label=name`, `.drive-folder-item-arriving` unless an animation
is running or reduced motion). `hydrate()` replaces its children with the real card (keeps the same element, re-tags
id/fp). Hydration happens on focus/click (click is forwarded to the real card), and for cards within the scroll
viewport ±240px (`hydrateVisibleDriveFolderCards`, after each render, scroll, resize, and animation end).
Visible range is computed from measured CSS grid tracks: for each grid read `gridTemplateColumns` (count `px`
tokens = columns), `gridTemplateRows` px list, `rowGap`, padding/border; plan cached by key
`count|columns|width|rowGap`; `InhouseRust.folderRows(rows, gap, columns, count).range(top, bottom, overscan)`
(binary search, folder.rs:58-110) → `{start,end}` item indices; plans invalidated by a ResizeObserver on the grids
and when a thumbnail changes a sheet ratio. Fallback when WASM/plan unavailable: per-card `getBoundingClientRect`.

#### 3.10.4 Folder open/close zoom (`playDriveFolderTransition`, app:10326; Rust `folder_motion`/`folder_fade`)
Skipped entirely (instant swap) when `prefers-reduced-motion: reduce` or `Element.animate` is unavailable.

Scene (`createDriveFolderGhost`, app:10274):
- `bounds` = `#drive-home` rect from `top = home.top + topbarBottom` to the bottom, full width.
- `rect` = live card art rect (or the entry’s saved `origin.rect`; if that viewport size changed or the rect is
  outside bounds, it is fitted/clamped 12px inside; with no origin: centred `width=min(180, W·.36, H·.4)`,
  `height=width·297/210`).
- `div.drive-folder-transition[aria-hidden][inert]` fixed at `bounds` (`z-index:70`, overflow hidden,
  `contain: layout paint`) appended to `#drive-home`; inside a deep clone of the art with
  `.drive-folder-ghost` (absolute, `transform-origin:0 0`, `--folder-px: calc(1px * var(--ghost-scale))`) and an
  empty `div.drive-folder-contents` inserted before its cover. Source art gets `visibility:hidden`.
  `#drive-folder-view.inert = true`; `ghost[data-direction=open|close]`.

Geometry (`folder_motion`, folder.rs:8-41; identical JS fallback in rust-core-v5.js:192-230):
```
scale      = max((W+4)/w, (H+4)/(h*0.94))
finalLeft  = (W - w*scale)/2
finalTop   = (H - h*0.94*scale)/2 - h*0.06*scale
dx = finalLeft - (rect.left - bounds.left);  dy = finalTop - (rect.top - bounds.top)
duration   = 600 ms open, 560 ms close   (DRIVE_FOLDER_OPEN_MS / DRIVE_FOLDER_CLOSE_MS, app:2044)
```
The ghost is laid out at its **final** size (`width=w·scale`, `height=h·scale`, `perspective=1100·scale px`,
`--ghost-scale=scale`) at the card position and shrunk by transform:
- zoom (on the ghost) open: `translate(0px,0px) scale(1/scale)` → `translate(dx px, dy px) scale(1)`;
  close: reverse. Easing `cubic-bezier(0.55, 0, 0.15, 1)`, `fill:both`.
- hinge (on the ghost’s `.drive-folder-cover`): open `closed → rotateY(-112deg)` easing
  `cubic-bezier(0.2,0,0.2,1)`; close `rotateY(-112deg) → closed` easing `cubic-bezier(0.4,0,0.2,1)`;
  `closed` = the card cover’s computed resting transform (e.g. the −12° hover tilt) or `rotateY(0deg)`.
- labels (`.drive-folder-print` and the cover’s `.drive-card-menu`), linear: open `1 → 0 @0.4 → 0`;
  close `0 → 0 @0.6 → 1`.
- All effects are created paused at t=0 and `play()`ed together in the tap’s task (first frame = the card;
  test: zoom is `running` and `pending`, scene not yet prepared); `zoom.finished → cleanup`.

Interior:
- **Open**: after one rAF + `setTimeout(0)` (`prepareScene`): if a folder view was already open (nested), its
  snapshot (`snapshotDriveFolderView`: deep clone without ids, offscreen whole rows replaced by
  `.drive-folder-snapshot-spacer` grid rows of exact height via `folderRows.span`, `inert`, `aria-hidden`) is
  inserted behind the ghost as `.drive-folder-transition-backdrop`. Then `swap()` runs (reveal + content), the live
  `#drive-folder-view` is moved (a `<!--folder-view-home-->` comment marks its place) into the ghost’s
  `.drive-folder-contents` with `top:-finalTop; left:-finalLeft; width:W; height:H; transform-origin:0 0;
  transform:none; opacity:0`, cards are hydrated in place, and a fade animation joins the zoom timeline at
  `zoom.ready`: keyframes from `folder_fade(elapsed,false)` = `[{0,0},{from,0},{to,1},{1,1}]`,
  `from=min(0.9, clamp(elapsed,0,1)+0.03)`, `to=max(0.7, min(1, from+0.15))`, linear, `startTime = zoom.startTime`.
  `handle.phase` goes `preparing → animating → complete`.
- **Close**: the outgoing view snapshot is mounted inside the ghost immediately (first frame) at the final layout
  with its scrollTop, and fades `[{0,1},{0.3,1},{1,0}]` linear. `prepareScene` then runs `swap()` (restore
  parent/root), hydrates, finds the destination card art; if it moved >0.5px the ghost is retargeted
  (`folderMotion` recomputed, keyframes replaced), and the destination art is hidden until cleanup.
- During motion: `.drive-folder-view.is-leaving` (pointer-events none); inside the transition, shimmer animations
  are paused, label `backdrop-filter` is `none`, nested folder arts are flat (`perspective:none`, cover
  `transform:none`, inside face hidden), `.drive-folder-item-arriving` animation suppressed; thumbnail swaps wait
  (`whenDriveFolderMotionSettled`); folder refreshes and renders requested meanwhile are queued
  (`handle.refresh`, `handle.pendingContents`).
- **Cleanup** (on finish, `resize`, `visibilitychange` to hidden, or `finishDriveFolderAnimation()` from any new
  navigation/tab switch): restore the live view to its marker (remove arriving classes, add
  `.is-settling-labels` for two frames so the 1.5px label blur eases back in over .3 s, restore inline styles and
  scrollTop), cancel all animations, remove the layer, unhide source/destination arts, `inert=false`, remove
  `.is-leaving`, restore focus (`focusTarget` or previous), then after rAF+`setTimeout(0)`: release motion waiters,
  apply pending contents/refresh, hydrate.

### 3.11 Pull-to-refresh (`setupMobileHomePullToRefresh`, app:10963)
Enabled only when `navigator.maxTouchPoints>0` or `ontouchstart` exists (no width condition — pinned by smoke test).
Constants: threshold 96px, maxOffset 82px, axis lock 8px, carousel vertical intent ratio 1.25.
- `touchstart` on `.drive-main` (passive): ignore while refreshing, multi-touch, or `scrollTop>1`; record start;
  `startedInCarousel` = target inside `.drive-recents`; axis `pending`.
- `touchmove` (non-passive): reset if `scrollTop>1`; lock axis once `max(|dx|,|dy|) ≥ 8`: carousel → vertical
  only if `|dy| > 1.25|dx|`, else vertical if `|dy| ≥ |dx|`; horizontal → clear pull; `dy ≤ 0` → pull 0; else
  `preventDefault` and `setPull(dy)`:
  `progress=min(1,d/96)`, `eased=1−(1−progress)³`, `offset=82·(1−e^(−d/78))`;
  CSS vars on `#drive-home`: `--home-pull-distance: offset px`, `--home-pull-scale: 0.84+eased·0.16`,
  `--home-pull-opacity: min(1, progress·2)`; class `home-is-pulling` when d>0; indicator `aria-label`
  `Release to refresh` at progress 1 else `Pull to refresh`.
- `touchend`: vertical and `d ≥ 96` → refresh; else reset. `touchcancel` → reset.
- refresh: classes `home-is-pulling home-is-refreshing`, distance 52px, scale 1, opacity 1, label `Refreshing`;
  `await refreshDriveFiles()`; keep visible ≥420 ms; then reset.
- reset: add `home-pull-resetting` for 220 ms, vars back to `0px/0.84/0`, label `Pull to refresh`.
- Visual (html:4923-5002): `.drive-main{transform:translateY(var(--home-pull-distance))}`; indicator absolute at
  `top: var(--home-pull-top)` (= topbar `offsetHeight`, measured at setup and next frame),
  `translate3d(-50%, calc(distance − 104%), 0) scale(var(--home-pull-scale))`, 38px white circle, 18px spinner
  (orange top border). Spinner animation is named only while pulling/refreshing/resetting
  (`home-pull-refresh-spin` 0.72 s linear infinite). Transitions: resetting `transform .3s cubic-bezier(.22,1,.36,1),
  opacity .2s`; refreshing `.24s` / `.16s`.

### 3.12 Auto-refresh (app:11114-11160)
`startDriveHomeAutoRefresh()` on `showDriveHome` → `setInterval(30000)` (`DRIVE_HOME_AUTO_REFRESH_MS`);
stopped on editor/welcome/sign-out. Each tick `refreshDriveHomeSnapshot({silent:true})` refreshes **Recents and
the current folder only** (not Starred/Shared), skipped when `isDriveHomeAutoRefreshBlocked()`: Home hidden; no
token, signed out or reconnect needed; document hidden; any open in progress (Drive, offline, quick access);
offline; a folder animation or smooth editor interaction; an open card menu or colour popup; folder picker or bin
modal visible; Home profile menu open. Not re-entrant.

### 3.13 Fast-scroll thumb (`installDocumentFastScroller`, app:35798; CSS offline-v5.css:24-28)
A single `#document-fast-scroll[role=scrollbar][tabindex=0][aria-label="Fast scroll"]` on `<body>` also serves Home:
target = folder view scroller if open, else `.drive-main` (gets `id="drive-main-scroll"` for `aria-controls`).
Hidden when scroll range <2px or travel <30px. Position `translateY(top+12 + ratio·travel)` where travel =
visible height − 72. Drag maps pointer delta/travel to scrollTop; keys ArrowUp/Down ±4 %, PageUp/Down ±20 %,
Home/End. 29×48 pill, radius `48px 4px 4px 48px`, `#df8143` (dark `#87502f`), white double-chevron.

### 3.14 Global keyboard/click handling (app:16120-16152)
- Document click: close bin item menu, card menu (unless the click is on a `.drive-card-more`), colour popup,
  presence popup, profile menus (unless inside menu/button).
- `Escape`: if no `.modal-overlay.visible`, card dropdown, colour popup or open Home profile menu, and a folder view
  is open on visible Home → `navigateDriveFolderUp()`. Always: close cover modal, folder picker, trash modal, bin
  modal, bin item menu, profile menus, presence popup.
- Touch/pen `pointerup` blurs pressed buttons (`.btn, .tool-btn, .drive-profile-btn, …`) next frame.

### 3.15 Offline decoration and sync badges (`lib.decorate`, lib:130; offline-v5.css:3,15-21)
Applied to file cards on render/diff and on every sync-progress/reload event
(`document.querySelectorAll('[data-drive-item-id]')`). Folders are skipped.
- `data-offline-available` = record has a blob or snapshot pages; `data-offline-pinned` = record `pinned`.
- Offline and not available → `.offline-unavailable` (`opacity:.42; filter:grayscale(1)`), `aria-disabled=true`,
  `title="Connect to download this document"` (card stays clickable; opening shows the error, §7.1).
- Badge `button.document-sync-status[type=button][aria-live=polite][data-phase]` (absolute top-left 8px, white
  92 %, radius 10, 11px/500) exists only while the document has an outbox operation or is the active sync item.
  Text: active phase → `Checking…`, `Preparing…` (building), `Merging…`, `Keeping both versions…` (preserving),
  `Uploading…`, else `Syncing…`; error → `Retry upload`; offline → `Saved offline`; else `Waiting to upload`.
  Spinner pseudo-element for uploading/building/checking/merging. Click: active → nothing; error →
  `#offline-sync-error` dialog (§6.7); else probe connectivity then retry sync.

### 3.16 Sign-out (`signOutDrive`, app:6895)
Checkpoint the open document offline first (failure → status `Keep this document open: its offline copy could not
be saved.` and abort). Then: mark session inactive, clear background auth, `lib.logout()`, set signed-out,
stop auto-refresh/collab/timers, abort saves, revoke the token (GIS), reset all document state, clear
Recents/Starred/Shared/folder caches and DOM, stack → root, hide the folder view, clear folder-meta cache,
status `Sign in to browse your Drive.`, reset share state, profile UI to defaults, remove `drive-token-v1`,
`drive-remember-v1`, `drive-hint-email`, `drive-profile-v1` (and `InhouseOfflineMedia.clear()`),
`drive-consent-granted`, `google-scope-version-v1`, `google-granted-scopes-v1`, `drive-refresh-token-v1`,
`drive-pkce-verifier-v1`, `drive-session-v1`; status `Signed out`; `showWelcomeView()`.

---------------------------------------------------------------------------------------------------------

## 4. Loading overlay (shared, html:10259; CSS html:4251-4338; app:31734-31780)
`#loading-overlay[role=status][aria-live=polite]` with `.loading-home-brand` (logo + “inhouse notes”), `.spinner`,
`#loading-text`. `showLoading(text)`: dark overlay `rgba(0,0,0,.7)`, white 48px spinner, `.visible`
(0.3 s fade, `z-index:3000`). `showHomeSaveLoading(text='Saving changes to Drive...')` adds
`.home-save-transition`: opaque `--bg-primary`, brand shown, 38px orange-top spinner, secondary text, 0.16 s.
`hideLoading()` → `.fading-out` for 360 ms (spinner animates only while visible or fading); the home-save class is
removed 180 ms later. Texts used by Home flows: `Opening…`, `Opening PDF...`, `Restoring document...`,
`Analyzing local copy...`, `Downloading... N%`, `Processing PDF...`, `Opening shared document...`,
`Checking <Diary|Agenda> <period>...`, `Creating PDF...`, `Creating copy...`, `Moving file...`,
`Moving to bin...`, `Restoring...`, `Saving on this device…` (Home button exit).

## 5. Editor → Home (logo button) — Home-side contract
`#btn-drive-home` (the editor logo, `role=button`, `tabindex=0`; clicks on `#doc-title` ignored; Enter/Space) runs
`goHome` (app:15981-16049): if anything is pending, show the home-save overlay `Saving on this device…`, interrupt
background work, `queueSave()` + `persistExitLocalCheckpoint()` (must be awaited; no `Promise.race`, no token wait);
verify the offline copy is durable when a Drive upload is pending; `clearPresence()`; `showDriveHome()`;
`#drive-status` = `Saved on this device · waiting to sync` or `All changes saved.`; kick the outbox; start the
lifecycle exit upload if online; `refreshDriveFiles()`. Failure → stays in the editor with
`Could not finish saving. The document is still open; please try Home again.` (details: persistence spec).

---------------------------------------------------------------------------------------------------------

## 6. Home dialogs

All `.modal-overlay` dialogs: fixed full-viewport (`100svh`), `rgba(0,0,0,.5)`, `z-index:3000`, opacity transition
0.2 s, `.visible` shows; `.modal` card 400px max/90 %, radius 16, padding 24, scale .95→1 (0.3 s). Clicking the
overlay itself closes (folder picker, new folder, trash, bin, download, share). CSS: html:2943-3097 and 6563-6586
(both blocks apply; later wins on conflicts).

### 6.1 Create menu (`installDriveCreateMenu`, app:11723; html:9853-9858; offline-v5.css:32-36)
`#drive-create-menu[role=menu][aria-label="Add file"][hidden]` with `button#drive-create-pdf[role=menuitem]`
“Create PDF” and `button#drive-upload-file` “Upload file” (22px icons), plus hidden `input#drive-upload-input[type=file]`
(any type). FAB click toggles; on open: `aria-expanded=true`, `left = clamp(8, innerWidth−menuW−8, fab.right−menuW)`,
`top = fab.bottom+8` if it fits below else `max(8, fab.top−h−8)`; focus the first item. Close on outside
`pointerdown`, `Escape` (refocus FAB), `resize`. ArrowDown/ArrowUp both move focus to the **next** item (quirk).
Create PDF → `openFolderPicker('new')`. Upload file → reset and click the input; `change` →
`openFolderPicker('upload', file)`. Menu: fixed, z 1400, min-width 190, radius 16, items 12px padding radius 10.

### 6.2 Folder picker (`openFolderPicker`, app:11975; html:9860-9883)
Modes and texts:
| mode | title | subtitle | confirm | name row | name default |
|---|---|---|---|---|---|
| `new` | `New PDF` | `Choose a folder and name your file.` | `Create` | shown, `.pdf` suffix | `normalizeDocTitleInput(state.exportName||'Untitled')` (the last document’s name — quirk) |
| `move` | `Move file` | `Choose the destination folder.` | `Move` | hidden | `''` |
| `copy` | `Copy file` | `Choose a destination and name your copy.` | `Copy` | shown, `.pdf` suffix | set by `makeDriveCopy`: `Copy of <driveFileName>` (includes `.pdf`) |
| `upload` | `Upload file` | `Choose where to upload <file.name>.` | `Upload` | shown, suffix hidden | `file.name` |
- Requires a token or a ready library. Initial stack = copy of `driveFolderStack`, current = `driveCurrentFolderId`.
- Meta block (move/copy): label `Moving`/`Copying`, file name, `Current: loading...` → `Current: My Drive / A / B`
  (`resolveDrivePathForFile`, app:11941: uses the Home stack when the file’s parents contain the current folder;
  else `GET files/{id}?fields=parents` then walk up to 20 ancestors via `GET files/{id}?fields=id,name,parents`
  cached in `driveFolderMetaCache`).
- Confirm disabled when the trimmed name is empty (except move). Enter in the input confirms. Focus+select the
  input next frame (not for move).
- Breadcrumbs `button.folder-picker-breadcrumb(.active)` + `/`; folder rows `div.folder-picker-item` (folder svg in
  orange or the folder’s `folderColorRgb`, name) → enter.
- Listing (`refreshFolderPickerFolders`, app:12105): offline/local → cached subfolders; else
  `GET files?q='<id>' in parents and mimeType='application/vnd.google-apps.folder' and trashed=false&orderBy=name&pageSize=200&fields=files(id,name,folderColorRgb)`,
  observed into the catalogue and merged with local pending folders. Error → empty (`No folders here.`).
- Confirm (`confirmFolderPicker`, app:12151): upload → button `Uploading…` (disabled) → `uploadDeviceFile`; errors
  are shown in the subtitle via `lib.syncErrorText`; move → close + `moveDriveFileToFolder`; copy → close +
  `executeDriveCopy(name, folder)`; new → close + `createNewDriveDocumentWithFolder(folder, name)`.

Actions:
- Move (app:12182): loading `Moving file...`; `GET files/{id}?fields=parents`; `PATCH files/{id}?addParents=<folder|root>&removeParents=<all,current>`
  (no body); `File moved`/`Move failed`; quiet refresh.
- Copy (app:11687): `POST files/{id}/copy` `{name, parents:[folder] unless root}`; `Copy created! Opening...` then
  `openDriveFile({id,name,parents})`; `Failed to create copy.` / `Error creating copy.`
- Upload (`uploadDeviceFile`, app:11744): empty → `Choose a non-empty file.`; PDF + library ready → durable local
  outbox entry `local-<uuid>` (`pending:true`) in the chosen folder, then render Home and kick sync; offline non-PDF →
  `Connect to upload this file. PDFs can also be queued offline.`; unsynced local folder → `This folder is still waiting
  to upload. Try again shortly.`; else resumable upload `POST https://www.googleapis.com/upload/drive/v3/files?uploadType=resumable&fields=id,name,mimeType,parents,modifiedTime,version`
  with metadata `{name, mimeType: file.type||'application/octet-stream', parents?}`, resume key
  `device-upload:<folder>:<name>:<size>:<lastModified>`; missing id → `Drive did not confirm the upload. Retry
  without closing this dialog.`; success → observed, quiet refresh, status `Uploaded: <name>` (made visible).

### 6.3 New folder (app:8909-8937; html:9885-9897)
`#modal-new-folder`: “New folder” / “Name your new folder.” / `input#new-folder-name[placeholder="Folder name"]`,
Cancel/Create. Opened from `#btn-drive-new-folder` (Files tab) and `#btn-folder-view-new-folder`; requires token or
library; input cleared and focused next frame. Enter confirms, Escape cancels. Empty name → nothing. Creates in
`driveCurrentFolderId` — library ready → offline `createFolder` (outbox); else
`POST files?fields=id,name,mimeType,starred,ownedByMe,folderColorRgb` `{name, mimeType: folder, parents?}` (always a
new folder, duplicates allowed). `Folder created` → `refreshDriveFolderContents()`; `Failed to create folder`.

### 6.4 Move to bin (app:12206, 12358; slider app:21499-21670; html:9921-9940)
`#modal-trash-file.modal-danger`: “Move to bin?”, “This will move **<name>** to your Drive bin.”, slide-to-confirm
`#trash-slider` (`.confirm-slider` 52px, radius 16; handle 44×44 orange radius 14 with chevron; fill
`rgba(224,122,60,.18)`; text “Slide to move to bin”), Cancel. Drag with pointer capture; release at ≥90 % of
`maxX = sliderWidth − handleWidth − 2·4` → `.complete` (fill `.9` alpha, white text), snap to end, confirm after
120 ms; else snap back. Confirm: `PATCH files/{id}` `{trashed:true}`, `Moving to bin...`, `Moved to bin` /
`Move to bin failed`, quiet refresh. Slider is reset on open/close.

### 6.5 Bin (app:12221-12356; html:9942-9957)
`#modal-drive-bin`: “Bin”, “Files currently in your Drive bin.”, `#drive-bin-loading` (mini spinner + “Loading bin...”),
`#drive-bin-list[aria-busy]`, `#drive-bin-empty` “Bin is empty.”, Close. Requires a token. Request:
`GET files?q=trashed=true and (mimeType='application/pdf' or mimeType='application/vnd.google-apps.folder')&orderBy=modifiedTime desc&pageSize=100&fields=files(id,name,modifiedTime,createdTime,mimeType,parents)`.
Row `div.bin-item`: folder/file svg, `.bin-item-name`, `.bin-item-meta` `Modified <Mon D, YYYY>` (`formatDriveDate`,
`{month:'short',day:'numeric',year:'numeric'}`), `.bin-item-menu` with horizontal ⋮ `.bin-item-more` and
`.bin-item-dropdown.hidden` containing `Restore`. Errors show the empty message. Restore:
`PATCH files/{id}?addParents=<parents joined>` `{trashed:false}`, `Restoring...`, `Restored`/`Restore failed`,
reload bin + quiet refresh.

### 6.6 Download app (html:10092-10113)
`#modal-download-app` (web only): close button (`aria-label="Cerrar"`), Android robot icon `#3ddc84` 92px, title
“Download the inhouse notes Android app”, link `#download-app-link` →
`https://github.com/miguelcoxcaballero/inhousenotes/releases/download/android-v1.0.11/inhouse-notes-release-v1.0.11.apk`
(`download` attr) “Download APK”; modal closes 100 ms after the link click.

### 6.7 Offline dialogs (lib:154, lib:549)
- `dialog#offline-settings.offline-settings` (“Offline documents”): retention days input (1–3650), note, queue
  status (`N pending · Waiting for connection|Will sync automatically` or `Up to date`), recovery rows with
  “This device”/“Other device” download buttons, Cancel/Save (Save → set retention, prune, reload).
- `dialog#offline-sync-error` (“Upload pending”): file name, `syncErrorText` summary, redacted error report
  `Inhouse Notes <version>\n<phase> · <code>\n<detail>`, Close / Copy error / Retry.

---------------------------------------------------------------------------------------------------------

## 7. Drive file open, quick access, new document

### 7.1 `openDriveFile(file)` (app:11261)
Guarded: needs `file.id`; ignored while another open (Drive or offline) runs; waits for app init.
1. **Offline library first** (when ready): `Opening…`; if the editor is visible, checkpoint it; `lib.openRecord(file)`
   (stamps `lastOpenedAt`) → if a local blob/snapshot exists → `InhouseOfflineApp.restore(doc)` (editor shows it;
   status `Available offline` or `Saved on this device · waiting to sync`; online & remote → collab polling +
   background revalidation) and stop. Offline and no copy → `Connect to download this document`. Local open errors:
   if the record is dirty, checkpointing failed or offline → `Could not open the local copy. Your changes are
   retained; try again.`; otherwise fall through to Drive bypassing caches.
2. **Drive**: new document session; detached exit uploads keep running; clear public preview/sign-in gate; reset
   zoom/pan and all document Drive state; read-only; title = `file.name` without `.pdf`; thumbnail placeholder;
   `showEditorView()`; `Opening PDF...`; prewarm live collaboration.
3. **Open cache** (IndexedDB `inhouse-original-pdfs-v1`, store `drive-open-cache`, scoped to the account) or the
   in-memory just-uploaded PDF: used when `cache.fileId===file.id` and `cache.meta.modifiedTime ≥ card.modifiedTime`
   (>0). Then import, `PDF loaded`/`PDF loaded (View Only)`, and background `revalidateCachedDriveOpen`
   (app:11182: `GET files/{id}?fields=capabilities(canEdit),ownedByMe,owners(displayName,emailAddress,photoLink),resourceKey,headRevisionId,modifiedTime,version,md5Checksum,size,parents`
   `[&resourceKey]`, `Cache-Control: no-cache`, 7 s; pull remote changes if head/version/modified/md5 differ).
4. Otherwise: token; metadata GET (same fields as above, 7 s timeout); `fetchStableDrivePdfSnapshot` (app:26013):
   up to 3 attempts of download + metadata re-check; download via
   `GET files/{id}/revisions/{headRevisionId}?alt=media` (owners only; 403/404 → remember and fall back) or
   `GET files/{id}?alt=media[&resourceKey]` (headers `Cache-Control:no-cache`, `X-Goog-Drive-Resource-Keys:
   id/key` when a resource key exists, 300 s timeout) verified by size+md5; progress `Downloading... N%`
   (≥2 MB with Content-Length); `Processing PDF...`; import; folder kind resolved in the background
   (Diary/Agenda by parent folder names); open cache saved; autosave = `capabilities.canEdit`; collab polling;
   status `PDF loaded`/`PDF loaded (View Only)`.
5. Failure (non-abort) → `showDriveHome()` + `Could not open this document. Try again.` (status visible).

### 7.2 Viewer mode (`updateViewerModeUI`, app:11599)
`isViewer = driveFileId && driveCanEdit===false`; anonymous public viewer = viewer + public preview id + no token +
no gate. Viewer: `#readonly-banner` shown (except anonymous), toolbar hidden, editing tools disabled, title not
editable, mode toggle hidden (anonymous: shown as a fixed bottom pill `Read only`, `aria-disabled=true`,
`.public-read-only-mode`), share button stays enabled. Banner default copy: **View Only** “You don't have
permission to edit this file.” with `Make a Copy` (`makeDriveCopy`) and `Ask to Edit`
(`askToEdit`: gate active without token → `signInDrive()`; else `window.open('https://drive.google.com/open?id=<id>','_blank')`).

### 7.3 Quick access Diary / Agenda (app:8979-9043)
- Config `DRIVE_QUICK_ACCESS_DOCS` (app:394): diary `{label:'Diary', folderName:'Diary', filePrefix:'Diary'}`,
  agenda `{label:'Agenda', folderName:'Agenda', filePrefix:'Agenda'}`.
- Period: diary → label `YYYY`, file `DiaryYYYY`; agenda → academic year starting in September
  (`month ≥ 8` → this year else previous): label `YYYY–YYYY+1` (en dash), file `AgendaYYYY-YYYY+1`.
- Flow: `Checking <label> <period>...`; token unless offline; `ensureNamedDriveFolder(folderName)` (app:8842):
  offline library first (existing folder by name at root, or create offline), else
  `GET files?q=name='<escaped>' and mimeType='application/vnd.google-apps.folder' and trashed=false&orderBy=createdTime&pageSize=1&fields=files(id,name)`
  (no parent constraint — any folder of that name), create with `POST files?fields=id,name` if absent.
  `findDrivePdfInFolder(folderId, base)` (app:8940): `(name='<base>.pdf' or name='<base>') and mimeType='application/pdf'
  and trashed=false and '<folder>' in parents`, `orderBy=modifiedTime desc&pageSize=5&fields=files(id,name,thumbnailLink,resourceKey,modifiedTime)`
  (escaping `\` and `'`). Found → `openDriveFile({...file, parents:[folder], driveQuickAccessKind})`; else create a new
  document with template `diary`/`agenda` (agenda pre-filled with weekly pages from the Monday of the week of
  1 Sep to 31 Aug, `buildAcademicAgenda`) and `refreshDriveFiles()`. Error → `Failed to open <label>`.
- These files are excluded from Recents (§3.6.2).

### 7.4 New document (`createNewDriveDocumentWithFolder`, app:11781)
Ignored during an open. Checkpoint the current editor offline (failure → abort with the “Keep this document open”
status). Builds a one-page document (A4 default; A5 559×794 for agenda/diary), title = sanitised name, shows the
editor; library ready → `InhouseOfflineApp.newDocument()` + `Created on this device`; else build the PDF and
`createDriveFileInFolder` (resumable `POST upload/drive/v3/files?uploadType=resumable&fields=id,headRevisionId,modifiedTime,version`,
metadata `{name:'<base>.pdf', mimeType:'application/pdf', parents?}`) → `PDF created` / `Failed to create PDF`.
`ensureDriveFolder()` (app:8815, folder `Inhouse Notes`, any location) is only used by the save path when a
document without a Drive id and folder is first uploaded.

---------------------------------------------------------------------------------------------------------

## 8. Shared and public links (§4.16b)

### 8.1 Link format and parsing
Generated by the Share modal “Copy link” (app:15681-15731):
`<origin><pathname>?drive=<id>&public=1&role=<reader|writer>&name=<file name>[&mode=edit][&resourcekey=<key>]`.
Parsed in `showFastInitialView` and again in `init` (app:2222, 2291): `drive` → `window.pendingSharedDriveId`;
`resourcekey` or `rk`; `name` trimmed, max 240 chars; role = `writer` if `role=writer` **or** `mode=edit`, else
`reader`. `public` is never read. The query string is never removed (reloads keep the link).

### 8.2 Signed-in open
From `updateDriveAuthUI` when a token exists and no open runs: status `Loading document info...`;
`GET files/{id}?fields=id,name,resourceKey[&resourceKey=<key>]` → `Opening document...` →
`openDriveFile({id,name,resourceKey})`; on metadata failure open with `pendingSharedDriveName || 'Shared Document'`.
Success clears the pending share and the sign-in gate; failure → `Could not open shared document`.

### 8.3 Signed-out open (`tryOpenPendingSharedWithoutSignIn` app:8797 → `openSharedFileWithoutSignIn` app:8680)
Once per file id (unless forced): read-only, `showDriveHome()`, loading `Opening shared document...`,
`fetchPublicSharedPdf(id, key)` (app:8083): endpoints (optional API key first — `PUBLIC_DRIVE_API_KEY` from
`window.INHOUSE_PUBLIC_DRIVE_API_KEY` or `<meta name="inhouse-public-drive-api-key">`, **absent in production**)
`https://drive.usercontent.google.com/download?id=<id>&export=download[&resourcekey=]` and
`https://drive.google.com/uc?export=download&id=<id>[&resourcekey=]`, `credentials:'omit'`, `cache:'no-store'`,
3 attempts; must start with `%PDF-`; with an API key, md5/size and before/after metadata must match.
On success: import read-only, `Shared PDF loaded (View Only)` (writer: `Shared PDF loaded. Sign in to edit and save.`
+ gate), start public polling and live collaboration.
**Latent defect (quirk):** `fetchWithDeadline(url, options, consumer)` is called with the consumer in the
`timeoutMs` slot (runtime-core-v5.js:4 expects `(url, options, timeoutMs, consumer)`), so it returns the raw
`Response`, destructuring yields `undefined`, every endpoint throws, and the download **always fails**. The
observable v5.17.22 behaviour is therefore always the anonymous preview below (same for
`fetchPublicSharedMetadata`).
Fallback → `#drive-status` `Opening public view — no sign-in required.` → `openPublicDrivePreview` (app:8510).

### 8.4 Anonymous inline preview (`showPublicInlinePreview`, app:8404; CSS html:1480-1590)
- In the editor viewport: clear pages, `div#public-inline-preview.public-inline-preview` (absolute inset 0, z 8,
  `cursor:grab`/`grabbing`, `touch-action:none`) > `div.public-preview-stage` (transform target). Each (re)load
  appends `iframe[title="Public shared PDF"][referrerpolicy=strict-origin-when-cross-origin][scrolling=yes]
  [tabindex=0]` with `src = https://drive.google.com/file/d/<id>/preview?embedded=true[&resourcekey=][&ihn_refresh=<Date.now()>]`,
  positioned `top:-52px; height:calc(100%+52px)` (hides Google’s header), fades in (`.ready`, 120 ms) and removes the
  previous iframes 140 ms after load. `#canvas-viewport.public-preview-active`, `body.public-preview-mode`,
  canvas container `visibility:hidden`, `#btn-edit-pages` disabled.
- State: `driveFileId=id`, `driveResourceKey`, `driveCanEdit=false`, `driveAutosave=false`, `driveOwnedByMe=false`,
  file name = option or `Shared document`, export name without `.pdf`; read-only; viewer UI; editor view.
- Controls `div.public-preview-controls[aria-label="Public document zoom"]` bottom-right (pill, blurred):
  `−` (Zoom out, ÷1.25), `Fit` (reset; shows `NN%` when scale≠1), `+` (×1.25).
- Gestures (`installPublicPreviewGestures`, app:8281): pointer pan (centroid), two-pointer pinch (distance ratio,
  anchored at the centroid), wheel pans (`ctrl/meta`+wheel zooms by `exp(−deltaY·0.002)`), double-click toggles
  `×1.5` / back to 1 when ≥2. Scale clamp [0.7, 4]; pan clamp ±`w·(0.16+(s−1)·0.5)`, ±`h·(0.2+(s−1)·0.5)`;
  transform `translate3d(px,py,0) scale(s)`.
- Status `Public document opened — no sign-in required.` (reader) / `Sign in to activate this editor link.` (writer).
- Writer → `showSharedEditorSignInGate` (app:8494): keep the pending share as writer; banner
  `data-shared-editor-gate="true"`, message “**Editor link** Sign in to edit and save.”, `Make a Copy` hidden,
  `Ask to Edit` relabelled `Sign in` (→ `signInDrive()`); after sign-in §8.2 opens the real file. Reader → pending
  share cleared. `clearSharedEditorSignInGate` restores the default banner copy.
- Polling (`startPublicSharedPolling`, app:8536): `setInterval(poll, PUBLIC_DRIVE_API_KEY ? 1800 : 2500)`, skipped
  while hidden or in flight; with no API key every poll re-downloads; a changed fingerprint (size + FNV-1a of the
  first and last 64 KiB) imports the new PDF and replaces the preview; on failure in preview mode the iframe is
  reloaded at most every 7000 ms. Net effect today: the public iframe is re-created roughly every 7.5 s.

---------------------------------------------------------------------------------------------------------

## 9. Share modal and permissions API (§4.21a)

### 9.1 Modal (`setupShareModalEvents`, app:15320; html:10320-10428; CSS html:6587-7801)
- `#btn-drive-share` (editor header) shown iff `state.driveFileId` (`updateDocTitle`, app:5744).
  Click → `#share-modal-overlay` gets `.visible` + `.loading` (skeleton `#share-modal-loading` shown, content hidden)
  → `refreshShareModalData()` → loading off. Close button / overlay click closes.
- Structure: header “Share document” (Comfortaa 1.2rem) + round close; `#share-owner-info` (“People with access” +
  `#share-access-list`); `form#share-invite-form.share-section` (“Add people”, `input#share-email-input[type=email]
  [placeholder="Email address"][required]`, `select#share-role-select` Viewer/Editor, `button#btn-send-share`
  “Send”); General access (`select#share-link-access-select` Restricted / Anyone with the link,
  `select#share-link-role-select.hidden` Viewer/Editor); `button#btn-copy-share-link` “Copy link”.
- Refresh: reset (owner info hidden, list empty, owner sections hidden, access `restricted`, role select hidden);
  `fetchDriveOwnerInfo` (app:9097): `GET files/{id}?fields=ownedByMe,capabilities(canEdit),owners(displayName,emailAddress,photoLink),resourceKey[&resourceKey]`
  → updates `driveOwnedByMe`, `driveCanEdit`, `driveResourceKey`, owner name/email/photo, session. Then
  `GET files/{id}/permissions?fields=permissions(id,type,role,emailAddress,displayName,photoLink)`.
  `isOwner` = `driveOwnedByMe !== false`, overridden by the permission matching the signed-in email
  (`role==='owner'`). Invite form, General access and Copy sections are visible only to the owner.
  - Non-owner: a single row with the owner (avatar 36px, name) under “People with access”.
  - Owner: rows for every non-`anyone` permission (an owner row is synthesised if missing): avatar
    (`photoLink` or fallback grey person SVG), name (`displayName`/email/`Unknown user`), email. Owner rows and
    rows without id show a role pill (`Owner`/`Editor`/`Viewer`); others get a role `<select>` (Viewer/Editor,
    `aria-label="Role for <name>"`) and a `Remove` button.
  - `anyone` permission present → access `anyone`, role select shown with its role (`writer`→Editor else Viewer).
  - Permission load error → sections by `driveOwnedByMe !== false`; non-owner gets the owner row.
- Actions (all guarded by owner; success statuses use `{icon:'check'}`, then re-render):
  - Invite: `POST files/{id}/permissions?sendNotificationEmail=true` `{type:'user', role, emailAddress}`; button
    `Sending...`; `Invitation sent` (input cleared) / `Failed to share`.
  - Row role change: `PATCH files/{id}/permissions/{pid}` `{role}`; `Access updated` / `Failed to update access`
    (select reverted).
  - Remove: `DELETE files/{id}/permissions/{pid}`; `Access revoked` / `Failed to revoke access`.
  - General access change → `setDriveLinkPermission(id, anyone ? roleSelect.value : 'restricted')`;
    `Link access updated` / `Failed to update access`. Role change (only when `anyone`) → `Link role updated` /
    `Failed to update role`.
  - Copy link: button text `Preparing public link...` (disabled; original innerHTML restored after);
    requested role = role select (`writer` or `reader`, never downgraded); `setDriveLinkPermission`; re-list
    permissions; require an `anyone` permission with role reader/commenter/writer (else `Google Drive did not
    confirm public access. Your organisation may block public sharing.`) and writer when requested (`Google Drive
    did not confirm editor access for this link.`); `GET files/{id}?fields=id,name,resourceKey`; build the §8.1 link;
    `navigator.clipboard.writeText`; set access/role selects; status `Editor link copied. Recipients sign in to save
    changes to Drive.` or `Public viewer link copied — no sign-in required.`; errors show the message or
    `Could not create a public link.`
- `setDriveLinkPermission(id, role)` (app:9060): list permissions; `restricted` → `DELETE` the `anyone` permission
  if any; existing `anyone` with a different role → `PATCH {role}`; none →
  `POST files/{id}/permissions?fields=id,type,role` `{type:'anyone', role, allowFileDiscovery:false}`.
- Styling essentials: overlay `rgba(10,10,10,.56)` + 5px blur, padding 20; container `min(92vw,520px)`, max-height
  `min(88svh,720px)`, radius 20, enter `translateY(14px) scale(.98)` → none in 0.22 s `cubic-bezier(.2,.8,.2,1)`;
  invite grid `minmax(0,1fr) minmax(104px,116px) auto` (≤600px: `minmax(0,1fr) 112px` with Send spanning
  `1 / -1`); inputs 48px radius 14; Send orange; link group 60px rounded box with 40px orange-tinted globe;
  Copy link orange-tinted, right-aligned. Smoke test forbids `share-modal-subtitle`, `share-section-description`,
  `share-link-access-hint`, `share-copy-copy`.

---------------------------------------------------------------------------------------------------------

## 10. Drive API catalogue for these flows

Base `https://www.googleapis.com/drive/v3`. Authenticated calls go through `driveFetch` (app:7798):
`Authorization: Bearer` from `ensureDriveToken(false)` (deadline includes auth), default timeout 60 s, JSON and
error bodies buffered, 401 → drop token and retry once (then mark “reconnect needed”), auth-looking errors mark
reconnect, non-OK → `Error("Drive API error <status>: <body>")` with `code/status`, abort → `Drive request aborted`,
timeout → `Drive request timed out`. Any successful response calls `InhouseOfflineLibrary.networkAvailable()`.

| Purpose | Method & path | Query / body | Ref |
|---|---|---|---|
| Profile | GET `/about` | `fields=user(displayName,emailAddress,photoLink)` | app:7044 |
| Recents / Starred / Shared / Folder lists | GET `/files` | §3.6.2 | app:10739-10835 |
| Folder picker list | GET `/files` | `'<id>' in parents and folder and trashed=false`, `orderBy=name`, `pageSize=200`, `files(id,name,folderColorRgb)` | app:12114 |
| Bin list | GET `/files` | §6.5 | app:12238 |
| Quick-access folder / PDF lookup | GET `/files` | §7.3 | app:8842, 8940 |
| `Inhouse Notes` folder lookup | GET `/files` | `name='Inhouse Notes' and folder and trashed=false`, `fields=files(id,name)` | app:8815 |
| Create folder | POST `/files` | `?fields=id,name,mimeType,starred,ownedByMe,folderColorRgb` (New folder) / `?fields=id,name` (quick access) / none (`Inhouse Notes`) | app:8894, 8876, 8824 |
| Star | PATCH `/files/{id}?fields=id,starred` | `{starred}` | app:9616 |
| Folder colour | PATCH `/files/{id}?fields=id,folderColorRgb` | `{folderColorRgb}` | app:9634 |
| Move | GET `/files/{id}?fields=parents`; PATCH `/files/{id}?addParents&removeParents` | no body | app:12182 |
| Trash / restore | PATCH `/files/{id}[?addParents]` | `{trashed:true|false}` | app:12358, 12334 |
| Rename (title) | PATCH `/files/{id}` | `{name:'<base>.pdf'}` | app:5944 |
| Copy | POST `/files/{id}/copy` | `{name, parents?}` | app:11691 |
| Path resolution | GET `/files/{id}?fields=parents`; GET `/files/{id}?fields=id,name,parents` | cached per folder | app:11921-11973 |
| Open metadata | GET `/files/{id}` | `fields=capabilities(canEdit),ownedByMe,owners(...),resourceKey,headRevisionId,modifiedTime,version,md5Checksum,size,parents[&resourceKey]`, `Cache-Control:no-cache`, 7 s | app:11414 |
| Revision meta | GET `/files/{id}` | `fields=headRevisionId,modifiedTime,version,md5Checksum,size` | app:25925 |
| Download | GET `/files/{id}/revisions/{rev}?alt=media` or `/files/{id}?alt=media` | `[&resourceKey]`, 300 s | app:25968 |
| Shared-link file info | GET `/files/{id}` | `fields=id,name,resourceKey[&resourceKey]` | app:6676-6681 |
| Owner info | GET `/files/{id}` | `fields=ownedByMe,capabilities(canEdit),owners(displayName,emailAddress,photoLink),resourceKey` | app:9097 |
| Permissions | GET/POST/PATCH/DELETE `/files/{id}/permissions[/{pid}]` | §9.1 | app:9046-9095 |
| Create PDF | POST `upload/drive/v3/files?uploadType=resumable&fields=id,headRevisionId,modifiedTime,version` | metadata + PDF | app:26466 |
| Device upload | POST `upload/drive/v3/files?uploadType=resumable&fields=id,name,mimeType,parents,modifiedTime,version` | metadata + file | app:11765 |
| Offline catalogue | GET `/files/root?fields=id`; GET `/files` (paged, `pageSize=1000`, `nextPageToken`) | `trashed=false and (pdf or folder)`, `fields=nextPageToken,files(id,name,mimeType,parents,modifiedTime,createdTime,thumbnailLink,starred,ownedByMe,resourceKey,folderColorRgb)` | lib:321 |
| Connectivity probe | GET `/about?fields=kind` (unauthenticated, `credentials:'omit'`, 5 s) | any status >0 = online | lib:92 |
| Thumbnails | `thumbnailLink` (`=s440` / `&sz=w312-h440`), `credentials:'omit'` | Cache Storage | app:9131 |
| Public | `drive.usercontent.google.com/download`, `drive.google.com/uc`, `drive.google.com/file/d/{id}/preview` iframe | §8 | app:8041-8160 |
| Ask to edit | `window.open('https://drive.google.com/open?id=<id>')` | — | app:11719 |

CSP (html:84) must keep: `img-src … https://*.googleusercontent.com https://www.googleapis.com https://drive.google.com`;
`connect-src … https://www.googleapis.com https://drive.google.com https://drive.usercontent.google.com https://*.googleusercontent.com`;
`frame-src 'self' https://accounts.google.com https://drive.google.com`.

---------------------------------------------------------------------------------------------------------

## 11. Persistence, navigation and back behaviour

| Key | Store | Value / use |
|---|---|---|
| `inhouse-theme` | localStorage | `dark`/`light` (boot + `applyTheme`) |
| `notebook-view-mode-v1` | localStorage | `drive`/`editor`, written on every view switch, **never used for routing** |
| `ihn-home-tab` | sessionStorage | last Home tab (restored per browser tab) |
| `drive-session-v1` | localStorage | `{fileId, offlineLocalId, fileName, folderId, parentIds, folderKind, exportName, autosave, canEdit, ownedByMe, ownerName, ownerEmail, ownerPhoto, resourceKey, lastDriveSaveAt, headRevisionId, revisionVersion, lastKnownModified}` (app:21788) |
| `drive-hint-email`, `drive-profile-v1` | localStorage | signed-in account and cached profile (also make boot “returning”) |
| `drive-token-v1`, `drive-remember-v1`, `drive-refresh-token-v1`, `google-scope-version-v1`, `google-granted-scopes-v1`, `drive-consent-granted`, `drive-pkce-verifier-v1` | localStorage | auth (auth spec) |
| `inhouseNotesAppMode` | localStorage | `1` once opened as the Android app (`?inhouse_app=1`, `?app=1`, or UA `InhouseNotesApp/x`); `0/false` clears it |
| `touchDrawEnabled` | localStorage | editor menu toggle |
| `ihn-network-offline` | sessionStorage | offline-library connectivity |
| `ihn-release-reload` | sessionStorage | release reload guard |
| `inhouse-offline-v1` (IndexedDB) | `entries`, `documents`, `document-index`, `outbox`, `settings` (`home-views`, `drive-root`, …) | catalogue, saved Home views, offline copies (`lastOpenedAt`, `pinned`) |
| `inhouse-original-pdfs-v1` (IndexedDB) | `drive-open-cache` | fast reopen cache |
| `inhouse-media-<account>` | Cache Storage | thumbnails/avatars |

Navigation: there is **no History API integration** (no `pushState`/`popstate`; only `replaceState` to strip the
OAuth hash). Browser/Android back is not intercepted; in-app back = `#btn-folder-view-back`, breadcrumbs, Escape,
same-tab nav click, editor logo. Folder navigation state is memory-only (lost on reload; reload lands on the Home
tab saved in sessionStorage at the root). Share links persist in the URL until changed by the user.

---------------------------------------------------------------------------------------------------------

## 12. Quirks to reproduce (or consciously change)
1. Public link import always fails (`fetchWithDeadline` argument order) → anonymous iframe preview, re-created ~every 7.5 s by polling (§8.3-8.4).
2. `#drive-status` is hidden by CSS; error/upload messages force it visible permanently (§3.4).
3. Starred ignores the search term; cached (offline-first) Recents/Starred/Shared also ignore it (§3.4).
4. Lists never page: >24 recents/shared results, >12 starred, >100 per folder are truncated (§3.6.2).
5. New-PDF picker pre-fills the previous document’s name; copy pre-fills `Copy of X.pdf` with a visible `.pdf` suffix (§6.2).
6. Create-menu ArrowUp moves forward like ArrowDown (§6.1).
7. Quick-access and `Inhouse Notes` folder lookups match a folder of that name anywhere in Drive (§7.3).
8. `notebook-view-mode-v1` is written but never read for routing; cold boot always opens Home (§1.2).
9. `public=1` in links is decorative; `mode=edit` alone grants the writer flow (§8.1).
10. Auto-refresh only refreshes Recents and the current folder (§3.12).

---------------------------------------------------------------------------------------------------------

## 13. CSS / asset port map (copy verbatim; same cascade order)
`index.html`: 149-179 (reset, app mode), 180-455 (tokens, header, ≤720 header), 1292-1338 (read-only banner),
1339-1449 (buttons), 1476-1590 (public preview), 2871-2912 (theme toggle), 2943-3097 (modal base, confirm slider,
modal input), 4251-4338 (loading overlay), 4340-4576 (welcome), 4577-5003 (Home header, download modal, profile,
pull-to-refresh), 5004-5781 (toolbar, sections, cards, dark overrides, dropdown, colour popup, folder picker, bin,
legacy folder card, empty), 6354-6586 (skeletons, doc title, legacy modal overrides), 6587-7801 (share modal,
≤720 Home/editor overrides, hover-none, reduced motion), 7802-8950 (Home tabs, A4 sheets, manila folders, folder
view, transition, nav). `offline-v5.css` (whole file). Tokens: `--bg-primary #f5f5f0/#151515`,
`--bg-secondary #fff/#1c1c1c`, `--text-primary #1a1a1a/#f5f5f5`, `--text-secondary #666/#a2a2a2`,
`--accent-orange #E07A3C`, `--accent-yellow #ffde00`; fonts Comfortaa (brand, folder names, headings) and DM Sans
(UI) from `vendor/fonts/ui.css`. Skeleton shimmer: `drive-skeleton-slide` (translateX −240→240px) 1.2 s linear,
**25 iterations then stops**; all shimmers off under reduced motion. Assets: `icons/welcome-pages.svg`.

---------------------------------------------------------------------------------------------------------

## 14. Tests pinning this behaviour
- `tests/e2e/home-action-menu.spec.mjs` — file and folder actions exist only in the ⋮ menu: 4 buttons with icons (`offline` vs `color`), trash opens `#modal-trash-file`.
- `tests/e2e/folder-navigation.spec.mjs:42` — cached folder contents show during opening; nested open/close keeps content inside the moving folder; focus returns to the origin card.
- `…folder-navigation.spec.mjs:77` — first frame of the ghost lid equals the card (title, rule, ⋮ within 1px); interior shadows/grain identical to landed.
- `…folder-navigation.spec.mjs:124` — zoom is `running`+`pending` and scene unprepared in the tap’s own task (open and back).
- `…folder-navigation.spec.mjs:140` — label backdrop blur is `none` while moving and `blur(1.5px)` after landing; folder sheets have no background image.
- `…folder-navigation.spec.mjs:155` — rapid back/reopen/tab switches leave no stale folder, ghost or hidden card.
- `…folder-navigation.spec.mjs:176` — reduced motion: no animation object, immediate nesting, focus restored.
- `…folder-navigation.spec.mjs:188` — slow Drive responses neither block cached contents nor overwrite a newer navigation.
- `…folder-navigation.spec.mjs:440/460/475/499` — zoom fills the content viewport at every nesting level and width; breadcrumb jumps zoom to the ancestor card; the lid reveals real children; cached cards stay opaque at hand-off.
- `…folder-navigation.spec.mjs:536/564/585/704/795/874` — scrolled nested return restores card/position; focus moved to search is preserved; dense folders paint intermediate frames on slow CPU; compositor keeps animating with a blocked main thread; interrupted warmup prepares once; 120 Hz interpolation on accelerated layers.
- `tests/e2e/folder-rust.spec.mjs` — Rust plans motion and bounded layout reads for 1000 cards; deep scrolling/back keeps the last item usable; mixed aspect ratios stay hydrated; decoded thumbnails rebuild row offsets; no WASM still navigates.
- `tests/rust-folder.test.mjs` — folder_motion/fade/rows contracts (endpoints, invalid input rejection, ordered fade offsets, row ranges incl. partial rows and overscan, disposal, memory growth, JS fallback/old ABI).
- `tests/e2e/idle-animations.spec.mjs:24,60` + `tests/idle-animations.test.mjs` — Home at rest runs no animation; pull spinner spins only while visible; placeholders slide on the compositor and stop after 25 cycles; reduced motion stops shimmers.
- `tests/e2e/preview-stability.spec.mjs` — rotating thumbnail links keep the same card/image/pixels; a changed preview decodes before replacing and a failed refresh keeps the good image.
- `tests/e2e/offline-library.spec.mjs` — expired remembered session never paints the sign-in screen; offline folders/pins survive reload; unavailable documents are grey and cannot replace the open document; offline folder/document creation via visible controls.
- `tests/e2e/offline-library-cached.spec.mjs:27,81,121` — online and cold-offline Home show the same lists, folder colours, previews, avatar and fonts; cached Home paints while auth/listing stall; private images stay visible offline.
- `tests/e2e/offline-reconnect.spec.mjs:9,27,174` — local/remote folder refreshes keep deterministic order; opening from Home doesn’t checkpoint the stale editor; Recents order stable across cached/network renders with pending aliases.
- `tests/offline-library-index.test.mjs` — directory index, aliases, search, ordering, saved Home views, recently-opened ordering immune to upload timestamp changes.
- `tests/e2e/offline-connectivity.spec.mjs:28` — failed uploads show the persisted reason on the card badge without opening the editor.
- `tests/e2e/create-upload-menu.spec.mjs` — device PDF is durable offline in the chosen folder before the picker closes; non-PDF uploads keep MIME, bytes, name and location.
- `tests/e2e/home-save-navigation.spec.mjs` — Home exit: unchanged docs need no Drive request; offline edits persist; Home never waits on stalled auth/uploads; failed checkpoint keeps the editor open.
- `tests/e2e/document-lifecycle.spec.mjs:79,113,226` — Home stops all document-scoped loops; reopening restarts them; Home cancels pending background suspension.
- `tests/release-v5-smoke.mjs` — `APP_VERSION` source + 3 `data-app-version` occurrences; pull-to-refresh has no width gate and checks `maxTouchPoints`; `initDriveAuth` before storage restore; goHome contract; share modal markup/CSS and “never downgrade editor link”; public preview markup/CSS strings, gate copy, polling interval expression; `showDriveHome` → `closeDocumentScopedWork`.
- `tests/e2e/release-check.spec.mjs` — published-release reload once, never while the editor is open.

---------------------------------------------------------------------------------------------------------

## 15. Ten invariants (summary)
1. Boot decides Home vs Welcome before app code runs (`__INHOUSE_BOOT_SIGNED_IN__`, skeletons pre-filled); cold boot never reopens the editor; share links override.
2. Home = 4 tabs (`home/starred/shared/files`) driven by `#drive-home[data-home-tab]`, persisted in `sessionStorage['ihn-home-tab']`; nav shows the icon only on the active pill.
3. Lists come from fixed Drive queries (Recents 24→12 non-Diary/Agenda PDFs, Starred 12 incl. folders, Shared 24 `ownedByMe=false`, folder 100+100, no paging) and are painted first from the offline catalogue; Recents are ordered by local `lastOpenedAt`.
4. Cards are DOM-diffed by `id|name|starred|folderColorRgb|mimeType`; thumbnails (`=s440`) are cached per account and only reloaded when `modifiedTime`/local preview changes, decoded off-screen and never blanked.
5. Files are A4 sheets (ratio from thumbnail, clamped 0.35–2.2) with a tone-sampled label; folders are manila art coloured by the documented HSL algorithm (`#8f8f8f`/invalid → `#e3c58e`), theme-aware.
6. Every action lives in the ⋮ menu (star, offline|colour, move, bin), moved to `<body>` while open; mutations PATCH Drive then quiet-refresh without skeletons.
7. Opening a folder plays one compositor-only scene: ghost laid out at final size, zoom `cubic-bezier(.55,0,.15,1)` 600/560 ms, lid to −112°, labels fade, interior fade joins at `zoom.ready`; reduced motion = instant; any navigation finishes the previous scene.
8. Opened-folder interiors use the pastel/near-black tint, paint cached contents immediately, hydrate only viewport cards (Rust row plans), and drop responses from superseded navigations.
9. Open order is offline copy → open cache (modified ≥ card) → stable checksummed Drive download; failures return to Home with `Could not open this document. Try again.`
10. Sharing: owner-only invite/general-access/copy; Copy link sets an `anyone` permission with the selected role and copies `?drive=…&public=1&role=…&name=…[&mode=edit][&resourcekey=…]`; signed-out recipients get the read-only Drive iframe preview (writer links add a Sign-in gate).
