//! 5.x and plain PDFs (ARCHITECTURE §9, SYNC §15.1–§15.3): the importer behind `doc::open`'s `sig::FOREIGN` seam,
//! the export-worker import role, and the foreign-overwrite recovery record. Owned by W3.4.
//!
//! An open hands over the downloaded revision. Facts are gathered (page sizes from pdf.js, SHA-256 of the bytes),
//! `v5::import::from_pdf` runs — in the export worker when the Keywords are large, so the main instance's memory never
//! holds a legacy JSON document — and its batches go through `Replica::receive` with `supersede` (5.x deletes by
//! leaving out). The source PDF the pages show is stored as an asset: the downloaded bytes, the
//! `IH_CLEAN_ORIGINAL.pdf` attachment, or the bytes with the overlay XObjects stripped by pdf-lib in the worker.
//! A plain PDF replacing an existing document keeps the v6 state and records the foreign revision for the recovered
//! copy (`take_recovery`, written by the export of W5.4).
// v5: app-v5.js:importPDFData / extractCleanOriginalAttachment / stripOverlaysFromPdfBytes / openDriveFile

use super::open::{self, pdf, Foreign, Kind};
use super::session::{self, sig};
use crate::abi::{chan, fam, fx, ret, Event, OutBuf};
use crate::app::{worker_role, Mod};
use crate::shell::fx::{self as sfx, Src};
use crate::testapi::json_str;
use crate::{App, Host};
use ihn_core::codec::{Reader, Writer};
use ihn_core::jsmath::round;
use ihn_core::id::{migration_source, ActorId, Hash, Id};
use ihn_core::op::{Background, Batch, Body, Op};
use ihn_core::snapshot::{encode_batch, EncodedBatch};
use ihn_core::v5::import::{self, PdfFacts, Source};
use ihn_core::v5::keywords::OverlayRef;
use ihn_core::v5::{check_page_count, idb::iso_ms};

/// Keywords longer than this are parsed in the export worker (their inflated JSON passes the 4 MiB of
/// `v5::WORKER_PARSE_THRESHOLD` at the usual 4–16× ratio of base64 zlib stroke JSON).
const WORKER_KEYWORDS: usize = 256 << 10;
const CLEAN_ORIGINAL: &str = "IH_CLEAN_ORIGINAL.pdf";
/// Worker ops of the import role: parse runs `from_pdf` in the worker's instance; strip is pdf-lib in
/// `host/export-worker.js`.
const OP_PARSE: u8 = 1;
const OP_STRIP: u8 = 2;

/// A plain PDF that replaced an existing document in Drive: saved as `"<name> (recovered Drive version).pdf"` next
/// to it before the next export overwrites it (SYNC §15.1; 5.18.0 recovery rule).
#[derive(Clone, Debug, PartialEq)]
pub struct Recovery {
    pub doc_id: String,
    /// The copy's file name.
    pub name: String,
    pub head: String,
    /// The foreign revision (a Blob handle; the taker owns it).
    pub blob: u32,
    pub size: f64,
}

/// The recovered copy to write before the next export of `doc_id`, if a foreign revision replaced it.
pub fn take_recovery(app: &mut App, doc_id: &str) -> Option<Recovery> {
    let r = &mut app.st.doc_v5open.recoveries;
    let at = r.iter().position(|r| r.doc_id == doc_id)?;
    Some(r.remove(at))
}

