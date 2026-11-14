//! Editor: Editor skeleton binding, read/edit mode, title.
//!
//! Follows the document session: an open or a creation shows an empty editor in Read mode with the file's name
//! (5.18.0 `openDriveFile`), `session::sig::OPENED` builds the page rows and centres the column
//! (`renderAllPages` + `centerViewOnTop`), `CHANGED` repaints, `CLOSED` empties the column. Read/Edit
//! (`setReadOnlyMode`), the viewer's toolbar (`updateViewerModeUI`) and the title (`updateDocTitle`, title
//! editing and the Drive rename) are here.

use super::{pages, viewport};
use crate::abi::{ety, ret, DomEvent};
use crate::doc::{create, open, session};
use crate::drive::client::{self, Req};
use crate::shell::{self, Status, StatusOpts};
use crate::{tag, App, Event, Host, Metric, Mod, Node, Prop, Ui};

/// Broadcasts of the editor view (`Event::Msg`, sent with `shell::signal`).
pub mod sig {
    /// Read/Edit changed; data = `read` | `edit`. Modules with editor popups close them on `read` (eraser menu,
    /// pages panel, selection); the history module sets its buttons on `edit`.
    pub const MODE: u8 = 0x60;
}

const L_MODE: u32 = tag(Mod::EditorView, 1);
const L_TITLE: u32 = tag(Mod::EditorView, 2);
const F_FOCUS_TITLE: u32 = 1;

/// Toolbar buttons without an id (`#toolbar button` = these five + `#btn-eraser`, `#btn-undo`, `#btn-redo`).
const TOOLBAR_BUTTONS: [&str; 4] = ["[title=\"Black Pen\"]", "[title=\"Blue Pen\"]", "[title=\"Red Pen\"]", "[title=\"Lasso Select\"]"];
const ERASER_SIZES: [&str; 4] = ["4", "12", "22", "32"];

pub struct State {
    pub read_only: bool,
    toolbar: Vec<Node>,
    eraser_buttons: Vec<Node>,
    title_editing: bool,
    title_original: String,
    /// The title shown (to compare on blur when the host reports no text).
    title: String,
    new_doc: bool,
    rename_req: u32,
    frames: u32,
}

impl Default for State {
    fn default() -> Self {
        State { read_only: true, toolbar: Vec::new(), eraser_buttons: Vec::new(), title_editing: false, title_original: String::new(), title: "Untitled".into(), new_doc: false, rename_req: 0, frames: 0 }
    }
}

/// v5 `state.isReadOnly`.
pub fn read_only(app: &App) -> bool { app.st.editor_view.read_only }

/// v5: `getBaseDocTitle(name)`: trimmed, trailing `.pdf` removed, `Untitled` when empty.
pub fn base_title(name: &str) -> String {
    let t = name.trim();
    let lower = t.to_ascii_lowercase();
    let stripped = if lower.ends_with(".pdf") { &t[..t.len() - 4] } else { t };
    if stripped.is_empty() { "Untitled".into() } else { stripped.into() }
}

/// v5: `normalizeDocTitleInput(value)` (+ `sanitizeFilename`).
pub fn normalize_title(value: &str) -> String {
    let cleaned = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        return "Untitled".into();
    }
    let no_ext = if cleaned.to_ascii_lowercase().ends_with(".pdf") { cleaned[..cleaned.len() - 4].trim().to_string() } else { cleaned };
    let mut out = String::new();
    let mut run = false;
    for c in no_ext.chars() {
        let bad = "\\/:*?\"<>|".contains(c);
        if !bad {
            out.push(c);
        } else if !run {
            out.push('_');
        }
        run = bad;
    }
    if out.is_empty() { "Untitled".into() } else { out }
}

