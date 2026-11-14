//! Opening a Drive document (ARCHITECTURE §9.1): the local copy first, then Drive; v6 containers are merged here,
//! 5.x and plain PDFs are handed to the importer (`sig::FOREIGN`). Owned by W3.1.
//!
//! Order of an open: `session::load` and the metadata request start together. A local copy is shown at once and
//! revalidated against Drive (an export this device has not seen is downloaded and merged); without one the PDF is
//! downloaded, opened with pdf.js (`host/pdf.js`), classified by its Keywords and loaded.
// v5: app-v5.js:openDriveFile / fetchStableDrivePdfSnapshot / updateViewerModeUI / showDriveOpenError

use super::session::{self, sig};
use crate::abi::{fam, ret, Event, Io, OutBuf};
use crate::app::Mod;
use crate::dom::Prop;
use crate::drive::client::{self, Req};
use crate::shell::fx::{self, deflate_flag, Response, Src};
use crate::shell::json::Json;
use crate::shell::{Status, StatusOpts, View};
use crate::store::sv::Value;
use crate::testapi::json_str;
use crate::ui_ids::Ui;
use crate::{auth, shell, App, Host};
use ihn_core::codec::{b64_decode, Reader};
use ihn_core::doc::PeerRef;
use ihn_core::id::Hash;
use ihn_core::snapshot::EncodedSnapshot;

/// Effect family 3.4 (executed by `host/pdf.js`).
pub mod pdf {
    use super::*;
    pub const LOAD: u8 = 0x50;
    pub const OPEN: u8 = 0x51;
    pub const PAGES: u8 = 0x52;
    pub const ATTACHMENT: u8 = 0x53;
    pub const RENDER: u8 = 0x54;
    pub const CLOSE: u8 = 0x55;
    /// `Io::status` of a blob that breaks the 64 MiB / type limit, and of a page count outside 1..=2000.
    pub const TOO_LARGE: i32 = -2;
    pub const TOO_MANY_PAGES: i32 = -3;

    fn rec(out: &mut OutBuf, op: u8, a: u32, b: u32) -> usize {
        let m = out.w.rec(fam::FX, op);
        out.w.u32(a);
        out.w.u32(b);
        m
    }
    /// Loads pdf.js ahead of the first open.
    pub fn load(out: &mut OutBuf, req: u32) { out.fx_id(LOAD, req); }
    /// Opens `blob` with the security-core parameters → `Io::obj` = the document, data read with [`Opened::parse`].
    pub fn open(out: &mut OutBuf, req: u32, blob: u32) {
        let m = rec(out, OPEN, req, blob);
        out.w.end(m);
    }
    /// Sizes of `count` pages from `first` (1-based) at scale 1 → `count × (f64 w, f64 h)`.
    pub fn pages(out: &mut OutBuf, req: u32, doc: u32, first: u32, count: u32) {
        let m = rec(out, PAGES, req, doc);
        out.w.u32(first);
        out.w.u32(count);
        out.w.end(m);
    }
    /// The embedded file `name` → `Io::obj` = a Blob, data = `f64 size`.
    pub fn attachment(out: &mut OutBuf, req: u32, doc: u32, name: &str) {
        let m = rec(out, ATTACHMENT, req, doc);
        out.w.str(name);
        out.w.end(m);
    }
    /// Page `page` (1-based) at `scale` on white as JPEG 0.92 → `Io::obj` = the Blob, data = `str url, f64 w, f64 h`.
    pub fn render(out: &mut OutBuf, req: u32, doc: u32, page: u32, scale: f64) {
        let m = rec(out, RENDER, req, doc);
        out.w.u32(page);
        out.w.f64(scale);
        out.w.end(m);
    }
    /// Destroys the document and frees its handle.
    pub fn close(out: &mut OutBuf, doc: u32) { out.fx_id(CLOSE, doc); }

    /// What `open` reports about a PDF.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct Opened {
        pub pages: u32,
        pub keywords: String,
        /// `(file name, size)` of the embedded files.
        pub attachments: Vec<(String, f64)>,
    }
    impl Opened {
        pub fn parse(data: &[u8]) -> Option<Opened> {
            let mut r = Reader::new(data);
            let (pages, keywords) = (r.u32()?, r.str()?.to_string());
            let attachments = (0..r.u32()?).map(|_| Some((r.str()?.to_string(), r.f64()?))).collect::<Option<_>>()?;
            Some(Opened { pages, keywords, attachments })
        }
    }
}

