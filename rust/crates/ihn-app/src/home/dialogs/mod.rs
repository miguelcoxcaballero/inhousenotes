//! Home dialogs (ARCHITECTURE §13, spec/home.md §3.9 and §6): the create menu and device upload, the folder picker
//! (new / move / copy / upload), new folder, the card ⋮ menu and colour popup, move to bin with its slider, the Bin,
//! the Offline documents and upload-error dialogs, and the Download-app modal. Owned by W5.5.
//!
//! Every dialog is 5.18.0's markup (static in `index.html`, or built here from 5.18.0's templates); the Drive
//! requests are `drive::manage`'s. Submodules: [`picker`] (folder picker, new folder, copy/move/upload/new),
//! [`menu`] (card menu, colour popup, star, offline pin), [`bin`] (move to bin, Bin), [`offline`] (Offline
//! documents, upload error, Download app).

use crate::abi::{ev, life, ret, DomEvent, Handles};
use crate::dom::Prop;
use crate::host::Metric;
use crate::shell::fx;
use crate::shell::json::Json;
use crate::testapi::json_str;
use crate::{auth, tag, App, Event, Host, Mod, Node, Ui};

pub mod bin;
pub mod menu;
pub mod offline;
pub mod picker;

const fn id(n: u32) -> u32 { tag(Mod::HomeDialogs, n) }
// Create menu.
const L_FAB: u32 = id(1);
const L_CREATE_PDF: u32 = id(2);
const L_UPLOAD: u32 = id(3);
const L_INPUT: u32 = id(4);
const L_DOC_DOWN: u32 = id(5);
const L_MENU_KEY: u32 = id(6);
// Shared document listeners.
const L_DOC_CLICK: u32 = id(7);
const L_DOC_KEY: u32 = id(8);
const L_MAKE_COPY: u32 = id(9);
// Ids 10.. belong to the submodules (picker 10–29, menu 30–59, bin 60–79, offline 80–99).

/// Work for the next animation frame.
pub(crate) mod frame {
    pub const PICKER_FOCUS: u32 = 1;
    pub const FOLDER_FOCUS: u32 = 2;
    pub const MENU_PLACE: u32 = 4;
    pub const POPUP_PLACE: u32 = 8;
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    create_open: bool,
    upload_take: u32,
    upload_obj: u32,
    frames: u32,
    pub(crate) picker: picker::State,
    pub(crate) menu: menu::State,
    pub(crate) bin: bin::State,
    pub(crate) offline: offline::State,
}

/// Asks for one animation frame for `work` ([`frame`] bits).
fn want_frame(app: &mut App, host: &mut dyn Host, work: u32) {
    if app.st.home_dialogs.frames == 0 {
        app.sched.frame(host.out(), Mod::HomeDialogs);
    }
    app.st.home_dialogs.frames |= work;
}

/// `innerWidth`, `innerHeight`.
fn window(host: &mut dyn Host) -> (f64, f64) { (host.metric(Node::WINDOW, Metric::InnerWidth), host.metric(Node::WINDOW, Metric::InnerHeight)) }

/// `<n>px` with JS number formatting.
fn px(n: f64) -> String { format!("{}px", ihn_core::jsfmt::num(n)) }

/// 5.18.0's guard of the dialogs that work offline too: a token or a ready local library.
fn can_manage(app: &App) -> bool { auth::token(app).is_some() || super::lists::library_ready(app) }

/// The click went to the overlay itself (5.18.0 `e.target === overlay` / `e.target.id === …`).
fn on_overlay(e: &DomEvent, overlay: Node) -> bool { e.has(ev::EXACT) && e.path.first() == overlay }

/// A failed Drive request as the `{code, message}` 5.18.0's `syncErrorText` reads.
fn error_of(status: i32, data: &[u8]) -> super::badges::SyncError {
    let code = if status <= -100 { (-status).to_string() } else { String::new() };
    super::badges::SyncError { code, message: String::from_utf8_lossy(data).into_owned(), ..Default::default() }
}

// ---- create menu ------------------------------------------------------------------------------------------------

// v5: app-v5.js:installDriveCreateMenu (close)
fn close_create(app: &mut App, host: &mut dyn Host) {
    app.st.home_dialogs.create_open = false;
    let out = host.out();
    out.prop(Ui::DRIVE_CREATE_MENU, "hidden", Prop::Bool(true));
    out.attr(Ui::BTN_DRIVE_NEW, "aria-expanded", Some("false"));
}

