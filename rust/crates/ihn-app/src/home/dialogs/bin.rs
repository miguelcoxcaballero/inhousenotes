//! Move to bin with its slide-to-confirm (`#modal-trash-file`, spec/home.md §6.4) and the Bin (`#modal-drive-bin`,
//! §6.5) with Restore.

use super::{id, on_overlay};
use crate::abi::{ptr, ret, DomEvent, Handles, Io, PointerBatch};
use crate::drive::{home::entry, manage};
use crate::home::cards::esc;
use crate::shell::fx::Response;
use crate::shell::json::Json;
use crate::shell::{self, Status, StatusOpts};
use crate::testapi::json_str;
use crate::{auth, App, Host, IntlOp, Mod, Node, Ui};
use ihn_core::catalog::{parse_time, Entry};
use ihn_core::jsfmt::num;

const L_DOWN: u32 = id(60);
const L_MOVE: u32 = id(61);
const L_UP: u32 = id(62);
const L_UP2: u32 = id(63);
const L_UP3: u32 = id(64);
const L_TRASH_CANCEL: u32 = id(65);
const L_TRASH_OVERLAY: u32 = id(66);
const L_BIN_OPEN: u32 = id(67);
const L_BIN_CLOSE: u32 = id(68);
const L_BIN_OVERLAY: u32 = id(69);
const L_BIN_LIST: u32 = id(70);
const T_CONFIRM: u32 = id(60);
/// `trashSliderState.padding`.
const PAD: f64 = 4.0;

const FOLDER_PATH: &str = r#"<path d="M3 6a2 2 0 0 1 2-2h5l2 2h9a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>"#;
const FILE_PATH: &str = r#"<path d="M6 2h8l4 4v16a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2z"/><path d="M14 2v6h6"/>"#;
const MORE: &str = "\n                        <svg viewBox=\"0 0 24 24\" width=\"16\" height=\"16\" fill=\"currentColor\" aria-hidden=\"true\">\n                            <circle cx=\"5\" cy=\"12\" r=\"2\"></circle>\n                            <circle cx=\"12\" cy=\"12\" r=\"2\"></circle>\n                            <circle cx=\"19\" cy=\"12\" r=\"2\"></circle>\n                        </svg>\n                    ";
const RESTORE: &str = "\n                        <button type=\"button\" data-action=\"restore\">\n                            <svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\">\n                                <polyline points=\"3 6 5 6 21 6\"/>\n                                <path d=\"M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6\"/>\n                                <path d=\"M10 11l2-2 2 2\"/>\n                                <path d=\"M12 9v6\"/>\n                            </svg>\n                            <span>Restore</span>\n                        </button>\n                    ";

struct Row {
    more: Node,
    dropdown: Node,
    restore: Node,
    id: String,
    parents: Vec<String>,
}

enum Job {
    Trash,
    List,
    Restore,
}

#[derive(Default)]
pub struct State {
    target: Option<Entry>,
    dragging: bool,
    x: f64,
    max_x: f64,
    handle_w: f64,
    offset: f64,
    open: bool,
    rows: Vec<Row>,
    /// The row whose menu is open (`activeBinMenu`).
    active: Option<usize>,
    jobs: Vec<(u32, Job)>,
}

impl State {
    pub fn trash_target(&self) -> Option<&str> { self.target.as_ref().map(|t| t.id.as_str()) }
}

pub fn init(_app: &mut App, host: &mut dyn Host) {
    let out = host.out();
    for (node, ty, l) in [
        (Ui::TRASH_SLIDER, "pointerdown", L_DOWN),
        (Ui::TRASH_SLIDER, "pointermove", L_MOVE),
        (Ui::TRASH_SLIDER, "pointerup", L_UP),
        (Ui::TRASH_SLIDER, "pointercancel", L_UP2),
        (Ui::TRASH_SLIDER, "lostpointercapture", L_UP3),
        (Ui::TRASH_FILE_CANCEL, "click", L_TRASH_CANCEL),
        (Ui::MODAL_TRASH_FILE, "click", L_TRASH_OVERLAY),
        (Ui::BTN_DRIVE_BIN, "click", L_BIN_OPEN),
        (Ui::DRIVE_BIN_CLOSE, "click", L_BIN_CLOSE),
        (Ui::MODAL_DRIVE_BIN, "click", L_BIN_OVERLAY),
        (Ui::DRIVE_BIN_LIST, "click", L_BIN_LIST),
    ] {
        out.listen(node, ty, l, 0);
    }
}