/// The marker of a v6 container inside `/Keywords` (ARCHITECTURE §8.2).
pub const V6_MARKER: &str = ";IHN6:";
/// 5.18.0 `DRIVE_META_TIMEOUT`.
const META_TIMEOUT_MS: f64 = 7000.0;
const META_FIELDS: &str = "capabilities(canEdit),ownedByMe,owners(displayName,emailAddress,photoLink),resourceKey,headRevisionId,modifiedTime,version,md5Checksum,size,parents";
/// Downloads tried while Drive keeps changing under the open (5.18.0: three).
const ATTEMPTS: u32 = 3;
const OPEN_ERROR: &str = "Could not open this document. Try again.";

/// What kind of PDF a Keywords string announces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A v6 container: the base64 of the deflated snapshot follows the marker.
    V6,
    /// A 5.x document (`STROKES_…`).
    V5,
    Plain,
}

/// Classifies a PDF by its Keywords and returns the v6 payload (base64 text) when there is one.
pub fn classify(keywords: &str) -> (Kind, &str) {
    if let Some(at) = keywords.find(V6_MARKER) {
        let rest = &keywords[at + V6_MARKER.len()..];
        return (Kind::V6, rest.split(';').next().unwrap_or(""));
    }
    (if keywords.starts_with("STROKES_") { Kind::V5 } else { Kind::Plain }, "")
}

/// A Drive file to open.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct File {
    pub id: String,
    pub name: String,
    pub resource_key: String,
}

/// The facts of a `files.get` answer that an open uses.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub can_edit: bool,
    pub owned_by_me: bool,
    pub owner_name: String,
    pub owner_email: String,
    pub owner_photo: String,
    pub resource_key: String,
    pub head: String,
    pub modified: String,
    pub parents: Vec<String>,
    pub size: f64,
}

impl Meta {
    pub fn parse(body: &str, requested_key: &str) -> Option<Meta> {
        let j = Json::parse(body)?;
        let owner = j.get("owners").and_then(|o| o.items().first());
        let text = |o: Option<&Json>, k: &str| o.map_or(String::new(), |o| o.text(k).to_string());
        Some(Meta {
            // v5: only an explicit `false` makes a viewer.
            can_edit: !matches!(j.get("capabilities").and_then(|c| c.get("canEdit")), Some(Json::Bool(false))),
            owned_by_me: !matches!(j.get("ownedByMe"), Some(Json::Bool(false))),
            owner_name: text(owner, "displayName"),
            owner_email: text(owner, "emailAddress"),
            owner_photo: text(owner, "photoLink"),
            resource_key: Some(j.text("resourceKey")).filter(|k| !k.is_empty()).unwrap_or(requested_key).to_string(),
            head: j.text("headRevisionId").into(),
            modified: j.text("modifiedTime").into(),
            parents: j.get("parents").map(|p| p.items().iter().filter_map(|i| i.as_str()).filter(|s| !s.is_empty()).map(String::from).collect()).unwrap_or_default(),
            size: j.get("size").and_then(|s| s.as_str().and_then(|t| t.parse().ok()).or(s.as_f64())).unwrap_or(0.0),
        })
    }
    /// `docs.driveMeta`.
    pub fn value(&self) -> Value {
        Value::obj([
            ("resourceKey", Value::from(&self.resource_key[..])),
            ("parents", Value::from(self.parents.iter().map(|p| Value::from(&p[..])).collect::<Vec<_>>())),
            ("canEdit", Value::from(self.can_edit)),
            ("ownedByMe", Value::from(self.owned_by_me)),
            ("ownerName", Value::from(&self.owner_name[..])),
            ("ownerEmail", Value::from(&self.owner_email[..])),
            ("ownerPhoto", Value::from(&self.owner_photo[..])),
            ("headRevisionId", Value::from(&self.head[..])),
            ("modifiedTime", Value::from(&self.modified[..])),
        ])
    }
    // v5: driveRevisionMetaMatches
    fn same_revision(&self, other: &Meta) -> bool {
        if !self.head.is_empty() && !other.head.is_empty() {
            return self.head == other.head;
        }
        !self.modified.is_empty() && self.modified == other.modified
    }
}