/// v5: `setReadOnlyMode(next, {force, user})`.
pub fn set_read_only(app: &mut App, host: &mut dyn Host, next: bool, force: bool) {
    let was = app.st.editor_view.read_only;
    if next == was && !force {
        return;
    }
    app.st.editor_view.read_only = next;
    let out = host.out();
    out.class(Ui::EDITOR_VIEW, "read-only", next);
    out.class(Node::BODY, "read-only", next);
    out.class(Ui::MODE_TOGGLE_BTN, "is-editing", !next);
    out.attr(Ui::MODE_TOGGLE_BTN, "aria-pressed", Some(if next { "false" } else { "true" }));
    out.attr(Ui::MODE_TOGGLE_BTN, "aria-label", Some(if next { "Read mode" } else { "Edit mode" }));
    out.text(Ui::MODE_TOGGLE_LABEL, if next { "Read" } else { "Edit" });
    out.attr(Ui::DOC_TITLE, "contenteditable", Some(if next { "false" } else { "true" }));
    out.attr(Ui::DOC_TITLE, "aria-readonly", Some(if next { "true" } else { "false" }));
    out.attr(Ui::DOC_TITLE, "tabindex", Some(if next { "-1" } else { "0" }));
    for n in [Ui::BTN_IMPORT_PDF, Ui::BTN_SCAN_PAGE, Ui::BTN_UNDO, Ui::BTN_REDO, Ui::BTN_ERASER, Ui::ERASER_STROKE_TOGGLE] {
        out.prop(n, "disabled", Prop::Bool(next));
    }
    for &n in app.st.editor_view.toolbar.iter().chain(app.st.editor_view.eraser_buttons.iter()) {
        out.prop(n, "disabled", Prop::Bool(next));
    }
    out.class(Ui::TOOLBAR, "toolbar-flash", false);
    if !next {
        host.metric(Ui::TOOLBAR, Metric::OffsetWidth); // v5: `void toolbar.offsetWidth` restarts the animation
        host.out().class(Ui::TOOLBAR, "toolbar-flash", true);
    }
    if next {
        if app.st.editor_view.title_editing {
            let text = app.st.editor_view.title.clone();
            finish_title(app, host, false, &text);
        }
        viewport::end_interaction(app, host);
        let v = &mut app.st.editor_viewport;
        v.pending_create = false;
        v.overscroll_y = 0.0;
        viewport::update_indicator(app, host);
    } else {
        pages::schedule_visible(app, host, false);
    }
    shell::signal(app, host, sig::MODE, if next { "read" } else { "edit" });
}

/// The name the editor shows: the Drive file name, else the document's name (v5 `driveFileName || exportName`).
fn doc_name(app: &App) -> String {
    match session::get(app) {
        Some(s) if !s.record.name.is_empty() => s.record.name.clone(),
        Some(s) if !s.doc.name.v.is_empty() => s.doc.name.v.clone(),
        _ => String::new(),
    }
}

fn show_title(app: &mut App, host: &mut dyn Host, name: &str) {
    if app.st.editor_view.title_editing {
        return;
    }
    let t = base_title(if name.is_empty() { "Untitled" } else { name });
    host.out().text(Ui::DOC_TITLE, &t);
    app.st.editor_view.title = t;
}

/// v5: `updateDocTitle()`: the title, and `#btn-drive-share` shown for a Drive document.
pub fn update_title(app: &mut App, host: &mut dyn Host) {
    let name = doc_name(app);
    show_title(app, host, &name);
    let has_file = session::get(app).is_some_and(|s| !s.doc_id.is_empty());
    host.out().class(Ui::BTN_DRIVE_SHARE, "hidden", !has_file);
}

/// v5: the toolbar part of `updateViewerModeUI()` (the banner, the toggle's display and the title's editability
/// are `doc::open`'s).
fn viewer_ui(app: &mut App, host: &mut dyn Host) {
    let viewer = session::get(app).is_some_and(|s| !s.can_edit);
    let out = host.out();
    out.prop(Ui::BTN_ERASER, "disabled", Prop::Bool(viewer));
    out.prop(Ui::BTN_SET_COVER, "disabled", Prop::Bool(viewer));
    out.style(Ui::TOOLBAR, "display", Some(if viewer { "none" } else { "" }));
    if viewer {
        out.style(Ui::BTN_DRIVE_SHARE, "pointer-events", Some("auto"));
        out.prop(Ui::BTN_DRIVE_SHARE, "disabled", Prop::Bool(false));
        set_read_only(app, host, true, true);
    }
    let ro = read_only(app);
    let out = host.out();
    out.attr(Ui::MODE_TOGGLE_BTN, "aria-disabled", None);
    out.attr(Ui::MODE_TOGGLE_BTN, "aria-pressed", Some(if ro { "false" } else { "true" }));
    out.attr(Ui::MODE_TOGGLE_BTN, "aria-label", Some(if ro { "Read mode" } else { "Edit mode" }));
    out.prop(Ui::MODE_TOGGLE_BTN, "title", Prop::Str("Toggle read/edit mode"));
    out.text(Ui::MODE_TOGGLE_LABEL, if ro { "Read" } else { "Edit" });
}

