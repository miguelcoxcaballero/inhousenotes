//! The card ⋮ menu and the folder colour popup (spec/home.md §3.9): star, offline pin, colour, move, move to bin.

use super::{frame, id, px, want_frame, window};
use crate::abi::{ret, DomEvent, Handles, Io};
use crate::drive::manage;
use crate::home::{self, Slot};
use crate::host::Rect;
use crate::shell::{self, Status, StatusOpts, View};
use crate::store::{self, Reply, Tx};
use crate::store::sv::Value;
use crate::{App, Host, Mod, Node, Ui};
use ihn_core::catalog::{DocInfo, Entry};

const L_SLOT: u32 = id(30); // + slot
const L_DROPDOWN: u32 = id(40);
const L_POPUP: u32 = id(41);

// v5: app-v5.js:DRIVE_FOLDER_COLORS
pub const COLORS: [&str; 24] = [
    "#f83a22", "#fa573c", "#d06b64", "#ac725e", "#ff7537", "#ffad46", "#fad165", "#fbe983", "#b3dc6c", "#7bd148", "#16a765", "#42d692",
    "#92e1c0", "#9fe1e7", "#9fc5e8", "#4986e7", "#b99aff", "#a479e2", "#cd74e6", "#f691b2", "#cca6ac", "#cabdbf", "#c2c2c2", "#8f8f8f",
];

// v5: app-v5.js:DRIVE_TOOL_ICONS
fn icon(name: &str) -> &'static str {
    match name {
        "color" => r#"<circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="3.5" fill="currentColor" opacity="0.3"/>"#,
        "star" => r#"<path d="M12 3.6l2.6 5.3 5.8.8-4.2 4.1 1 5.7-5.2-2.8-5.2 2.8 1-5.7-4.2-4.1 5.8-.8z"/>"#,
        "offline" => r#"<path d="M7 18.5h10a4 4 0 0 0 .6-7.95A6 6 0 0 0 6.2 9.6 4.5 4.5 0 0 0 7 18.5z"/><path d="M12 10.5v6m-2.5-2.5 2.5 2.5 2.5-2.5"/>"#,
        "offlineOn" => r#"<path d="M7 18.5h10a4 4 0 0 0 .6-7.95A6 6 0 0 0 6.2 9.6 4.5 4.5 0 0 0 7 18.5z"/><path d="m9.5 13.8 2 2 3.6-3.6"/>"#,
        "move" => r#"<path d="M3 6.5A2.5 2.5 0 0 1 5.5 4H9l2 2.2h7.5A2.5 2.5 0 0 1 21 8.7v8.8a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5z"/><path d="M9.5 13.2h6m-2.4-2.4 2.4 2.4-2.4 2.4"/>"#,
        _ => r#"<path d="M4 7h16"/><path d="M18 7v12a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V7"/><path d="M9 7V5a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2"/>"#,
    }
}