struct Job {
    f: Foreign,
    pages_req: u32,
    digest_req: u32,
    /// Request id of the worker job, of the attachment read or of the strip.
    wait: u32,
    sizes: Option<Vec<(u32, u32)>>,
    sha: Option<Hash>,
    facts: PdfFacts,
    /// The source asset still to store: `(variant, hash)`.
    source: Option<(Source, Hash)>,
    /// The downloaded blob became an asset (not released at the end).
    blob_kept: bool,
    /// Import batches received so far, and what `supersede` needs of them.
    encoded: Vec<EncodedBatch>,
    skeletons: Vec<Batch>,
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    job: Option<Job>,
    worker: bool,
    recoveries: Vec<Recovery>,
    /// `(doc id, outcome)`: `worker`, `v5:N` / `plain:N` (pages), `source:<variant>`, `recovery`, `error:<message>`
    /// (test command).
    log: Vec<(String, String)>,
    /// Main-instance memory at the start, after the batches were applied and at the end of each import (test).
    mem: Vec<f64>,
    /// Request completed when an [`import`] ends (0 ≙ an open's import).
    notify: u32,
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(_app: &mut App, _host: &mut dyn Host) {}

fn job(app: &mut App) -> Option<&mut Job> { app.st.doc_v5open.job.as_mut() }
fn log(app: &mut App, id: &str, what: String) { app.st.doc_v5open.log.push((id.into(), what)); }
/// Bytes of this instance's linear memory (0 off wasm).
fn memory() -> f64 {
    #[cfg(target_arch = "wasm32")]
    return core::arch::wasm32::memory_size(0) as f64 * 65536.0;
    #[cfg(not(target_arch = "wasm32"))]
    0.0
}
fn mark(app: &mut App) { app.st.doc_v5open.mem.push(memory()); }

/// `sig::FOREIGN`: claim the PDF.
fn start(app: &mut App, host: &mut dyn Host) {
    let Some(f) = open::take_foreign(app) else { return };
    abandon(app, host); // a previous import still running is abandoned
    begin(app, host, f, 0);
}

/// Imports `f` into the open session outside an open (the 5.x migration's Drive base, W5.6). `notify` completes
/// with 0 once the batches and the source asset are in the session, -1 when the import failed.
pub fn import(app: &mut App, host: &mut dyn Host, f: Foreign, notify: u32) {
    abandon(app, host);
    begin(app, host, f, notify);
}

fn begin(app: &mut App, host: &mut dyn Host, f: Foreign, notify: u32) {
    app.st.doc_v5open.notify = notify;
    if f.kind == Kind::Plain && session::get(app).is_some_and(|s| s.doc.order().iter().any(|p| s.doc.pages.get(p).is_some_and(|p| !p.deleted.v))) {
        // A foreign app replaced an existing document: keep the v6 state, remember the revision.
        let name = format!("{} (recovered Drive version).pdf", base_name(&f.file.name));
        pdf::close(host.out(), f.pdf);
        let (doc_id, head) = (f.file.id.clone(), f.meta.head.clone());
        app.st.doc_v5open.recoveries.retain(|r| r.doc_id != doc_id || {
            host.out().obj_release(r.blob);
            false
        });
        app.st.doc_v5open.recoveries.push(Recovery { doc_id: doc_id.clone(), name, head, blob: f.blob, size: f.size });
        log(app, &doc_id, "recovery".into());
        return finish(app, host, true);
    }
    if check_page_count(f.opened.pages).is_err() {
        log(app, &f.file.id, "error:PDF page count exceeds the safe limit".into());
        return close(app, host, f);
    }
    mark(app);
    let (pages_req, digest_req) = (app.req(Mod::DocV5open), app.req(Mod::DocV5open));
    let out = host.out();
    pdf::pages(out, pages_req, f.pdf, 1, f.opened.pages);
    sfx::digest(out, digest_req, Src::blob(f.blob));
    app.st.doc_v5open.job = Some(Job { f, pages_req, digest_req, wait: 0, sizes: None, sha: None, facts: PdfFacts::default(), source: None, blob_kept: false, encoded: Vec::new(), skeletons: Vec::new() });
}

// v5: openDriveFile — `file.name.replace(/\.pdf$/i, '')`.
fn base_name(name: &str) -> &str {
    let n = name.len();
    if n >= 4 && name.is_char_boundary(n - 4) && name[n - 4..].eq_ignore_ascii_case(".pdf") {
        &name[..n - 4]
    } else {
        name
    }
}

/// The import actor and revision time of a revision (SYNC §15.2): both from the `files.get` of the open. A
/// download without a revision id (signed-out public link) is keyed by its bytes.
fn actor_and_time(f: &Foreign, sha: &Hash) -> (ActorId, u64) {
    let key = if f.meta.head.is_empty() { crate::store::sv::hex(sha) } else { f.meta.head.clone() };
    (ActorId::import(&key), iso_ms(&f.meta.modified).map_or(1, |t| t.max(1.0) as u64))
}

/// Both facts arrived: parse here or in the worker.
fn facts_ready(app: &mut App, host: &mut dyn Host) {
    let req = app.req(Mod::DocV5open);
    let Some(j) = job(app) else { return };
    let (true, Some(sha)) = (j.sizes.is_some(), j.sha) else { return };
    let sizes = j.sizes.take().unwrap_or_default();
    j.facts = PdfFacts {
        doc_id: j.f.file.id.clone(),
        name: base_name(&j.f.file.name).to_string(),
        page_sizes: sizes,
        revision_sha256: sha,
        clean_attachment: j.f.opened.attachments.iter().any(|a| a.0 == CLEAN_ORIGINAL),
        strict: true,
    };
    let (actor, time) = actor_and_time(&j.f, &sha);
    if j.f.kind == Kind::V5 && j.f.opened.keywords.len() > WORKER_KEYWORDS {
        let mut w = Writer::default();
        w.u8(worker_role::IMPORT);
        w.u8(OP_PARSE);
        j.wait = req;
        w.u32(req);
        put_facts(&mut w, &j.facts);
        w.u64(actor.0);
        w.u64(time);
        let id = j.f.file.id.clone();
        log(app, &id, "worker".into());
        start_worker(app, host);
        // The Keywords (megabytes) go straight into the out-buffer: no message buffer of their size.
        let (kw, out) = (&app.st.doc_v5open.job.as_ref().unwrap().f.opened.keywords, host.out());
        let m = out.w.rec(fam::FX, fx::WORKER_POST);
        out.w.u32(0);
        out.room(w.buf.len() + kw.len() + 8);
        out.w.u32((w.buf.len() + 4 + kw.len()) as u32);
        out.w.raw(&w.buf);
        out.w.str(kw);
        out.w.end(m);
        return;
    }
    import_here(app, host, actor, time);
}

/// Posts a message to the export worker, starting it once; shared with the export role (`export::pdf`, W5.4).
pub fn worker_send(app: &mut App, host: &mut dyn Host, obj: u32, bytes: &[u8]) { post(app, host, obj, bytes) }

fn start_worker(app: &mut App, host: &mut dyn Host) {
    if !std::mem::replace(&mut app.st.doc_v5open.worker, true) {
        host.out().fx0(fx::WORKER_START);
    }
}

fn post(app: &mut App, host: &mut dyn Host, obj: u32, bytes: &[u8]) {
    start_worker(app, host);
    worker_post(host.out(), obj, bytes);
}

fn worker_post(out: &mut OutBuf, obj: u32, bytes: &[u8]) {
    let m = out.w.rec(fam::FX, fx::WORKER_POST);
    out.w.u32(obj);
    out.room(bytes.len() + 4);
    out.w.bytes(bytes);
    out.w.end(m);
}

fn put_facts(w: &mut Writer, f: &PdfFacts) {
    w.str(&f.doc_id);
    w.str(&f.name);
    w.u32(f.page_sizes.len() as u32);
    f.page_sizes.iter().for_each(|&(a, b)| {
        w.u32(a);
        w.u32(b);
    });
    w.raw(&f.revision_sha256);
    w.u8(f.clean_attachment as u8 | (f.strict as u8) << 1);
}

fn get_facts(r: &mut Reader) -> Option<PdfFacts> {
    let (doc_id, name) = (r.str()?.into(), r.str()?.into());
    let page_sizes = (0..r.u32()?).map(|_| Some((r.u32()?, r.u32()?))).collect::<Option<_>>()?;
    let revision_sha256 = r.take(32)?.try_into().ok()?;
    let flags = r.u8()?;
    Some(PdfFacts { doc_id, name, page_sizes, revision_sha256, clean_attachment: flags & 1 != 0, strict: flags & 2 != 0 })
}

/// Worker answers to a parse, in order: one message per batch and per asset, then the end (or an error):
/// `[IMPORT][OP_PARSE][u32 req][u8 kind][…]`.
mod part {
    pub const ERROR: u8 = 0;
    pub const BATCH: u8 = 1;
    pub const ASSET: u8 = 2;
    pub const END: u8 = 3;
    /// An `ASSET` turned into a Blob by `host/export-worker.js` (`postAsset`): `[32 hash][str mime][f64 size]`,
    /// the message's object is the Blob.
    pub const ASSET_BLOB: u8 = 4;
}

fn part_header(req: u32, kind: u8) -> Writer {
    let mut w = Writer::default();
    w.u8(worker_role::IMPORT);
    w.u8(OP_PARSE);
    w.u32(req);
    w.u8(kind);
    w
}

/// Export-worker entry for this role (`ihn_worker`, first payload byte = `app::worker_role`); runs in the
/// worker's own instance of the module. One message per batch keeps the page's buffers at the size of one page.
pub fn worker(host: &mut dyn Host, msg: &[u8]) {
    let mut r = Reader::new(msg);
    if r.u8() != Some(OP_PARSE) {
        return;
    }
    let Some(req) = r.u32() else { return };
    let parsed = (|| Some((get_facts(&mut r)?, ActorId(r.u64()?), r.u64()?, r.str()?)))();
    let mut send = |w: Writer| {
        worker_post(host.out(), 0, &w.buf);
        host.flush();
    };
    let imp = match parsed.map(|(facts, actor, time, kw)| import::from_pdf(kw, &facts, actor, time)) {
        Some(Ok(mut i)) => {
            super::versions::add_v5(&mut i);
            i
        }
        e => {
            let mut w = part_header(req, part::ERROR);
            w.str(match e {
                Some(Err(e)) => e.message(),
                _ => "bad request",
            });
            return send(w);
        }
    };
    for b in &imp.batches {
        let mut w = part_header(req, part::BATCH);
        w.bytes(&encode_batch(b).0);
        send(w);
    }
    for a in &imp.assets {
        let mut w = part_header(req, part::ASSET);
        w.raw(&a.hash);
        w.str(&a.mime);
        w.bytes(&a.bytes);
        send(w);
    }
    let mut w = part_header(req, part::END);
    match imp.source {
        Some((s, h)) => {
            w.u8(s as u8 + 1);
            w.raw(&h);
        }
        None => w.u8(0),
    }
    w.u8(imp.v5 as u8);
    w.f64(memory());
    w.u32(imp.open_yields);
    send(w);
}

/// The ids `supersede` needs of a batch: its header, its pages and items, without bodies.
fn skeleton(b: &Batch) -> Batch {
    let ops = b
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::PageNew { .. } => Some(op.clone()),
            &Op::ItemPut { item, page, z, .. } => Some(Op::ItemPut { item, page, z, body: Body::Dropped }),
            _ => None,
        })
        .collect();
    Batch { ops, stamps: None, ..*b }
}

