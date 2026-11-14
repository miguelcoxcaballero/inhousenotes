//! Export job planning and the export-worker export role (ARCHITECTURE §8.2, §4.5). Owned by W5.4.
//!
//! [`start`] builds the PDF of the open document: the template backgrounds are encoded here (JPEG 0.85 of 5.18.0's
//! template canvas at the export scale), every blob the PDF needs is posted to the export worker, then the snapshot
//! and the page plan. The worker's instance of this module ([`worker`]) turns the plan into one ink content stream per
//! page (`render::pdfink`) and `host/export-worker.js` assembles the file with pdf-lib: source pages copied unchanged,
//! template/photo pages drawn, the ink as one Form XObject `/IHN6Ink` behind one extra content stream, image assets
//! attached as `IHN6-<hex>`, `Keywords = "STROKES_Z:!;IHN6:" base64(zlib(snapshot))`.
//!
//! [`rebuild`] restores a source PDF this device lacks from the latest export (§4.5): the worker copies the export's
//! pages that carry it, strips `/IHN6Ink`, and fills the other page numbers with blank pages, so page `n` of the
//! rebuilt blob is page `n` of the source (no remap table is needed to render it).
// v5: app-v5.js:buildPdfBlobWithPdfLib (template JPEG cache, page plan, metadata) / buildPdfBlob

use crate::abi::{chan, fam, fx, Event, Io, OutBuf};
use crate::app::{worker_role, Mod};
use crate::doc::open::{self, pdf as pdfjs, Kind};
use crate::doc::{backgrounds, session};
use crate::drive::revisions;
use crate::editor::pages;
use crate::shell::fx::{self as sfx, deflate_flag, Src};
use crate::store::sv::hex;
use crate::{App, Host};
use ihn_core::codec::{b64_decode, Reader, Writer};
use ihn_core::doc::Doc;
use ihn_core::id::{Hash, Id};
use ihn_core::op::{Background, Body, Template};
use ihn_core::render::cmd::Img;
use ihn_core::render::{pdfink, PageItems};
use ihn_core::snapshot::{self, EncodedSnapshot};
use ihn_core::stamp::VersionVector;

/// Worker ops of the export role.
mod op {
    /// main → worker: `[u32 job][u32 key]`, obj = a Blob the job uses.
    pub const BLOB: u8 = 1;
    /// main → worker: `[u32 job]` + request plan + snapshot (→ [`super::worker`]).
    pub const BUILD: u8 = 2;
    /// worker Rust → worker JS: `[u32 job]` + assembly plan (never leaves the worker).
    pub const ASSEMBLE: u8 = 3;
    /// worker → main: `[u32 job][u8 ok]` + the PDF bytes, or an error text.
    pub const BUILT: u8 = 4;
    /// main → worker: `[u32 job][u32 n][n × (u32 export page 0-based, u32 source page 1-based)]`, obj = the export.
    pub const REBUILD: u8 = 5;
    /// worker → main: `[u32 job][u8 ok]`, obj = the rebuilt source Blob.
    pub const REBUILT: u8 = 6;
}

/// Page background kinds of the plan.
const BG_TEMPLATE: u8 = 0;
const BG_PDF: u8 = 1;
const BG_IMAGE: u8 = 2;
const BG_WHITE: u8 = 3;

/// 5.18.0 export overlay scales: `options.highQuality ? min(3.5, CANVAS_SCALE * 1.75) : 2.5`.
const SCALE_STD: f64 = 2.5;
const SCALE_HQ: f64 = 3.5;
const TEMPLATE_JPEG_QUALITY: f64 = 0.85;

/// Who asked for a build; the result goes back to it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Purpose {
    /// The sync engine's export desk (`core.export.build = Some(job)`).
    Desk(u32),
    /// The first upload of a document created on this device.
    Create,
    /// Export to the user of a document that is not in Drive (high quality).
    User,
}

