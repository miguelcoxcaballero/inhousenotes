//! User "Import PDF" (`#btn-import-pdf` → `#pdf-input`, spec/documents.md; BEHAVIOUR-CHANGES: Import PDF replaces the
//! open document's pages). Owned by W5.1.
//!
//! The chosen file is opened with pdf.js (security-core limits in `host/pdf.js`), its page sizes and SHA-256 are read,
//! and its content is converted the way a Drive open converts it: a 5.x document or a plain PDF through
//! `v5::import::from_pdf` (lenient, as 5.18.0's user import), a v6 container through its snapshot. The result is
//! built in a scratch `Doc`, then written to the open document as **one native batch** (every old page deleted,
//! every imported page and item created with this device's ids), so collaborators receive it like any other edit.
//! The source PDF and inline images become assets. As in 5.18.0 the undo history is cleared.
// v5: app-v5.js:importPDF / importPDFData

use super::open::{classify, pdf, Kind};
use super::session;
use crate::abi::{mode, Event};
use crate::Prop;
use crate::app::Mod;
use crate::editor::{history, pages, selection, view, viewport};
use crate::panels::pages_panel;
use crate::shell::fx::{self, deflate_flag, Src};
use crate::shell::json::Json;
use crate::shell::{self, Status, StatusOpts};
use crate::store::sv::hex;
use crate::{App, Host, Ui};
use ihn_core::codec::{b64_decode, Reader};
use ihn_core::doc::Doc;
use ihn_core::id::{ActorId, Hash, Id};
use ihn_core::jsmath::round;
use ihn_core::op::{Body, ImageRect};
use ihn_core::snapshot::EncodedSnapshot;
use ihn_core::v5::import::{self, PdfFacts, Source};

const L_CHANGE: u32 = crate::abi::tag(Mod::DocImport, 1);
const CLEAN_ORIGINAL: &str = "IH_CLEAN_ORIGINAL.pdf";
/// v5 `securityCore.MAX_PDF_BYTES`.
const MAX_BYTES: f64 = 64.0 * 1024.0 * 1024.0;
const TOO_LARGE: &str = "This PDF is larger than the 64 MB safety limit.";
const FAILED: &str = "Error importing PDF";
const FILES: u32 = 4;

#[derive(Default)]
struct Job {
    doc: String,
    name: String,
    blob: u32,
    size: f64,
    pdf: u32,
    kind: Option<Kind>,
    keywords: String,
    attachments: Vec<(String, f64)>,
    sizes: Option<Vec<(u32, u32)>>,
    sha: Option<Hash>,
    /// v6: the decoded snapshot (`None` until inflated).
    v6: Option<Doc>,
    scratch: Option<Doc>,
    /// Attachments still to read: `(asset, attachment name)`.
    wanted: Vec<(Hash, String)>,
    blob_kept: bool,
    strokes: usize,
    v5: bool,
    /// Requests in flight.
    reqs: Vec<(u32, u8)>,
}

const R_TAKE: u8 = 1;
const R_OPEN: u8 = 2;
const R_PAGES: u8 = 3;
const R_DIGEST: u8 = 4;
const R_INFLATE: u8 = 5;
const R_ATTACH: u8 = 6;

#[derive(Default)]
pub struct State {
    job: Option<Job>,
    take_base: u32,
    /// Outcomes (test command): `pages:N`, `error:<text>`.
    log: Vec<String>,
}

/// v5: the `#btn-import-pdf` click → `pdfInput.click()`.
pub fn pick(app: &mut App, host: &mut dyn Host) {
    if view::read_only(app) {
        return;
    }
    host.out().click(Ui::PDF_INPUT);
}

fn job(app: &mut App) -> Option<&mut Job> { app.st.doc_import.job.as_mut() }

fn request(app: &mut App, what: u8) -> u32 {
    let req = app.req(Mod::DocImport);
    if let Some(j) = job(app) {
        j.reqs.push((req, what));
    }
    req
}