/// One import batch, kept encoded until the import is complete.
fn add_batch(app: &mut App, b: &Batch, encoded: EncodedBatch) {
    if let Some(j) = job(app) {
        j.skeletons.push(skeleton(b));
        j.encoded.push(encoded);
    }
}

fn add_asset(app: &mut App, host: &mut dyn Host, hash: Hash, mime: &str, bytes: &[u8]) {
    let obj = sfx::new_obj(app);
    sfx::blob_new(host.out(), obj, mime, bytes);
    session::put_asset(app, host, hash, obj, mime, bytes.len() as f64);
}

/// The import of the main instance (small Keywords, plain PDFs, a failed worker).
fn import_here(app: &mut App, host: &mut dyn Host, actor: ActorId, time: u64) {
    let Some(j) = job(app) else { return };
    match import::from_pdf(&j.f.opened.keywords, &j.facts, actor, time) {
        Ok(mut imp) => {
            super::versions::add_v5(&mut imp);
            for b in imp.batches {
                let e = encode_batch(&b);
                add_batch(app, &b, e);
            }
            imp.assets.iter().for_each(|a| add_asset(app, host, a.hash, &a.mime, &a.bytes));
            imported(app, host, Ok((imp.source, imp.v5, imp.open_yields)));
        }
        Err(e) => imported(app, host, Err(e.message().into())),
    }
}