/// The finished PDF and the version vector it holds.
pub type Built = Option<(Vec<u8>, VersionVector)>;

struct Job {
    id: u32,
    purpose: Purpose,
    doc: String,
    vv: VersionVector,
    /// Template JPEGs being encoded: `(request, key)`.
    tpl_wait: Vec<(u32, u32)>,
    /// Blob handles this job made (template JPEGs), released at the end.
    owned: Vec<u32>,
    /// The `BUILD` payload, posted once every template JPEG is in the worker.
    build: Vec<u8>,
}

struct Rebuild {
    hashes: Vec<Hash>,
    media: u32,
    blob: u32,
    pdf_req: u32,
    pdf: u32,
    keywords: String,
    inflate_req: u32,
    /// `(hash, worker job)` of the rebuilds posted.
    posted: Vec<(Hash, u32)>,
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    job: Option<Job>,
    queue: Vec<Purpose>,
    next: u32,
    rebuild: Option<Rebuild>,
    /// Heads whose rebuild already ran (a head is tried once per session).
    rebuilt_heads: Vec<String>,
    /// Test: `(purpose, ok, size)` of the finished builds.
    log: Vec<(String, bool, usize)>,
}

fn post(app: &mut App, host: &mut dyn Host, obj: u32, bytes: &[u8]) { crate::doc::v5open::worker_send(app, host, obj, bytes) }

fn header(w: &mut Writer, o: u8, job: u32) {
    w.u8(worker_role::EXPORT);
    w.u8(o);
    w.u32(job);
}

pub fn busy(app: &App) -> bool { app.st.export_pdf.job.is_some() || !app.st.export_pdf.queue.is_empty() }

/// Starts (or queues) a build of the open document for `purpose`. One build runs at a time.
pub fn start(app: &mut App, host: &mut dyn Host, purpose: Purpose) {
    let st = &mut app.st.export_pdf;
    if st.job.as_ref().is_some_and(|j| j.purpose == purpose) || st.queue.contains(&purpose) {
        return;
    }
    if st.job.is_some() {
        st.queue.push(purpose);
        return;
    }
    begin(app, host, purpose);
}

/// A key per distinct blob of the plan.
#[derive(Default)]
struct Keys {
    assets: Vec<(Hash, u32)>,
    templates: Vec<(Template, u32, u32, u32)>,
    next: u32,
}