fn st(app: &mut App) -> &mut State { &mut app.st.home_dialogs.bin }

// ---- slider -----------------------------------------------------------------------------------------------------

/// v5 `updateTrashSliderMetrics`; returns the slider's left edge.
fn metrics(app: &mut App, host: &mut dyn Host) -> f64 {
    let (r, h) = (host.rect(Ui::TRASH_SLIDER), host.rect(Ui::TRASH_SLIDER_HANDLE));
    let s = st(app);
    s.handle_w = if h.w != 0.0 { h.w } else { 44.0 };
    s.max_x = (r.w - s.handle_w - PAD * 2.0).max(0.0);
    r.x
}

// v5: app-v5.js:setTrashSliderPosition
fn set_pos(app: &mut App, host: &mut dyn Host, x: f64, animate: bool) {
    metrics(app, host);
    let s = st(app);
    let x = x.min(s.max_x).max(0.0);
    s.x = x;
    let fill = x + s.handle_w / 2.0 + PAD;
    let out = host.out();
    out.class(Ui::TRASH_SLIDER, "dragging", !animate);
    out.style(Ui::TRASH_SLIDER_HANDLE, "transform", Some(&format!("translateX({}px)", num(x))));
    out.style(Ui::TRASH_SLIDER_FILL, "width", Some(&format!("{}px", num(fill))));
}

// v5: app-v5.js:resetTrashSlider
fn reset(app: &mut App, host: &mut dyn Host) {
    host.out().class(Ui::TRASH_SLIDER, "complete", false);
    set_pos(app, host, 0.0, true);
}

pub fn pointer(app: &mut App, host: &mut dyn Host, p: &PointerBatch) -> u32 {
    match p.listener {
        // v5: app-v5.js:startTrashSliderDrag
        L_DOWN if p.kind == ptr::DOWN && p.button <= 0 => {
            st(app).dragging = true;
            let out = host.out();
            out.class(Ui::TRASH_SLIDER, "complete", false);
            out.class(Ui::TRASH_SLIDER, "dragging", true);
            out.pointer_capture(Ui::TRASH_SLIDER, p.pointer_id, true);
            let left = metrics(app, host);
            let h = host.rect(Ui::TRASH_SLIDER_HANDLE);
            let offset = (p.raw.x - h.x).max(0.0).min(h.w);
            st(app).offset = offset;
            set_pos(app, host, p.raw.x - left - PAD - offset, false);
        }
        // v5: app-v5.js:onTrashSliderDrag
        L_MOVE if app.st.home_dialogs.bin.dragging => {
            let left = host.rect(Ui::TRASH_SLIDER).x;
            let x = p.raw.x - left - PAD - app.st.home_dialogs.bin.offset;
            set_pos(app, host, x, false);
        }
        // v5: app-v5.js:endTrashSliderDrag
        L_UP | L_UP2 | L_UP3 if app.st.home_dialogs.bin.dragging => {
            st(app).dragging = false;
            host.out().class(Ui::TRASH_SLIDER, "dragging", false);
            let s = &app.st.home_dialogs.bin;
            if s.x >= s.max_x * 0.9 {
                let max = s.max_x;
                host.out().class(Ui::TRASH_SLIDER, "complete", true);
                set_pos(app, host, max, true);
                app.sched.timer(host.out(), T_CONFIRM, 120.0);
            } else {
                reset(app, host);
            }
        }
        _ => {}
    }
    0
}

// ---- move to bin ------------------------------------------------------------------------------------------------

// v5: app-v5.js:openTrashModal
pub fn open_trash(app: &mut App, host: &mut dyn Host, entry: &Entry) {
    st(app).target = Some(entry.clone());
    host.out().text(Ui::TRASH_FILE_NAME, if entry.name.is_empty() { "this file" } else { &entry.name });
    reset(app, host);
    host.out().class(Ui::MODAL_TRASH_FILE, "visible", true);
}

// v5: app-v5.js:closeTrashModal
pub fn close_trash(app: &mut App, host: &mut dyn Host) {
    st(app).target = None;
    host.out().class(Ui::MODAL_TRASH_FILE, "visible", false);
    reset(app, host);
}

