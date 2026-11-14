//! Signed-out public links (SYNC §12.4, spec/home.md §8.3–8.4): 5.18.0's mechanism unchanged. The PDF is
//! downloaded anonymously (`drive.usercontent.google.com`, then `drive.google.com/uc`, credentials omitted, a
//! `%PDF-` check, three attempts), opened read-only (`doc::open::open_public`) and re-downloaded every 2.5 s while
//! visible; a new size + FNV-1a fingerprint of its first and last 64 KiB is merged into the open document. When the
//! download is refused, Google's preview iframe is shown instead (pan/zoom gestures, reloaded at most every 7 s while
//! the poll fails). Writer links add the sign-in gate on the read-only banner. No request carries an API key.
//!
//! Also the viewer banner's `Ask to Edit` button (5.18.0 `askToEdit`).
// v5: app-v5.js:tryOpenPendingSharedWithoutSignIn / openSharedFileWithoutSignIn / fetchPublicSharedPdf /
// startPublicSharedPolling / openPublicDrivePreview / showPublicInlinePreview / installPublicPreviewGestures /
// showSharedEditorSignInGate / clearSharedEditorSignInGate / updateViewerModeUI (anonymous viewer) / askToEdit

use super::links;
use super::open::{self, File};
use super::session;
use crate::abi::{ev, life, listen, mode, ptr, ret, tag, Event, Io, Node};
use crate::dom::Prop;
use crate::shell::fx::{self, fetch_flag, Nav, Response};
use crate::shell::{sig, Status, StatusOpts, View};
use crate::testapi::json_str;
use crate::ui_ids::Ui;
use crate::{auth, shell, App, Host, Mod};
use ihn_core::jsfmt::num;
use ihn_core::jsmath;

const fn id(n: u32) -> u32 { tag(Mod::DocPublic, n) }
const T_POLL: u32 = id(1);
const T_FRAMES: u32 = id(2);
const L_ASK: u32 = id(3);
const L_DOWN: u32 = id(4);
const L_MOVE: u32 = id(5);
const L_UP: u32 = id(6);
const L_CANCEL: u32 = id(7);
const L_WHEEL: u32 = id(8);
const L_DBL: u32 = id(9);
const L_CONTROLS: u32 = id(10);
/// `+ n` for the n-th preview frame's `load`.
const L_FRAME: u32 = id(0x100);

/// 5.18.0 `setInterval(poll, PUBLIC_DRIVE_API_KEY ? 1800 : 2500)`; production has no key.
pub const POLL_MS: f64 = 2500.0;
const DOWNLOAD_TIMEOUT_MS: f64 = 45000.0;
const ATTEMPTS: u32 = 3;
const SAMPLE: f64 = 65536.0;
const IFRAME_RETRY_MS: f64 = 7000.0;
const VIEW_ONLY_HTML: &str = "<strong>View Only</strong> You don't have permission to edit this file.";

/// The anonymous download endpoints of 5.18.0 (without an API key).
pub fn endpoints(id: &str, resource_key: &str) -> [String; 2] {
    let suffix = if resource_key.is_empty() { String::new() } else { format!("&resourcekey={}", fx::url_encode(resource_key)) };
    let id = fx::url_encode(id);
    [format!("https://drive.usercontent.google.com/download?id={id}&export=download{suffix}"), format!("https://drive.google.com/uc?export=download&id={id}{suffix}")]
}

/// `buildPublicDrivePreviewUrl` (URLSearchParams order: embedded, resourcekey, ihn_refresh).
pub fn preview_url(id: &str, resource_key: &str, bust: f64) -> String {
    let mut q = String::from("embedded=true");
    if !resource_key.is_empty() {
        q.push_str(&format!("&resourcekey={}", fx::form_encode(resource_key)));
    }
    q.push_str(&format!("&ihn_refresh={}", num(bust)));
    format!("https://drive.google.com/file/d/{}/preview?{q}", fx::url_encode(id))
}

/// `parseFilenameFromContentDisposition`.
pub fn disposition_name(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if let Some(at) = lower.find("filename*=utf-8''") {
        let raw = value[at + 17..].split(';').next().unwrap_or("").trim().trim_start_matches(['"', '\'']).trim_end_matches(['"', '\'']);
        if let Some(decoded) = percent_decode(raw) {
            if !decoded.is_empty() {
                return decoded;
            }
        }
    }
    let Some(at) = lower.find("filename=") else { return String::new() };
    let rest = value[at + 9..].strip_prefix('"').unwrap_or(&value[at + 9..]);
    rest.split(['"', ';']).next().unwrap_or("").trim().to_string()
}