fn begin(app: &mut App, host: &mut dyn Host, purpose: Purpose) {
    let Some(s) = session::get(app) else { return finish(app, host, purpose, None, String::new()) };
    let scale = if purpose == Purpose::User { SCALE_HQ } else { SCALE_STD };
    let doc = &s.doc;
    let mut keys = Keys::default();
    let mut blobs: Vec<(u32, u32)> = Vec::new();
    let mut plan = Writer::default();
    let order: Vec<Id> = doc.order().to_vec();
    plan.u32(order.len() as u32);
    let mut missing: Vec<Hash> = Vec::new();
    let asset_key = |keys: &mut Keys, blobs: &mut Vec<(u32, u32)>, h: Hash| -> Option<u32> {
        if let Some(k) = keys.assets.iter().find(|a| a.0 == h) {
            return Some(k.1);
        }
        let blob = s.asset_blob(&h)?;
        keys.next += 1;
        keys.assets.push((h, keys.next));
        blobs.push((keys.next, blob));
        Some(keys.next)
    };
    for &id in &order {
        let Some(page) = doc.pages.get(&id) else { continue };
        let (w, h) = page.size.v;
        plan.raw(&id.0.to_le_bytes());
        plan.f64(w as f64);
        plan.f64(h as f64);
        let (bg, key, src) = match page.background.v {
            Background::Pdf { asset, page: n } => match asset_key(&mut keys, &mut blobs, asset) {
                Some(k) => (BG_PDF, k, n),
                None => {
                    missing.push(asset);
                    (BG_WHITE, 0, 0)
                }
            },
            Background::Custom { asset } => match asset_key(&mut keys, &mut blobs, asset) {
                Some(k) => (BG_IMAGE, k, 0),
                None => (BG_WHITE, 0, 0),
            },
            Background::Template => {
                let kind = page.template.v;
                let (tw, th) = (((w as f64) * scale).round().max(1.0) as u32, ((h as f64) * scale).round().max(1.0) as u32);
                let key = match keys.templates.iter().find(|t| (t.0, t.1, t.2) == (kind, tw, th)) {
                    Some(t) => t.3,
                    None => {
                        keys.next += 1;
                        keys.templates.push((kind, tw, th, keys.next));
                        keys.next
                    }
                };
                (BG_TEMPLATE, key, 0)
            }
        };
        plan.u8(bg);
        plan.u32(key);
        plan.u32(src);
    }
    // Image assets: drawn on their pages and attached (`IHN6-<hex>`, ARCHITECTURE §8.2).
    let mut images: Vec<Hash> = Vec::new();
    for item in doc.items.values().filter(|i| i.is_live()) {
        if let Body::Image { asset, .. } = item.body.v {
            if !images.contains(&asset) {
                images.push(asset);
            }
        }
    }
    for page in doc.pages.values().filter(|p| !p.deleted.v) {
        if let Background::Custom { asset } = page.background.v {
            if !images.contains(&asset) {
                images.push(asset);
            }
        }
    }
    let mut attach = Vec::new();
    for h in images {
        if let Some(k) = asset_key(&mut keys, &mut blobs, h) {
            attach.push((h, k));
        }
    }
    let assets = keys.assets.clone();
    plan.u32(assets.len() as u32);
    for (h, k) in &assets {
        plan.raw(h);
        plan.u32(*k);
    }
    plan.u32(attach.len() as u32);
    for (h, k) in &attach {
        plan.str(&format!("IHN6-{}", hex(h)));
        plan.u32(*k);
    }
    let vv = doc.vv.clone();
    let snap = snapshot::encode(doc).0;
    let doc_id = s.doc_id.clone();
    if !missing.is_empty() {
        // A PDF page whose source this device lacks: it is rebuilt from the latest export first.
        finish(app, host, purpose, None, "a PDF background source is missing".into());
        return rebuild(app, host, missing);
    }
    let st = &mut app.st.export_pdf;
    st.next += 1;
    let id = st.next;
    let mut build = Writer::default();
    header(&mut build, op::BUILD, id);
    build.raw(&plan.buf);
    build.raw(&snap);
    st.job = Some(Job { id, purpose, doc: doc_id, vv, tpl_wait: Vec::new(), owned: Vec::new(), build: build.buf });
    for (key, blob) in blobs {
        send_blob(app, host, id, key, blob);
    }
    // v5: drawTemplateBackground(null, w, h, 1, kind) → canvasToBlobSafe(tplCanvas, 'image/jpeg', 0.85).
    let mut wait = Vec::new();
    for (kind, tw, th, key) in keys.templates {
        let req = app.req(Mod::ExportPdf);
        pages::with_gfx(app, |_, g| {
            g.tpl.draw(&mut g.cmds, None, tw, th, kind);
            if let Some(surf) = g.tpl.cached(kind, tw, th) {
                g.cmds.to_blob(surf, req, "image/jpeg", TEMPLATE_JPEG_QUALITY);
            }
        });
        wait.push((req, key));
    }
    pages::flush(app, host);
    if let Some(j) = app.st.export_pdf.job.as_mut() {
        j.tpl_wait = wait;
    }
    maybe_build(app, host);
}

fn send_blob(app: &mut App, host: &mut dyn Host, job: u32, key: u32, blob: u32) {
    let mut w = Writer::default();
    header(&mut w, op::BLOB, job);
    w.u32(key);
    post(app, host, blob, &w.buf);
}