// v5: app-v5.js:confirmTrashFile
fn confirm_trash(app: &mut App, host: &mut dyn Host) {
    let Some(target) = app.st.home_dialogs.bin.target.clone().filter(|t| !t.id.is_empty()) else { return };
    close_trash(app, host);
    shell::loading(app, host, "Moving to bin...");
    let req = manage::trash(app, host, Mod::HomeDialogs, &target.id);
    st(app).jobs.push((req, Job::Trash));
}

pub fn timer(app: &mut App, host: &mut dyn Host, t: u32) {
    if t == T_CONFIRM {
        confirm_trash(app, host);
    }
}

// ---- Bin --------------------------------------------------------------------------------------------------------

// v5: app-v5.js:openDriveBinModal
fn open_bin(app: &mut App, host: &mut dyn Host) {
    if auth::token(app).is_none() {
        return;
    }
    st(app).open = true;
    host.out().class(Ui::MODAL_DRIVE_BIN, "visible", true);
    refresh(app, host);
}

pub fn close_bin(app: &mut App, host: &mut dyn Host) {
    st(app).open = false;
    host.out().class(Ui::MODAL_DRIVE_BIN, "visible", false);
}

fn clear_rows(app: &mut App, host: &mut dyn Host) {
    let out = host.out();
    for r in std::mem::take(&mut app.st.home_dialogs.bin.rows) {
        [r.more, r.dropdown, r.restore].into_iter().for_each(|n| out.release(n));
    }
    app.st.home_dialogs.bin.active = None;
}

// v5: app-v5.js:refreshDriveBin
fn refresh(app: &mut App, host: &mut dyn Host) {
    if auth::token(app).is_none() {
        return;
    }
    clear_rows(app, host);
    let out = host.out();
    out.html(Ui::DRIVE_BIN_LIST, "");
    out.attr(Ui::DRIVE_BIN_LIST, "aria-busy", Some("true"));
    out.class(Ui::DRIVE_BIN_EMPTY, "hidden", true);
    out.class(Ui::DRIVE_BIN_LOADING, "hidden", false);
    let req = manage::bin_list(app, host, Mod::HomeDialogs);
    st(app).jobs.push((req, Job::List));
}

// v5: app-v5.js:formatDriveDate
fn format_date(host: &mut dyn Host, value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    match parse_time(value) {
        Some(ms) => host.intl(IntlOp::Date, &format!("{}\0date\0\0{{\"month\":\"short\",\"day\":\"numeric\",\"year\":\"numeric\"}}", num(ms))),
        None => "Invalid Date".into(),
    }
}

fn render(app: &mut App, host: &mut dyn Host, files: Vec<(Entry, String)>) {
    let mut html = String::new();
    for (e, modified) in &files {
        html.push_str(&format!(
            "<div class=\"bin-item\">\n                        <svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\">\n                            {}\n                        </svg>\n                        <div>\n                            <div class=\"bin-item-name\">{}</div>\n                            <div class=\"bin-item-meta\">Modified {}</div>\n                        </div>\n                    <div class=\"bin-item-menu\"><button type=\"button\" class=\"bin-item-more\" title=\"More actions\" aria-label=\"More actions\">{MORE}</button><div class=\"bin-item-dropdown hidden\">{RESTORE}</div></div></div>",
            if e.is_folder() { FOLDER_PATH } else { FILE_PATH },
            esc(if e.name.is_empty() { "Untitled" } else { &e.name }),
            esc(modified)
        ));
    }
    let out = host.out();
    out.html(Ui::DRIVE_BIN_LIST, &html);
    let rows = files
        .into_iter()
        .enumerate()
        .map(|(i, (e, _))| {
            let at = i as u8;
            Row { more: out.child(Ui::DRIVE_BIN_LIST, &[at, 2, 0]), dropdown: out.child(Ui::DRIVE_BIN_LIST, &[at, 2, 1]), restore: out.child(Ui::DRIVE_BIN_LIST, &[at, 2, 1, 0]), id: e.id, parents: e.parents.unwrap_or_default() }
        })
        .collect();
    st(app).rows = rows;
}