fn percent_decode(s: &str) -> Option<String> {
    let (b, mut out, mut i) = (s.as_bytes(), Vec::with_capacity(s.len()), 0);
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `isPdfArrayBuffer`: `%PDF-` within the first 1024 bytes.
pub fn is_pdf(head: &[u8]) -> bool { head.len() >= 5 && head[..head.len().min(1024)].windows(5).any(|w| w == b"%PDF-") }

/// FNV-1a over `bytes`, continuing from `hash` (5.18.0 `fingerprintPublicPdfBlob`).
pub fn fnv(mut hash: u32, bytes: &[u8]) -> u32 {
    for &b in bytes {
        hash = (hash ^ b as u32).wrapping_mul(16777619);
    }
    hash
}

/// `${size}:${hash.toString(36)}`.
pub fn fingerprint(size: f64, hash: u32) -> String {
    let (mut digits, mut h) = (Vec::new(), hash);
    loop {
        digits.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(h % 36) as usize]);
        h /= 36;
        if h == 0 {
            break;
        }
    }
    digits.reverse();
    format!("{}:{}", num(size), String::from_utf8(digits).unwrap_or_default())
}

/// One anonymous download (`fetchPublicSharedPdf`) followed by its fingerprint.
#[derive(Default)]
struct Dl {
    poll: bool,
    generation: u32,
    attempt: u32,
    endpoint: usize,
    req: u32,
    stage: u8,
    blob: u32,
    size: f64,
    name: String,
    hash: u32,
}

#[derive(Default)]
struct Preview {
    host: Node,
    stage: Node,
    controls: Node,
    buttons: [Node; 3],
    frames: Vec<Node>,
    frame_serial: u32,
    scale: f64,
    px: f64,
    py: f64,
    pointers: Vec<(i32, f64, f64)>,
    center: Option<(f64, f64)>,
    distance: f64,
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    hidden: bool,
    /// 5.18.0 `lastPendingSharedPublicAttemptId`.
    attempted: String,
    /// 5.18.0 `sharedEditorSignInGateActive`.
    gate: bool,
    /// The public document (`publicSharedPreviewFileId`, its key, its name) while one is shown or opening.
    link: Option<links::Link>,
    opening: bool,
    open_req: u32,
    dl: Option<Dl>,
    /// Poll: generation (5.18.0 `publicSharedPollGeneration`), running, preview only, last fingerprint, in flight.
    generation: u32,
    polling: bool,
    preview_only: bool,
    print: String,
    in_flight: bool,
    refresh_req: u32,
    iframe_at: f64,
    preview: Option<Preview>,
    mode_group: Node,
    /// Downloads made (test command `state`).
    downloads: u32,
    updates: u32,
}

/// The public document on screen or opening (`id`, resource key), for the share button and `Ask to Edit`.
pub fn file(app: &App) -> Option<(&str, &str)> { app.st.doc_public.link.as_ref().map(|l| (&l.id[..], &l.resource_key[..])) }
/// The sign-in gate of a writer link is up.
pub fn gate(app: &App) -> bool { app.st.doc_public.gate }

fn connected(app: &App) -> bool {
    auth::token(app).is_some() || auth::reconnecting(app) || auth::local_library(app) || app.cfg.has(shell::fx::BOOT_SIGNED_IN)
}

// v5: tryOpenPendingSharedWithoutSignIn
fn try_open(app: &mut App, host: &mut dyn Host, force: bool) {
    let Some(link) = links::pending(app).cloned() else { return };
    let st = &app.st.doc_public;
    if auth::token(app).is_some() || st.opening || open::busy(app) || (!force && st.attempted == link.id) {
        return;
    }
    app.st.doc_public.attempted = link.id.clone();
    // v5: openSharedFileWithoutSignIn
    stop_polling(app, host);
    let st = &mut app.st.doc_public;
    st.opening = true;
    st.link = Some(link);
    shell::show_view(app, host, View::Home);
    shell::loading(app, host, "Opening shared document...");
    download(app, host, false);
}

fn download(app: &mut App, host: &mut dyn Host, poll: bool) {
    let generation = app.st.doc_public.generation;
    app.st.doc_public.dl = Some(Dl { poll, generation, ..Dl::default() });
    fetch_next(app, host);
}

/// The next endpoint / attempt, or the failure of the whole download.
fn fetch_next(app: &mut App, host: &mut dyn Host) {
    let req = app.req(Mod::DocPublic);
    let st = &mut app.st.doc_public;
    let (Some(dl), Some(link)) = (st.dl.as_mut(), st.link.as_ref()) else { return };
    if dl.blob != 0 {
        host.out().obj_release(std::mem::take(&mut dl.blob));
    }
    if dl.endpoint == 2 {
        (dl.endpoint, dl.attempt) = (0, dl.attempt + 1);
    }
    if dl.attempt >= ATTEMPTS {
        let dl = st.dl.take().unwrap_or_default();
        return downloaded(app, host, dl, false);
    }
    let url = endpoints(&link.id, &link.resource_key)[dl.endpoint].clone();
    (dl.endpoint, dl.req, dl.stage, dl.hash) = (dl.endpoint + 1, req, 1, 2166136261);
    st.downloads += 1;
    fx::fetch(host.out(), req, &fx::Fetch {
        url: &url,
        flags: fetch_flag::BLOB | fetch_flag::NO_STORE | fetch_flag::OMIT_CREDENTIALS,
        deadline_ms: DOWNLOAD_TIMEOUT_MS,
        ..fx::Fetch::default()
    });
}