fn maybe_build(app: &mut App, host: &mut dyn Host) {
    let Some(j) = app.st.export_pdf.job.as_mut().filter(|j| j.tpl_wait.is_empty() && !j.build.is_empty()) else { return };
    let bytes = std::mem::take(&mut j.build);
    post(app, host, 0, &bytes);
}

fn template_done(app: &mut App, host: &mut dyn Host, io: &Io) -> bool {
    let Some(j) = app.st.export_pdf.job.as_mut() else { return false };
    let Some(at) = j.tpl_wait.iter().position(|t| t.0 == io.req) else { return false };
    let (_, key) = j.tpl_wait.swap_remove(at);
    let id = j.id;
    if io.obj != 0 {
        j.owned.push(io.obj);
        send_blob(app, host, id, key, io.obj);
    }
    // A template that failed to encode leaves a white page (5.18.0 warns and continues).
    maybe_build(app, host);
    true
}

fn finish(app: &mut App, host: &mut dyn Host, purpose: Purpose, out: Built, error: String) {
    let size = out.as_ref().map_or(0, |o| o.0.len());
    app.st.export_pdf.log.push((format!("{purpose:?}"), out.is_some(), size));
    if out.is_none() {
        host.log(2, &format!("export: build failed: {error}"));
    }
    match purpose {
        Purpose::Desk(_) | Purpose::Create => super::upload::built(app, host, purpose, out),
        Purpose::User => super::user::built(app, host, out),
    }
    if app.st.export_pdf.job.is_none() && !app.st.export_pdf.queue.is_empty() {
        let next = app.st.export_pdf.queue.remove(0);
        begin(app, host, next);
    }
}

fn worker_msg(app: &mut App, host: &mut dyn Host, obj: u32, data: &[u8]) -> bool {
    let mut r = Reader::new(data);
    let (Some(worker_role::EXPORT), Some(o), Some(job)) = (r.u8(), r.u8(), r.u32()) else { return false };
    match o {
        op::BUILT => {
            let Some(j) = app.st.export_pdf.job.take_if(|j| j.id == job) else { return true };
            for blob in &j.owned {
                host.out().obj_release(*blob);
            }
            let ok = r.u8() == Some(1);
            let (out, error) = if ok { (Some((r.buf.to_vec(), j.vv)), String::new()) } else { (None, String::from_utf8_lossy(r.buf).into_owned()) };
            let _ = j.doc;
            finish(app, host, j.purpose, out, error);
        }
        op::REBUILT => rebuilt(app, host, job, r.u8() == Some(1), obj),
        _ => {}
    }
    true
}

/// The worker died: the running build fails (the desk retries with backoff).
fn worker_failed(app: &mut App, host: &mut dyn Host) {
    if let Some(j) = app.st.export_pdf.job.take() {
        for blob in &j.owned {
            host.out().obj_release(*blob);
        }
        finish(app, host, j.purpose, None, "worker".into());
    }
    if let Some(rb) = app.st.export_pdf.rebuild.take() {
        end_rebuild(host, rb);
    }
}

// ---- the worker's instance ----------------------------------------------------------------------------------------

/// Export-worker entry for this role (`ihn_worker`, first payload byte = `app::worker_role`); runs in the
/// worker's own instance of the module. `BUILD` → `ASSEMBLE` for `host/export-worker.js`.
pub fn worker(host: &mut dyn Host, msg: &[u8]) {
    let mut r = Reader::new(msg);
    let (Some(op::BUILD), Some(job)) = (r.u8(), r.u32()) else { return };
    let out = assemble_plan(job, r.buf).unwrap_or_else(|| {
        let mut w = Writer::default();
        header(&mut w, op::BUILT, job);
        w.u8(0);
        w.raw(b"bad export plan");
        w.buf
    });
    let o: &mut OutBuf = host.out();
    let m = o.w.rec(fam::FX, fx::WORKER_POST);
    o.w.u32(0);
    o.w.bytes(&out);
    o.w.end(m);
}