/// The import is complete: `supersede` against the document as it was, then the batches, then the source asset.
fn imported(app: &mut App, host: &mut dyn Host, r: Result<(Option<(Source, Hash)>, bool, u32), String>) {
    let Some(mut j) = app.st.doc_v5open.job.take() else { return };
    let id = j.f.file.id.clone();
    let (source, v5, yields) = match r {
        Ok(r) => r,
        Err(e) => {
            log(app, &id, format!("error:{e}"));
            return close(app, host, j.f);
        }
    };
    // This installation's migration actor holds 5.x data no revision has seen: never deleted by one (SYNC §15.4).
    let sup = session::get(app).and_then(|s| {
        let keep: Vec<ActorId> = s.author.iter().map(|a| ActorId::import(&migration_source(a.actor.inst(), &s.doc_id))).collect();
        import::supersede(&s.doc, &j.skeletons, &keep)
    });
    let pages = j.skeletons.iter().flat_map(|b| &b.ops).filter(|o| matches!(o, Op::PageNew { .. })).count();
    j.skeletons = Vec::new();
    // One `receive` for the whole import: the session announces the changes once, so the editor builds its pages
    // once instead of once per batch (21 rebuilds of every page for a 20-page 5.x document).
    let mut batches = std::mem::take(&mut j.encoded);
    batches.extend(sup.as_ref().map(encode_batch));
    session::receive_owned(app, host, batches);
    mark(app);
    log(app, &id, format!("{}:{pages}", if v5 { "v5" } else { "plain" }));
    if app.st.doc_v5open.notify == 0 {
        open::set_import_yields(app, yields);
    }
    app.st.doc_v5open.job = Some(Job { source, ..j });
    store_source(app, host);
}