fn download_step(app: &mut App, host: &mut dyn Host, io: &Io) {
    let req = app.req(Mod::DocPublic);
    let Some(dl) = app.st.doc_public.dl.as_mut() else { return };
    let read = |dl: &mut Dl, host: &mut dyn Host, start: f64, end: f64| {
        dl.req = req;
        fx::blob_read(host.out(), req, dl.blob, start, end);
    };
    match dl.stage {
        1 => {
            if !(200..300).contains(&io.status) || io.obj == 0 {
                if io.obj != 0 {
                    host.out().obj_release(io.obj);
                }
                return fetch_next(app, host);
            }
            let r = Response::parse(io.data);
            (dl.blob, dl.size, dl.name) = (io.obj, r.blob_size(), disposition_name(r.header("content-disposition").unwrap_or("")));
            dl.stage = 2;
            read(dl, host, 0.0, 1024.0);
        }
        2 if io.status < 0 || !is_pdf(io.data) => fetch_next(app, host),
        2 => {
            dl.stage = 3;
            read(dl, host, 0.0, SAMPLE.min(dl.size));
        }
        3 | 4 if io.status < 0 => fetch_next(app, host),
        3 => {
            dl.hash = fnv(dl.hash, io.data);
            dl.stage = 4;
            let size = dl.size;
            read(dl, host, (size - SAMPLE.min(size)).max(0.0), size);
        }
        _ => {
            dl.hash = fnv(dl.hash, io.data);
            let dl = app.st.doc_public.dl.take().unwrap_or_default();
            downloaded(app, host, dl, true);
        }
    }
}

fn downloaded(app: &mut App, host: &mut dyn Host, dl: Dl, ok: bool) {
    let st = &mut app.st.doc_public;
    if dl.poll && dl.generation != st.generation {
        // The poll was stopped while it ran.
        if dl.blob != 0 {
            host.out().obj_release(dl.blob);
        }
        return;
    }
    let Some(link) = st.link.clone() else { return };
    let print = fingerprint(dl.size, dl.hash);
    if !dl.poll {
        if !ok {
            // v5: the anonymous Drive preview.
            st.opening = false;
            shell::hide_loading(app, host);
            shell::home_status(host, "Opening public view — no sign-in required.");
            return open_preview(app, host, &link);
        }
        let name = [&dl.name[..], &link.name[..], "Shared Document"].into_iter().find(|n| !n.is_empty()).unwrap_or_default().to_string();
        st.print = print;
        let req = app.req(Mod::DocPublic);
        app.st.doc_public.open_req = req;
        let file = File { id: link.id, name, resource_key: link.resource_key };
        if !open::open_public(app, host, file, dl.blob, dl.size, false, req) {
            host.out().obj_release(dl.blob);
            app.st.doc_public.opening = false;
            shell::hide_loading(app, host);
        }
        return;
    }
    if !ok || print == st.print {
        if dl.blob != 0 {
            host.out().obj_release(dl.blob);
        }
        st.in_flight = false;
        if !ok && st.preview_only && host.time() - st.iframe_at >= IFRAME_RETRY_MS {
            show_preview(app, host, &link);
        }
        return;
    }
    // v5: applyPublicPdfUpdate — the new export replaces what is shown.
    st.print = print;
    let req = app.req(Mod::DocPublic);
    app.st.doc_public.refresh_req = req;
    let name = if dl.name.is_empty() { session::get(app).map(|s| s.record.name.clone()).unwrap_or_default() } else { dl.name };
    let file = File { id: link.id, name: if name.is_empty() { "Shared Document".into() } else { name }, resource_key: link.resource_key };
    if !open::open_public(app, host, file, dl.blob, dl.size, true, req) {
        host.out().obj_release(dl.blob);
        app.st.doc_public.in_flight = false;
    }
}

// v5: the success tail of openSharedFileWithoutSignIn
fn opened(app: &mut App, host: &mut dyn Host, ok: bool) {
    let st = &mut app.st.doc_public;
    st.opening = false;
    let Some(link) = st.link.clone() else { return };
    if !ok {
        shell::home_status(host, "Opening public view — no sign-in required.");
        return open_preview(app, host, &link);
    }
    clear_preview(app, host);
    start_polling(app, host, false);
    shell::show_view(app, host, View::Editor);
    // The column was centred at `OPENED`, while the editor was still hidden (no size): centre it now that it shows
    // (5.18.0 shows the editor before `centerViewOnTop`).
    crate::editor::viewport::center_on_top(app, host);
    viewer_ui(app, host, link.writer);
    if link.writer {
        show_gate(app, host);
    } else {
        shell::set_pending_share(app, None);
    }
    app.st.doc_public.attempted = if link.writer { link.id } else { String::new() };
    shell::status(app, host, if link.writer { "Shared PDF loaded. Sign in to edit and save." } else { "Shared PDF loaded (View Only)" }, StatusOpts::from(Status::Info).preserve());
}