/// Request plan (+ snapshot) → assembly plan (+ snapshot): the ink of every page as a PDF content stream.
fn assemble_plan(job: u32, bytes: &[u8]) -> Option<Vec<u8>> {
    let mut r = Reader::new(bytes);
    struct P {
        id: Id,
        w: f64,
        h: f64,
        bg: u8,
        key: u32,
        src: u32,
    }
    let n = r.u32()? as usize;
    let mut plan = Vec::with_capacity(n.min(4096));
    for _ in 0..n {
        let id = Id(u128::from_le_bytes(r.take(16)?.try_into().ok()?));
        plan.push(P { id, w: r.f64()?, h: r.f64()?, bg: r.u8()?, key: r.u32()?, src: r.u32()? });
    }
    let mut assets: Vec<(Hash, u32)> = Vec::new();
    for _ in 0..r.u32()? {
        let h: Hash = r.take(32)?.try_into().ok()?;
        assets.push((h, r.u32()?));
    }
    let mut attach: Vec<(String, u32)> = Vec::new();
    for _ in 0..r.u32()? {
        let name = r.str()?.to_string();
        attach.push((name, r.u32()?));
    }
    let snap = r.buf;
    let doc: Doc = snapshot::decode(snap).ok()?;
    let mut w = Writer::default();
    header(&mut w, op::ASSEMBLE, job);
    w.u32(plan.len() as u32);
    let key_of = |h: Hash| assets.iter().find(|a| a.0 == h).map(|a| Img(a.1));
    for p in &plan {
        w.f64(p.w);
        w.f64(p.h);
        w.u8(p.bg);
        w.u32(p.key);
        w.u32(p.src);
        let mut items = PageItems::collect(&doc, p.id, &key_of);
        items.images.retain(|i| i.img.is_some());
        match pdfink::content_stream(p.h, &items) {
            Some(ink) => {
                w.str(&ink.content);
                w.u32(ink.images.len() as u32);
                for i in ink.images {
                    w.u32(items.images[i].img.map_or(0, |m| m.0));
                }
            }
            None => {
                w.str("");
                w.u32(0);
            }
        }
    }
    w.u32(attach.len() as u32);
    for (name, key) in &attach {
        w.str(name);
        w.u32(*key);
    }
    w.raw(snap);
    Some(w.buf)
}

// ---- source rebuild (ARCHITECTURE §4.5) -----------------------------------------------------------------------------

/// Sources the background renderer could not find: download the latest export and rebuild them from it.
fn rebuild(app: &mut App, host: &mut dyn Host, extra: Vec<Hash>) {
    if app.st.export_pdf.rebuild.is_some() {
        return;
    }
    let Some(s) = session::get(app) else { return };
    let head = s.record.last_export.as_ref().map(|l| l.head.clone()).or_else(|| s.record.exports.last().map(|e| e.0.clone())).unwrap_or_default();
    if head.is_empty() || app.st.export_pdf.rebuilt_heads.contains(&head) || !s.record.flags.created {
        return;
    }
    let mut hashes = backgrounds::take_missing(app);
    for h in extra {
        if !hashes.contains(&h) {
            hashes.push(h);
        }
    }
    if hashes.is_empty() {
        return;
    }
    app.st.export_pdf.rebuilt_heads.push(head);
    let doc = session::doc_id(app).to_string();
    let key = session::get(app).map_or(String::new(), |s| s.record.drive_meta.get("resourceKey").text());
    let media = revisions::download(app, host, Mod::ExportPdf, &doc, &key);
    app.st.export_pdf.rebuild = Some(Rebuild { hashes, media, blob: 0, pdf_req: 0, pdf: 0, keywords: String::new(), inflate_req: 0, posted: Vec::new() });
}

fn end_rebuild(host: &mut dyn Host, rb: Rebuild) {
    if rb.pdf != 0 {
        pdfjs::close(host.out(), rb.pdf);
    }
    host.out().obj_release(rb.blob);
}