// v5: app-v5.js:installDriveCreateMenu (FAB click)
fn toggle_create(app: &mut App, host: &mut dyn Host) {
    if app.st.home_dialogs.create_open {
        return close_create(app, host);
    }
    app.st.home_dialogs.create_open = true;
    let out = host.out();
    out.prop(Ui::DRIVE_CREATE_MENU, "hidden", Prop::Bool(false));
    out.attr(Ui::BTN_DRIVE_NEW, "aria-expanded", Some("true"));
    let rect = host.rect(Ui::BTN_DRIVE_NEW);
    let (w, h) = (host.metric(Ui::DRIVE_CREATE_MENU, Metric::OffsetWidth), host.metric(Ui::DRIVE_CREATE_MENU, Metric::OffsetHeight));
    let (iw, ih) = window(host);
    let left = 8f64.max((iw - w - 8.0).min(rect.right() - w));
    let top = if rect.bottom() + h + 8.0 < ih { rect.bottom() + 8.0 } else { 8f64.max(rect.y - h - 8.0) };
    let out = host.out();
    out.style(Ui::DRIVE_CREATE_MENU, "left", Some(&px(left)));
    out.style(Ui::DRIVE_CREATE_MENU, "top", Some(&px(top)));
    out.focus(Ui::DRIVE_CREATE_PDF, false);
}

/// ArrowDown / ArrowUp in the menu (BEHAVIOUR-CHANGES B-H2: ArrowUp goes to the previous item).
fn menu_key(app: &mut App, host: &mut dyn Host, e: &DomEvent) -> u32 {
    match e.key {
        "Escape" => {
            close_create(app, host);
            host.out().focus(Ui::BTN_DRIVE_NEW, false);
            ret::PREVENT_DEFAULT
        }
        "ArrowDown" | "ArrowUp" => {
            // Two items: either key moves to the other one.
            let next = if e.path.contains(Ui::DRIVE_CREATE_PDF) { Ui::DRIVE_UPLOAD_FILE } else { Ui::DRIVE_CREATE_PDF };
            host.out().focus(next, false);
            ret::PREVENT_DEFAULT
        }
        _ => 0,
    }
}

/// A file was chosen in `#drive-upload-input`: it becomes object `upload_obj` and the picker opens for it.
fn upload_chosen(app: &mut App, host: &mut dyn Host, status: i32, data: &[u8]) {
    let obj = std::mem::take(&mut app.st.home_dialogs.upload_obj);
    let files = Json::parse(std::str::from_utf8(data).unwrap_or("")).unwrap_or(Json::Null);
    let Some(file) = files.items().first().filter(|_| status > 0) else {
        host.out().obj_release(obj);
        return;
    };
    let upload = picker::Upload {
        obj,
        name: file.text("name").into(),
        mime: file.text("type").into(),
        size: file.get("size").and_then(Json::as_f64).unwrap_or(0.0),
    };
    picker::open(app, host, picker::Mode::Upload, None, Some(upload));
}

// ---- events -----------------------------------------------------------------------------------------------------

pub fn init(app: &mut App, host: &mut dyn Host) {
    if app.cfg.has(crate::abi::mode::HEADLESS) {
        return;
    }
    let out = host.out();
    for (node, ty, l) in [
        (Ui::BTN_DRIVE_NEW, "click", L_FAB),
        (Ui::DRIVE_CREATE_PDF, "click", L_CREATE_PDF),
        (Ui::DRIVE_UPLOAD_FILE, "click", L_UPLOAD),
        (Ui::DRIVE_UPLOAD_INPUT, "change", L_INPUT),
        (Ui::DRIVE_CREATE_MENU, "keydown", L_MENU_KEY),
        (Node::DOCUMENT, "pointerdown", L_DOC_DOWN),
        (Node::DOCUMENT, "click", L_DOC_CLICK),
        (Node::DOCUMENT, "keydown", L_DOC_KEY),
        (Ui::BTN_MAKE_COPY, "click", L_MAKE_COPY),
    ] {
        out.listen(node, ty, l, 0);
    }
    picker::init(app, host);
    menu::init(app, host);
    bin::init(app, host);
    offline::init(app, host);
}