fn refreshed(app: &mut App, host: &mut dyn Host, ok: bool) {
    let st = &mut app.st.doc_public;
    st.in_flight = false;
    if !ok {
        return;
    }
    st.updates += 1;
    st.preview_only = false;
    let writer = st.link.as_ref().is_some_and(|l| l.writer);
    clear_preview(app, host);
    shell::show_view(app, host, View::Editor);
    viewer_ui(app, host, writer);
    shell::status(app, host, "Public document updated.", StatusOpts::from(Status::Info).preserve());
}

// v5: openPublicDrivePreview
fn open_preview(app: &mut App, host: &mut dyn Host, link: &links::Link) {
    session::close(app, host);
    show_preview(app, host, link);
    shell::show_view(app, host, View::Editor);
    viewer_ui(app, host, link.writer);
    if link.writer {
        show_gate(app, host);
    } else {
        shell::set_pending_share(app, None);
    }
    app.st.doc_public.attempted = if link.writer { link.id.clone() } else { String::new() };
    start_polling(app, host, true);
    shell::status(app, host, if link.writer { "Sign in to activate this editor link." } else { "Public document opened — no sign-in required." }, StatusOpts::from(Status::Info).preserve());
}

// v5: startPublicSharedPolling (no API key: every poll downloads the PDF)
fn start_polling(app: &mut App, host: &mut dyn Host, preview_only: bool) {
    stop_polling(app, host);
    let st = &mut app.st.doc_public;
    (st.polling, st.preview_only) = (true, preview_only);
    if preview_only {
        st.print.clear();
    }
    app.sched.interval(host.out(), T_POLL, POLL_MS);
    poll(app, host);
}

fn poll(app: &mut App, host: &mut dyn Host) {
    let st = &mut app.st.doc_public;
    if !st.polling || st.in_flight || st.hidden || st.opening {
        return;
    }
    st.in_flight = true;
    download(app, host, true);
}

// v5: stopPublicSharedPolling
fn stop_polling(app: &mut App, host: &mut dyn Host) {
    let st = &mut app.st.doc_public;
    st.generation += 1;
    if std::mem::take(&mut st.polling) {
        app.sched.clear(host.out(), T_POLL);
    }
    // A merge still running is not shown any more (5.18.0 `assertPublicPollContext`).
    (st.in_flight, st.refresh_req) = (false, 0);
    if st.dl.as_ref().is_some_and(|d| d.poll) {
        if let Some(dl) = st.dl.take() {
            if dl.blob != 0 {
                host.out().obj_release(dl.blob);
            }
        }
    }
}

// ---- viewer UI and the sign-in gate ----------------------------------------------------------------------------

// v5: updateViewerModeUI for the anonymous reader (banner hidden, `Read only` toggle); a writer link shows the gate.
fn viewer_ui(app: &mut App, host: &mut dyn Host, writer: bool) {
    let anonymous = !writer && auth::token(app).is_none();
    let group = app.st.doc_public.mode_group;
    // v5: updateDocTitle (`getBaseDocTitle` of the file name) and the hidden toolbar of a viewer.
    let name = session::get(app).map(|s| s.record.name.clone()).filter(|n| !n.is_empty());
    let name = name.or_else(|| app.st.doc_public.link.as_ref().map(|l| l.name.clone())).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Shared document".into());
    let trimmed = name.trim();
    let cut = trimmed.len().saturating_sub(4);
    let base = if trimmed.get(cut..).is_some_and(|e| e.eq_ignore_ascii_case(".pdf")) { &trimmed[..cut] } else { trimmed };
    let out = host.out();
    out.text(Ui::DOC_TITLE, if base.is_empty() { "Untitled" } else { base });
    out.style(Ui::TOOLBAR, "display", Some("none"));
    out.class(Ui::READONLY_BANNER, "hidden", anonymous);
    out.style(Ui::MODE_TOGGLE_BTN, "display", Some(if anonymous { "" } else { "none" }));
    out.prop(Ui::DOC_TITLE, "contentEditable", Prop::Str("false"));
    out.style(Ui::DOC_TITLE, "pointer-events", Some("none"));
    out.class(group, "public-read-only-mode", anonymous);
    if anonymous {
        out.class(Ui::MODE_TOGGLE_BTN, "is-editing", false);
        out.attr(Ui::MODE_TOGGLE_BTN, "aria-pressed", Some("false"));
        out.attr(Ui::MODE_TOGGLE_BTN, "aria-disabled", Some("true"));
        out.attr(Ui::MODE_TOGGLE_BTN, "aria-label", Some("Read only"));
        out.prop(Ui::MODE_TOGGLE_BTN, "title", Prop::Str("Read only"));
        out.text(Ui::MODE_TOGGLE_LABEL, "Read only");
    }
}