/// A PDF that is not a v6 container, waiting for its importer ([`take_foreign`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Foreign {
    pub file: File,
    pub kind: Kind,
    /// The downloaded revision (a Blob handle; the taker owns it).
    pub blob: u32,
    pub size: f64,
    /// The pdf.js document (the taker closes it with `pdf::close`).
    pub pdf: u32,
    pub opened: pdf::Opened,
    pub meta: Meta,
}

#[derive(Default)]
struct Job {
    file: File,
    notify: u32,
    load_req: u32,
    meta_req: u32,
    media_req: u32,
    check_req: u32,
    pdf_req: u32,
    inflate_req: u32,
    attach_req: u32,
    /// The session answered: `Some(true)` = a local copy is on screen.
    local: Option<bool>,
    meta: Option<Meta>,
    meta_failed: bool,
    attempt: u32,
    blob: u32,
    size: f64,
    pdf: u32,
    opened: pdf::Opened,
    /// Embedded assets still to store: `(hash, attachment name)`.
    attachments: Vec<(Hash, String)>,
    attaching: Option<Hash>,
    foreign: Option<Foreign>,
    /// The document is on screen (the loading overlay is gone); what follows is silent.
    shown: bool,
    /// A signed-out public download handed over by `doc::public` ([`open_public`]).
    public: bool,
    /// [`open_public`] refreshing the open document: no overlay, a failure leaves it as it is.
    quiet: bool,
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    job: Option<Job>,
    /// Opens finished: `(doc id, ok)` (test command `log`).
    log: Vec<(String, bool)>,
    probe: Option<Probe>,
}

/// The executor self-test (`doc.open.pdf_probe`): every op of `host/pdf.js` on a Drive file.
#[derive(Default)]
struct Probe {
    req: u32,
    stage: u8,
    blob: u32,
    pdf: u32,
    pages: u32,
    attachment: String,
    out: String,
    done: bool,
}

fn probe_step(app: &mut App, host: &mut dyn Host, io: &Io) {
    let req = app.req(Mod::DocOpen);
    let Some(p) = app.st.doc_open.probe.as_mut() else { return };
    p.req = req;
    p.stage += 1;
    let out = host.out();
    if io.status < 0 {
        p.out = format!("{{\"stage\":{},\"error\":{},\"message\":{}}}", p.stage, io.status, json_str(std::str::from_utf8(io.data).unwrap_or("")));
        p.done = true;
        return;
    }
    let mut r = Reader::new(io.data);
    match p.stage {
        1 => {
            p.blob = io.obj;
            pdf::open(out, req, io.obj);
        }
        2 => {
            let opened = pdf::Opened::parse(io.data).unwrap_or_default();
            (p.pdf, p.pages) = (io.obj, opened.pages);
            p.attachment = opened.attachments.first().map_or(String::new(), |a| a.0.clone());
            p.out = format!("{{\"pages\":{},\"kind\":{},\"attachments\":{}", opened.pages, classify(&opened.keywords).0 as u8, opened.attachments.len());
            pdf::pages(out, req, io.obj, 1, opened.pages);
        }
        3 => {
            let sizes: Vec<String> = (0..p.pages * 2).map(|_| r.f64().unwrap_or(-1.0).to_string()).collect();
            p.out.push_str(&format!(",\"sizes\":[{}]", sizes.join(",")));
            pdf::render(out, req, p.pdf, 1, 0.5);
        }
        4 => {
            let (url, w, h) = (r.str().unwrap_or(""), r.f64().unwrap_or(0.0), r.f64().unwrap_or(0.0));
            p.out.push_str(&format!(",\"raster\":[{},{w},{h}]", url.starts_with("blob:")));
            fx::url_revoke(out, url);
            out.obj_release(io.obj);
            pdf::attachment(out, req, p.pdf, &p.attachment);
        }
        _ => {
            p.out.push_str(&format!(",\"attachmentSize\":{}}}", r.f64().unwrap_or(-1.0)));
            out.obj_release(io.obj);
            pdf::close(out, p.pdf);
            out.obj_release(p.blob);
            p.done = true;
        }
    }
}