fn rebuild_io(app: &mut App, host: &mut dyn Host, io: &Io) -> bool {
    let Some(rb) = app.st.export_pdf.rebuild.as_mut() else { return false };
    if io.req == rb.media {
        rb.media = 0;
        if io.status < 0 || io.obj == 0 || !(200..300).contains(&io.status) {
            host.out().obj_release(io.obj);
            app.st.export_pdf.rebuild = None;
            return true;
        }
        rb.blob = io.obj;
        let req = app.req(Mod::ExportPdf);
        let rb = app.st.export_pdf.rebuild.as_mut().unwrap();
        rb.pdf_req = req;
        pdfjs::open(host.out(), req, rb.blob);
    } else if io.req == rb.pdf_req {
        rb.pdf_req = 0;
        let opened = (io.status >= 0).then(|| pdfjs::Opened::parse(io.data)).flatten();
        rb.pdf = io.obj;
        let payload = opened.as_ref().map(|o| open::classify(&o.keywords)).filter(|c| c.0 == Kind::V6).and_then(|c| b64_decode(c.1.as_bytes()));
        let Some(bytes) = payload else {
            let rb = app.st.export_pdf.rebuild.take().unwrap();
            end_rebuild(host, rb);
            return true;
        };
        rb.keywords.clear();
        let req = app.req(Mod::ExportPdf);
        app.st.export_pdf.rebuild.as_mut().unwrap().inflate_req = req;
        sfx::deflate(host.out(), req, deflate_flag::INFLATE | deflate_flag::ZLIB, Src::Bytes(&bytes));
    } else if io.req == rb.inflate_req {
        rb.inflate_req = 0;
        let doc = (io.status >= 0).then(|| EncodedSnapshot(io.data.to_vec()).decode().ok()).flatten();
        let mut rb = app.st.export_pdf.rebuild.take().unwrap();
        if let Some(doc) = doc {
            for h in rb.hashes.clone() {
                let pages = carried(&doc, &h);
                if pages.is_empty() {
                    continue;
                }
                app.st.export_pdf.next += 1;
                let job = app.st.export_pdf.next;
                let mut w = Writer::default();
                header(&mut w, op::REBUILD, job);
                w.u32(pages.len() as u32);
                for (at, src) in pages {
                    w.u32(at);
                    w.u32(src);
                }
                post(app, host, rb.blob, &w.buf);
                rb.posted.push((h, job));
            }
        }
        if rb.posted.is_empty() {
            end_rebuild(host, rb);
        } else {
            app.st.export_pdf.rebuild = Some(rb);
        }
    } else {
        return false;
    }
    true
}

/// `(position in the export, source page)` of the exported pages that carry source `h`.
fn carried(doc: &Doc, h: &Hash) -> Vec<(u32, u32)> {
    doc.order()
        .iter()
        .enumerate()
        .filter_map(|(at, id)| match doc.pages.get(id)?.background.v {
            Background::Pdf { asset, page } if asset == *h => Some((at as u32, page)),
            _ => None,
        })
        .collect()
}

fn rebuilt(app: &mut App, host: &mut dyn Host, job: u32, ok: bool, obj: u32) {
    let Some(rb) = app.st.export_pdf.rebuild.as_mut() else { return host.out().obj_release(obj) };
    let Some(at) = rb.posted.iter().position(|p| p.1 == job) else { return host.out().obj_release(obj) };
    let (hash, _) = rb.posted.swap_remove(at);
    let done = rb.posted.is_empty();
    if ok && obj != 0 {
        session::put_asset(app, host, hash, obj, "application/pdf", 0.0);
        backgrounds::asset_arrived(app, host, hash);
    } else {
        host.out().obj_release(obj);
    }
    if done {
        if let Some(rb) = app.st.export_pdf.rebuild.take() {
            end_rebuild(host, rb);
        }
    }
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(_app: &mut App, _host: &mut dyn Host) {}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Io(io) => {
            if !template_done(app, host, &io) && !rebuild_io(app, host, &io) {
                host.out().obj_release(io.obj);
            }
        }
        Event::Msg(m) if m.chan == chan::WORKER => {
            worker_msg(app, host, m.obj, m.data);
        }
        Event::Msg(m) if m.chan == chan::WORKER_ERROR => worker_failed(app, host),
        Event::Msg(m) if m.chan == session::sig::STATE || m.chan == session::sig::CHANGED => rebuild(app, host, Vec::new()),
        Event::Msg(m) if m.chan == session::sig::CLOSED => {
            app.st.export_pdf.rebuilt_heads.clear();
            if let Some(rb) = app.st.export_pdf.rebuild.take() {
                end_rebuild(host, rb);
            }
        }
        _ => {}
    }
    0
}

