//! The folder picker (`#modal-folder-picker`, spec/home.md §6.2) in its four modes and what it confirms — a new
//! document, a move, a copy of the open document, a device upload — and the New folder modal (§6.3).

use super::{can_manage, error_of, frame, id, on_overlay, want_frame};
use crate::abi::{ety, ret, DomEvent, Io};
use crate::dom::Prop;
use crate::drive::{home::entry, manage};
use crate::home::{badges, cards::esc, lists};
use crate::shell::fx::Response;
use crate::shell::json::Json;
use crate::shell::{Status, StatusOpts};
use crate::testapi::json_str;
use crate::{doc, shell, App, Host, Mod, Node, Ui};
use ihn_core::catalog::Entry;

const L_CANCEL: u32 = id(10);
const L_CONFIRM: u32 = id(11);
const L_OVERLAY: u32 = id(12);
const L_NAME_INPUT: u32 = id(13);
const L_NAME_KEY: u32 = id(14);
const L_CRUMBS: u32 = id(15);
const L_LIST: u32 = id(16);
const L_NEW_FOLDER: u32 = id(20);
const L_VIEW_NEW_FOLDER: u32 = id(21);
const L_VIEW_NEW_PDF: u32 = id(22);
const L_FOLDER_CANCEL: u32 = id(23);
const L_FOLDER_CONFIRM: u32 = id(24);
const L_FOLDER_OVERLAY: u32 = id(25);
const L_FOLDER_INPUT: u32 = id(26);
const L_FOLDER_KEY: u32 = id(27);

const FOLDER_SVG: &str = "\n                    <svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\">\n                        <path d=\"M3 6a2 2 0 0 1 2-2h5l2 2h9a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z\"></path>\n                    </svg>\n                    ";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    New,
    Move,
    Copy,
    Upload,
}

impl Mode {
    fn name(self) -> &'static str { ["new", "move", "copy", "upload"][self as usize] }
}

/// The file a move or copy is about.
#[derive(Clone, Debug, Default)]
pub struct Target {
    pub id: String,
    pub name: String,
    pub parents: Vec<String>,
}

/// The device file of an upload (a File object handle and what `files_take` reported).
#[derive(Clone, Debug, Default)]
pub struct Upload {
    pub obj: u32,
    pub name: String,
    pub mime: String,
    pub size: f64,
}

enum Job {
    List(String),
    Parents(String),
    Folder(String),
    Upload(u32),
    Put(u32),
    MoveParents { id: String, folder: String },
    Move,
    CopyProps { id: String, name: String, folder: String },
    Copy,
    NewFolder,
}

#[derive(Default)]
pub struct State {
    mode: Option<Mode>,
    target: Option<Target>,
    upload: Option<Upload>,
    /// Bumped whenever the picker opens with another file (5.18.0 compares `folderPickerTargetFile`).
    ticket: u32,
    uploading: bool,
    stack: Vec<(String, String)>,
    current: String,
    name: String,
    folders: Vec<(Node, Entry)>,
    crumbs: Vec<Node>,
    jobs: Vec<(u32, Job)>,
    /// `driveFolderMetaCache`: id → (name, parents).
    meta: Vec<(String, String, Vec<String>)>,
    /// The path being resolved: the file, the names found so far, the ids visited.
    path: Option<(String, Vec<String>, Vec<String>)>,
    file_ext: Node,
    folder_name: String,
}