/// The name of the file being opened (the editor shows it as the title at once, 5.18.0 `openDriveFile`).
pub fn opening_name(app: &App) -> Option<&str> { app.st.doc_open.job.as_ref().map(|j| &j.file.name[..]) }

/// An open is running (5.18.0 `driveOpenInProgress`).
pub fn busy(app: &App) -> bool { app.st.doc_open.job.as_ref().is_some_and(|j| !j.shown) }

/// Opens `file` in the editor. `notify` (0 = nobody) completes with status 0 when the document is on screen and
/// with -1 when the open failed. False when another open is running.
pub fn open_file(app: &mut App, host: &mut dyn Host, file: File, notify: u32) -> bool {
    if file.id.is_empty() || busy(app) {
        return false;
    }
    abort(app, host);
    let account = auth::known_account(host);
    let mut job = Job { file, notify, load_req: app.req(Mod::DocOpen), ..Job::default() };
    job.meta_req = request_meta(app, host, &job.file, META_FIELDS);
    let (id, load_req) = (job.file.id.clone(), job.load_req);
    app.st.doc_open.job = Some(job);
    // v5: openDriveFile — the editor with the loading overlay appears at once (the editor view reads the file's
    // name from the job for the title).
    shell::show_view(app, host, View::Editor);
    shell::loading(app, host, "Opening PDF...");
    session::load(app, host, &account, &id, load_req);
    true
}

/// Opens a PDF that `doc::public` downloaded signed out (5.18.0 `openSharedFileWithoutSignIn`): read-only, memory
/// only, no Drive calls. `blob` passes to the open. With `refresh` the open document is merged with a newer download
/// silently (5.18.0's public poll re-import). `notify` completes with 0 (shown) or -1 (failed).
pub fn open_public(app: &mut App, host: &mut dyn Host, file: File, blob: u32, size: f64, refresh: bool, notify: u32) -> bool {
    if file.id.is_empty() || busy(app) {
        return false;
    }
    abort(app, host);
    let meta = Meta { can_edit: false, owned_by_me: false, resource_key: file.resource_key.clone(), ..Meta::default() };
    let mut job = Job { file, notify, blob, size, meta: Some(meta), public: true, quiet: refresh, ..Job::default() };
    let id = job.file.id.clone();
    if refresh && session::doc_id(app) == id {
        job.local = Some(false);
        app.st.doc_open.job = Some(job);
        start_pdf(app, host);
    } else {
        job.load_req = app.req(Mod::DocOpen);
        let load_req = job.load_req;
        app.st.doc_open.job = Some(job);
        session::load(app, host, "", &id, load_req);
    }
    true
}

fn request_meta(app: &mut App, host: &mut dyn Host, file: &File, fields: &str) -> u32 {
    let mut url = format!("https://www.googleapis.com/drive/v3/files/{}?fields={}", file.id, fx::url_encode(fields));
    if !file.resource_key.is_empty() {
        url.push_str(&format!("&resourceKey={}", fx::url_encode(&file.resource_key)));
    }
    client::request(app, host, Mod::DocOpen, Req { headers: "Cache-Control: no-cache\n".into(), timeout_ms: META_TIMEOUT_MS, ..Req::get(&url) })
}

fn abort(app: &mut App, host: &mut dyn Host) {
    let Some(job) = app.st.doc_open.job.take() else { return };
    for req in [job.meta_req, job.media_req, job.check_req] {
        if req != 0 {
            client::abort(app, host, req);
        }
    }
    release(host, &job);
}

fn release(host: &mut dyn Host, job: &Job) {
    if job.pdf != 0 {
        pdf::close(host.out(), job.pdf);
    }
    if job.blob != 0 {
        host.out().obj_release(job.blob);
    }
}

// v5: app-v5.js:updateViewerModeUI — the banner, the mode toggle and the title. The toolbar and its buttons follow
// `sig::OPENED` in their own module.
fn viewer_ui(app: &mut App, host: &mut dyn Host) {
    let viewer = session::get(app).is_some_and(|s| !s.can_edit);
    let out = host.out();
    out.class(Ui::READONLY_BANNER, "hidden", !viewer);
    out.style(Ui::MODE_TOGGLE_BTN, "display", Some(if viewer { "none" } else { "" }));
    out.prop(Ui::DOC_TITLE, "contentEditable", Prop::Str(if viewer { "false" } else { "true" }));
    out.style(Ui::DOC_TITLE, "pointer-events", Some(if viewer { "none" } else { "auto" }));
}

