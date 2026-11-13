//! Shell (ARCHITECTURE §2.3): boot routing between Welcome, Home and the editor, theme, the welcome sheet, both
//! profile menus, the Soon toast, the loading overlay, the status lines, fullscreen, connectivity and the global
//! click/keyboard handlers. Ported from 5.18.0 `app-v5.js`; each function names the one it reproduces.
//!
//! # What other modules use
//! - [`show_view`], [`view`], [`home_status`], [`status`], [`soon`], [`loading`], [`loading_home_save`],
//!   [`hide_loading`], [`close_profile_menus`], [`offline`], [`network_available`].
//! - [`signal`] / [`sig`]: notifications broadcast to every module as `Event::Msg` with `chan == sig::X`. They
//!   stand where 5.18.0 calls straight into another subsystem (`refreshDriveFiles()`, `closeDocumentScopedWork()`,
//!   `repaintDriveFolders()` …), so a later unit hooks in from its own `event` function.
//! - [`fx`]: the effect builders of families 3.3, 3.6, 3.8 and 3.10; [`json`]: a small JSON reader.

pub mod fx;
pub mod json;
#[cfg(test)]
pub(crate) mod testkit;

use crate::abi::{self, ety, ev, life, listen, mode, ptr, ret, tag, Event, Node};
use crate::dom::Prop;
use crate::testapi::json_str;
use crate::{auth, App, Host, LsOp, Mod, Ui};
use ihn_core::codec::Writer;

include!("consts.rs");

/// Signals: `Event::Msg { chan, data }` broadcast with [`signal`]. `data` is UTF-8 text.
pub mod sig {
    /// The auth state or its UI changed (after 5.18.0 `updateDriveAuthUI`). Read it with the `auth` accessors.
    pub const AUTH: u8 = 0x20;
    /// 5.18.0 `refreshDriveFiles()`: list Drive again (sign-in, menu Refresh, boot with a token).
    pub const REFRESH: u8 = 0x21;
    /// Sign-out is about to happen. A module that cannot let go (an unsaved local copy) returns
    /// `ret::PREVENT_DEFAULT` after reporting why; sign-out is then abandoned.
    pub const SIGNING_OUT: u8 = 0x22;
    /// Signed out: drop every per-account state (caches, sessions, timers).
    pub const SIGNED_OUT: u8 = 0x23;
    /// The visible view changed; data = `welcome` | `home` | `editor`. Close view-scoped popups and work.
    pub const VIEW: u8 = 0x24;
    /// The Drive profile was loaded; data = the account e-mail.
    pub const ACCOUNT: u8 = 0x25;
    /// Connectivity changed; data = `1` (Drive reachable) | `0`.
    pub const NETWORK: u8 = 0x26;
    /// The theme changed; data = `dark` | `light`.
    pub const THEME: u8 = 0x27;
    /// Drive needs a new sign-in to keep syncing (5.18.0 `markDriveAuthNeedsReconnect`); data = the reason.
    pub const RECONNECT: u8 = 0x28;
    /// A profile menu opened; data = `home` | `editor` (the editor menu shows the document stats).
    pub const MENU: u8 = 0x29;
    /// [`super::status`] was called; data = the text. The save-indicator state machine (W3.1) follows it.
    pub const STATUS: u8 = 0x2a;
}

/// The three top-level views.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum View {
    #[default]
    Welcome,
    Home,
    Editor,
}

impl View {
    pub fn name(self) -> &'static str {
        match self {
            View::Welcome => "welcome",
            View::Home => "home",
            View::Editor => "editor",
        }
    }
}

/// What [`status`] shows on the editor's save indicator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    /// Class `saved`.
    Info,
    Saving,
    Error,
    /// One of 5.18.0's `SAVE_STATES`: `paused`, `building`, `uploading`, `syncing`, `saving`, `saved`, `error`.
    Phase(&'static str),
}

/// A shared link waiting to be opened (`?drive=<id>`); the link itself belongs to `doc::links`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PendingShare {
    /// The anonymous open already failed (5.18.0 `sharedLinkSignInPrompted`).
    pub open_failed: bool,
}

#[derive(Default)]
pub struct State {
    view: View,
    booted: bool,
    home_menu: bool,
    editor_menu: bool,
    sheet_open: bool,
    dark: bool,
    fullscreen: bool,
    soon: Node,
    loading_visible: bool,
    loading_home: bool,
    blur: bool,
    pending_share: Option<PendingShare>,
    // Connectivity. v5: offline-library-v5.js
    disconnected: bool,
    reachable: bool,
    epoch: u32,
    ping: u32,
    ping_epoch: u32,
    probe_delay: f64,
    probe_armed: bool,
    pub(crate) next_obj: u32,
    pub(crate) probe_log: Vec<(u8, String)>,
    pub(crate) probe: fx::Probe,
}

const fn id(n: u32) -> u32 { tag(Mod::Shell, n) }
const L_LOGIN: u32 = id(1);
const L_WELCOME_LOGIN: u32 = id(2);
const L_WELCOME_START: u32 = id(3);
const L_SHEET_BACKDROP: u32 = id(4);
const L_SHEET_KEY: u32 = id(5);
const L_SOON_DIARY: u32 = id(6);
const L_SOON_AGENDA: u32 = id(7);
const L_REFRESH: u32 = id(8);
const L_REFRESH_EDITOR: u32 = id(9);
const L_LOGOUT: u32 = id(10);
const L_LOGOUT_EDITOR: u32 = id(11);
const L_PROFILE: u32 = id(12);
const L_PROFILE_EDITOR: u32 = id(13);
const L_THEME: u32 = id(14);
const L_THEME_EDITOR: u32 = id(15);
const L_AVATAR: u32 = id(16); // ..=19
const L_DOC_CLICK: u32 = id(20);
const L_DOC_KEY: u32 = id(21);
const L_FULLSCREEN: u32 = id(22);
const L_FULLSCREEN_HOME: u32 = id(23);
const L_POINTER_UP: u32 = id(24);
const L_POINTER_CANCEL: u32 = id(25);
const L_ACT_TOUCH: u32 = id(26);
const L_ACT_WHEEL: u32 = id(27);
const L_ACT_KEY: u32 = id(28);
const T_SOON: u32 = id(40);
const T_LOADING_FADE: u32 = id(41);
const T_LOADING_CLEANUP: u32 = id(42);
const T_FULLSCREEN_RESIZE: u32 = id(43);
const T_PROBE: u32 = id(44);