fn on_dom(app: &mut App, host: &mut dyn Host, e: &DomEvent) -> u32 {
    match e.listener {
        L_FAB => {
            toggle_create(app, host);
            return ret::STOP_PROPAGATION;
        }
        L_CREATE_PDF => {
            close_create(app, host);
            picker::open(app, host, picker::Mode::New, None, None);
        }
        L_UPLOAD => {
            close_create(app, host);
            let out = host.out();
            out.prop(Ui::DRIVE_UPLOAD_INPUT, "value", Prop::Str(""));
            out.click(Ui::DRIVE_UPLOAD_INPUT);
        }
        L_INPUT if e.obj != 0 => {
            let base = fx::new_obj(app);
            let req = app.req(Mod::HomeDialogs);
            let out = host.out();
            fx::files_take(out, req, e.obj, base, false);
            out.obj_release(e.obj);
            let st = &mut app.st.home_dialogs;
            if st.upload_obj != 0 {
                host.out().obj_release(st.upload_obj);
            }
            (st.upload_take, st.upload_obj) = (req, base);
        }
        L_MENU_KEY => return menu_key(app, host, e),
        L_DOC_CLICK => {
            bin::document_click(app, host, &e.path);
            menu::document_click(app, host, &e.path);
        }
        // v5: app-v5.js:setupEventListeners `document keydown` (Escape)
        L_DOC_KEY if e.key == "Escape" => {
            picker::close(app, host);
            bin::close_trash(app, host);
            bin::close_bin(app, host);
            bin::close_item_menu(app, host);
        }
        L_MAKE_COPY => picker::make_copy(app, host),
        l if l >= id(10) && l < id(30) => return picker::dom(app, host, e),
        l if l >= id(30) && l < id(60) => return menu::dom(app, host, e),
        l if l >= id(60) && l < id(80) => return bin::dom(app, host, e),
        l if l >= id(80) && l < id(100) => return offline::dom(app, host, e),
        _ => {}
    }
    0
}

/// The create menu closes on a `pointerdown` outside it and the FAB.
fn outside_down(app: &mut App, host: &mut dyn Host, path: &Handles) {
    if app.st.home_dialogs.create_open && !path.contains(Ui::DRIVE_CREATE_MENU) && !path.contains(Ui::BTN_DRIVE_NEW) {
        close_create(app, host);
    }
}

pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Dom(e) => return on_dom(app, host, &e),
        Event::Pointer(p) if p.listener == L_DOC_DOWN => outside_down(app, host, &p.path),
        Event::Pointer(p) => return bin::pointer(app, host, &p),
        Event::Frame(_) => {
            let work = std::mem::take(&mut app.st.home_dialogs.frames);
            if work & frame::PICKER_FOCUS != 0 {
                let out = host.out();
                out.focus(Ui::FOLDER_PICKER_NAME, false);
                out.select(Ui::FOLDER_PICKER_NAME);
            }
            if work & frame::FOLDER_FOCUS != 0 {
                host.out().focus(Ui::NEW_FOLDER_NAME, false);
            }
            if work & frame::MENU_PLACE != 0 {
                menu::place_menu(app, host);
            }
            if work & frame::POPUP_PLACE != 0 {
                menu::place_popup(app, host);
            }
        }
        Event::Timer(t) => {
            bin::timer(app, host, t);
            offline::timer(app, host, t);
        }
        Event::Io(io) if io.req != 0 && io.req == app.st.home_dialogs.upload_take => {
            app.st.home_dialogs.upload_take = 0;
            upload_chosen(app, host, io.status, io.data);
        }
        Event::Io(io) => {
            let _ = picker::io(app, host, &io) || menu::io(app, host, &io) || bin::io(app, host, &io) || offline::io(app, host, &io) || {
                if io.obj != 0 {
                    host.out().obj_release(io.obj);
                }
                false
            };
        }
        Event::Life(l) if l.code == life::RESIZE && app.st.home_dialogs.create_open => close_create(app, host),
        Event::Msg(m) => match m.chan {
            super::sig::OPEN_FILE => open_card(app, host, std::str::from_utf8(m.data).unwrap_or("")),
            super::sig::SYNC_ERROR => offline::show_error(app, host, std::str::from_utf8(m.data).unwrap_or("")),
            _ => {}
        },
        _ => {}
    }
    0
}

/// A document card was activated: 5.18.0 `openDriveFile(card.driveFile)`.
fn open_card(app: &mut App, host: &mut dyn Host, json: &str) {
    let Some(file) = Json::parse(json) else { return };
    menu::close_menu(app, host);
    let file = crate::doc::open::File { id: file.text("id").into(), name: file.text("name").into(), resource_key: file.text("resourceKey").into() };
    crate::doc::open::open_file(app, host, file, 0);
}

/// `__IHN_TEST_API__.cmd('home.dialogs.<name>', args)`: `state` → what is open.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    let st = &app.st.home_dialogs;
    Some(match name {
        "state" => format!(
            "{{\"create\":{},\"picker\":{},\"menu\":{},\"popup\":{},\"trash\":{},\"bin\":{},\"settings\":{},\"error\":{}}}",
            st.create_open,
            picker::state_json(&st.picker),
            st.menu.open_id().map_or("null".into(), json_str),
            st.menu.popup_open(),
            st.bin.trash_target().map_or("null".into(), json_str),
            bin::state_json(&st.bin),
            st.offline.settings_open(),
            st.offline.error_open()
        ),
        _ => return None,
    })
}