/// The document is ready: Drive facts, viewer state, status, overlay.
fn show(app: &mut App, host: &mut dyn Host) {
    let Some(job) = app.st.doc_open.job.as_mut() else { return };
    let (first, notify, id) = (!std::mem::replace(&mut job.shown, true), job.notify, job.file.id.clone());
    viewer_ui(app, host);
    if first {
        let can_edit = session::get(app).is_some_and(|s| s.can_edit);
        shell::status(app, host, if can_edit { "PDF loaded" } else { "PDF loaded (View Only)" }, StatusOpts::from(Status::Info).preserve());
        shell::hide_loading(app, host);
        shell::signal(app, host, sig::OPENED, &id);
        app.st.doc_open.log.push((id, true));
        if notify != 0 {
            shell::deliver(app, host, notify, 0, 0, &[]);
        }
    }
}

fn done(app: &mut App, host: &mut dyn Host) {
    if let Some(job) = app.st.doc_open.job.take() {
        release(host, &job);
    }
}

// v5: the catch of openDriveFile + showDriveOpenError. A failure after the local copy was shown changes nothing.
fn fail(app: &mut App, host: &mut dyn Host) {
    let Some(job) = app.st.doc_open.job.take() else { return };
    release(host, &job);
    if job.shown || job.quiet {
        if job.quiet && job.notify != 0 {
            shell::deliver(app, host, job.notify, -1, 0, &[]);
        }
        return;
    }
    session::close(app, host);
    shell::show_view(app, host, View::Home);
    shell::status(app, host, OPEN_ERROR, StatusOpts { error: true, preserve_state: true, ..StatusOpts::default() });
    shell::home_status(host, OPEN_ERROR);
    host.out().style(Ui::DRIVE_STATUS, "display", Some("block"));
    shell::hide_loading(app, host);
    app.st.doc_open.log.push((job.file.id, false));
    if job.notify != 0 {
        shell::deliver(app, host, job.notify, -1, 0, &[]);
    }
}

fn job(app: &mut App) -> Option<&mut Job> { app.st.doc_open.job.as_mut() }

/// Both the session and the metadata answered (or one of them makes the other unnecessary).
fn advance(app: &mut App, host: &mut dyn Host) {
    let Some(j) = job(app) else { return };
    let Some(local) = j.local else { return };
    if local && !j.shown {
        // v5: the cached open ("Restoring document...") — no network needed.
        shell::loading(app, host, "Restoring document...");
        show(app, host);
    }
    let Some(j) = job(app) else { return };
    if j.meta_failed {
        return if local { done(app, host) } else { fail(app, host) };
    }
    let Some(meta) = j.meta.clone() else { return };
    if j.media_req != 0 || j.blob != 0 {
        // A public download (`open_public`) goes to pdf.js once the session is loaded.
        if j.public && j.pdf_req == 0 && j.pdf == 0 && j.inflate_req == 0 {
            start_pdf(app, host);
        }
        return;
    }
    let known = session::get(app).is_some_and(|s| !meta.head.is_empty() && (s.record.exports.iter().any(|e| e.0 == meta.head) || s.record.last_export.as_ref().is_some_and(|l| l.head == meta.head)));
    if local && known {
        // Nothing new in Drive: refresh the Drive facts (a permission may have changed) and stop.
        apply_meta(app, host, &meta, None);
        viewer_ui(app, host);
        return done(app, host);
    }
    download(app, host);
}

fn download(app: &mut App, host: &mut dyn Host) {
    let Some(j) = job(app) else { return };
    j.attempt += 1;
    let mut url = format!("https://www.googleapis.com/drive/v3/files/{}?alt=media", j.file.id);
    let key = j.meta.as_ref().map_or("", |m| &m.resource_key[..]);
    if !key.is_empty() {
        url.push_str(&format!("&resourceKey={}", fx::url_encode(key)));
    }
    let req = client::request(app, host, Mod::DocOpen, Req { blob: true, timeout_ms: 120_000.0, ..Req::get(&url) });
    if let Some(j) = job(app) {
        j.media_req = req;
    }
}