const THEME_KEY: &str = "inhouse-theme";
const NETWORK_KEY: &str = "ihn-network-offline";
const AVATARS: [Node; 4] = [Ui::DRIVE_PROFILE_AVATAR, Ui::DRIVE_PROFILE_AVATAR_MENU, Ui::DRIVE_PROFILE_AVATAR_EDITOR, Ui::DRIVE_PROFILE_AVATAR_MENU_EDITOR];
// v5: offline-library-v5.js PROBE_BASE_MS / PROBE_MAX_MS
const PROBE_BASE_MS: f64 = 30000.0;
const PROBE_MAX_MS: f64 = 300000.0;
// v5: app-v5.js:SAVE_STATES
const SAVE_STATES: [&str; 7] = ["paused", "building", "uploading", "syncing", "saving", "saved", "error"];

// ---- signals and delivery ---------------------------------------------------------------------------------------

/// Broadcasts a [`sig`] notification to every module; returns the OR of their `abi::ret` bits.
pub fn signal(app: &mut App, host: &mut dyn Host, sig: u8, data: &str) -> u32 {
    let mut w = Writer::default();
    w.u8(sig);
    w.u32(0);
    w.raw(data.as_bytes());
    app.event(host, abi::kind::MESSAGE, &w.buf)
}

/// Completes request `req` of another module: its `event` receives `Event::Io { req, status, obj, data }`.
pub fn deliver(app: &mut App, host: &mut dyn Host, req: u32, status: i32, obj: u32, data: &[u8]) {
    if let Some(module) = crate::app::MODULES.get(abi::tag_mod(req)) {
        (module.event)(app, host, &Event::Io(abi::Io { req, status, obj, data }));
    }
}

// ---- views ------------------------------------------------------------------------------------------------------

pub fn view(app: &App) -> View { app.st.shell.view }

/// Switches the visible view.
// v5: app-v5.js:showDriveHome / showWelcomeView / showEditorView
pub fn show_view(app: &mut App, host: &mut dyn Host, view: View) {
    let out = host.out();
    out.class(Ui::WELCOME_VIEW, "hidden", view != View::Welcome);
    if view == View::Welcome {
        out.prop(Ui::WELCOME_SHEET, "hidden", Prop::Bool(true));
        out.prop(Ui::WELCOME_SHEET_BACKDROP, "hidden", Prop::Bool(true));
        app.st.shell.sheet_open = false;
    }
    out.class(Ui::DRIVE_HOME, "hidden", view != View::Home);
    out.class(Ui::EDITOR_VIEW, "hidden", view != View::Editor);
    if view != View::Editor {
        // v5: closeTimelinePanelIfOpen — editor overlays never linger outside the editor.
        out.class(Ui::TIMELINE_PANEL, "visible", false);
        out.attr(Ui::TIMELINE_PANEL, "aria-hidden", Some("true"));
        close_profile_menus(app, host);
    }
    app.st.shell.view = view;
    // The save-details and presence popups, the public preview, the sign-in gate, document-scoped work, Drive sync
    // timers and the Home auto refresh follow this signal in their own modules.
    signal(app, host, sig::VIEW, view.name());
}

/// The Home status line (`#drive-status`).
pub fn home_status(host: &mut dyn Host, text: &str) { host.out().text(Ui::DRIVE_STATUS, text); }

/// The editor's save indicator: class, tooltip and label. The save-state machine (debounced confirmation, save
/// details, timestamp) belongs to `doc::save_state`, which follows [`sig::STATUS`].
// v5: app-v5.js:showStatus
pub fn status(app: &mut App, host: &mut dyn Host, text: &str, kind: Status) {
    let out = host.out();
    let class = match kind {
        Status::Info => "saved",
        Status::Saving => "saving",
        Status::Error => "error",
        Status::Phase(p) => p,
    };
    for s in SAVE_STATES {
        out.class(Ui::SAVE_INDICATOR, s, s == class);
    }
    out.prop(Ui::SAVE_INDICATOR, "title", Prop::Str(text));
    out.attr(Ui::SAVE_INDICATOR, "aria-label", Some(text));
    signal(app, host, sig::STATUS, text);
}

/// "Soon": the answer of the features 5.18.0 removed (Diary, Agenda, Scan).
// v5: app-v5.js:showSoonNotice
pub fn soon(app: &mut App, host: &mut dyn Host) {
    let out = host.out();
    if app.st.shell.soon.is_none() {
        app.st.shell.soon = out.insert_html(Node::BODY, Node::NONE, "<div id=\"soon-toast\" role=\"status\"></div>");
    }
    let toast = app.st.shell.soon;
    out.text(toast, "Soon");
    out.class(toast, "visible", true);
    app.sched.timer(out, T_SOON, 1800.0);
}

// ---- loading overlay --------------------------------------------------------------------------------------------

fn loading_begin(app: &mut App, host: &mut dyn Host, text: &str, home: Option<bool>) {
    let out = host.out();
    app.sched.clear(out, T_LOADING_CLEANUP);
    // v5: cancelLoadingFadeOut
    app.sched.clear(out, T_LOADING_FADE);
    out.class(Ui::LOADING_OVERLAY, "fading-out", false);
    if let Some(home) = home {
        out.class(Ui::LOADING_OVERLAY, "home-save-transition", home);
        app.st.shell.loading_home = home;
    }
    out.text(Ui::LOADING_TEXT, text);
    out.attr(Ui::LOADING_OVERLAY, "aria-busy", Some("true"));
    out.class(Ui::LOADING_OVERLAY, "visible", true);
    app.st.shell.loading_visible = true;
}

/// The dark loading overlay with a spinner and `text`.
// v5: app-v5.js:showLoading
pub fn loading(app: &mut App, host: &mut dyn Host, text: &str) { loading_begin(app, host, text, Some(false)); }

/// The opaque "going Home" overlay (5.18.0 default text: `Saving changes to Drive...`).
// v5: app-v5.js:showHomeSaveLoading
pub fn loading_home_save(app: &mut App, host: &mut dyn Host, text: &str) { loading_begin(app, host, text, Some(true)); }

// v5: app-v5.js:hideLoading
pub fn hide_loading(app: &mut App, host: &mut dyn Host) {
    let (out, st) = (host.out(), &mut app.st.shell);
    if st.loading_visible {
        out.class(Ui::LOADING_OVERLAY, "fading-out", true);
        // Matches the overlay's 0.3s opacity/visibility transition.
        app.sched.timer(out, T_LOADING_FADE, 360.0);
    }
    out.class(Ui::LOADING_OVERLAY, "visible", false);
    out.attr(Ui::LOADING_OVERLAY, "aria-busy", Some("false"));
    st.loading_visible = false;
    if st.loading_home {
        app.sched.timer(out, T_LOADING_CLEANUP, 180.0);
    } else {
        out.class(Ui::LOADING_OVERLAY, "home-save-transition", false);
    }
}