/// Leaves the anonymous reader's toggle (the editor sets its own state on the next open).
fn viewer_ui_reset(app: &mut App, host: &mut dyn Host) {
    let group = app.st.doc_public.mode_group;
    let out = host.out();
    out.style(Ui::TOOLBAR, "display", Some(""));
    out.class(group, "public-read-only-mode", false);
    out.attr(Ui::MODE_TOGGLE_BTN, "aria-disabled", None);
    out.attr(Ui::MODE_TOGGLE_BTN, "aria-pressed", Some("false"));
    out.attr(Ui::MODE_TOGGLE_BTN, "aria-label", Some("Read mode"));
    out.prop(Ui::MODE_TOGGLE_BTN, "title", Prop::Str("Toggle read/edit mode"));
    out.text(Ui::MODE_TOGGLE_LABEL, "Read");
}

// v5: showSharedEditorSignInGate + updateSharedEditorSignInGateUI
fn show_gate(app: &mut App, host: &mut dyn Host) {
    app.st.doc_public.gate = true;
    gate_ui(host);
}

fn gate_ui(host: &mut dyn Host) {
    let out = host.out();
    out.attr(Ui::READONLY_BANNER, "data-shared-editor-gate", Some("true"));
    out.class(Ui::READONLY_BANNER, "hidden", false);
    out.html(Ui::READONLY_BANNER_MESSAGE, "<strong>Editor link</strong> Sign in to edit and save.");
    out.style(Ui::BTN_MAKE_COPY, "display", Some("none"));
    out.style(Ui::BTN_ASK_EDIT, "display", Some(""));
    out.text(Ui::BTN_ASK_EDIT, "Sign in");
}

// v5: clearSharedEditorSignInGate
fn clear_gate(app: &mut App, host: &mut dyn Host) {
    if !std::mem::take(&mut app.st.doc_public.gate) {
        return;
    }
    let out = host.out();
    out.attr(Ui::READONLY_BANNER, "data-shared-editor-gate", None);
    out.html(Ui::READONLY_BANNER_MESSAGE, VIEW_ONLY_HTML);
    out.style(Ui::BTN_MAKE_COPY, "display", Some(""));
    out.style(Ui::BTN_ASK_EDIT, "display", Some(""));
    out.text(Ui::BTN_ASK_EDIT, "Ask to Edit");
}

// v5: askToEdit
fn ask_to_edit(app: &mut App, host: &mut dyn Host) {
    let id = match file(app) {
        Some((id, _)) => id.to_string(),
        None => session::doc_id(app).to_string(),
    };
    if id.is_empty() {
        return;
    }
    if app.st.doc_public.gate && auth::token(app).is_none() {
        return auth::sign_in(app, host);
    }
    fx::navigate(host.out(), Nav::Open, &format!("https://drive.google.com/open?id={id}"));
}

// ---- the anonymous preview -------------------------------------------------------------------------------------

// v5: showPublicInlinePreview
fn show_preview(app: &mut App, host: &mut dyn Host, link: &links::Link) {
    if app.st.doc_public.preview.is_none() {
        let out = host.out();
        out.html(Ui::CANVAS_CONTAINER, "");
        let root = out.insert_html(Ui::CANVAS_VIEWPORT, Node::NONE, concat!(
            "<div id=\"public-inline-preview\" class=\"public-inline-preview\" data-gestures-ready=\"true\">",
            "<div class=\"public-preview-stage\" style=\"transform: translate3d(0px, 0px, 0px) scale(1);\"></div>",
            "<div class=\"public-preview-controls\" aria-label=\"Public document zoom\">",
            "<button type=\"button\" class=\"public-preview-control\" data-public-preview-action=\"out\" aria-label=\"Zoom out\">−</button>",
            "<button type=\"button\" class=\"public-preview-control\" data-public-preview-action=\"fit\" aria-label=\"Fit document\">Fit</button>",
            "<button type=\"button\" class=\"public-preview-control\" data-public-preview-action=\"in\" aria-label=\"Zoom in\">+</button>",
            "</div></div>"
        ));
        let (stage, controls) = (out.child(root, &[0]), out.child(root, &[1]));
        let buttons = [out.child(controls, &[0]), out.child(controls, &[1]), out.child(controls, &[2])];
        for (event, l, flags) in [("pointerdown", L_DOWN, 0), ("pointermove", L_MOVE, 0), ("pointerup", L_UP, 0), ("pointercancel", L_CANCEL, 0), ("wheel", L_WHEEL, listen::ACTIVE), ("dblclick", L_DBL, 0)] {
            out.listen(root, event, l, flags);
        }
        out.listen(controls, "click", L_CONTROLS, 0);
        app.st.doc_public.preview = Some(Preview { host: root, stage, controls, buttons, scale: 1.0, ..Preview::default() });
    }
    let now = host.time();
    let st = &mut app.st.doc_public;
    let Some(p) = st.preview.as_mut() else { return };
    let out = host.out();
    out.class(Ui::CANVAS_VIEWPORT, "public-preview-active", true);
    out.class(Node::BODY, "public-preview-mode", true);
    out.style(Ui::CANVAS_CONTAINER, "visibility", Some("hidden"));
    out.prop(Ui::BTN_EDIT_PAGES, "disabled", Prop::Bool(true));
    let src = crate::home::cards::esc(&preview_url(&link.id, &link.resource_key, now));
    let frame = out.insert_html(p.stage, Node::NONE, &format!("<iframe title=\"Public shared PDF\" referrerpolicy=\"strict-origin-when-cross-origin\" scrolling=\"yes\" tabindex=\"0\" src=\"{src}\"></iframe>"));
    out.listen(frame, "load", L_FRAME + (p.frame_serial & 0xff), listen::ONCE);
    p.frame_serial += 1;
    p.frames.push(frame);
    st.iframe_at = now;
}