/// Ends the job: releases what it holds; `Err` shows 5.18.0's status.
fn end(app: &mut App, host: &mut dyn Host, r: Result<String, &str>) {
    let Some(j) = app.st.doc_import.job.take() else { return };
    let out = host.out();
    if j.pdf != 0 {
        pdf::close(out, j.pdf);
    }
    if !j.blob_kept && j.blob != 0 {
        out.obj_release(j.blob);
    }
    out.prop(Ui::PDF_INPUT, "value", Prop::Str(""));
    shell::hide_loading(app, host);
    match r {
        Ok(text) => {
            shell::status(app, host, &text, StatusOpts::from(Status::Info).preserve());
            app.st.doc_import.log.push(text);
        }
        Err(e) => {
            shell::status(app, host, e, StatusOpts::from(Status::Info).preserve());
            app.st.doc_import.log.push(format!("error:{e}"));
        }
    }
}

/// The file arrived: v5 `importPDFData(objectUrl, true, {fileSize, sourceBlob})`.
fn start(app: &mut App, host: &mut dyn Host, blob: u32, name: String, size: f64) {
    if view::read_only(app) || session::get(app).is_none_or(|s| !s.writable()) {
        host.out().obj_release(blob);
        return;
    }
    if size > MAX_BYTES {
        host.out().obj_release(blob);
        host.out().prop(Ui::PDF_INPUT, "value", Prop::Str(""));
        shell::status(app, host, TOO_LARGE, StatusOpts::from(Status::Error).preserve());
        app.st.doc_import.log.push(format!("error:{TOO_LARGE}"));
        return;
    }
    let name = name.strip_suffix(".pdf").or_else(|| name.strip_suffix(".PDF")).unwrap_or(&name).to_string();
    app.st.doc_import.job = Some(Job { doc: session::doc_id(app).to_string(), name, blob, size, ..Job::default() });
    shell::loading(app, host, "Analyzing PDF...");
    let req = request(app, R_OPEN);
    pdf::open(host.out(), req, blob);
}

fn opened(app: &mut App, host: &mut dyn Host, status: i32, obj: u32, data: &[u8]) {
    let Some(o) = (status >= 0).then(|| pdf::Opened::parse(data)).flatten() else {
        return end(app, host, Err(if status == pdf::TOO_LARGE { TOO_LARGE } else { FAILED }));
    };
    let (kind, payload) = classify(&o.keywords);
    let v6 = if kind == Kind::V6 { b64_decode(payload.as_bytes()) } else { None };
    if kind == Kind::V6 && v6.is_none() {
        return end(app, host, Err(FAILED));
    }
    let Some(j) = job(app) else { return };
    let (blob, pages) = (j.blob, o.pages);
    (j.pdf, j.kind, j.keywords, j.attachments) = (obj, Some(kind), o.keywords, o.attachments);
    let (a, b) = (request(app, R_PAGES), request(app, R_DIGEST));
    let out = host.out();
    pdf::pages(out, a, obj, 1, pages);
    fx::digest(out, b, Src::blob(blob));
    if let Some(bytes) = v6 {
        let c = request(app, R_INFLATE);
        fx::deflate(host.out(), c, deflate_flag::INFLATE | deflate_flag::ZLIB, Src::Bytes(&bytes));
    }
}

/// Every fact is in: convert into the scratch document.
fn facts_ready(app: &mut App, host: &mut dyn Host) {
    let Some(j) = job(app) else { return };
    if !j.reqs.is_empty() || j.scratch.is_some() {
        return;
    }
    let (Some(sizes), Some(sha)) = (j.sizes.take(), j.sha) else { return end(app, host, Err(FAILED)) };
    if j.kind == Some(Kind::V6) {
        let Some(d) = j.v6.take() else { return end(app, host, Err(FAILED)) };
        let wanted: Vec<Hash> = image_assets(&d);
        j.strokes = d.items.values().filter(|i| i.is_live() && matches!(i.body.v, Body::Stroke { .. }) && d.pages.get(&i.page.v).is_some_and(|p| !p.deleted.v)).count();
        j.v5 = true;
        let names: Vec<String> = j.attachments.iter().map(|a| a.0.clone()).collect();
        j.wanted = wanted.into_iter().filter_map(|h| Some((h, names.iter().find(|n| n.strip_prefix("IHN6-") == Some(&hex(&h)[..]))?.clone()))).collect();
        j.scratch = Some(d);
        return next_asset(app, host);
    }
    let facts = PdfFacts { doc_id: String::new(), name: j.name.clone(), page_sizes: sizes, revision_sha256: sha, clean_attachment: j.attachments.iter().any(|a| a.0 == CLEAN_ORIGINAL), strict: false };
    let actor = ActorId::import(&hex(&sha));
    let imp = match import::from_pdf(&j.keywords, &facts, actor, host.time().max(1.0) as u64) {
        Ok(imp) => imp,
        Err(_) => return end(app, host, Err(FAILED)),
    };
    let mut d = Doc::default();
    for b in &imp.batches {
        d.apply(b);
    }
    let j = job(app).expect("job");
    (j.strokes, j.v5) = (imp.strokes, imp.v5);
    j.scratch = Some(d);
    for a in &imp.assets {
        let obj = fx::new_obj(app);
        fx::blob_new(host.out(), obj, &a.mime, &a.bytes);
        session::put_asset(app, host, a.hash, obj, &a.mime, a.bytes.len() as f64);
    }
    let j = job(app).expect("job");
    match imp.source {
        Some((Source::Attachment | Source::Stripped, hash)) if facts.clean_attachment => j.wanted.push((hash, CLEAN_ORIGINAL.into())),
        Some((_, hash)) => {
            // The file itself (a stripped copy needs the export worker; the saved bytes are the fallback 5.18.0 uses).
            j.blob_kept = true;
            let (blob, size) = (j.blob, j.size);
            session::put_asset(app, host, hash, blob, "application/pdf", size);
        }
        None => {}
    }
    next_asset(app, host);
}