fn listed(app: &mut App, host: &mut dyn Host, io: &Io) {
    let out = host.out();
    out.class(Ui::DRIVE_BIN_LOADING, "hidden", true);
    out.attr(Ui::DRIVE_BIN_LIST, "aria-busy", Some("false"));
    let files: Vec<(Entry, String)> = (io.status >= 0)
        .then(|| Json::parse(Response::parse(io.data).text()))
        .flatten()
        .map(|b| b.get("files").map(|f| f.items().iter().map(|j| (entry(j), if j.text("modifiedTime").is_empty() { j.text("createdTime") } else { j.text("modifiedTime") }.to_string())).collect()).unwrap_or_default())
        .unwrap_or_default();
    let files: Vec<(Entry, String)> = files.into_iter().filter(|(e, _)| !app.st.home.catalog.hidden(e)).collect();
    if files.is_empty() {
        host.out().class(Ui::DRIVE_BIN_EMPTY, "hidden", false);
        return;
    }
    let files = files.into_iter().map(|(e, m)| { let d = format_date(host, &m); (e, d) }).collect();
    render(app, host, files);
}

// v5: app-v5.js:toggleBinItemMenu
fn toggle_item(app: &mut App, host: &mut dyn Host, i: usize) {
    let s = &app.st.home_dialogs.bin;
    let out = host.out();
    if let Some(a) = s.active.filter(|a| *a != i) {
        out.class(s.rows[a].dropdown, "hidden", true);
    }
    let open = s.active != Some(i);
    out.class(s.rows[i].dropdown, "hidden", !open);
    st(app).active = open.then_some(i);
}

pub fn close_item_menu(app: &mut App, host: &mut dyn Host) {
    let s = &mut app.st.home_dialogs.bin;
    if let Some(a) = s.active.take() {
        host.out().class(s.rows[a].dropdown, "hidden", true);
    }
}

pub fn document_click(app: &mut App, host: &mut dyn Host, path: &Handles) {
    let s = &app.st.home_dialogs.bin;
    if s.active.is_some_and(|a| !path.contains(s.rows[a].dropdown)) {
        close_item_menu(app, host);
    }
}

// v5: app-v5.js:restoreDriveFile
fn restore(app: &mut App, host: &mut dyn Host, i: usize) {
    close_item_menu(app, host);
    let (id, parents) = { let r = &app.st.home_dialogs.bin.rows[i]; (r.id.clone(), r.parents.clone()) };
    shell::loading(app, host, "Restoring...");
    let req = manage::restore(app, host, Mod::HomeDialogs, &id, &parents);
    st(app).jobs.push((req, Job::Restore));
}

pub fn dom(app: &mut App, host: &mut dyn Host, e: &DomEvent) -> u32 {
    match e.listener {
        L_TRASH_CANCEL => close_trash(app, host),
        L_TRASH_OVERLAY if on_overlay(e, Ui::MODAL_TRASH_FILE) => close_trash(app, host),
        L_BIN_OPEN => open_bin(app, host),
        L_BIN_CLOSE => close_bin(app, host),
        L_BIN_OVERLAY if on_overlay(e, Ui::MODAL_DRIVE_BIN) => close_bin(app, host),
        L_BIN_LIST => {
            let rows = &app.st.home_dialogs.bin.rows;
            if let Some(i) = rows.iter().position(|r| e.path.contains(r.more)) {
                toggle_item(app, host, i);
                return ret::STOP_PROPAGATION;
            }
            if let Some(i) = rows.iter().position(|r| e.path.contains(r.dropdown)) {
                if e.path.contains(rows[i].restore) {
                    restore(app, host, i);
                }
                return ret::STOP_PROPAGATION;
            }
        }
        _ => {}
    }
    0
}

pub fn io(app: &mut App, host: &mut dyn Host, io: &Io) -> bool {
    let Some(at) = st(app).jobs.iter().position(|j| j.0 == io.req) else { return false };
    let (_, job) = st(app).jobs.swap_remove(at);
    let ok = io.status >= 0;
    let preserve = StatusOpts::from(Status::Info).preserve();
    match job {
        Job::Trash => {
            shell::status(app, host, if ok { "Moved to bin" } else { "Move to bin failed" }, preserve);
            if ok {
                crate::home::refresh_quiet(app, host);
            }
            shell::hide_loading(app, host);
        }
        Job::List => listed(app, host, io),
        Job::Restore => {
            shell::status(app, host, if ok { "Restored" } else { "Restore failed" }, preserve);
            if ok {
                refresh(app, host);
                crate::home::refresh_quiet(app, host);
            }
            shell::hide_loading(app, host);
        }
    }
    true
}

pub fn state_json(s: &State) -> String {
    format!("{{\"open\":{},\"rows\":[{}],\"menu\":{}}}", s.open, s.rows.iter().map(|r| json_str(&r.id)).collect::<Vec<_>>().join(","), s.active.map_or("null".into(), |a| a.to_string()))
}