/// Stores the source PDF the pages show, by its variant; every failure falls back to the next variant.
fn store_source(app: &mut App, host: &mut dyn Host) {
    let req = app.req(Mod::DocV5open);
    let Some(j) = job(app) else { return };
    let Some((variant, hash)) = j.source else { return done(app, host) };
    if session::get(app).is_some_and(|s| s.asset_blob(&hash).is_some()) {
        return done(app, host);
    }
    let j = job(app).unwrap();
    j.wait = req;
    match variant {
        Source::Stripped => {
            let refs = import::overlay_refs(&j.f.opened.keywords);
            let mut w = Writer::default();
            w.u8(worker_role::IMPORT);
            w.u8(OP_STRIP);
            w.u32(req);
            w.u32(refs.len() as u32);
            for (page, r) in refs {
                w.u32(page as u32);
                match r {
                    OverlayRef::Object(n) => {
                        w.u8(0);
                        w.u32(n);
                    }
                    OverlayRef::Name(n) => {
                        w.u8(1);
                        w.str(n);
                    }
                }
            }
            let blob = j.f.blob;
            post(app, host, blob, &w.buf);
        }
        Source::Attachment => pdf::attachment(host.out(), req, j.f.pdf, CLEAN_ORIGINAL),
        Source::Saved => {
            let (blob, size) = (j.f.blob, j.f.size);
            j.blob_kept = true;
            let id = j.f.file.id.clone();
            log(app, &id, "source:Saved".into());
            session::put_asset(app, host, hash, blob, "application/pdf", size);
            done(app, host);
        }
    }
}

/// A derived source (stripped or attachment) arrived, or failed (`obj` 0).
fn source_blob(app: &mut App, host: &mut dyn Host, obj: u32, size: f64) {
    let Some(j) = job(app) else { return };
    j.wait = 0;
    let Some((variant, hash)) = j.source else { return };
    if obj != 0 {
        let id = j.f.file.id.clone();
        log(app, &id, format!("source:{variant:?}"));
        session::put_asset(app, host, hash, obj, "application/pdf", size);
        return done(app, host);
    }
    // v5: the recovery order falls back to the attachment, then to the saved bytes.
    let next = if variant == Source::Stripped && j.facts.clean_attachment { Source::Attachment } else { Source::Saved };
    j.source = Some((next, hash));
    store_source(app, host);
}

fn done(app: &mut App, host: &mut dyn Host) {
    if let Some(j) = app.st.doc_v5open.job.take() {
        close_with(host, &j.f, j.blob_kept);
        // The Keywords (megabytes) go before the open goes on to its compaction and the editor's pages.
        drop(j);
        mark(app);
        finish(app, host, true);
    }
}

fn finish(app: &mut App, host: &mut dyn Host, ok: bool) {
    match std::mem::take(&mut app.st.doc_v5open.notify) {
        0 => open::finish_foreign(app, host, ok),
        n => crate::shell::deliver(app, host, n, if ok { 0 } else { -1 }, 0, &[]),
    }
}

fn close_with(host: &mut dyn Host, f: &Foreign, blob_kept: bool) {
    let out = host.out();
    pdf::close(out, f.pdf);
    if !blob_kept {
        out.obj_release(f.blob);
    }
}

fn close(app: &mut App, host: &mut dyn Host, f: Foreign) {
    close_with(host, &f, false);
    finish(app, host, false);
}