/// The start of an open or a creation (v5 `openDriveFile` before the download): empty column, zoom 1, Read.
fn begin(app: &mut App, host: &mut dyn Host) {
    // v5 `beginDocumentSession()` notes user activity: the open's first visible passes wait for quiet input (240 ms).
    let now = host.now();
    app.sched.activity(now);
    let name = open::opening_name(app).map(str::to_string).or_else(|| create::pending_name(app).map(|n| format!("{n}.pdf")));
    app.st.editor_view.new_doc = create::pending_name(app).is_some();
    pages::clear(app, host);
    pages::reset_template_urls(app);
    viewport::reset(app, host);
    set_read_only(app, host, true, true);
    if let Some(name) = name {
        show_title(app, host, &name);
    }
    viewport::align_top(app, host, super::layout::A4_WIDTH);
}

fn opened(app: &mut App, host: &mut dyn Host) {
    session::take_changes(app);
    pages::rebuild_opened(app, host);
    pages::set_active_page(app, 0);
    viewport::center_on_top(app, host);
    update_title(app, host);
    let new_doc = std::mem::take(&mut app.st.editor_view.new_doc);
    set_read_only(app, host, true, true);
    viewer_ui(app, host);
    if new_doc && session::get(app).is_some_and(|s| s.can_edit) {
        set_read_only(app, host, false, false);
    }
}

// ---- title editing ----------------------------------------------------------------------------------------------

fn finish_title(app: &mut App, host: &mut dyn Host, commit: bool, text: &str) {
    if read_only(app) && commit {
        return;
    }
    let next = normalize_title(text);
    let original = normalize_title(&app.st.editor_view.title_original);
    app.st.editor_view.title_editing = false;
    host.out().class(Ui::DOC_TITLE, "editing", false);
    if !commit {
        let t = base_title(&original);
        host.out().text(Ui::DOC_TITLE, &t);
        update_title(app, host);
        return;
    }
    if next != original {
        session::edit(app, host, |tx| tx.set_name(&next));
        // Only a document already in Drive is PATCHed (5.18.0 `renameDriveFile` needs `driveFileId`); one created on
        // this device and not in Drive yet carries the new name in its create.
        let local = session::get(app).is_none_or(|s| !s.record.flags.created || s.account.is_empty());
        if local {
            if let Some(s) = session::get_mut(app) {
                s.record.name = format!("{next}.pdf");
            }
        } else if let Some(id) = session::get(app).map(|s| s.doc_id.clone()) {
            let body = format!("{{\"name\":{}}}", crate::testapi::json_str(&format!("{next}.pdf")));
            let req = Req {
                method: "PATCH",
                headers: "Content-Type: application/json\n".into(),
                body: client::Body::Bytes(body.into_bytes()),
                ..Req::get(&format!("https://www.googleapis.com/drive/v3/files/{id}"))
            };
            app.st.editor_view.rename_req = client::request(app, host, Mod::EditorView, req);
            // The logical name changes at once; the Drive name follows the answer.
            let actor = session::get(app).and_then(|s| s.author.as_ref()).map_or(ihn_core::id::ActorId(0), |a| a.actor);
            crate::export::upload::pushed_name(app, host, format!("{next}.pdf"), actor);
        }
    }
    update_title(app, host);
}