// v5: clearPublicInlinePreview
fn clear_preview(app: &mut App, host: &mut dyn Host) {
    let Some(p) = app.st.doc_public.preview.take() else { return };
    let out = host.out();
    out.remove(p.host);
    for n in p.frames.into_iter().chain(p.buttons).chain([p.stage, p.controls, p.host]) {
        out.release(n);
    }
    out.class(Ui::CANVAS_VIEWPORT, "public-preview-active", false);
    out.class(Node::BODY, "public-preview-mode", false);
    out.style(Ui::CANVAS_CONTAINER, "visibility", Some(""));
    out.prop(Ui::BTN_EDIT_PAGES, "disabled", Prop::Bool(false));
}

// v5: clampPublicPreviewPan + applyPublicPreviewTransform
fn apply_transform(app: &mut App, host: &mut dyn Host) {
    let Some(h) = app.st.doc_public.preview.as_ref().map(|p| p.host) else { return };
    let r = host.rect(h);
    let Some(p) = app.st.doc_public.preview.as_mut() else { return };
    let over = (p.scale - 1.0).max(0.0);
    let (mx, my) = (r.w * (0.16 + over * 0.5), r.h * (0.2 + over * 0.5));
    p.px = p.px.min(mx).max(-mx);
    p.py = p.py.min(my).max(-my);
    let out = host.out();
    out.style(p.stage, "transform", Some(&format!("translate3d({}px, {}px, 0) scale({})", num(p.px), num(p.py), num(p.scale))));
    out.text(p.buttons[1], &if p.scale == 1.0 { "Fit".to_string() } else { format!("{}%", num(jsmath::round(p.scale * 100.0))) });
}

// v5: setPublicPreviewZoom
fn zoom(app: &mut App, host: &mut dyn Host, next: f64, at: Option<(f64, f64)>) {
    let Some(h) = app.st.doc_public.preview.as_ref().map(|p| p.host) else { return };
    let r = host.rect(h);
    let Some(p) = app.st.doc_public.preview.as_mut() else { return };
    let next = if next.is_finite() && next != 0.0 { next } else { 1.0 }.min(4.0).max(0.7);
    let (ax, ay) = at.map_or((0.0, 0.0), |(x, y)| (x - r.x - r.w / 2.0, y - r.y - r.h / 2.0));
    let (cx, cy) = ((ax - p.px) / p.scale, (ay - p.py) / p.scale);
    p.scale = next;
    p.px = ax - cx * next;
    p.py = ay - cy * next;
    apply_transform(app, host);
}

fn metrics(p: &Preview) -> (Option<(f64, f64)>, f64) {
    if p.pointers.is_empty() {
        return (None, 0.0);
    }
    let n = p.pointers.len() as f64;
    let c = p.pointers.iter().fold((0.0, 0.0), |c, q| (c.0 + q.1 / n, c.1 + q.2 / n));
    let d = if p.pointers.len() >= 2 { jsmath::hypot(p.pointers[0].1 - p.pointers[1].1, p.pointers[0].2 - p.pointers[1].2) } else { 0.0 };
    (Some(c), d)
}