/// Abandons a running import (a new open replaced it, or the document closed); the open is not told.
fn abandon(app: &mut App, host: &mut dyn Host) {
    if let Some(j) = app.st.doc_v5open.job.take() {
        close_with(host, &j.f, j.blob_kept);
    }
    app.st.doc_v5open.notify = 0;
}

fn worker_msg(app: &mut App, host: &mut dyn Host, obj: u32, data: &[u8]) {
    let mut r = Reader::new(data);
    let (Some(worker_role::IMPORT), Some(op), Some(req)) = (r.u8(), r.u8(), r.u32()) else { return };
    if job(app).is_none_or(|j| j.wait != req) {
        if obj != 0 {
            host.out().obj_release(obj);
        }
        return;
    }
    let kind = r.u8();
    match (op, kind) {
        (OP_PARSE, Some(part::BATCH)) => match r.bytes().map(|b| EncodedBatch(b.to_vec())) {
            Some(e) => match e.decode() {
                Ok(b) => add_batch(app, &b, e),
                Err(_) => imported(app, host, Err("bad worker answer".into())),
            },
            None => imported(app, host, Err("bad worker answer".into())),
        },
        (OP_PARSE, Some(part::ASSET)) => {
            // Written from the message into the new blob: no copy of its own (a 5.x version snapshot is megabytes).
            if let Some((hash, mime, bytes)) = (|| Some((r.take(32)?.try_into().ok()?, r.str()?, r.bytes()?)))() {
                add_asset(app, host, hash, mime, bytes);
            }
        }
        (OP_PARSE, Some(part::ASSET_BLOB)) => {
            match (|| Some((r.take(32)?.try_into().ok()?, r.str()?, r.f64()?)))() {
                Some((hash, mime, size)) if obj != 0 => session::put_asset(app, host, hash, obj, mime, size),
                _ => host.out().obj_release(obj),
            }
        }
        (OP_PARSE, Some(part::END)) => {
            let source = match r.u8() {
                Some(s @ 1..=3) => r.take(32).and_then(|h| h.try_into().ok()).map(|h| ([Source::Saved, Source::Stripped, Source::Attachment][s as usize - 1], h)),
                _ => None,
            };
            let v5 = r.u8() == Some(1);
            // A parse grows the worker's memory to several times the Keywords and wasm memory never shrinks: the
            // worker is terminated after it (ARCHITECTURE §9) unless an export is using it; the next job starts a
            // fresh one.
            let _worker_memory = r.f64();
            let yields = r.u32().unwrap_or(0);
            if !crate::export::pdf::busy(app) {
                stop_worker(app, host);
            }
            imported(app, host, Ok((source, v5, yields)));
        }
        (OP_PARSE, _) => imported(app, host, Err(r.str().unwrap_or("bad worker answer").to_string())),
        (OP_STRIP, ok) => {
            let size = r.f64().unwrap_or(0.0);
            if !crate::export::pdf::busy(app) {
                stop_worker(app, host);
            }
            source_blob(app, host, if ok == Some(1) { obj } else { 0 }, size);
        }
        _ => {}
    }
}

fn stop_worker(app: &mut App, host: &mut dyn Host) {
    if std::mem::take(&mut app.st.doc_v5open.worker) {
        host.out().fx0(fx::WORKER_STOP);
    }
}

