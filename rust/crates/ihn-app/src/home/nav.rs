//! Home's tabs and bottom navigation, the search box, Escape, and the 30 s refresh timer.

use super::{folders, lists, scroll};
use crate::abi::DomEvent;
use crate::dom::Prop;
use crate::host::{LsOp, Metric};
use crate::{App, Host, Node, Ui};

// v5: app-v5.js:DRIVE_HOME_TABS
pub const TABS: [&str; 4] = ["home", "starred", "shared", "files"];
const TAB_KEY: &str = "ihn-home-tab";
// v5: app-v5.js:DRIVE_HOME_AUTO_REFRESH_MS
pub const AUTO_REFRESH_MS: f64 = 30000.0;

pub fn tab(app: &App) -> &'static str { TABS[app.st.home.tab as usize] }

// v5: app-v5.js:setDriveHomeTab
pub fn set_tab(app: &mut App, host: &mut dyn Host, tab: &str, keep_scroll: bool) {
    let next = TABS.iter().position(|t| *t == tab).unwrap_or(0);
    // A closing snapshot can still be on screen after the live view went back to the root.
    folders::finish_animation(app, host);
    if folders::view_open(app) || app.st.home.stack.len() > 1 {
        folders::close_view(app, host, false);
    }
    app.st.home.tab = next as u8;
    let out = host.out();
    out.attr(Ui::DRIVE_HOME, "data-home-tab", Some(TABS[next]));
    for (i, button) in app.st.home.tabs.into_iter().enumerate() {
        out.class(button, "is-active", i == next);
        out.attr(button, "aria-selected", Some(if i == next { "true" } else { "false" }));
    }
    if !keep_scroll {
        out.prop(app.st.home.main, "scrollTop", Prop::Num(0.0));
    }
    host.ls(LsOp::SessionSet, TAB_KEY, Some(TABS[next]));
    scroll::schedule(app, host);
}

/// The tab of the last visit to Home in this browser tab.
pub(super) fn restore_tab(app: &mut App, host: &mut dyn Host) {
    if let Some(saved) = host.ls(LsOp::SessionGet, TAB_KEY, None).filter(|t| !t.is_empty()) {
        set_tab(app, host, &saved, true);
    }
}

/// A click in the bottom navigation.
pub(super) fn nav_clicked(app: &mut App, host: &mut dyn Host, e: &DomEvent) {
    let Some(i) = app.st.home.tabs.iter().position(|b| e.path.contains(*b)) else { return };
    if i == app.st.home.tab as usize && folders::view_open(app) {
        folders::close_view(app, host, true);
    } else {
        set_tab(app, host, TABS[i], false);
    }
}

/// The card ⋮ menu and the colour popup belong to the card-menu unit; a folder navigation closes them as a
/// click outside does.
pub(super) fn close_card_menu(_app: &mut App, host: &mut dyn Host) {
    let out = host.out();
    let open = out.query(Node::DOCUMENT, ".drive-card-dropdown:not(.hidden)");
    out.class(open, "hidden", true);
    out.release(open);
}

/// Escape leaves the open folder unless something is open above it.
// v5: app-v5.js:setupEventListeners `document keydown`
pub(super) fn escape(app: &mut App, host: &mut dyn Host) {
    if !folders::view_open(app) || crate::shell::view(app) != crate::shell::View::Home {
        return;
    }
    let overlay = host.out().query(Node::DOCUMENT, ".modal-overlay.visible, .drive-card-dropdown:not(.hidden), .drive-color-popup, #drive-profile-menu:not(.hidden)");
    let open = host.metric(overlay, Metric::Connected) != 0.0;
    host.out().release(overlay);
    if !open {
        folders::navigate_up(app, host);
    }
}

/// Typing in the search box lists again 250 ms after the last key.
pub(super) fn search_input(app: &mut App, host: &mut dyn Host, e: &DomEvent) {
    app.st.home.search = e.value.to_string();
    app.sched.timer(host.out(), super::T_SEARCH, 250.0);
}

// v5: app-v5.js:startDriveHomeAutoRefresh / stopDriveHomeAutoRefresh
pub(super) fn auto_refresh(app: &mut App, host: &mut dyn Host, on: bool) {
    if std::mem::replace(&mut app.st.home.auto_on, on) == on {
        return;
    }
    if on {
        app.sched.interval(host.out(), super::T_AUTO, AUTO_REFRESH_MS);
    } else {
        app.sched.clear(host.out(), super::T_AUTO);
        app.st.home.auto_busy = false;
    }
}

pub(super) fn auto_tick(app: &mut App, host: &mut dyn Host) { lists::refresh_snapshot(app, host); }