fn apply_meta(app: &mut App, host: &mut dyn Host, meta: &Meta, export: Option<(String, Vec<u8>)>) {
    let Some(j) = job(app) else { return };
    let (name, size) = (j.file.name.clone(), j.size);
    if let Some(s) = session::get_mut(app) {
        if size > 0.0 {
            s.export_size = size;
        }
    }
    session::set_meta(app, host, if name.is_empty() { "Untitled" } else { &name }, meta.can_edit, meta.value(), export);
}

fn media(app: &mut App, host: &mut dyn Host, io: &Io) {
    let Some(j) = job(app) else { return };
    j.media_req = 0;
    if io.status < 0 || io.obj == 0 {
        return fail(app, host);
    }
    j.blob = io.obj;
    j.size = Response::parse(io.data).blob_size();
    // v5: fetchStableDrivePdfSnapshot — the revision must be the same after the download.
    let file = j.file.clone();
    let req = request_meta(app, host, &file, "headRevisionId,modifiedTime,version,md5Checksum,size");
    if let Some(j) = job(app) {
        j.check_req = req;
    }
}

fn checked(app: &mut App, host: &mut dyn Host, io: &Io) {
    let Some(j) = job(app) else { return };
    j.check_req = 0;
    let after = if io.status >= 0 { Meta::parse(Response::parse(io.data).text(), "") } else { None };
    let (Some(after), Some(before)) = (after, j.meta.as_mut()) else { return fail(app, host) };
    if !before.same_revision(&after) {
        before.head = after.head;
        before.modified = after.modified;
        host.out().obj_release(std::mem::take(&mut j.blob));
        if j.attempt >= ATTEMPTS {
            return fail(app, host);
        }
        return download(app, host);
    }
    start_pdf(app, host);
}

fn start_pdf(app: &mut App, host: &mut dyn Host) {
    let Some(j) = job(app) else { return };
    if !j.shown && !j.quiet {
        shell::loading(app, host, "Processing PDF...");
    }
    let req = app.req(Mod::DocOpen);
    let Some(j) = job(app) else { return };
    j.pdf_req = req;
    pdf::open(host.out(), req, j.blob);
}

fn pdf_opened(app: &mut App, host: &mut dyn Host, io: &Io) {
    let Some(j) = job(app) else { return };
    j.pdf_req = 0;
    let Some(opened) = (io.status >= 0).then(|| pdf::Opened::parse(io.data)).flatten() else { return fail(app, host) };
    j.pdf = io.obj;
    let (kind, payload) = classify(&opened.keywords);
    let bytes = if kind == Kind::V6 { b64_decode(payload.as_bytes()) } else { None };
    j.opened = opened;
    match (kind, bytes) {
        (Kind::V6, Some(bytes)) => {
            let req = app.req(Mod::DocOpen);
            if let Some(j) = job(app) {
                j.inflate_req = req;
            }
            fx::deflate(host.out(), req, deflate_flag::INFLATE | deflate_flag::ZLIB, Src::Bytes(&bytes));
        }
        (Kind::V6, None) => fail(app, host),
        _ => {
            // Not a v6 container: the importer claims it or the open fails (5.x and plain PDFs are `doc::v5open`'s).
            let Some(meta) = j.meta.clone() else { return fail(app, host) };
            j.foreign = Some(Foreign { file: j.file.clone(), kind, blob: std::mem::take(&mut j.blob), size: j.size, pdf: std::mem::take(&mut j.pdf), opened: j.opened.clone(), meta });
            let claimed = shell::signal(app, host, sig::FOREIGN, if kind == Kind::V5 { "v5" } else { "plain" }) & ret::PREVENT_DEFAULT != 0;
            if !claimed {
                if let Some(f) = job(app).and_then(|j| j.foreign.take()) {
                    pdf::close(host.out(), f.pdf);
                    host.out().obj_release(f.blob);
                }
                fail(app, host);
            }
        }
    }
}

/// The PDF the importer was told about with `sig::FOREIGN`; it then owns the blob and the pdf.js document, loads
/// the session (`session::with_replica` / `session::edit`) and ends with [`finish_foreign`].
pub fn take_foreign(app: &mut App) -> Option<Foreign> { job(app).and_then(|j| j.foreign.take()) }