/// The worker died: a parse runs here instead, a strip falls back to the next source.
fn worker_failed(app: &mut App, host: &mut dyn Host) {
    app.st.doc_v5open.worker = false;
    let Some(j) = job(app) else { return };
    if j.wait == 0 {
        return;
    }
    j.wait = 0;
    if j.source.is_some() {
        return source_blob(app, host, 0, 0.0);
    }
    let (actor, time) = actor_and_time(&j.f, &j.facts.revision_sha256);
    (j.skeletons, j.encoded) = (Vec::new(), Vec::new());
    import_here(app, host, actor, time);
}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Msg(m) if m.chan == sig::FOREIGN => {
            start(app, host);
            return ret::PREVENT_DEFAULT;
        }
        Event::Msg(m) if m.chan == chan::WORKER => worker_msg(app, host, m.obj, m.data),
        Event::Msg(m) if m.chan == chan::WORKER_ERROR => worker_failed(app, host),
        Event::Msg(m) if m.chan == sig::CLOSED => abandon(app, host),
        Event::Io(io) => {
            let Some(j) = job(app) else {
                if io.obj != 0 {
                    host.out().obj_release(io.obj);
                }
                return 0;
            };
            if io.req == j.pages_req {
                let mut r = Reader::new(io.data);
                let sizes = (0..j.f.opened.pages).map(|_| Some((round(r.f64()?) as u32, round(r.f64()?) as u32))).collect::<Option<Vec<_>>>();
                match sizes.filter(|_| io.status >= 0) {
                    Some(s) => {
                        j.sizes = Some(s);
                        facts_ready(app, host);
                    }
                    None => {
                        let f = app.st.doc_v5open.job.take().unwrap().f;
                        close(app, host, f);
                    }
                }
            } else if io.req == j.digest_req {
                match io.data.try_into().ok().filter(|_| io.status >= 0) {
                    Some(h) => {
                        j.sha = Some(h);
                        facts_ready(app, host);
                    }
                    None => {
                        let f = app.st.doc_v5open.job.take().unwrap().f;
                        close(app, host, f);
                    }
                }
            } else if io.req == j.wait {
                // The `IH_CLEAN_ORIGINAL.pdf` attachment.
                let size = Reader::new(io.data).f64().unwrap_or(0.0);
                source_blob(app, host, if io.status >= 0 { io.obj } else { 0 }, size);
            } else if io.obj != 0 {
                host.out().obj_release(io.obj);
            }
        }
        _ => {}
    }
    0
}