pub fn init(app: &mut App, host: &mut dyn Host) {
    let out = host.out();
    for (node, ty, l) in [
        (Ui::FOLDER_PICKER_CANCEL, "click", L_CANCEL),
        (Ui::FOLDER_PICKER_CONFIRM, "click", L_CONFIRM),
        (Ui::MODAL_FOLDER_PICKER, "click", L_OVERLAY),
        (Ui::FOLDER_PICKER_NAME, "input", L_NAME_INPUT),
        (Ui::FOLDER_PICKER_NAME, "keydown", L_NAME_KEY),
        (Ui::FOLDER_PICKER_BREADCRUMBS, "click", L_CRUMBS),
        (Ui::FOLDER_PICKER_LIST, "click", L_LIST),
        (Ui::BTN_DRIVE_NEW_FOLDER, "click", L_NEW_FOLDER),
        (Ui::BTN_FOLDER_VIEW_NEW_FOLDER, "click", L_VIEW_NEW_FOLDER),
        (Ui::BTN_FOLDER_VIEW_NEW_PDF, "click", L_VIEW_NEW_PDF),
        (Ui::NEW_FOLDER_CANCEL, "click", L_FOLDER_CANCEL),
        (Ui::NEW_FOLDER_CONFIRM, "click", L_FOLDER_CONFIRM),
        (Ui::MODAL_NEW_FOLDER, "click", L_FOLDER_OVERLAY),
        (Ui::NEW_FOLDER_NAME, "input", L_FOLDER_INPUT),
        (Ui::NEW_FOLDER_NAME, "keydown", L_FOLDER_KEY),
    ] {
        out.listen(node, ty, l, 0);
    }
    app.st.home_dialogs.picker.file_ext = out.query(Ui::FOLDER_PICKER_NAME_ROW, ".file-ext");
}

fn st(app: &mut App) -> &mut State { &mut app.st.home_dialogs.picker }

fn request(app: &mut App, job: Job, req: u32) { st(app).jobs.push((req, job)); }

fn set_name(app: &mut App, host: &mut dyn Host, value: &str) {
    st(app).name = value.into();
    host.out().prop(Ui::FOLDER_PICKER_NAME, "value", Prop::Str(value));
}

// v5: app-v5.js:updateFolderPickerConfirmState
fn update_confirm(app: &mut App, host: &mut dyn Host) {
    let s = &app.st.home_dialogs.picker;
    let disabled = s.mode != Some(Mode::Move) && s.name.trim().is_empty();
    host.out().prop(Ui::FOLDER_PICKER_CONFIRM, "disabled", Prop::Bool(disabled));
}

/// The open document as the target of `Make a Copy` (5.18.0 `makeDriveCopy`).
pub fn make_copy(app: &mut App, host: &mut dyn Host) {
    let Some(s) = doc::session::get(app) else { return };
    let target = Target { id: s.doc_id.clone(), name: s.record.name.clone(), parents: Vec::new() };
    if target.id.is_empty() {
        return;
    }
    set_name(app, host, &format!("Copy of {}", if target.name.is_empty() { "Document" } else { &target.name }));
    open(app, host, Mode::Copy, Some(target), None);
}