/// The importer finished. On success the session's state is recorded as holding the downloaded revision (so the
/// next open does not download it again) and the document is shown; otherwise the open fails.
pub fn finish_foreign(app: &mut App, host: &mut dyn Host, ok: bool) {
    if ok {
        loaded(app, host);
    } else {
        fail(app, host);
    }
}

fn inflated(app: &mut App, host: &mut dyn Host, io: &Io) {
    let Some(j) = job(app) else { return };
    j.inflate_req = 0;
    if io.status < 0 {
        return fail(app, host);
    }
    let snapshot = EncodedSnapshot(io.data.to_vec());
    if snapshot.decode().is_err() {
        return fail(app, host);
    }
    let names: Vec<String> = j.opened.attachments.iter().map(|a| a.0.clone()).collect();
    let missing = session::with_replica(app, host, |r| {
        r.load_snapshot(&snapshot, PeerRef::Export);
        r.missing_assets()
    })
    .unwrap_or_default();
    let Some(j) = job(app) else { return };
    // Image assets travel as attachments `IHN6-<hex hash>` (ARCHITECTURE §8.2).
    j.attachments = missing.into_iter().filter_map(|h| Some((h, names.iter().find(|n| n.strip_prefix("IHN6-") == Some(&crate::store::sv::hex(&h)[..]))?.clone()))).collect();
    next_attachment(app, host);
}

fn next_attachment(app: &mut App, host: &mut dyn Host) {
    let Some(j) = job(app) else { return };
    let Some((hash, name)) = j.attachments.pop() else { return loaded(app, host) };
    j.attaching = Some(hash);
    let pdf = j.pdf;
    let req = app.req(Mod::DocOpen);
    if let Some(j) = job(app) {
        j.attach_req = req;
    }
    pdf::attachment(host.out(), req, pdf, &name);
}

/// The session holds the downloaded revision.
fn loaded(app: &mut App, host: &mut dyn Host) {
    let Some(j) = job(app) else { return };
    let Some(meta) = j.meta.clone() else { return fail(app, host) };
    let vv = session::get(app).map(|s| {
        let mut w = ihn_core::codec::Writer::default();
        s.doc.vv.encode(&mut w);
        w.buf
    });
    apply_meta(app, host, &meta, vv.filter(|_| !meta.head.is_empty()).map(|vv| (meta.head.clone(), vv)));
    show(app, host);
    done(app, host);
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(_app: &mut App, _host: &mut dyn Host) {}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Io(io) if app.st.doc_open.probe.as_ref().is_some_and(|p| p.req == io.req) => probe_step(app, host, &io),
        Event::Io(io) => {
            let Some(j) = job(app) else {
                // A completion of an abandoned open: free what it carries.
                if io.obj != 0 {
                    host.out().obj_release(io.obj);
                }
                return 0;
            };
            if io.req == j.load_req {
                j.load_req = 0;
                j.local = Some(io.status == session::LOADED_LOCAL);
                advance(app, host);
            } else if io.req == j.meta_req {
                j.meta_req = 0;
                j.meta = if io.status >= 0 { Meta::parse(Response::parse(io.data).text(), &j.file.resource_key) } else { None };
                j.meta_failed = j.meta.is_none();
                advance(app, host);
            } else if io.req == j.media_req {
                media(app, host, &io);
            } else if io.req == j.check_req {
                checked(app, host, &io);
            } else if io.req == j.pdf_req {
                pdf_opened(app, host, &io);
            } else if io.req == j.inflate_req {
                inflated(app, host, &io);
            } else if io.req == j.attach_req {
                j.attach_req = 0;
                if let (true, Some(hash)) = (io.status >= 0 && io.obj != 0, j.attaching.take()) {
                    let size = Reader::new(io.data).f64().unwrap_or(-1.0);
                    session::put_asset(app, host, hash, io.obj, "", size);
                }
                next_attachment(app, host);
            } else if io.obj != 0 {
                host.out().obj_release(io.obj);
            }
        }
        Event::Msg(m) if m.chan == shell::sig::SIGNED_OUT => abort(app, host),
        _ => {}
    }
    0
}