/// `__IHN_TEST_API__.cmd('doc.v5open.<name>', args)`: `state` → `{busy, worker, log: [[id, outcome]…],
/// recoveries: [[id, name, head]…]}`; `doc` → the open document in order: `{name, pages: [{id, w, h, template,
/// bg, asset, strokes: [id…], images, covers}]}` (ids as 32 hex digits); `legacy` (5.x ids) → their v6 ids.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, args: &str) -> Option<String> {
    let st = &app.st.doc_v5open;
    match name {
        "state" => {
            let log: Vec<String> = st.log.iter().map(|(a, b)| format!("[{},{}]", json_str(a), json_str(b))).collect();
            let rec: Vec<String> = st.recoveries.iter().map(|r| format!("[{},{},{}]", json_str(&r.doc_id), json_str(&r.name), json_str(&r.head))).collect();
            let mem: Vec<String> = st.mem.iter().map(f64::to_string).collect();
            Some(format!("{{\"busy\":{},\"worker\":{},\"log\":[{}],\"recoveries\":[{}],\"mem\":[{}]}}", st.job.is_some(), st.worker, log.join(","), rec.join(","), mem.join(",")))
        }
        "legacy" => {
            let ids = crate::shell::json::Json::parse(args)?;
            Some(format!("[{}]", ids.items().iter().filter_map(|i| i.as_str()).map(|l| format!("\"{}\"", Id::legacy(l).hex())).collect::<Vec<_>>().join(",")))
        }
        "doc" => {
            let s = session::get(app)?;
            let d = &s.doc;
            let pages: Vec<String> = d
                .order()
                .iter()
                .filter_map(|id| Some((id, d.pages.get(id).filter(|p| !p.deleted.v)?)))
                .map(|(id, p)| {
                    let list = d.draw_list(*id);
                    let strokes: Vec<String> = list.strokes.iter().map(|r| format!("\"{}\"", r.id.hex())).collect();
                    let (images, covers) = (list.images.len(), list.covers.len());
                    let bg = match p.background.v {
                        Background::Template => "template".to_string(),
                        Background::Pdf { page, .. } => format!("pdf:{page}"),
                        Background::Custom { .. } => "custom".into(),
                    };
                    let assets_ok = match p.background.v {
                        Background::Pdf { asset, .. } | Background::Custom { asset } => s.asset_blob(&asset).is_some(),
                        Background::Template => true,
                    };
                    format!(
                        "{{\"id\":\"{}\",\"w\":{},\"h\":{},\"template\":\"{:?}\",\"bg\":\"{bg}\",\"asset\":{assets_ok},\"strokes\":[{}],\"images\":{images},\"covers\":{covers}}}",
                        id.hex(),
                        p.size.v.0,
                        p.size.v.1,
                        p.template.v,
                        strokes.join(",")
                    )
                })
                .collect();
            let strokes = d.items.values().filter(|i| i.is_live() && matches!(i.body.v, Body::Stroke { .. })).count();
            Some(format!("{{\"name\":{},\"strokes\":{strokes},\"pages\":[{}]}}", json_str(&d.name.v), pages.join(",")))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parts a worker call posted: `(kind, rest of the payload)`.
    fn parts(req_args: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut host = crate::host::FakeHost::default();
        worker(&mut host, req_args);
        let buf = host.out().w.buf.clone();
        let (mut out, mut at) = (Vec::new(), 0);
        while at < buf.len() {
            assert_eq!((buf[at], buf[at + 1]), (fam::FX, fx::WORKER_POST));
            let size = u32::from_le_bytes(buf[at + 2..at + 6].try_into().unwrap()) as usize;
            let mut r = Reader::new(&buf[at + 10..at + 6 + size]);
            let mut b = Reader::new(r.bytes().unwrap());
            assert_eq!((b.u8(), b.u8(), b.u32()), (Some(worker_role::IMPORT), Some(OP_PARSE), Some(9)));
            out.push((b.u8().unwrap(), b.buf.to_vec()));
            at += 6 + size;
        }
        out
    }

    fn parse_request(facts: &PdfFacts, keywords: &str) -> Vec<u8> {
        let mut w = Writer::default();
        w.u8(OP_PARSE);
        w.u32(9);
        put_facts(&mut w, facts);
        w.u64(ActorId::import("r").0);
        w.u64(3);
        w.str(keywords);
        w.buf
    }

    #[test]
    fn facts_round_trip() {
        let facts = PdfFacts { doc_id: "d".into(), name: "N".into(), page_sizes: vec![(595, 842), (10, 20)], revision_sha256: [7; 32], clean_attachment: true, strict: true };
        let mut w = Writer::default();
        put_facts(&mut w, &facts);
        let back = get_facts(&mut Reader::new(&w.buf)).unwrap();
        assert_eq!((back.doc_id, back.name, back.page_sizes, back.revision_sha256, back.clean_attachment, back.strict), (facts.doc_id.clone(), facts.name.clone(), facts.page_sizes.clone(), facts.revision_sha256, true, true));
    }

    #[test]
    fn the_worker_streams_an_import_batch_by_batch() {
        let facts = PdfFacts { doc_id: "d".into(), name: "N".into(), page_sizes: vec![(595, 842), (10, 20)], revision_sha256: [7; 32], strict: true, ..PdfFacts::default() };
        let want = import::from_pdf("", &facts, ActorId::import("r"), 3).unwrap();
        let got = parts(&parse_request(&facts, ""));
        assert_eq!(got.len(), want.batches.len() + 1);
        for (b, (kind, rest)) in want.batches.iter().zip(&got) {
            assert_eq!(*kind, part::BATCH);
            assert_eq!(&EncodedBatch(Reader::new(rest).bytes().unwrap().to_vec()).decode().unwrap(), b);
        }
        let (kind, end) = got.last().unwrap();
        assert_eq!((*kind, end[0], &end[1..33], end[33]), (part::END, Source::Saved as u8 + 1, &want.source.unwrap().1[..], 0));
    }

    #[test]
    fn the_worker_reports_a_parse_error() {
        let facts = PdfFacts { page_sizes: vec![(100, 200)], strict: true, ..PdfFacts::default() };
        let got = parts(&parse_request(&facts, "STROKES_Z:%%%"));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, part::ERROR);
        assert!(!Reader::new(&got[0].1).str().unwrap().is_empty());
    }

    #[test]
    fn a_skeleton_keeps_ids_and_drops_bodies() {
        let facts = PdfFacts { page_sizes: vec![(100, 200)], ..PdfFacts::default() };
        let b = &import::from_pdf("", &facts, ActorId::import("r"), 3).unwrap().batches[0];
        let s = skeleton(b);
        assert_eq!((s.actor, s.seq, s.clock, s.ops.len()), (b.actor, b.seq, b.clock, 1));
    }

    #[test]
    fn names_lose_the_pdf_extension() {
        assert_eq!(base_name("Notes.PDF"), "Notes");
        assert_eq!(base_name("a.pdf.pdf"), "a.pdf");
        assert_eq!(base_name("pdf"), "pdf");
        assert_eq!(base_name("ñ.pdf"), "ñ");
    }
}