// v5: installPublicPreviewGestures
fn pointer(app: &mut App, host: &mut dyn Host, b: &crate::PointerBatch) -> u32 {
    let Some(p) = app.st.doc_public.preview.as_mut() else { return 0 };
    let (x, y, idp) = (b.raw.x, b.raw.y, b.pointer_id);
    match b.kind {
        ptr::DOWN => {
            if b.path.contains(p.controls) {
                return ret::STOP_PROPAGATION;
            }
            host.out().pointer_capture(p.host, idp, true);
            p.pointers.retain(|q| q.0 != idp);
            p.pointers.push((idp, x, y));
            (p.center, p.distance) = metrics(p);
            host.out().class(p.host, "dragging", true);
            ret::STOP_PROPAGATION | ret::PREVENT_DEFAULT
        }
        ptr::MOVE => {
            let Some(q) = p.pointers.iter_mut().find(|q| q.0 == idp) else { return ret::STOP_PROPAGATION };
            (q.1, q.2) = (x, y);
            let (prev_c, prev_d) = (p.center, p.distance);
            let (c, d) = metrics(p);
            if let (Some(a), Some(b)) = (prev_c, c) {
                p.px += b.0 - a.0;
                p.py += b.1 - a.1;
            }
            if p.pointers.len() >= 2 && prev_d > 0.0 && d > 0.0 {
                let (scale, at) = (p.scale * (d / prev_d), c);
                zoom(app, host, scale, at);
            } else {
                apply_transform(app, host);
            }
            if let Some(p) = app.st.doc_public.preview.as_mut() {
                (p.center, p.distance) = (c, d);
            }
            ret::STOP_PROPAGATION | ret::PREVENT_DEFAULT
        }
        ptr::UP | ptr::CANCEL => {
            p.pointers.retain(|q| q.0 != idp);
            host.out().pointer_capture(p.host, idp, false);
            (p.center, p.distance) = metrics(p);
            if p.pointers.is_empty() {
                host.out().class(p.host, "dragging", false);
            }
            ret::STOP_PROPAGATION
        }
        _ => 0,
    }
}

fn reset_transform(app: &mut App, host: &mut dyn Host) {
    let Some(p) = app.st.doc_public.preview.as_mut() else { return };
    (p.scale, p.px, p.py, p.center, p.distance) = (1.0, 0.0, 0.0, None, 0.0);
    p.pointers.clear();
    host.out().class(p.host, "dragging", false);
    apply_transform(app, host);
}

// ---- module contract -------------------------------------------------------------------------------------------

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(app: &mut App, host: &mut dyn Host) {
    app.st.doc_public.hidden = app.cfg.has(mode::HIDDEN);
    if app.cfg.has(mode::HEADLESS) {
        return;
    }
    let out = host.out();
    app.st.doc_public.mode_group = out.query(Ui::EDITOR_VIEW, ".mode-toggle-group");
    out.listen(Ui::BTN_ASK_EDIT, "click", L_ASK, 0);
}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        // v5: updateDriveAuthUI — signed out with a pending link: the gate stays, else the anonymous open.
        Event::Msg(m) if m.chan == sig::AUTH => {
            if connected(app) || shell::pending_share(app).is_none() {
                return 0;
            }
            if app.st.doc_public.gate {
                shell::show_view(app, host, View::Editor);
                gate_ui(host);
            } else if !app.st.doc_public.opening {
                shell::show_view(app, host, View::Home);
                try_open(app, host, false);
            }
        }
        Event::Msg(m) if m.chan == sig::VIEW && m.data == b"home" => {
            // v5: showDriveHome → clearPublicInlinePreview, clearSharedEditorSignInGate, closeDocumentScopedWork.
            if !app.st.doc_public.opening {
                stop_polling(app, host);
                clear_preview(app, host);
                if app.st.doc_public.link.take().is_some() {
                    viewer_ui_reset(app, host);
                }
            }
            clear_gate(app, host);
        }
        Event::Msg(m) if m.chan == session::sig::OPENED => {
            // Another open replaced the public document (a signed-in open of the link, a Home card).
            let st = &app.st.doc_public;
            if st.open_req == 0 && st.refresh_req == 0 && st.link.as_ref().is_some_and(|l| l.id.as_bytes() != m.data || auth::token(app).is_some()) {
                stop_polling(app, host);
                clear_preview(app, host);
                clear_gate(app, host);
                app.st.doc_public.link = None;
                viewer_ui_reset(app, host);
            }
        }
        Event::Msg(m) if m.chan == sig::SIGNED_OUT => {
            app.st.doc_public.attempted.clear();
            clear_gate(app, host);
        }
        Event::Life(l) if l.code == life::VISIBILITY => app.st.doc_public.hidden = l.a == 1.0,
        Event::Timer(T_POLL) => poll(app, host),
        Event::Timer(T_FRAMES) => {
            // v5: the previous frames leave 140 ms after the new one loaded.
            if let Some(p) = app.st.doc_public.preview.as_mut() {
                let keep = p.frames.pop();
                for f in p.frames.drain(..) {
                    host.out().remove(f);
                    host.out().release(f);
                }
                p.frames.extend(keep);
            }
        }
        Event::Io(io) if io.req == app.st.doc_public.open_req => {
            app.st.doc_public.open_req = 0;
            opened(app, host, io.status >= 0);
        }
        Event::Io(io) if io.req == app.st.doc_public.refresh_req => {
            app.st.doc_public.refresh_req = 0;
            refreshed(app, host, io.status >= 0);
        }
        Event::Io(io) if app.st.doc_public.dl.as_ref().is_some_and(|d| d.req == io.req) => download_step(app, host, &io),
        Event::Io(io) if io.obj != 0 => host.out().obj_release(io.obj),
        Event::Pointer(ref b) if (L_DOWN..=L_CANCEL).contains(&b.listener) => return pointer(app, host, b),
        Event::Dom(ref d) => match d.listener {
            L_ASK => ask_to_edit(app, host),
            L_WHEEL => {
                // v5: ctrl/meta + wheel zooms, a plain wheel pans.
                if d.flags & (ev::CTRL | ev::META) != 0 {
                    let scale = app.st.doc_public.preview.as_ref().map_or(1.0, |p| p.scale) * jsmath::exp(-d.dy * 0.002);
                    zoom(app, host, scale, Some((d.x, d.y)));
                } else if let Some(p) = app.st.doc_public.preview.as_mut() {
                    p.px -= d.dx;
                    p.py -= d.dy;
                    apply_transform(app, host);
                }
                return ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION;
            }
            L_DBL => {
                let Some(p) = app.st.doc_public.preview.as_ref() else { return 0 };
                if !d.path.contains(p.controls) {
                    let next = if p.scale >= 2.0 { 1.0 } else { p.scale * 1.5 };
                    zoom(app, host, next, Some((d.x, d.y)));
                }
                return ret::STOP_PROPAGATION;
            }
            L_CONTROLS => {
                let Some(p) = app.st.doc_public.preview.as_ref() else { return 0 };
                let scale = p.scale;
                match p.buttons.iter().position(|b| d.path.contains(*b)) {
                    Some(0) => zoom(app, host, scale / 1.25, None),
                    Some(1) => reset_transform(app, host),
                    Some(2) => zoom(app, host, scale * 1.25, None),
                    _ => {}
                }
            }
            l if l & !0xff == L_FRAME => {
                if let Some(p) = app.st.doc_public.preview.as_ref() {
                    if d.node != Node::NONE {
                        host.out().class(d.node, "ready", true);
                    }
                    if p.frames.len() > 1 {
                        app.sched.timer(host.out(), T_FRAMES, 140.0);
                    }
                }
            }
            _ => {}
        },
        _ => {}
    }
    0
}