struct Open {
    entry: Entry,
    /// The card's `.drive-card-menu`, its button and its dropdown (moved to `<body>` while open).
    menu: Node,
    button: Node,
    dropdown: Node,
    actions: Vec<(Node, &'static str)>,
    rect: Rect,
}

struct Popup {
    node: Node,
    swatches: Vec<(Node, &'static str)>,
    entry: Entry,
    anchor: Rect,
}

enum Job {
    Star(bool),
    Color,
    Pin(String),
    Pinned { id: String, pinned: bool },
}

#[derive(Default)]
pub struct State {
    open: Option<Open>,
    popup: Option<Popup>,
    jobs: Vec<(u32, Job)>,
}

impl State {
    pub fn open_id(&self) -> Option<&str> { self.open.as_ref().map(|o| o.entry.id.as_str()) }
    pub fn popup_open(&self) -> bool { self.popup.is_some() }
}

pub fn init(_app: &mut App, host: &mut dyn Host) {
    let out = host.out();
    for slot in Slot::ALL {
        out.listen(slot.node(), "click", L_SLOT + slot as u32, 0);
    }
}

fn st(app: &mut App) -> &mut State { &mut app.st.home_dialogs.menu }

/// The card whose ⋮ menu an event's path goes through: `(entry, its .drive-card-menu)`.
fn menu_at(app: &App, path: &Handles) -> Option<(Entry, Node)> {
    Slot::ALL.into_iter().find_map(|slot| {
        let card = app.st.home.slots[slot as usize].cards.iter().find(|c| !c.menu.is_none() && path.contains(c.menu))?;
        Some((card.entry.clone(), card.menu))
    })
}

// v5: app-v5.js:createDriveItemMenu (renderActions) + toggleDriveCardMenu
fn toggle(app: &mut App, host: &mut dyn Host, entry: Entry, menu: Node) {
    close_popup(app, host);
    if app.st.home_dialogs.menu.open.as_ref().is_some_and(|o| o.menu == menu) {
        return close_menu(app, host);
    }
    close_menu(app, host);
    let folder = entry.is_folder();
    let pinned = app.st.home.catalog.doc(&entry.id).is_some_and(|d| d.pinned);
    let actions: Vec<(&'static str, &str, &str)> = [
        Some(("star", if entry.starred { "Remove from favorites" } else { "Add to favorites" }, "star")),
        (!folder).then_some(("offline", if pinned { "Unpin offline copy" } else { "Save offline" }, if pinned { "offlineOn" } else { "offline" })),
        folder.then_some(("color", "Change color", "color")),
        Some(("move", "Move", "move")),
        Some(("trash", "Move to bin", "trash")),
    ]
    .into_iter()
    .flatten()
    .collect();
    let html: String = actions
        .iter()
        .map(|(action, label, icon_name)| {
            format!(
                "\n                    <button type=\"button\" data-action=\"{action}\"{}>\n                        <svg viewBox=\"0 0 24 24\" fill=\"{}\" stroke=\"currentColor\" stroke-width=\"1.9\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\">{}</svg>\n                        <span>{label}</span>\n                    </button>",
                if *action == "trash" { " class=\"danger\"" } else { "" },
                if *action == "star" && entry.starred { "currentColor" } else { "none" },
                icon(icon_name)
            )
        })
        .collect();
    let out = host.out();
    let button = out.child(menu, &[0]);
    let dropdown = out.child(menu, &[1]);
    out.html(dropdown, &html);
    let handles: Vec<(Node, &'static str)> = actions.iter().enumerate().map(|(i, a)| (out.child(dropdown, &[i as u8]), a.0)).collect();
    out.insert(Node::BODY, Node::NONE, dropdown);
    out.class(dropdown, "hidden", false);
    out.listen(dropdown, "click", L_DROPDOWN, 0);
    let rect = host.rect(button);
    let (iw, _) = window(host);
    let out = host.out();
    out.style(dropdown, "top", Some(&px(rect.bottom() + 4.0)));
    out.style(dropdown, "right", Some(&px(iw - rect.right())));
    out.style(dropdown, "left", Some("auto"));
    st(app).open = Some(Open { entry, menu, button, dropdown, actions: handles, rect });
    want_frame(app, host, frame::MENU_PLACE);
}

/// The next frame of an opened menu: flip above the button when it would pass the bottom bar, clamp the left edge.
pub fn place_menu(app: &mut App, host: &mut dyn Host) {
    let Some((dropdown, rect)) = app.st.home_dialogs.menu.open.as_ref().map(|o| (o.dropdown, o.rect)) else { return };
    let m = host.rect(dropdown);
    let (_, ih) = window(host);
    let inset = if shell::view(app) == View::Home { host.rect(Ui::HOME_NAV).h } else { 0.0 };
    let out = host.out();
    if m.bottom() > ih - 8.0 - inset {
        out.style(dropdown, "top", Some(&px(8f64.max(rect.y - m.h - 4.0))));
    }
    if m.x < 8.0 {
        out.style(dropdown, "left", Some("8px"));
        out.style(dropdown, "right", Some("auto"));
    }
}

// v5: app-v5.js:closeDriveCardMenu
pub fn close_menu(app: &mut App, host: &mut dyn Host) {
    let Some(o) = st(app).open.take() else { return };
    let connected = host.metric(o.menu, crate::host::Metric::Connected) != 0.0;
    let out = host.out();
    out.unlisten(L_DROPDOWN);
    out.class(o.dropdown, "hidden", true);
    if connected {
        out.insert(o.menu, Node::NONE, o.dropdown);
    } else {
        out.remove(o.dropdown);
    }
    o.actions.iter().for_each(|a| out.release(a.0));
    out.release(o.button);
    out.release(o.dropdown);
}

// v5: app-v5.js:openColorPopup
fn open_popup(app: &mut App, host: &mut dyn Host, entry: Entry, anchor: Rect) {
    close_popup(app, host);
    let current = entry.folder_color.to_ascii_lowercase();
    let swatches: String = COLORS
        .iter()
        .map(|c| format!("<div class=\"drive-color-swatch{}\" style=\"background: {c};\" data-color=\"{c}\" title=\"{c}\"></div>", if *c == current { " active" } else { "" }))
        .collect();
    let out = host.out();
    let node = out.insert_html(Node::BODY, Node::NONE, &format!("<div class=\"drive-color-popup\">{swatches}</div>"));
    let handles = COLORS.iter().enumerate().map(|(i, c)| (out.child(node, &[i as u8]), *c)).collect();
    out.style(node, "top", Some(&px(anchor.bottom() + 6.0)));
    out.style(node, "left", Some(&px(anchor.x)));
    out.listen(node, "click", L_POPUP, 0);
    st(app).popup = Some(Popup { node, swatches: handles, entry, anchor });
    want_frame(app, host, frame::POPUP_PLACE);
}

pub fn place_popup(app: &mut App, host: &mut dyn Host) {
    let Some((node, anchor)) = app.st.home_dialogs.menu.popup.as_ref().map(|p| (p.node, p.anchor)) else { return };
    let r = host.rect(node);
    let (iw, ih) = window(host);
    let out = host.out();
    if r.right() > iw - 8.0 {
        out.style(node, "left", Some(&px(8f64.max(iw - r.w - 8.0))));
    }
    if r.bottom() > ih - 8.0 {
        out.style(node, "top", Some(&px(8f64.max(anchor.y - r.h - 6.0))));
    }
}

// v5: app-v5.js:closeColorPopup
fn close_popup(app: &mut App, host: &mut dyn Host) {
    let Some(p) = st(app).popup.take() else { return };
    let out = host.out();
    out.unlisten(L_POPUP);
    p.swatches.iter().for_each(|s| out.release(s.0));
    out.remove(p.node);
    out.release(p.node);
}

/// A document click outside closes the menu (not on a ⋮ button) and the popup.
pub fn document_click(app: &mut App, host: &mut dyn Host, path: &Handles) {
    let menu = app.st.home_dialogs.menu.open.as_ref().map(|o| o.dropdown);
    if menu.is_some_and(|d| !path.contains(d)) && menu_at(app, path).is_none() {
        close_menu(app, host);
    }
    if app.st.home_dialogs.menu.popup.as_ref().is_some_and(|p| !path.contains(p.node)) {
        close_popup(app, host);
    }
}

fn action(app: &mut App, host: &mut dyn Host, path: &Handles) {
    let Some(o) = app.st.home_dialogs.menu.open.as_ref() else { return };
    let Some((button, action)) = o.actions.iter().find(|a| path.contains(a.0)).copied() else { return };
    let entry = o.entry.clone();
    if action == "color" {
        let rect = host.rect(button);
        close_menu(app, host);
        return open_popup(app, host, entry, rect);
    }
    close_menu(app, host);
    match action {
        "offline" => pin(app, host, &entry),
        // v5: app-v5.js:updateDriveStarStatus
        "star" => {
            let on = !entry.starred;
            let req = manage::star(app, host, Mod::HomeDialogs, &entry.id, on);
            st(app).jobs.push((req, Job::Star(on)));
        }
        "move" => super::picker::open(app, host, super::picker::Mode::Move, Some(super::picker::Target { id: entry.id.clone(), name: entry.name.clone(), parents: entry.parents.clone().unwrap_or_default() }), None),
        _ => super::bin::open_trash(app, host, &entry),
    }
}

// v5: offline-library-v5.js:pin — the `docs` record's `pinned` flag (BEHAVIOUR-CHANGES N-13: a document with no
// copy on this device is not downloaded here; it is kept once it has been opened).
fn pin(app: &mut App, host: &mut dyn Host, entry: &Entry) {
    if app.st.home.account.is_empty() {
        return;
    }
    let mut tx = Tx::new();
    tx.doc_get(&entry.id);
    let req = app.req(Mod::HomeDialogs);
    store::tx(app, host, req, tx);
    st(app).jobs.push((req, Job::Pin(entry.id.clone())));
}

fn pin_read(app: &mut App, host: &mut dyn Host, id: String, io: &Io) {
    let mut reply = Reply::new(io);
    let record = if reply.ok() { reply.doc() } else { None };
    let info = app.st.home.catalog.doc(&id).copied().unwrap_or_default();
    let pinned = record.as_ref().is_some_and(|r| r.pinned);
    if !pinned && (record.is_none() || !info.available) {
        let text = if shell::offline(app) { "Connect once to download this document" } else { "Open this document once to keep it offline" };
        shell::status(app, host, text, Status::Error);
        return;
    }
    let now = host.time();
    let mut tx = Tx::new();
    if pinned {
        tx.doc_patch(&id, &[(&["pinned"], Value::Bool(false))]);
    } else {
        tx.doc_patch(&id, &[(&["pinned"], Value::Bool(true)), (&["lastOpenedAt"], Value::Num(now))]);
    }
    let req = app.req(Mod::HomeDialogs);
    store::tx(app, host, req, tx);
    st(app).jobs.push((req, Job::Pinned { id, pinned: !pinned }));
}

pub fn dom(app: &mut App, host: &mut dyn Host, e: &DomEvent) -> u32 {
    match e.listener {
        l if (L_SLOT..L_SLOT + 7).contains(&l) => {
            let Some((entry, menu)) = menu_at(app, &e.path) else { return 0 };
            toggle(app, host, entry, menu);
            ret::STOP_PROPAGATION
        }
        L_DROPDOWN => {
            action(app, host, &e.path);
            ret::STOP_PROPAGATION
        }
        L_POPUP => {
            let Some(p) = app.st.home_dialogs.menu.popup.as_ref() else { return 0 };
            if let Some(color) = p.swatches.iter().find(|s| e.path.contains(s.0)).map(|s| s.1) {
                let id = p.entry.id.clone();
                close_popup(app, host);
                // v5: app-v5.js:updateDriveFolderColor
                let req = manage::color(app, host, Mod::HomeDialogs, &id, color);
                st(app).jobs.push((req, Job::Color));
            }
            ret::STOP_PROPAGATION
        }
        _ => 0,
    }
}

pub fn io(app: &mut App, host: &mut dyn Host, io: &Io) -> bool {
    let Some(at) = st(app).jobs.iter().position(|j| j.0 == io.req) else { return false };
    let (_, job) = st(app).jobs.swap_remove(at);
    let ok = io.status >= 0;
    let preserve = StatusOpts::from(Status::Info).preserve();
    match job {
        Job::Star(on) => {
            let text = if !ok { "Failed to update favorites" } else if on { "Added to favorites" } else { "Removed from favorites" };
            shell::status(app, host, text, preserve);
            if ok {
                home::refresh_quiet(app, host);
            }
        }
        Job::Color => {
            shell::status(app, host, if ok { "Folder color updated" } else { "Failed to update folder color" }, preserve);
            if ok {
                home::refresh_quiet(app, host);
            }
        }
        Job::Pin(id) => pin_read(app, host, id, io),
        Job::Pinned { id, pinned } => {
            if !ok {
                shell::status(app, host, "Offline copy could not be saved. Keep this document open.", Status::Error);
                return true;
            }
            shell::status(app, host, if pinned { "Saved for offline use" } else { "Automatic offline retention restored" }, preserve);
            let mut info: DocInfo = app.st.home.catalog.doc(&id).copied().unwrap_or_default();
            info.pinned = pinned;
            if pinned {
                info.last_opened_at = host.time();
            }
            home::doc_changed(app, host, &id, Some(info));
        }
    }
    true
}