// v5: app-v5.js:openFolderPicker
pub fn open(app: &mut App, host: &mut dyn Host, mode: Mode, target: Option<Target>, upload: Option<Upload>) {
    if !can_manage(app) {
        if let Some(u) = upload {
            host.out().obj_release(u.obj);
        }
        return;
    }
    let home = &app.st.home;
    let stack: Vec<(String, String)> = home.stack.iter().map(|c| (c.id.clone(), c.name.clone())).collect();
    let (stack, current) = if stack.is_empty() { (vec![("root".into(), "My Drive".into())], "root".into()) } else { (stack, if home.current.is_empty() { "root".into() } else { home.current.clone() }) };
    let previous = st(app).upload.take();
    if let Some(p) = previous {
        host.out().obj_release(p.obj);
    }
    let s = st(app);
    s.ticket += 1;
    (s.mode, s.target, s.stack, s.current) = (Some(mode), target.clone(), stack, current);
    let file_name = upload.as_ref().map(|u| u.name.clone());
    s.upload = upload;
    s.uploading = false;
    let out = host.out();
    let (title, subtitle, confirm) = match mode {
        Mode::Move => ("Move file", "Choose the destination folder.".to_string(), "Move"),
        Mode::Copy => ("Copy file", "Choose a destination and name your copy.".into(), "Copy"),
        Mode::Upload => ("Upload file", format!("Choose where to upload {}.", file_name.as_deref().filter(|n| !n.is_empty()).unwrap_or("your file")), "Upload"),
        Mode::New => ("New PDF", "Choose a folder and name your file.".into(), "Create"),
    };
    out.text(Ui::FOLDER_PICKER_TITLE, title);
    out.text(Ui::FOLDER_PICKER_SUBTITLE, &subtitle);
    match target.as_ref().filter(|_| matches!(mode, Mode::Move | Mode::Copy)) {
        Some(t) => {
            out.class(Ui::FOLDER_PICKER_META, "hidden", false);
            let label = out.query(Ui::FOLDER_PICKER_META, ".folder-picker-meta-label");
            out.text(label, if mode == Mode::Move { "Moving" } else { "Copying" });
            out.release(label);
            out.text(Ui::FOLDER_PICKER_FILE_NAME, if t.name.is_empty() { "Untitled" } else { &t.name });
            out.text(Ui::FOLDER_PICKER_FILE_PATH, "Current: loading...");
            resolve_path(app, host, t);
        }
        None => host.out().class(Ui::FOLDER_PICKER_META, "hidden", true),
    }
    let ext = st(app).file_ext;
    let out = host.out();
    out.style(Ui::FOLDER_PICKER_NAME_ROW, "display", Some(if mode == Mode::Move { "none" } else { "flex" }));
    out.prop(ext, "hidden", Prop::Bool(mode == Mode::Upload));
    match mode {
        Mode::Move => set_name(app, host, ""),
        Mode::Upload => set_name(app, host, &file_name.unwrap_or_default()),
        Mode::Copy => {} // set by `make_copy`
        Mode::New => {
            // v5: normalizeDocTitleInput(state.exportName || 'Untitled') — the open document's name.
            let last = doc::session::get(app).map(|s| s.record.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Untitled".into());
            set_name(app, host, &doc::create::normalize_title(&last));
        }
    }
    host.out().text(Ui::FOLDER_PICKER_CONFIRM, confirm);
    update_confirm(app, host);
    render_crumbs(app, host);
    refresh_folders(app, host);
    host.out().class(Ui::MODAL_FOLDER_PICKER, "visible", true);
    if mode != Mode::Move {
        want_frame(app, host, frame::PICKER_FOCUS);
    }
}

// v5: app-v5.js:closeFolderPicker
pub fn close(app: &mut App, host: &mut dyn Host) {
    let s = st(app);
    (s.mode, s.target, s.path) = (None, None, None);
    s.ticket += 1;
    if let Some(u) = s.upload.take() {
        host.out().obj_release(u.obj);
    }
    let out = host.out();
    out.class(Ui::MODAL_FOLDER_PICKER, "visible", false);
    out.class(Ui::FOLDER_PICKER_META, "hidden", true);
    out.text(Ui::FOLDER_PICKER_FILE_NAME, "");
    out.text(Ui::FOLDER_PICKER_FILE_PATH, "");
}

// v5: app-v5.js:renderFolderPickerBreadcrumbs
fn render_crumbs(app: &mut App, host: &mut dyn Host) {
    let s = &app.st.home_dialogs.picker;
    let last = s.stack.len().saturating_sub(1);
    let mut html = String::new();
    for (i, (_, name)) in s.stack.iter().enumerate() {
        html.push_str(&format!("<button class=\"folder-picker-breadcrumb{}\">{}</button>", if i == last { " active" } else { "" }, esc(name)));
        if i < last {
            html.push_str("<span>/</span>");
        }
    }
    let count = s.stack.len();
    let out = host.out();
    let old = std::mem::take(&mut app.st.home_dialogs.picker.crumbs);
    old.into_iter().for_each(|n| out.release(n));
    out.html(Ui::FOLDER_PICKER_BREADCRUMBS, &html);
    let crumbs = (0..count).map(|i| out.child(Ui::FOLDER_PICKER_BREADCRUMBS, &[(i * 2) as u8])).collect();
    st(app).crumbs = crumbs;
}

// v5: app-v5.js:refreshFolderPickerFolders
fn refresh_folders(app: &mut App, host: &mut dyn Host) {
    let current = st(app).current.clone();
    if shell::offline(app) && lists::library_ready(app) {
        let cached = local_folders(app, host, &current, None);
        return render_list(app, host, cached);
    }
    if crate::auth::token(app).is_none() {
        return;
    }
    let req = manage::subfolders(app, host, Mod::HomeDialogs, &current);
    request(app, Job::List(current), req);
}

/// The library's folders of `id` (merged with a Drive listing): 5.18.0 `library.folder` / `library.combine`.
fn local_folders(app: &App, host: &mut dyn Host, id: &str, remote: Option<Vec<Entry>>) -> Vec<Entry> {
    let mut order = |a: &str, b: &str| match host.intl(crate::IntlOp::Compare, &format!("{a}\0{b}\0\0")).as_str() {
        "-1" => std::cmp::Ordering::Less,
        "1" => std::cmp::Ordering::Greater,
        _ => std::cmp::Ordering::Equal,
    };
    let c = &app.st.home.catalog;
    let list = match remote {
        Some(remote) => c.combine(remote, id, None, &mut order),
        None => c.folder(id, None, &mut order),
    };
    list.into_iter().filter(Entry::is_folder).collect()
}

// v5: app-v5.js:renderFolderPickerList
fn render_list(app: &mut App, host: &mut dyn Host, folders: Vec<Entry>) {
    let mut html = String::new();
    for f in &folders {
        let color = f.folder_color.trim();
        let valid = color.len() == 7 && color.starts_with('#') && color[1..].bytes().all(|b| b.is_ascii_hexdigit());
        let svg = if valid { FOLDER_SVG.replacen("stroke-width=\"1.8\"", &format!("stroke-width=\"1.8\" style=\"color: {color};\""), 1) } else { FOLDER_SVG.into() };
        html.push_str(&format!("<div class=\"folder-picker-item\">{svg}<span>{}</span>\n                </div>", esc(if f.name.is_empty() { "Folder" } else { &f.name })));
    }
    let out = host.out();
    for (n, _) in std::mem::take(&mut app.st.home_dialogs.picker.folders) {
        out.release(n);
    }
    out.html(Ui::FOLDER_PICKER_LIST, &html);
    out.class(Ui::FOLDER_PICKER_EMPTY, "hidden", !folders.is_empty());
    let rows = folders.into_iter().enumerate().map(|(i, f)| (out.child(Ui::FOLDER_PICKER_LIST, &[i as u8]), f)).collect();
    st(app).folders = rows;
}

fn navigate(app: &mut App, host: &mut dyn Host) {
    let s = st(app);
    s.current = s.stack.last().map_or("root".into(), |c| c.0.clone());
    render_crumbs(app, host);
    refresh_folders(app, host);
}

// ---- the path of a moved / copied file --------------------------------------------------------------------------

// v5: app-v5.js:resolveDrivePathForFile
fn resolve_path(app: &mut App, host: &mut dyn Host, t: &Target) {
    let home = &app.st.home;
    if !t.parents.is_empty() && t.parents.contains(&home.current) && !home.stack.is_empty() {
        let label = home.stack.iter().map(|c| if c.name.is_empty() { "Folder" } else { &c.name[..] }).collect::<Vec<_>>().join(" / ");
        return show_path(app, host, &t.id, &label);
    }
    st(app).path = Some((t.id.clone(), Vec::new(), Vec::new()));
    if t.parents.is_empty() {
        let req = manage::get(app, host, Mod::HomeDialogs, &t.id, "parents");
        return request(app, Job::Parents(t.id.clone()), req);
    }
    walk(app, host, t.parents.first().cloned());
}

/// One step up the ancestors (at most 20), from the folder metadata cache or Drive.
fn walk(app: &mut App, host: &mut dyn Host, mut next: Option<String>) {
    loop {
        let Some((file, parts, visited)) = st(app).path.as_mut() else { return };
        let Some(current) = next.take().filter(|c| !c.is_empty() && c != "root" && !visited.contains(c) && parts.len() < 20) else {
            let label = std::iter::once("My Drive".to_string()).chain(parts.iter().rev().cloned()).collect::<Vec<_>>().join(" / ");
            let file = file.clone();
            st(app).path = None;
            return show_path(app, host, &file, &label);
        };
        visited.push(current.clone());
        let cached = app.st.home_dialogs.picker.meta.iter().find(|m| m.0 == current).cloned();
        match cached {
            Some((_, name, parents)) => {
                let Some((_, parts, _)) = st(app).path.as_mut() else { return };
                parts.push(if name.is_empty() { "Folder".into() } else { name });
                next = parents.first().cloned();
            }
            None => {
                let req = manage::get(app, host, Mod::HomeDialogs, &current, "id,name,parents");
                return request(app, Job::Folder(current), req);
            }
        }
    }
}

fn show_path(app: &mut App, host: &mut dyn Host, file: &str, label: &str) {
    let s = &app.st.home_dialogs.picker;
    if matches!(s.mode, Some(Mode::Move | Mode::Copy)) && s.target.as_ref().is_some_and(|t| t.id == file) {
        host.out().text(Ui::FOLDER_PICKER_FILE_PATH, &format!("Current: {label}"));
    }
}

fn strings(v: Option<&Json>) -> Vec<String> { v.map(|p| p.items().iter().filter_map(|i| i.as_str().map(String::from)).collect()).unwrap_or_default() }

// ---- confirm ----------------------------------------------------------------------------------------------------

// v5: app-v5.js:confirmFolderPicker
fn confirm(app: &mut App, host: &mut dyn Host) {
    let s = &app.st.home_dialogs.picker;
    let (Some(mode), name, folder) = (s.mode, s.name.trim().to_string(), s.current.clone()) else { return };
    match mode {
        Mode::Upload => {
            let Some(u) = s.upload.clone() else { return };
            if name.is_empty() || s.uploading {
                return;
            }
            st(app).uploading = true;
            let out = host.out();
            out.prop(Ui::FOLDER_PICKER_CONFIRM, "disabled", Prop::Bool(true));
            out.text(Ui::FOLDER_PICKER_CONFIRM, "Uploading…");
            upload(app, host, &u, &name, &folder);
        }
        Mode::Move => {
            let target = s.target.clone();
            close(app, host);
            if let Some(t) = target {
                // v5: app-v5.js:moveDriveFileToFolder
                shell::loading(app, host, "Moving file...");
                let req = manage::get(app, host, Mod::HomeDialogs, &t.id, "parents");
                request(app, Job::MoveParents { id: t.id, folder }, req);
            }
        }
        Mode::Copy => {
            let Some(t) = s.target.clone() else { return };
            if name.is_empty() {
                return;
            }
            close(app, host);
            // v5: app-v5.js:executeDriveCopy — the source's properties are read first so the copy can clear the
            // `ihn6_*` sync keys (ARCHITECTURE §13).
            shell::loading(app, host, "Creating copy...");
            let req = manage::get(app, host, Mod::HomeDialogs, &t.id, "properties");
            request(app, Job::CopyProps { id: t.id, name, folder }, req);
        }
        Mode::New => {
            if name.is_empty() {
                return;
            }
            close(app, host);
            doc::create::new_document(app, host, if folder == "root" { "" } else { &folder }, &name);
        }
    }
}

// v5: app-v5.js:uploadDeviceFile (BEHAVIOUR-CHANGES N-13: nothing is queued offline)
fn upload(app: &mut App, host: &mut dyn Host, u: &Upload, name: &str, folder: &str) {
    let ticket = st(app).ticket;
    if u.size <= 0.0 {
        return upload_failed(app, host, ticket, "Choose a non-empty file.");
    }
    if shell::offline(app) {
        return upload_failed(app, host, ticket, "Connect to upload this file.");
    }
    if app.st.home.catalog.entry(folder).is_some_and(|e| e.pending) {
        return upload_failed(app, host, ticket, "This folder is still waiting to upload. Try again shortly.");
    }
    let mime = if u.mime.is_empty() { "application/octet-stream" } else { &u.mime };
    let req = manage::upload_start(app, host, Mod::HomeDialogs, name, mime, u.size, folder);
    request(app, Job::Upload(ticket), req);
}

/// The upload ended for the picker that started it (5.18.0's `finally`).
fn upload_done(app: &mut App, host: &mut dyn Host, ticket: u32) -> bool {
    let s = st(app);
    if s.ticket != ticket || s.mode != Some(Mode::Upload) {
        return false;
    }
    s.uploading = false;
    let out = host.out();
    out.prop(Ui::FOLDER_PICKER_CONFIRM, "disabled", Prop::Bool(false));
    out.text(Ui::FOLDER_PICKER_CONFIRM, "Upload");
    true
}

fn upload_failed(app: &mut App, host: &mut dyn Host, ticket: u32, text: &str) {
    if upload_done(app, host, ticket) {
        host.out().text(Ui::FOLDER_PICKER_SUBTITLE, text);
    }
}

fn uploaded(app: &mut App, host: &mut dyn Host, ticket: u32, io: &Io) {
    let file = (io.status >= 0).then(|| Json::parse(Response::parse(io.data).text())).flatten();
    let Some(file) = file.filter(|f| !f.text("id").is_empty()) else {
        let text = if io.status >= 0 { "Drive did not confirm the upload. Retry without closing this dialog.".into() } else { badges::error_text(&error_of(io.status, io.data)) };
        return upload_failed(app, host, ticket, &text);
    };
    let name = app.st.home_dialogs.picker.name.trim().to_string();
    if app.st.home_dialogs.picker.ticket == ticket {
        close(app, host);
    }
    let mut e = entry(&file);
    e.name = name.clone();
    crate::home::entry_changed(app, host, &e);
    crate::home::refresh_quiet(app, host);
    let out = host.out();
    out.text(Ui::DRIVE_STATUS, &format!("Uploaded: {name}"));
    out.style(Ui::DRIVE_STATUS, "display", Some("block"));
}

pub fn io(app: &mut App, host: &mut dyn Host, io: &Io) -> bool {
    let Some(at) = st(app).jobs.iter().position(|j| j.0 == io.req) else { return false };
    let (_, job) = st(app).jobs.swap_remove(at);
    let ok = io.status >= 0;
    let body = || if ok { Json::parse(Response::parse(io.data).text()) } else { None };
    match job {
        Job::List(folder) => {
            let s = &app.st.home_dialogs.picker;
            if s.mode.is_none() || s.current != folder {
                return true;
            }
            let folders = body().map(|b| b.get("files").map(|f| f.items().iter().map(entry).collect::<Vec<_>>()).unwrap_or_default()).unwrap_or_default();
            let folders: Vec<Entry> = folders.into_iter().map(|mut e| { e.mime = manage::FOLDER.into(); e }).collect();
            let folders = app.st.home.catalog.visible(folders);
            let folders = if lists::library_ready(app) { local_folders(app, host, &folder, Some(folders)) } else { folders };
            render_list(app, host, folders);
        }
        Job::Parents(file) => {
            if st(app).path.as_ref().is_some_and(|p| p.0 == file) {
                let parents = strings(body().as_ref().and_then(|b| b.get("parents")));
                if parents.is_empty() {
                    st(app).path = None;
                    show_path(app, host, &file, "My Drive");
                } else {
                    walk(app, host, parents.first().cloned());
                }
            }
        }
        Job::Folder(folder) => {
            let meta = body().filter(|b| !b.text("id").is_empty());
            let next = meta.map(|m| {
                let (name, parents) = (m.text("name").to_string(), strings(m.get("parents")));
                let s = st(app);
                s.meta.push((folder, name.clone(), parents.clone()));
                if let Some((_, parts, _)) = s.path.as_mut() {
                    parts.push(if name.is_empty() { "Folder".into() } else { name });
                }
                parents.first().cloned()
            });
            match next {
                Some(next) => walk(app, host, next),
                // v5: `if (!meta) break;`
                None => walk(app, host, None),
            }
        }
        Job::Upload(ticket) => {
            let session = if ok { Response::parse(io.data).header("location").unwrap_or("").to_string() } else { String::new() };
            let file = app.st.home_dialogs.picker.upload.clone().filter(|_| app.st.home_dialogs.picker.ticket == ticket);
            match file {
                Some(u) if !session.is_empty() => {
                    let req = manage::upload_put(app, host, Mod::HomeDialogs, &session, u.obj, u.size);
                    request(app, Job::Put(ticket), req);
                }
                Some(_) if ok => upload_failed(app, host, ticket, "Upload failed: Drive did not return a resumable upload session"),
                _ => upload_failed(app, host, ticket, &badges::error_text(&error_of(io.status, io.data))),
            }
        }
        Job::Put(ticket) => uploaded(app, host, ticket, io),
        Job::MoveParents { id, folder } => {
            if !ok {
                shell::status(app, host, "Move failed", StatusOpts::from(Status::Info).preserve());
                shell::hide_loading(app, host);
                return true;
            }
            let parents = strings(body().as_ref().and_then(|b| b.get("parents")));
            let req = manage::move_to(app, host, Mod::HomeDialogs, &id, &folder, &parents);
            request(app, Job::Move, req);
        }
        Job::Move => {
            shell::status(app, host, if ok { "File moved" } else { "Move failed" }, StatusOpts::from(Status::Info).preserve());
            if ok {
                crate::home::refresh_quiet(app, host);
            }
            shell::hide_loading(app, host);
        }
        Job::CopyProps { id, name, folder } => {
            if !ok {
                shell::hide_loading(app, host);
                shell::status(app, host, "Error creating copy.", Status::Error);
                return true;
            }
            let props = body().and_then(|b| b.get("properties").cloned()).unwrap_or(Json::Null);
            let req = manage::copy(app, host, Mod::HomeDialogs, &id, &name, &folder, &props);
            request(app, Job::Copy, req);
        }
        Job::Copy => {
            shell::hide_loading(app, host);
            let file = body();
            match file.as_ref().filter(|f| !f.text("id").is_empty()) {
                Some(f) => {
                    shell::status(app, host, "Copy created! Opening...", Status::Info);
                    let file = doc::open::File { id: f.text("id").into(), name: f.text("name").into(), resource_key: String::new() };
                    doc::open::open_file(app, host, file, 0);
                }
                None if ok => shell::status(app, host, "Failed to create copy.", Status::Error),
                None => shell::status(app, host, "Error creating copy.", Status::Error),
            }
        }
        Job::NewFolder => {
            if ok {
                shell::status(app, host, "Folder created", StatusOpts::from(Status::Info).preserve());
                lists::refresh_folder_contents(app, host, false, None);
            } else {
                shell::status(app, host, "Failed to create folder", StatusOpts::from(Status::Info).preserve());
            }
        }
    }
    true
}

// ---- New folder -------------------------------------------------------------------------------------------------

// v5: app-v5.js:openNewFolderModal
fn open_new_folder(app: &mut App, host: &mut dyn Host) {
    if !can_manage(app) {
        return;
    }
    st(app).folder_name.clear();
    let out = host.out();
    out.prop(Ui::NEW_FOLDER_NAME, "value", Prop::Str(""));
    out.class(Ui::MODAL_NEW_FOLDER, "visible", true);
    want_frame(app, host, frame::FOLDER_FOCUS);
}

fn close_new_folder(host: &mut dyn Host) { host.out().class(Ui::MODAL_NEW_FOLDER, "visible", false); }

// v5: app-v5.js:confirmNewFolderModal / createDriveFolder (BEHAVIOUR-CHANGES N-13: Drive only)
fn confirm_new_folder(app: &mut App, host: &mut dyn Host) {
    let name = st(app).folder_name.trim().to_string();
    if name.is_empty() {
        return;
    }
    close_new_folder(host);
    if shell::offline(app) || crate::auth::token(app).is_none() {
        shell::status(app, host, "Failed to create folder", StatusOpts::from(Status::Info).preserve());
        return;
    }
    let parent = app.st.home.current.clone();
    let req = manage::create_folder(app, host, Mod::HomeDialogs, &parent, &name);
    request(app, Job::NewFolder, req);
}

pub fn dom(app: &mut App, host: &mut dyn Host, e: &DomEvent) -> u32 {
    match e.listener {
        L_CANCEL => close(app, host),
        L_CONFIRM => confirm(app, host),
        L_OVERLAY if on_overlay(e, Ui::MODAL_FOLDER_PICKER) => close(app, host),
        L_NAME_INPUT => {
            st(app).name = e.value.into();
            update_confirm(app, host);
        }
        L_NAME_KEY if e.key == "Enter" => {
            confirm(app, host);
            return ret::PREVENT_DEFAULT;
        }
        L_CRUMBS => {
            let s = st(app);
            let last = s.crumbs.len().saturating_sub(1);
            if let Some(i) = s.crumbs.iter().position(|c| e.path.contains(*c)).filter(|i| *i < last) {
                s.stack.truncate(i + 1);
                navigate(app, host);
            }
        }
        L_LIST => {
            let s = st(app);
            if let Some(f) = s.folders.iter().find(|f| e.path.contains(f.0)).map(|f| f.1.clone()) {
                if !f.id.is_empty() {
                    s.stack.push((f.id.clone(), if f.name.is_empty() { "Folder".into() } else { f.name }));
                    navigate(app, host);
                }
            }
        }
        L_NEW_FOLDER | L_VIEW_NEW_FOLDER => open_new_folder(app, host),
        L_VIEW_NEW_PDF => open(app, host, Mode::New, None, None),
        L_FOLDER_CANCEL => close_new_folder(host),
        L_FOLDER_CONFIRM => confirm_new_folder(app, host),
        L_FOLDER_OVERLAY if on_overlay(e, Ui::MODAL_NEW_FOLDER) => close_new_folder(host),
        L_FOLDER_INPUT => st(app).folder_name = e.value.into(),
        L_FOLDER_KEY if e.ty == ety::KEYDOWN && e.key == "Enter" => {
            confirm_new_folder(app, host);
            return ret::PREVENT_DEFAULT;
        }
        L_FOLDER_KEY if e.key == "Escape" => {
            close_new_folder(host);
            return ret::PREVENT_DEFAULT;
        }
        _ => {}
    }
    0
}

pub fn state_json(s: &State) -> String {
    match s.mode {
        None => "null".into(),
        Some(m) => format!(
            "{{\"mode\":\"{}\",\"current\":{},\"stack\":[{}],\"folders\":[{}],\"name\":{},\"uploading\":{}}}",
            m.name(),
            json_str(&s.current),
            s.stack.iter().map(|c| json_str(&c.1)).collect::<Vec<_>>().join(","),
            s.folders.iter().map(|f| json_str(&f.1.id)).collect::<Vec<_>>().join(","),
            json_str(&s.name),
            s.uploading
        ),
    }
}