/// `__IHN_TEST_API__.cmd('doc.public.<name>')`: `state` → `{file, opening, polling, previewOnly, fingerprint,
/// gate, preview, frames, downloads, updates, scale}`; `endpoints` `{id, resourceKey}` → the two URLs.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, args: &str) -> Option<String> {
    let st = &app.st.doc_public;
    match name {
        "state" => Some(format!(
            "{{\"file\":{},\"opening\":{},\"polling\":{},\"previewOnly\":{},\"fingerprint\":{},\"gate\":{},\"preview\":{},\"frames\":{},\"downloads\":{},\"updates\":{},\"scale\":{}}}",
            json_str(st.link.as_ref().map_or("", |l| &l.id)),
            st.opening,
            st.polling,
            st.preview_only,
            json_str(&st.print),
            st.gate,
            st.preview.is_some(),
            st.preview.as_ref().map_or(0, |p| p.frames.len()),
            st.downloads,
            st.updates,
            num(st.preview.as_ref().map_or(1.0, |p| p.scale)),
        )),
        "endpoints" => {
            let j = crate::shell::json::Json::parse(args)?;
            let [a, b] = endpoints(j.text("id"), j.text("resourceKey"));
            Some(format!("[{},{}]", json_str(&a), json_str(&b)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_urls_and_names_match_5_18_0() {
        assert_eq!(endpoints("abc", ""), ["https://drive.usercontent.google.com/download?id=abc&export=download".to_string(), "https://drive.google.com/uc?export=download&id=abc".to_string()]);
        assert!(endpoints("abc", "0-k y")[1].ends_with("&resourcekey=0-k%20y"));
        assert!(endpoints("a", "k").iter().all(|u| !u.contains("key=") || u.contains("resourcekey=")));
        assert_eq!(preview_url("a b", "k y", 1700000000000.0), "https://drive.google.com/file/d/a%20b/preview?embedded=true&resourcekey=k+y&ihn_refresh=1700000000000");
        assert_eq!(disposition_name("attachment; filename=\"Notes.pdf\""), "Notes.pdf");
        assert_eq!(disposition_name("attachment; filename*=UTF-8''Caf%C3%A9.pdf; filename=\"x.pdf\""), "Café.pdf");
        assert_eq!(disposition_name("inline"), "");
        assert!(is_pdf(b"\n\n%PDF-1.7") && !is_pdf(b"<html>") && !is_pdf(b"%PDF"));
    }

    #[test]
    fn fingerprints_match_5_18_0() {
        // JS: let h = 2166136261; for (b of [0x61,0x62,0x63]) { h ^= b; h = Math.imul(h, 16777619) } (h>>>0).toString(36)
        let h = fnv(2166136261, b"abc");
        assert_eq!(h, 0x1a47e90b);
        assert_eq!(fingerprint(3.0, h), "3:7aigaz");
        assert_eq!(fingerprint(0.0, 0), "0:0");
    }
}