/// `__IHN_TEST_API__.cmd('doc.open.<name>', args)`: `state` → `{busy, log: [[id, ok]…]}`; `open` `{id, name}`
/// starts an open as Home's card click will; `pdf_probe` `{id}` runs every op of `host/pdf.js` on that Drive file
/// and `pdf_probe_result` returns what came back (`null` until it finished).
pub fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, args: &str) -> Option<String> {
    match name {
        "state" => {
            let log: Vec<String> = app.st.doc_open.log.iter().map(|(id, ok)| format!("[{},{ok}]", json_str(id))).collect();
            Some(format!("{{\"busy\":{},\"running\":{},\"log\":[{}]}}", busy(app), app.st.doc_open.job.is_some(), log.join(",")))
        }
        "pdf_probe" => {
            let url = format!("https://www.googleapis.com/drive/v3/files/{}?alt=media", Json::parse(args)?.text("id"));
            let req = client::request(app, host, Mod::DocOpen, Req { blob: true, ..Req::get(&url) });
            app.st.doc_open.probe = Some(Probe { req, ..Probe::default() });
            Some("true".into())
        }
        "pdf_probe_result" => Some(app.st.doc_open.probe.as_ref().filter(|p| p.done).map_or("null".into(), |p| p.out.clone())),
        "open" => {
            let j = Json::parse(args)?;
            Some(open_file(app, host, File { id: j.text("id").into(), name: j.text("name").into(), resource_key: j.text("resourceKey").into() }, 0).to_string())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_classify_the_container() {
        assert_eq!(classify("STROKES_Z:!;IHN6:QUJD"), (Kind::V6, "QUJD"));
        assert_eq!(classify("STROKES_Z:!;IHN6:QUJD;IH_X:1"), (Kind::V6, "QUJD"));
        assert_eq!(classify("STROKES_Z:eJw=;IH_STRUCT:e30="), (Kind::V5, ""));
        assert_eq!(classify("STROKES_DATA:[]"), (Kind::V5, ""));
        assert_eq!(classify(""), (Kind::Plain, ""));
        assert_eq!(classify("holiday, photos"), (Kind::Plain, ""));
    }

    #[test]
    fn drive_metadata_is_read_like_5_18_0() {
        let body = r#"{"capabilities":{"canEdit":false},"ownedByMe":false,"owners":[{"displayName":"Ana","emailAddress":"ana@example.test","photoLink":"p"}],"headRevisionId":"r1","modifiedTime":"2026-10-29T09:00:00.000Z","size":"1234","parents":["f1",""]}"#;
        let m = Meta::parse(body, "rk").unwrap();
        assert_eq!((m.can_edit, m.owned_by_me, m.owner_name.as_str(), m.resource_key.as_str(), m.head.as_str(), m.size, m.parents.len()), (false, false, "Ana", "rk", "r1", 1234.0, 1));
        // Only an explicit `false` makes a viewer.
        let m = Meta::parse("{}", "").unwrap();
        assert!(m.can_edit && m.owned_by_me && m.head.is_empty());
        assert_eq!(Meta::parse("not json", ""), None);
        let (a, mut b) = (Meta { head: "r1".into(), ..Meta::default() }, Meta { head: "r2".into(), modified: "t".into(), ..Meta::default() });
        assert!(!a.same_revision(&b));
        b.head.clear();
        assert!(!a.same_revision(&b), "no revision id and no common time");
        assert!(b.same_revision(&Meta { modified: "t".into(), ..Meta::default() }));
        assert_eq!(m.value().get("canEdit").as_bool(), Some(true));
    }

    #[test]
    fn the_pdf_answer_is_decoded() {
        let mut w = ihn_core::codec::Writer::default();
        w.u32(3);
        w.str("STROKES_Z:!;IHN6:AA==");
        w.u32(1);
        w.str("IHN6-6a");
        w.f64(42.0);
        let o = pdf::Opened::parse(&w.buf).unwrap();
        assert_eq!((o.pages, o.keywords.as_str(), o.attachments), (3, "STROKES_Z:!;IHN6:AA==", vec![("IHN6-6a".to_string(), 42.0)]));
        assert_eq!(pdf::Opened::parse(&w.buf[..w.buf.len() - 1]), None);
    }
}