// ---- profile menus ----------------------------------------------------------------------------------------------

fn menu(app: &mut App, host: &mut dyn Host, editor: bool, open: bool) {
    let st = &mut app.st.shell;
    let (flag, menu, button) = if editor {
        (&mut st.editor_menu, Ui::DRIVE_PROFILE_MENU_EDITOR, Ui::BTN_DRIVE_PROFILE_EDITOR)
    } else {
        (&mut st.home_menu, Ui::DRIVE_PROFILE_MENU, Ui::BTN_DRIVE_PROFILE)
    };
    if std::mem::replace(flag, open) != open {
        host.out().class(menu, "hidden", !open);
        host.out().attr(button, "aria-expanded", Some(if open { "true" } else { "false" }));
    }
}

// v5: app-v5.js:closeDriveProfileMenus
pub fn close_profile_menus(app: &mut App, host: &mut dyn Host) {
    menu(app, host, false, false);
    menu(app, host, true, false);
}

// v5: app-v5.js:toggleDriveProfileMenu
fn toggle_menu(app: &mut App, host: &mut dyn Host, editor: bool) {
    let open = !(if editor { app.st.shell.editor_menu } else { app.st.shell.home_menu });
    menu(app, host, !editor, false);
    menu(app, host, editor, open);
    if open {
        signal(app, host, sig::MENU, if editor { "editor" } else { "home" });
    }
}

// ---- theme ------------------------------------------------------------------------------------------------------

// v5: app-v5.js:applyTheme
fn apply_theme(app: &mut App, host: &mut dyn Host, dark: bool, persist: bool) {
    let name = if dark { "dark" } else { "light" };
    host.out().attr(Node::HTML, "data-theme", Some(name));
    app.st.shell.dark = dark;
    signal(app, host, sig::THEME, name);
    if persist && host.ls(LsOp::Set, THEME_KEY, Some(name)).is_none() {
        host.log(3, "Failed to save theme:");
    }
    // v5: updateThemeToggleState
    host.out().prop(Ui::THEME_TOGGLE, "checked", Prop::Bool(dark));
    host.out().prop(Ui::THEME_TOGGLE_EDITOR, "checked", Prop::Bool(dark));
}

fn stored_theme(host: &mut dyn Host) -> Option<bool> {
    match host.ls(LsOp::Get, THEME_KEY, None).as_deref() {
        Some("dark") => Some(true),
        Some("light") => Some(false),
        _ => None,
    }
}

/// `html[data-theme] == 'dark'`.
pub fn dark(app: &App) -> bool { app.st.shell.dark }

// ---- fullscreen -------------------------------------------------------------------------------------------------

// v5: app-v5.js:handleFullscreenChange / updateFullscreenButton
fn fullscreen_changed(app: &mut App, host: &mut dyn Host) {
    let (out, active) = (host.out(), app.st.shell.fullscreen);
    // In app mode both buttons were removed at boot (v5: `document.getElementById` finds nothing).
    if !app.cfg.has(mode::APP) {
        let label = if active { "Exit fullscreen" } else { "Fullscreen" };
        for button in [Ui::BTN_FULLSCREEN, Ui::BTN_DRIVE_FULLSCREEN] {
            out.prop(button, "title", Prop::Str(label));
            out.attr(button, "aria-label", Some(label));
            out.class(button, "active", active);
            out.html(button, if active { FULLSCREEN_ICON_EXIT } else { FULLSCREEN_ICON_ENTER });
        }
    }
    // Allow layout to settle before recalculating layout-dependent positions.
    app.sched.timer(out, T_FULLSCREEN_RESIZE, 50.0);
}

// ---- connectivity -----------------------------------------------------------------------------------------------

/// Drive is known to be unreachable. v5: offline-library-v5.js:offline / app-v5.js:driveIsOffline
pub fn offline(app: &App) -> bool {
    let st = &app.st.shell;
    st.disconnected || (!app.cfg.has(mode::ONLINE) && !st.reachable)
}

// v5: offline-library-v5.js:connectivity
fn connectivity(app: &mut App, host: &mut dyn Host, reachable: bool) {
    let before = offline(app);
    let st = &mut app.st.shell;
    st.reachable = reachable;
    st.disconnected = !reachable;
    host.ls(LsOp::SessionSet, NETWORK_KEY, Some(if reachable { "0" } else { "1" }));
    let changed = before != offline(app);
    if changed {
        signal(app, host, sig::NETWORK, if reachable { "1" } else { "0" });
    }
    if reachable {
        stop_probes(app, host);
    } else if changed {
        schedule_probe(app, host);
    }
}

// v5: offline-library-v5.js:stopOfflineProbes
fn stop_probes(app: &mut App, host: &mut dyn Host) {
    app.sched.clear(host.out(), T_PROBE);
    app.st.shell.probe_armed = false;
    app.st.shell.probe_delay = PROBE_BASE_MS;
}

// v5: offline-library-v5.js:scheduleOfflineProbe — a growing interval, only while visible and offline.
fn schedule_probe(app: &mut App, host: &mut dyn Host) {
    if app.cfg.has(mode::HEADLESS) || !offline(app) || app.cfg.has(mode::HIDDEN) {
        app.sched.clear(host.out(), T_PROBE);
        app.st.shell.probe_armed = false;
        return;
    }
    app.st.shell.probe_armed = true;
    app.sched.timer(host.out(), T_PROBE, app.st.shell.probe_delay);
}

// v5: offline-library-v5.js:probeConnection — probe the upload service itself; an unauthenticated 401/403 is
// still proof that Drive is reachable.
fn probe_connection(app: &mut App, host: &mut dyn Host) {
    if app.st.shell.ping != 0 {
        return;
    }
    let req = app.req(Mod::Shell);
    app.st.shell.ping = req;
    app.st.shell.ping_epoch = app.st.shell.epoch;
    fx::fetch(host.out(), req, &fx::Fetch {
        url: "https://www.googleapis.com/drive/v3/about?fields=kind",
        flags: fx::fetch_flag::NO_STORE | fx::fetch_flag::OMIT_CREDENTIALS | fx::fetch_flag::HEAD_ONLY,
        deadline_ms: 5000.0,
        ..Default::default()
    });
}