fn title_event(app: &mut App, host: &mut dyn Host, e: &DomEvent) -> u32 {
    let ro = read_only(app);
    match e.ty {
        ety::FOCUS => {
            if ro {
                host.out().blur(Ui::DOC_TITLE);
            } else {
                app.st.editor_view.title_editing = true;
                app.st.editor_view.title_original = app.st.editor_view.title.clone();
                host.out().class(Ui::DOC_TITLE, "editing", true);
            }
            0
        }
        ety::BLUR => {
            if !ro && app.st.editor_view.title_editing {
                finish_title(app, host, true, e.value);
            }
            0
        }
        ety::KEYDOWN => {
            if ro {
                return 0;
            }
            match e.key {
                "Enter" => {
                    host.out().blur(Ui::DOC_TITLE);
                    ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION
                }
                "Escape" => {
                    finish_title(app, host, false, e.value);
                    host.out().blur(Ui::DOC_TITLE);
                    ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION
                }
                _ => ret::STOP_PROPAGATION,
            }
        }
        ety::CLICK => {
            if ro {
                set_read_only(app, host, false, false);
                app.st.editor_view.frames |= F_FOCUS_TITLE;
                app.sched.frame(host.out(), Mod::EditorView);
            }
            ret::STOP_PROPAGATION
        }
        _ => ret::STOP_PROPAGATION, // pointerdown
    }
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(app: &mut App, host: &mut dyn Host) {
    if app.cfg.has(crate::abi::mode::HEADLESS) {
        return; // the `?offline-worker` page has no editor UI
    }
    let out = host.out();
    let toolbar: Vec<Node> = TOOLBAR_BUTTONS.iter().map(|s| out.query(Ui::TOOLBAR, &format!("button{s}"))).collect();
    let eraser: Vec<Node> = ERASER_SIZES.iter().map(|s| out.query(Ui::ERASER_MENU, &format!("button[data-eraser-size=\"{s}\"]"))).collect();
    out.listen(Ui::MODE_TOGGLE_BTN, "click", L_MODE, 0);
    for ev in ["focus", "blur", "keydown", "pointerdown", "click"] {
        out.listen(Ui::DOC_TITLE, ev, L_TITLE, 0);
    }
    app.st.editor_view.toolbar = toolbar;
    app.st.editor_view.eraser_buttons = eraser;
    // v5: init() forces Read mode.
    set_read_only(app, host, true, true);
}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Dom(e) if e.listener == L_MODE => {
            let next = !read_only(app);
            set_read_only(app, host, next, false);
            0
        }
        Event::Dom(e) if e.listener == L_TITLE => title_event(app, host, &e),
        Event::Pointer(p) if p.listener == L_TITLE => ret::STOP_PROPAGATION,
        Event::Frame(_) => {
            if std::mem::take(&mut app.st.editor_view.frames) & F_FOCUS_TITLE != 0 {
                host.out().focus(Ui::DOC_TITLE, false);
            }
            0
        }
        Event::Io(io) if io.req == app.st.editor_view.rename_req => {
            app.st.editor_view.rename_req = 0;
            if io.status >= 0 {
                let body = crate::shell::fx::Response::parse(io.data).text();
                let name = crate::shell::json::Json::parse(body).map(|j| j.text("name").to_string()).unwrap_or_default();
                if let (Some(s), false) = (session::get_mut(app), name.is_empty()) {
                    s.record.name = name;
                }
                update_title(app, host);
            } else {
                update_title(app, host);
                shell::status(app, host, "Rename failed", StatusOpts::from(Status::Info).preserve());
            }
            0
        }
        Event::Msg(m) => {
            let data = std::str::from_utf8(m.data).unwrap_or("");
            match m.chan {
                shell::sig::VIEW if data == "editor" => {
                    if open::busy(app) || create::pending_name(app).is_some() {
                        begin(app, host);
                    }
                }
                session::sig::OPENED if !data.is_empty() && data == session::doc_id(app) => opened(app, host),
                session::sig::CHANGED if shell::view(app) == shell::View::Editor => {
                    let c = session::take_changes(app);
                    pages::changed(app, host, &c);
                    if c.name {
                        update_title(app, host);
                    }
                }
                session::sig::CLOSED => {
                    pages::clear(app, host);
                    viewport::update_indicator(app, host);
                }
                _ => {}
            }
            0
        }
        _ => 0,
    }
}

/// `__IHN_TEST_API__.cmd('editor.view.state')`.
pub fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    match name {
        "state" => Some(format!(
            "{{\"readOnly\":{},\"title\":{},\"editing\":{}}}",
            read_only(app),
            crate::testapi::json_str(&app.st.editor_view.title),
            app.st.editor_view.title_editing
        )),
        "edit" => {
            set_read_only(app, host, false, false);
            Some("{}".into())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_follow_v5() {
        assert_eq!(base_title("  Notes.PDF "), "Notes");
        assert_eq!(base_title(".pdf"), "Untitled");
        assert_eq!(base_title(""), "Untitled");
        assert_eq!(normalize_title("  a   b.pdf "), "a b");
        assert_eq!(normalize_title("x/:y"), "x_y");
        assert_eq!(normalize_title("   "), "Untitled");
    }
}