/// `__IHN_TEST_API__.cmd('export.pdf.<name>', args)`: `state` → `{busy, log: [[purpose, ok, size]…]}`.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    let st = &app.st.export_pdf;
    match name {
        "state" => {
            let log: Vec<String> = st.log.iter().map(|(p, ok, n)| format!("[\"{p}\",{ok},{n}]")).collect();
            Some(format!("{{\"busy\":{},\"rebuilding\":{},\"log\":[{}]}}", busy(app), st.rebuild.is_some(), log.join(",")))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ihn_core::op::Color;

    #[test]
    fn plan_has_ink_images_and_snapshot() {
        let doc = session::fixture();
        let order = doc.order().to_vec();
        let mut plan = Writer::default();
        plan.u32(order.len() as u32);
        for id in &order {
            plan.raw(&id.0.to_le_bytes());
            plan.f64(794.0);
            plan.f64(1123.0);
            plan.u8(BG_TEMPLATE);
            plan.u32(7);
            plan.u32(0);
        }
        plan.u32(1);
        plan.raw(&session::FIXTURE_ASSET);
        plan.u32(3);
        plan.u32(1);
        plan.str("IHN6-x");
        plan.u32(3);
        let snap = snapshot::encode(&doc).0;
        plan.raw(&snap);
        let out = assemble_plan(9, &plan.buf).unwrap();
        let mut r = Reader::new(&out);
        assert_eq!((r.u8(), r.u8(), r.u32(), r.u32()), (Some(worker_role::EXPORT), Some(op::ASSEMBLE), Some(9), Some(order.len() as u32)));
        let mut inked = 0;
        for _ in 0..order.len() {
            let (_, _, bg, key, _) = (r.f64(), r.f64(), r.u8(), r.u32(), r.u32());
            assert_eq!((bg, key), (Some(BG_TEMPLATE), Some(7)));
            let content = r.str().unwrap().to_string();
            let n = r.u32().unwrap();
            for _ in 0..n {
                assert_eq!(r.u32(), Some(3));
            }
            if !content.is_empty() {
                inked += 1;
                assert!(content.starts_with("q\n") && content.ends_with("Q\n"));
            }
        }
        assert!(inked > 0);
        assert_eq!((r.u32(), r.str(), r.u32()), (Some(1), Some("IHN6-x"), Some(3)));
        assert_eq!(r.buf, &snap[..]);
        assert!(assemble_plan(1, &[1, 2, 3]).is_none());
        let _ = Color::Hex(0);
    }

    #[test]
    fn carried_pages_follow_the_export_order() {
        let doc = session::fixture();
        let pdf_pages: Vec<(u32, u32)> = doc.order().iter().enumerate().filter_map(|(at, id)| match doc.pages[id].background.v {
            Background::Pdf { page, .. } => Some((at as u32, page)),
            _ => None,
        }).collect();
        let hash = doc.order().iter().find_map(|id| match doc.pages[id].background.v {
            Background::Pdf { asset, .. } => Some(asset),
            _ => None,
        });
        if let Some(h) = hash {
            assert_eq!(carried(&doc, &h), pdf_pages);
        }
        assert!(carried(&doc, &[9; 32]).is_empty());
    }
}