/// An authenticated Drive response arrived: it outranks an older failed probe and a stale `navigator.onLine`.
// v5: offline-library-v5.js:networkAvailable
pub fn network_available(app: &mut App, host: &mut dyn Host) {
    app.st.shell.epoch += 1;
    if app.st.shell.disconnected || !app.st.shell.reachable {
        connectivity(app, host, true);
    }
}

// ---- shared links (routing only) --------------------------------------------------------------------------------

pub fn pending_share(app: &App) -> Option<PendingShare> { app.st.shell.pending_share }
/// `doc::links` reports the state of the `?drive=` link (None once it opened or was dropped).
pub fn set_pending_share(app: &mut App, share: Option<PendingShare>) { app.st.shell.pending_share = share; }

// ---- boot -------------------------------------------------------------------------------------------------------

// v5: app-v5.js:init (the shell's part, in 5.18.0's order)
pub fn init(app: &mut App, host: &mut dyn Host) {
    // v5: OFFLINE_WORKER runs InhouseOfflineApp.worker() instead of init(): no UI.
    if app.cfg.has(mode::HEADLESS) {
        return;
    }
    let st = &mut app.st.shell;
    st.probe_delay = PROBE_BASE_MS;
    st.disconnected = host.ls(LsOp::SessionGet, NETWORK_KEY, None).as_deref() == Some("1");
    // v5: applyInhouseAppMode
    if app.cfg.has(mode::APP) {
        host.out().remove(Ui::BTN_DRIVE_FULLSCREEN);
        host.out().remove(Ui::BTN_FULLSCREEN);
    }
    // v5: initTheme — a stored choice wins, else the system scheme (followed while nothing is stored).
    let dark = stored_theme(host).unwrap_or(app.cfg.has(mode::DARK_SCHEME));
    apply_theme(app, host, dark, false);
    auth::restore_cached_profile(app, host);
    // v5: showFastInitialView
    if fx::query_param(fx::url_query(&app.cfg.url), "drive").is_some_and(|id| !id.is_empty()) {
        app.st.shell.pending_share = Some(PendingShare::default());
    }
    let stored = auth::has_stored_session(app, host);
    let first = match (app.st.shell.pending_share.is_some(), auth::token(app).is_some()) {
        (true, true) => View::Editor,
        (true, false) => View::Home,
        (false, _) if stored => View::Home,
        _ => View::Welcome,
    };
    show_view(app, host, first);
    // v5: InhouseOfflineLibrary.init → probeConnection()
    probe_connection(app, host);
    auth::init_drive_auth(app, host);
    host.out().attr(Node::HTML, "data-app-ready", Some("true"));
    listeners(app, host);
    fullscreen_changed(app, host);
    // v5: init "Handle shared drive links on load" — cold boot always lands on Home when signed in.
    let signed_in = auth::token(app).is_some() || auth::has_stored_session(app, host) || auth::local_library(app);
    let route = match (app.st.shell.pending_share.is_some(), auth::token(app).is_some()) {
        (true, true) => View::Editor,
        (true, false) => View::Home,
        (false, _) if signed_in => View::Home,
        _ => View::Welcome,
    };
    show_view(app, host, route);
    app.st.shell.booted = true;
}

// v5: app-v5.js:setupEventListeners (shell entries) and installWelcomeSheet
fn listeners(_app: &mut App, host: &mut dyn Host) {
    let out = host.out();
    for (node, listener) in [
        (Ui::BTN_DRIVE_LOGIN, L_LOGIN),
        (Ui::BTN_WELCOME_LOGIN, L_WELCOME_LOGIN),
        (Ui::BTN_WELCOME_START, L_WELCOME_START),
        (Ui::WELCOME_SHEET_BACKDROP, L_SHEET_BACKDROP),
        (Ui::BTN_DRIVE_MENU_DIARY, L_SOON_DIARY),
        (Ui::BTN_DRIVE_MENU_AGENDA, L_SOON_AGENDA),
        (Ui::BTN_DRIVE_MENU_REFRESH, L_REFRESH),
        (Ui::BTN_DRIVE_MENU_REFRESH_EDITOR, L_REFRESH_EDITOR),
        (Ui::BTN_DRIVE_MENU_LOGOUT, L_LOGOUT),
        (Ui::BTN_DRIVE_MENU_LOGOUT_EDITOR, L_LOGOUT_EDITOR),
        (Ui::BTN_DRIVE_PROFILE, L_PROFILE),
        (Ui::BTN_DRIVE_PROFILE_EDITOR, L_PROFILE_EDITOR),
        (Ui::BTN_FULLSCREEN, L_FULLSCREEN),
        (Ui::BTN_DRIVE_FULLSCREEN, L_FULLSCREEN_HOME),
        (Node::DOCUMENT, L_DOC_CLICK),
    ] {
        out.listen(node, "click", listener, 0);
    }
    out.listen(Ui::WELCOME_SHEET, "keydown", L_SHEET_KEY, 0);
    out.listen(Ui::THEME_TOGGLE, "change", L_THEME, 0);
    out.listen(Ui::THEME_TOGGLE_EDITOR, "change", L_THEME_EDITOR, 0);
    for (i, avatar) in AVATARS.into_iter().enumerate() {
        out.listen(avatar, "error", L_AVATAR + i as u32, 0);
    }
    out.listen(Node::DOCUMENT, "keydown", L_DOC_KEY, 0);
    // v5: clearTouchPressedState
    out.listen(Node::DOCUMENT, "pointerup", L_POINTER_UP, 0);
    out.listen(Node::DOCUMENT, "pointercancel", L_POINTER_CANCEL, 0);
    // v5: "Global hard interrupt" — fresh input postpones background work. Pointer events are recorded by
    // `App::dispatch`; a touch always arrives as pointer events too, so `touchmove` needs no listener.
    out.listen(Node::DOCUMENT, "touchstart", L_ACT_TOUCH, listen::CAPTURE | listen::PASSIVE);
    out.listen(Node::DOCUMENT, "wheel", L_ACT_WHEEL, listen::CAPTURE | listen::PASSIVE);
    out.listen(Node::DOCUMENT, "keydown", L_ACT_KEY, listen::CAPTURE);
}

fn close_sheet(app: &mut App, host: &mut dyn Host) {
    if std::mem::take(&mut app.st.shell.sheet_open) {
        let out = host.out();
        out.prop(Ui::WELCOME_SHEET, "hidden", Prop::Bool(true));
        out.prop(Ui::WELCOME_SHEET_BACKDROP, "hidden", Prop::Bool(true));
        out.focus(Ui::BTN_WELCOME_START, false);
    }
}