/// The image assets a v6 document references.
fn image_assets(d: &Doc) -> Vec<Hash> {
    let mut v: Vec<Hash> = d.items.values().filter_map(|i| match &i.body.v {
        Body::Image { asset, .. } if i.is_live() => Some(*asset),
        _ => None,
    }).collect();
    v.sort();
    v.dedup();
    v
}

fn next_asset(app: &mut App, host: &mut dyn Host) {
    loop {
        let Some(j) = job(app) else { return };
        let Some((hash, name)) = j.wanted.last().cloned() else { return commit(app, host) };
        let pdf = j.pdf;
        if session::get(app).is_some_and(|s| s.asset_blob(&hash).is_some()) {
            job(app).expect("job").wanted.pop();
            continue;
        }
        let req = request(app, R_ATTACH);
        pdf::attachment(host.out(), req, pdf, &name);
        return;
    }
}

/// One batch: the old pages deleted, the imported pages and items created.
fn commit(app: &mut App, host: &mut dyn Host) {
    let Some(j) = app.st.doc_import.job.as_mut() else { return };
    let Some(src) = j.scratch.take() else { return end(app, host, Err(FAILED)) };
    let (strokes, v5, doc) = (j.strokes, j.v5, j.doc.clone());
    if session::doc_id(app) != doc {
        return end(app, host, Err(FAILED));
    }
    selection::clear(app, host);
    let old: Vec<Id> = session::get(app).map(|s| s.doc.order().to_vec()).unwrap_or_default();
    let mut count = 0;
    let edit = session::edit(app, host, |tx| {
        for p in &old {
            tx.page_delete(*p);
        }
        let mut prev: Option<Id> = None;
        for id in src.order() {
            let Some(p) = src.pages.get(id) else { continue };
            let page = tx.page_new(prev.or(Some(Id::NONE)), p.template.v, p.size.v, p.background.v);
            prev = Some(page);
            count += 1;
            for r in src.draw_list(*id).iter() {
                let Some(item) = src.live(r.id) else { continue };
                match &item.body.v {
                    Body::Stroke { .. } => {
                        tx.add_stroke(page, item.body.v.clone());
                    }
                    Body::Image { asset, cx, cy, w, h, rotation } => {
                        tx.add_image(page, *asset, ImageRect { cx: *cx, cy: *cy, w: *w, h: *h, rotation: *rotation });
                    }
                    Body::Dropped => {}
                }
            }
        }
    });
    let Some(e) = edit else { return end(app, host, Err(FAILED)) };
    history::clear(app, host);
    pages::changed(app, host, &e.changes);
    pages::set_active_page(app, 0);
    viewport::center_on_top(app, host);
    pages_panel::refresh(app, host);
    let text = if v5 { format!("PDF imported: {count} pages, {strokes} strokes restored") } else { format!("PDF imported: {count} pages (as background)") };
    end(app, host, Ok(text));
}

pub fn init(app: &mut App, host: &mut dyn Host) {
    if !app.cfg.has(mode::HEADLESS) {
        host.out().listen(Ui::PDF_INPUT, "change", L_CHANGE, 0);
    }
}

pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match event {
        Event::Dom(d) if d.listener == L_CHANGE => {
            if d.obj == 0 {
                return 0;
            }
            if app.st.doc_import.job.is_some() || view::read_only(app) {
                host.out().obj_release(d.obj);
                host.out().prop(Ui::PDF_INPUT, "value", Prop::Str(""));
                return 0;
            }
            let base = fx::new_obj(app);
            for _ in 1..FILES {
                fx::new_obj(app);
            }
            app.st.doc_import.take_base = base;
            app.st.doc_import.job = Some(Job::default());
            let req = request(app, R_TAKE);
            let out = host.out();
            fx::files_take(out, req, d.obj, base, false);
            out.obj_release(d.obj);
        }
        Event::Io(io) => {
            let Some(j) = job(app) else {
                host.out().obj_release(io.obj);
                return 0;
            };
            let Some(at) = j.reqs.iter().position(|r| r.0 == io.req) else {
                host.out().obj_release(io.obj);
                return 0;
            };
            let what = j.reqs.remove(at).1;
            match what {
                R_TAKE => {
                    app.st.doc_import.job = None;
                    let base = app.st.doc_import.take_base;
                    let list = Json::parse(std::str::from_utf8(io.data).unwrap_or("")).unwrap_or(Json::Null);
                    let n = if io.status > 0 { (io.status as usize).min(list.items().len()).min(FILES as usize) } else { 0 };
                    for i in 1..n {
                        host.out().obj_release(base + i as u32);
                    }
                    if n > 0 {
                        let f = &list.items()[0];
                        let (name, size) = (f.text("name").to_string(), f.get("size").and_then(Json::as_f64).unwrap_or(0.0));
                        start(app, host, base, name, size);
                    }
                }
                R_OPEN => opened(app, host, io.status, io.obj, io.data),
                R_PAGES => {
                    let mut r = Reader::new(io.data);
                    let mut sizes = Vec::new();
                    while let (Some(w), Some(h)) = (r.f64(), r.f64()) {
                        sizes.push((round(w).max(1.0) as u32, round(h).max(1.0) as u32));
                    }
                    if io.status < 0 || sizes.is_empty() {
                        end(app, host, Err(FAILED));
                    } else {
                        job(app).expect("job").sizes = Some(sizes);
                        facts_ready(app, host);
                    }
                }
                R_DIGEST => {
                    let sha = io.data.get(..32).and_then(|d| <Hash>::try_from(d).ok()).filter(|_| io.status >= 0);
                    if sha.is_none() {
                        end(app, host, Err(FAILED));
                    } else {
                        job(app).expect("job").sha = sha;
                        facts_ready(app, host);
                    }
                }
                R_INFLATE => match EncodedSnapshot(io.data.to_vec()).decode().ok().filter(|_| io.status >= 0) {
                    Some(d) => {
                        job(app).expect("job").v6 = Some(d);
                        facts_ready(app, host);
                    }
                    None => end(app, host, Err(FAILED)),
                },
                R_ATTACH => {
                    let Some((hash, _)) = job(app).and_then(|j| j.wanted.pop()) else { return 0 };
                    if io.status >= 0 && io.obj != 0 {
                        let size = Reader::new(io.data).f64().unwrap_or(0.0);
                        session::put_asset(app, host, hash, io.obj, "application/pdf", size);
                    } else if let Some(j) = job(app).filter(|j| j.kind != Some(Kind::V6)) {
                        // The clean original could not be read: the saved file is the source (5.18.0's fallback).
                        j.blob_kept = true;
                        let (blob, size) = (j.blob, j.size);
                        session::put_asset(app, host, hash, blob, "application/pdf", size);
                    }
                    next_asset(app, host);
                }
                _ => {}
            }
        }
        Event::Msg(m) if m.chan == session::sig::CLOSED => {
            if app.st.doc_import.job.is_some() {
                end(app, host, Err(FAILED));
            }
        }
        _ => {}
    }
    0
}

/// `doc.import.state` → `{busy, log: [...]}`.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    match name {
        "state" => Some(format!(
            "{{\"busy\":{},\"log\":[{}]}}",
            app.st.doc_import.job.is_some(),
            app.st.doc_import.log.iter().map(|l| crate::testapi::json_str(l)).collect::<Vec<_>>().join(",")
        )),
        _ => None,
    }
}