fn on_dom(app: &mut App, host: &mut dyn Host, e: &abi::DomEvent) -> u32 {
    match e.listener {
        L_LOGIN | L_WELCOME_LOGIN => auth::sign_in(app, host),
        L_WELCOME_START => {
            app.st.shell.sheet_open = true;
            let out = host.out();
            out.prop(Ui::WELCOME_SHEET, "hidden", Prop::Bool(false));
            out.prop(Ui::WELCOME_SHEET_BACKDROP, "hidden", Prop::Bool(false));
            out.focus(Ui::BTN_WELCOME_LOGIN, false);
        }
        L_SHEET_BACKDROP => close_sheet(app, host),
        L_SHEET_KEY if e.key == "Escape" => {
            close_sheet(app, host);
            return ret::PREVENT_DEFAULT;
        }
        L_SOON_DIARY | L_SOON_AGENDA => {
            close_profile_menus(app, host);
            soon(app, host);
        }
        L_REFRESH | L_REFRESH_EDITOR => {
            close_profile_menus(app, host);
            signal(app, host, sig::REFRESH, "");
        }
        L_LOGOUT | L_LOGOUT_EDITOR => {
            close_profile_menus(app, host);
            auth::sign_out(app, host);
        }
        L_PROFILE | L_PROFILE_EDITOR => {
            toggle_menu(app, host, e.listener == L_PROFILE_EDITOR);
            return ret::STOP_PROPAGATION;
        }
        L_THEME | L_THEME_EDITOR => apply_theme(app, host, e.has(ev::CHECKED), true),
        L_FULLSCREEN | L_FULLSCREEN_HOME => fx::fullscreen(host.out(), !app.st.shell.fullscreen),
        // v5: document click — close the profile menus unless the click is inside one of them or on its button.
        L_DOC_CLICK => {
            let inside = [Ui::DRIVE_PROFILE_MENU, Ui::DRIVE_PROFILE_MENU_EDITOR, Ui::BTN_DRIVE_PROFILE, Ui::BTN_DRIVE_PROFILE_EDITOR];
            if !inside.iter().any(|n| e.path.contains(*n)) {
                close_profile_menus(app, host);
            }
        }
        L_DOC_KEY if e.key == "Escape" => close_profile_menus(app, host),
        L_ACT_TOUCH | L_ACT_WHEEL | L_ACT_KEY => {
            // v5: isSaveDebugInteraction — the save indicator is watched, not worked on.
            let watched = [Ui::SAVE_INDICATOR, Ui::SAVE_STATUS_LABEL, Ui::SAVE_TIMESTAMP].iter().any(|n| e.path.contains(*n));
            if !watched && !(e.ty == ety::KEYDOWN && e.has(ev::COMPOSING)) {
                app.sched.activity(e.t);
            }
        }
        l if (L_AVATAR..L_AVATAR + 4).contains(&l) => {
            host.out().prop(AVATARS[(l - L_AVATAR) as usize], "src", Prop::Str(DEFAULT_AVATAR));
        }
        _ => {}
    }
    0
}

pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Dom(e) if e.listener == fx::PROBE_FILES => fx::probe_files(app, host, e.obj),
        Event::Dom(e) => return on_dom(app, host, &e),
        // v5: clearTouchPressedState — a tapped button must not keep its pressed look; blur it in the next frame.
        Event::Pointer(p) if matches!(p.listener, L_POINTER_UP | L_POINTER_CANCEL) && p.ptype != ptr::MOUSE => {
            app.st.shell.blur = true;
            app.sched.frame(host.out(), Mod::Shell);
        }
        Event::Frame(_) if std::mem::take(&mut app.st.shell.blur) => {
            let out = host.out();
            let pressed = out.query(Node::DOCUMENT, "button:focus, .btn:focus, .tool-btn:focus, .drive-profile-btn:focus, .presence-popup-close:focus");
            out.blur(pressed);
            out.release(pressed);
        }
        Event::Timer(T_SOON) => host.out().class(app.st.shell.soon, "visible", false),
        Event::Timer(T_LOADING_FADE) => host.out().class(Ui::LOADING_OVERLAY, "fading-out", false),
        Event::Timer(T_LOADING_CLEANUP) => {
            if !app.st.shell.loading_visible {
                host.out().class(Ui::LOADING_OVERLAY, "home-save-transition", false);
                app.st.shell.loading_home = false;
            }
        }
        Event::Timer(T_FULLSCREEN_RESIZE) => fx::dispatch_resize(host.out()),
        Event::Timer(T_PROBE) => {
            let st = &mut app.st.shell;
            st.probe_armed = false;
            st.probe_delay = (st.probe_delay * 2.0).min(PROBE_MAX_MS);
            probe_connection(app, host);
        }
        Event::Io(io) if io.req == app.st.shell.ping && io.req != 0 => {
            app.st.shell.ping = 0;
            if app.st.shell.ping_epoch == app.st.shell.epoch {
                connectivity(app, host, io.status > 0);
            }
            if offline(app) && !app.st.shell.probe_armed {
                schedule_probe(app, host);
            }
        }
        Event::Io(io) => {
            app.st.shell.probe.results.push((io.req, io.status, io.obj, io.data.to_vec()));
        }
        Event::Life(l) if app.st.shell.booted => match l.code {
            // v5: initTheme media listener — follow the system only while nothing is stored.
            life::SCHEME => {
                if stored_theme(host).is_none() {
                    apply_theme(app, host, l.a != 0.0, false);
                }
            }
            life::FULLSCREEN => {
                app.st.shell.fullscreen = l.a != 0.0;
                fullscreen_changed(app, host);
            }
            life::OFFLINE => {
                app.st.shell.epoch += 1;
                connectivity(app, host, false);
            }
            life::ONLINE => {
                stop_probes(app, host);
                probe_connection(app, host);
            }
            life::VISIBILITY => {
                if l.a != 0.0 {
                    app.sched.clear(host.out(), T_PROBE);
                    app.st.shell.probe_armed = false;
                } else {
                    app.st.shell.probe_delay = PROBE_BASE_MS;
                    probe_connection(app, host);
                }
            }
            _ => {}
        },
        Event::Msg(m) if app.cfg.has(mode::E2E) => {
            let text = String::from_utf8_lossy(m.data).into_owned();
            if m.chan >= sig::AUTH {
                app.st.shell.probe_log.push((m.chan, text));
            } else {
                app.st.shell.probe.messages.push((m.chan, text));
            }
        }
        _ => {}
    }
    0
}

pub fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, args: &str) -> Option<String> {
    let arg = json::Json::parse(args).unwrap_or(json::Json::Null);
    Some(match name {
        "state" => {
            let st = &app.st.shell;
            let signals: Vec<String> = st.probe_log.iter().map(|(c, d)| format!("[{c},{}]", json_str(d))).collect();
            format!(
                "{{\"view\":{},\"offline\":{},\"dark\":{},\"fullscreen\":{},\"menus\":[{},{}],\"signals\":[{}]}}",
                json_str(st.view.name()), offline(app), st.dark, st.fullscreen, st.home_menu, st.editor_menu, signals.join(",")
            )
        }
        "fx" => return fx::probe(app, host, args),
        "fx_results" => fx::probe_results(app),
        "soon" => {
            soon(app, host);
            "true".into()
        }
        "loading" => {
            if arg.get("home").is_some_and(json::Json::truthy) {
                loading_home_save(app, host, arg.text("text"));
            } else {
                loading(app, host, arg.text("text"));
            }
            "true".into()
        }
        "hide_loading" => {
            hide_loading(app, host);
            "true".into()
        }
        "status" => {
            let kind = match arg.text("kind") {
                "error" => Status::Error,
                "saving" => Status::Saving,
                "paused" => Status::Phase("paused"),
                _ => Status::Info,
            };
            status(app, host, arg.text("text"), kind);
            "true".into()
        }
        "show" => {
            let view = match arg.text("view") {
                "home" => View::Home,
                "editor" => View::Editor,
                _ => View::Welcome,
            };
            show_view(app, host, view);
            "true".into()
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;

    #[test]
    fn signed_out_boot_shows_welcome_and_probes_drive() {
        let (mut app, mut host) = boot(0, |_| {});
        let text = out(&mut host);
        assert_eq!(view(&app), View::Welcome);
        assert!(text.contains("attr 3 \"data-theme\" \"light\"") && text.contains("attr 3 \"data-app-ready\" \"true\""));
        assert!(text.contains(&format!("class {} \"hidden\" 0", Ui::WELCOME_VIEW.0)) && text.contains(&format!("class {} \"hidden\" 1", Ui::DRIVE_HOME.0)));
        // Until Google Identity Services has loaded, the auth UI keeps the markup's state (v5: initDriveAuth).
        assert!(!text.contains("\"display\""));
        io(&mut app, &mut host, req_of(&text, "gis_init"), 0, b"");
        let text = text + &out(&mut host);
        // v5: updateDriveProfileUI(null) at boot.
        assert!(text.contains(&format!("prop {} \"src\" {DEFAULT_AVATAR:?}", Ui::DRIVE_PROFILE_AVATAR.0)));
        assert!(text.contains(&format!("text {} \"Signed in\"", Ui::DRIVE_PROFILE_NAME.0)));
        assert!(text.contains(&format!("text {} \"Sign in to browse your Drive.\"", Ui::DRIVE_STATUS.0)));
        assert!(text.contains(&format!("style {} \"display\" \"inline-flex\"", Ui::BTN_DRIVE_LOGIN.0)));
        // v5: handleFullscreenChange() at the end of setupEventListeners.
        assert!(text.contains(&format!("html {} {FULLSCREEN_ICON_ENTER:?}", Ui::BTN_DRIVE_FULLSCREEN.0)));
        assert!(text.contains("timer_set") && text.contains("\nready\n"));
        // The connectivity probe: unauthenticated, uncached, 5 s.
        let probe = line_of(&text, "fetch");
        assert!(probe.ends_with("\"GET\" \"https://www.googleapis.com/drive/v3/about?fields=kind\" \"\" 28 5000 0 -"), "{probe}");
        assert!(!offline(&app));
        io(&mut app, &mut host, req_of(&text, "fetch"), -1, b"Failed to fetch");
        assert!(offline(&app));
        assert_eq!(host.session.get(NETWORK_KEY).map(String::as_str), Some("1"));
        assert_eq!(out(&mut host), format!("timer_set {T_PROBE} 30000 0\n"));
        assert_eq!(signals(&mut app).last(), Some(&(sig::NETWORK, "0".to_string())));
        // The probe interval doubles up to five minutes; any answer from Drive, even a 401, ends it.
        timer(&mut app, &mut host, T_PROBE);
        let text = out(&mut host);
        io(&mut app, &mut host, req_of(&text, "fetch"), -2, b"");
        assert_eq!(out(&mut host), format!("timer_set {T_PROBE} 60000 0\n"));
        timer(&mut app, &mut host, T_PROBE);
        let text = out(&mut host);
        io(&mut app, &mut host, req_of(&text, "fetch"), 401, b"");
        assert!(!offline(&app));
        assert_eq!(out(&mut host), format!("timer_clear {T_PROBE}\n"));
        assert_eq!(host.session.get(NETWORK_KEY).map(String::as_str), Some("0"));
        // `offline` event: offline at once; `online`: probe again.
        life(&mut app, &mut host, life::OFFLINE, 0.0);
        assert!(offline(&app));
        out(&mut host);
        life(&mut app, &mut host, life::ONLINE, 0.0);
        assert!(out(&mut host).contains("about?fields=kind"));
        // A Drive answer to an authenticated request outranks the pending probe.
        network_available(&mut app, &mut host);
        assert!(!offline(&app));
    }

    #[test]
    fn a_remembered_session_boots_to_home_and_an_offline_tab_stays_offline() {
        let (mut app, mut host) = boot(fx::BOOT_SIGNED_IN, |h| {
            h.session.insert(NETWORK_KEY.into(), "1".into());
            h.local.insert(THEME_KEY.into(), "dark".into());
        });
        let text = out(&mut host);
        io(&mut app, &mut host, req_of(&text, "gis_init"), 0, b"");
        let text = text + &out(&mut host);
        assert_eq!(view(&app), View::Home);
        assert!(offline(&app) && dark(&app));
        assert!(text.contains("attr 3 \"data-theme\" \"dark\""));
        assert!(text.contains(&format!("prop {} \"checked\" true", Ui::THEME_TOGGLE_EDITOR.0)));
        assert!(text.contains(&format!("style {} \"display\" \"none\"", Ui::BTN_DRIVE_LOGIN.0)));
        assert!(text.contains(&format!("class {} \"hidden\" 0", Ui::DRIVE_PROFILE.0)));
    }

    #[test]
    fn app_mode_removes_the_fullscreen_buttons_and_headless_has_no_ui() {
        let (_, mut host) = boot(mode::APP, |_| {});
        let text = out(&mut host);
        assert!(text.contains(&format!("remove {}\nremove {}\n", Ui::BTN_DRIVE_FULLSCREEN.0, Ui::BTN_FULLSCREEN.0)));
        assert!(!text.contains("html ") && text.contains("updater_create"));
        let (_, mut host) = boot(mode::APP | mode::HEADLESS, |_| {});
        let text = out(&mut host);
        assert!(!text.contains("listen") && !text.contains("fetch") && !text.contains("updater_create"), "{text}");
    }

    #[test]
    fn a_shared_link_boots_to_home_as_backdrop() {
        let (mut app, mut host) = boot_at(0, "http://127.0.0.1:4400/?e2e=1&drive=abc&role=writer", |_| {});
        assert_eq!((view(&app), pending_share(&app)), (View::Home, Some(PendingShare::default())));
        let text = out(&mut host);
        io(&mut app, &mut host, req_of(&text, "gis_init"), 0, b"");
        assert!(out(&mut host).contains(&format!("text {} \"Opening shared document...\"", Ui::DRIVE_STATUS.0)));
    }

    #[test]
    fn theme_toggle_persists_and_the_system_scheme_is_followed_until_then() {
        let (mut app, mut host) = boot(0, |_| {});
        out(&mut host);
        life(&mut app, &mut host, life::SCHEME, 1.0);
        assert!(dark(&app) && !host.local.contains_key(THEME_KEY));
        assert!(out(&mut host).starts_with("attr 3 \"data-theme\" \"dark\"\n"));
        dom(&mut app, &mut host, L_THEME, Ui::THEME_TOGGLE, ety::CHANGE, 0, "", &[Ui::THEME_TOGGLE]);
        assert_eq!(host.local.get(THEME_KEY).map(String::as_str), Some("light"));
        assert_eq!(
            out(&mut host),
            format!("attr 3 \"data-theme\" \"light\"\nprop {} \"checked\" false\nprop {} \"checked\" false\n", Ui::THEME_TOGGLE.0, Ui::THEME_TOGGLE_EDITOR.0)
        );
        life(&mut app, &mut host, life::SCHEME, 1.0);
        assert!(!dark(&app) && out(&mut host).is_empty());
        dom(&mut app, &mut host, L_THEME_EDITOR, Ui::THEME_TOGGLE_EDITOR, ety::CHANGE, ev::CHECKED, "", &[]);
        assert_eq!(host.local.get(THEME_KEY).map(String::as_str), Some("dark"));
        assert!(signals(&mut app).contains(&(sig::THEME, "dark".into())));
    }

    #[test]
    fn profile_menus_welcome_sheet_and_soon() {
        let (mut app, mut host) = boot(fx::BOOT_SIGNED_IN, |_| {});
        out(&mut host);
        let (home, editor) = (Ui::DRIVE_PROFILE_MENU.0, Ui::DRIVE_PROFILE_MENU_EDITOR.0);
        assert_eq!(click(&mut app, &mut host, L_PROFILE, Ui::BTN_DRIVE_PROFILE), ret::STOP_PROPAGATION);
        assert_eq!(out(&mut host), format!("class {home} \"hidden\" 0\nattr {} \"aria-expanded\" \"true\"\n", Ui::BTN_DRIVE_PROFILE.0));
        // Opening the other menu closes this one.
        click(&mut app, &mut host, L_PROFILE_EDITOR, Ui::BTN_DRIVE_PROFILE_EDITOR);
        assert_eq!(
            out(&mut host),
            format!("class {home} \"hidden\" 1\nattr {} \"aria-expanded\" \"false\"\nclass {editor} \"hidden\" 0\nattr {} \"aria-expanded\" \"true\"\n", Ui::BTN_DRIVE_PROFILE.0, Ui::BTN_DRIVE_PROFILE_EDITOR.0)
        );
        assert_eq!(signals(&mut app).last(), Some(&(sig::MENU, "editor".to_string())));
        // A click inside the menu keeps it; a click elsewhere or Escape closes it.
        dom(&mut app, &mut host, L_DOC_CLICK, Node::DOCUMENT, ety::CLICK, 0, "", &[Ui::DOC_STATS, Ui::DRIVE_PROFILE_MENU_EDITOR, Node::BODY]);
        assert_eq!(out(&mut host), "");
        dom(&mut app, &mut host, L_DOC_CLICK, Node::DOCUMENT, ety::CLICK, 0, "", &[Node::BODY]);
        assert!(out(&mut host).starts_with(&format!("class {editor} \"hidden\" 1\n")));
        click(&mut app, &mut host, L_PROFILE, Ui::BTN_DRIVE_PROFILE);
        dom(&mut app, &mut host, L_DOC_KEY, Node::DOCUMENT, ety::KEYDOWN, 0, "Escape", &[Node::BODY]);
        assert!(out(&mut host).ends_with(&format!("class {home} \"hidden\" 1\nattr {} \"aria-expanded\" \"false\"\n", Ui::BTN_DRIVE_PROFILE.0)));
        // Diary answers "Soon" for 1.8 s; the toast node is created once.
        click(&mut app, &mut host, L_PROFILE, Ui::BTN_DRIVE_PROFILE);
        out(&mut host);
        click(&mut app, &mut host, L_SOON_DIARY, Ui::BTN_DRIVE_MENU_DIARY);
        let toast = app.st.shell.soon.0;
        assert_eq!(
            out(&mut host),
            format!("class {home} \"hidden\" 1\nattr {} \"aria-expanded\" \"false\"\ninsert_html {toast} 4 0 \"<div id=\\\"soon-toast\\\" role=\\\"status\\\"></div>\"\ntext {toast} \"Soon\"\nclass {toast} \"visible\" 1\ntimer_set {T_SOON} 1800 0\n", Ui::BTN_DRIVE_PROFILE.0)
        );
        click(&mut app, &mut host, L_SOON_AGENDA, Ui::BTN_DRIVE_MENU_AGENDA);
        assert_eq!(out(&mut host), format!("text {toast} \"Soon\"\nclass {toast} \"visible\" 1\ntimer_set {T_SOON} 1800 0\n"));
        timer(&mut app, &mut host, T_SOON);
        assert_eq!(out(&mut host), format!("class {toast} \"visible\" 0\n"));
        // Refresh closes the menu and asks Home to list again.
        click(&mut app, &mut host, L_REFRESH, Ui::BTN_DRIVE_MENU_REFRESH);
        assert_eq!(signals(&mut app).last(), Some(&(sig::REFRESH, String::new())));

        // Welcome sheet: open focuses the real sign-in button; Escape closes and refocuses the first button.
        show_view(&mut app, &mut host, View::Welcome);
        out(&mut host);
        click(&mut app, &mut host, L_WELCOME_START, Ui::BTN_WELCOME_START);
        assert_eq!(
            out(&mut host),
            format!("prop {s} \"hidden\" false\nprop {b} \"hidden\" false\nfocus {} 0\n", Ui::BTN_WELCOME_LOGIN.0, s = Ui::WELCOME_SHEET.0, b = Ui::WELCOME_SHEET_BACKDROP.0)
        );
        assert_eq!(dom(&mut app, &mut host, L_SHEET_KEY, Ui::WELCOME_SHEET, ety::KEYDOWN, 0, "Tab", &[]), 0);
        assert_eq!(dom(&mut app, &mut host, L_SHEET_KEY, Ui::WELCOME_SHEET, ety::KEYDOWN, 0, "Escape", &[]), ret::PREVENT_DEFAULT);
        assert_eq!(
            out(&mut host),
            format!("prop {s} \"hidden\" true\nprop {b} \"hidden\" true\nfocus {} 0\n", Ui::BTN_WELCOME_START.0, s = Ui::WELCOME_SHEET.0, b = Ui::WELCOME_SHEET_BACKDROP.0)
        );
        click(&mut app, &mut host, L_SHEET_BACKDROP, Ui::WELCOME_SHEET_BACKDROP);
        assert_eq!(out(&mut host), "");
    }

    #[test]
    fn loading_overlay_follows_v5_timing() {
        let (mut app, mut host) = boot(0, |_| {});
        out(&mut host);
        let o = Ui::LOADING_OVERLAY.0;
        loading(&mut app, &mut host, "Opening document...");
        assert_eq!(
            out(&mut host),
            format!("timer_clear {T_LOADING_CLEANUP}\ntimer_clear {T_LOADING_FADE}\nclass {o} \"fading-out\" 0\nclass {o} \"home-save-transition\" 0\ntext {} \"Opening document...\"\nattr {o} \"aria-busy\" \"true\"\nclass {o} \"visible\" 1\n", Ui::LOADING_TEXT.0)
        );
        hide_loading(&mut app, &mut host);
        assert_eq!(
            out(&mut host),
            format!("class {o} \"fading-out\" 1\ntimer_set {T_LOADING_FADE} 360 0\nclass {o} \"visible\" 0\nattr {o} \"aria-busy\" \"false\"\nclass {o} \"home-save-transition\" 0\n")
        );
        timer(&mut app, &mut host, T_LOADING_FADE);
        assert_eq!(out(&mut host), format!("class {o} \"fading-out\" 0\n"));
        // Hidden twice: nothing fades the second time.
        hide_loading(&mut app, &mut host);
        assert!(!out(&mut host).contains("fading-out"));
        // The Home transition keeps its opaque background for 180 ms after hiding.
        loading_home_save(&mut app, &mut host, "Saving on this device…");
        assert!(out(&mut host).contains(&format!("class {o} \"home-save-transition\" 1\n")));
        hide_loading(&mut app, &mut host);
        assert!(out(&mut host).ends_with(&format!("timer_set {T_LOADING_CLEANUP} 180 0\n")));
        timer(&mut app, &mut host, T_LOADING_CLEANUP);
        assert_eq!(out(&mut host), format!("class {o} \"home-save-transition\" 0\n"));
        // Shown again before the cleanup: the cleanup is cancelled.
        loading_home_save(&mut app, &mut host, "x");
        hide_loading(&mut app, &mut host);
        loading_home_save(&mut app, &mut host, "y");
        out(&mut host);
        timer(&mut app, &mut host, T_LOADING_CLEANUP);
        assert_eq!(out(&mut host), "");
    }

    #[test]
    fn fullscreen_status_and_touch_blur() {
        let (mut app, mut host) = boot(0, |_| {});
        out(&mut host);
        click(&mut app, &mut host, L_FULLSCREEN_HOME, Ui::BTN_DRIVE_FULLSCREEN);
        assert_eq!(out(&mut host), "fullscreen 1\n");
        life(&mut app, &mut host, life::FULLSCREEN, 1.0);
        let text = out(&mut host);
        assert!(text.contains(&format!("prop {b} \"title\" \"Exit fullscreen\"\nattr {b} \"aria-label\" \"Exit fullscreen\"\nclass {b} \"active\" 1\nhtml {b} {FULLSCREEN_ICON_EXIT:?}\n", b = Ui::BTN_FULLSCREEN.0)));
        assert!(text.ends_with(&format!("timer_set {T_FULLSCREEN_RESIZE} 50 0\n")));
        timer(&mut app, &mut host, T_FULLSCREEN_RESIZE);
        assert_eq!(out(&mut host), "dispatch_resize\n");
        click(&mut app, &mut host, L_FULLSCREEN, Ui::BTN_FULLSCREEN);
        assert_eq!(out(&mut host), "fullscreen 0\n");

        status(&mut app, &mut host, "Drive sign-in needed to keep syncing", Status::Phase("paused"));
        let text = out(&mut host);
        let i = Ui::SAVE_INDICATOR.0;
        assert!(text.starts_with(&format!("class {i} \"paused\" 1\nclass {i} \"building\" 0\n")));
        assert!(text.ends_with(&format!("class {i} \"error\" 0\nprop {i} \"title\" \"Drive sign-in needed to keep syncing\"\nattr {i} \"aria-label\" \"Drive sign-in needed to keep syncing\"\n")));

        // A touch or pen release blurs the pressed button in the next frame; a mouse release does not.
        let pointer = |app: &mut App, host: &mut FakeHost, ptype: u8| {
            let mut w = Writer::default();
            w.u32(L_POINTER_UP);
            w.u32(Node::DOCUMENT.0);
            w.u8(ptr::UP);
            w.u8(ptype);
            w.i32(1);
            w.i32(0);
            w.u32(0);
            for _ in 0..3 {
                w.f64(0.0);
            }
            for _ in 0..3 {
                w.f32(0.0);
            }
            w.u32(0);
            w.u32(0);
            w.u32(0);
            app.event(host, abi::kind::POINTER, &w.buf);
        };
        pointer(&mut app, &mut host, ptr::MOUSE);
        assert_eq!(out(&mut host), "");
        pointer(&mut app, &mut host, ptr::TOUCH);
        assert_eq!(out(&mut host), "raf_request\n");
        app.frame(&mut host, 16.0);
        let text = out(&mut host);
        assert!(text.starts_with("query 1024 1 \"button:focus, .btn:focus") && text.ends_with("blur 1024\nrelease 1024\n"), "{text}");
    }

    use crate::host::FakeHost;
}
