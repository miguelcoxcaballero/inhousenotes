//! First create (reserved id) and the answers of the sync engine's export desk (ARCHITECTURE §8.3, SYNC §14).
//! Owned by W5.4.
//!
//! * **Create.** A document made on this device (`flags.created = false`) is created in Drive with its reserved id
//!   as soon as it can be: its first export, multipart POST into its folder (409 = an earlier create whose answer was
//!   lost). Then `flags.created`, the export descriptor, and `collab::created` (the engine starts polling).
//! * **Desk** ([`desk`], called by `collab` after every engine call): `build` → `export::pdf` → `built`; `merge` →
//!   the downloaded revision is classified by its Keywords (SYNC §15.1): a v6 export is merged with
//!   `load_snapshot` (+ its image attachments), a plain PDF over an existing document is saved first as the sibling
//!   `"<name> (recovered Drive version).pdf"`, then `merged`. The recovered copy of a foreign revision found at
//!   open (`doc::v5open::take_recovery`) is written before the next build.
//! * **Name** (§14.7): after an export, `name` is PATCHed when the `DocName` register is newer than the Drive name.

use super::pdf::{self, Built, Purpose};
use crate::abi::{Event, Io};
use crate::app::Mod;
use crate::doc::open::{self, pdf as pdfjs, Kind};
use crate::doc::{session, v5open};
use crate::drive::revisions;
use crate::shell::fx::{self as sfx, deflate_flag, Response, Src};
use crate::shell::json::Json;
use crate::store::schema::{self, Record};
use crate::store::sv::hex;
use crate::{auth, collab, shell, tag, App, Host};
use ihn_core::codec::{b64_decode, Reader, Writer};
use ihn_core::doc::{ExportPhase, PeerRef, SyncStatus};
use ihn_core::id::Hash;
use ihn_core::snapshot::EncodedSnapshot;
use ihn_core::stamp::{Stamp, VersionVector};
use ihn_sync::export::{push_name, recovered_name, vv_tag};
use ihn_sync::SyncRecord;

const T_RETRY: u32 = tag(Mod::ExportUpload, 1);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Building,
    Uploading(u32),
}

struct Create {
    doc: String,
    stage: Stage,
    vv: VersionVector,
}

/// A merge of a downloaded revision (desk `merge`).
struct Merge {
    gen: u32,
    job: u32,
    blob: u32,
    pdf_req: u32,
    pdf: u32,
    inflate_req: u32,
    vv: Option<VersionVector>,
    /// Image attachments still to read: `(hash, name)`, and the read in flight.
    attach: Vec<(Hash, String)>,
    attach_req: u32,
    attaching: Option<Hash>,
    copy_req: u32,
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    create: Option<Create>,
    fails: u32,
    retry_at: f64,
    /// `(engine gen, desk job)` of the build / merge already started.
    built_job: (u32, u32),
    merge_job: (u32, u32),
    merge: Option<Merge>,
    /// A recovered copy written before the build `(gen, job)`: its request.
    recovery: Option<(u32, String)>,
    /// Foreign heads already saved as a recovered copy: a merge of one of them only answers.
    preserved: Vec<String>,
    /// The newest `desk.done` handled (name push).
    done: Option<String>,
    rename_req: u32,
    /// Test: what happened, in order (`create:<status>`, `copy:<status>`, `merge:<kind>`, `name`).
    log: Vec<String>,
}

fn gen(app: &App) -> u32 { app.st.collab.run.as_ref().map_or(0, |r| r.gen) }
fn vv_bytes(vv: &VersionVector) -> Vec<u8> {
    let mut w = Writer::default();
    vv.encode(&mut w);
    w.buf
}

/// A creation or the recovered copy is running (the indicator shows it as an export phase).
pub fn overlay(app: &App, status: &mut SyncStatus) {
    let st = &app.st.export_upload;
    if st.create.as_ref().is_some_and(|c| c.doc == session::doc_id(app)) && matches!(status.export, ExportPhase::Idle | ExportPhase::Failed { .. }) {
        status.export = match st.create.as_ref().map(|c| c.stage) {
            Some(Stage::Building) => ExportPhase::Building,
            _ => ExportPhase::Uploading,
        };
    }
    if st.recovery.is_some() || st.merge.as_ref().is_some_and(|m| m.copy_req != 0) {
        status.export = ExportPhase::Preserving;
    }
}

/// A document created on this device is waiting for its Drive file.
pub fn creating(app: &App) -> bool { app.st.export_upload.create.is_some() }

/// Writes a sync record to the session (and the engine's mirror when it runs for that document).
fn record(app: &mut App, host: &mut dyn Host, rec: SyncRecord) {
    let r = match &rec {
        SyncRecord::KnownExport { head, vv } => Record::KnownExport { head: head.clone(), vv: vv_bytes(vv) },
        SyncRecord::LastExport { head, vv, etag, at } => Record::LastExport(schema::LastExport { head: head.clone(), vv: vv_bytes(vv), etag: etag.clone(), at: *at as f64 }),
        SyncRecord::Flags { relay_pending, export_pending, created } => Record::Flags { relay_pending: *relay_pending, export_pending: *export_pending, created: *created },
        SyncRecord::DriveName { v, observed_at: s } => Record::DriveName(schema::DriveName { v: v.clone(), observed_at: schema::Stamp { clock: s.clock, actor: s.actor.0, idx: s.idx } }),
        _ => return,
    };
    session::persist(app, host, r);
    collab::with_sync(app, |s| s.core.stored.apply(&rec));
}

// ---- create -------------------------------------------------------------------------------------------------------

/// Starts the first create of the open document when it is due and possible.
pub fn maybe_create(app: &mut App, host: &mut dyn Host) {
    if app.st.export_upload.create.is_some() || host.now() < app.st.export_upload.retry_at || shell::offline(app) || auth::token(app).is_none() {
        return;
    }
    let Some(s) = session::get(app) else { return };
    if s.record.flags.created || !s.persistent || !s.writable() || s.account.is_empty() {
        return;
    }
    let doc = s.doc_id.clone();
    app.st.export_upload.create = Some(Create { doc, stage: Stage::Building, vv: VersionVector::default() });
    pdf::start(app, host, Purpose::Create);
    collab::kick(app, host);
}

fn create_failed(app: &mut App, host: &mut dyn Host) {
    let st = &mut app.st.export_upload;
    st.create = None;
    st.fails += 1;
    let delay = (2_000.0 * (1u32 << st.fails.clamp(1, 8) - 1) as f64).min(300_000.0);
    st.retry_at = host.now() + delay;
    app.sched.timer(host.out(), T_RETRY, delay);
    collab::kick(app, host);
}

fn create_built(app: &mut App, host: &mut dyn Host, out: Built) {
    let Some(c) = app.st.export_upload.create.as_ref() else { return };
    let doc = c.doc.clone();
    let (Some((bytes, vv)), Some(s)) = (out, session::get(app).filter(|s| s.doc_id == doc)) else { return create_failed(app, host) };
    let mut name = s.record.name.clone();
    if !name.to_ascii_lowercase().ends_with(".pdf") {
        name.push_str(".pdf");
    }
    let parents: Vec<String> = s.record.drive_meta.get("parents").items().iter().filter_map(|p| p.as_str().map(String::from)).collect();
    let req = revisions::create(app, host, Mod::ExportUpload, &doc, &name, &parents, &vv_tag(&vv), &bytes);
    if let Some(c) = app.st.export_upload.create.as_mut() {
        (c.stage, c.vv) = (Stage::Uploading(req), vv);
    }
    collab::kick(app, host);
}

fn create_answer(app: &mut App, host: &mut dyn Host, io: &Io) {
    let Some(c) = app.st.export_upload.create.take() else { return };
    let r = Response::parse(io.data);
    app.st.export_upload.log.push(format!("create:{}", io.status));
    if !(io.status >= 200 && io.status < 300 || io.status == 409) {
        app.st.export_upload.create = Some(c);
        return create_failed(app, host);
    }
    app.st.export_upload.fails = 0;
    let head = Json::parse(r.text()).map_or(String::new(), |j| j.text("headRevisionId").to_string());
    if session::doc_id(app) != c.doc {
        return; // the next session of that document sees `created = false` and creates again (409 → created)
    }
    record(app, host, SyncRecord::Flags { relay_pending: None, export_pending: None, created: Some(true) });
    if !head.is_empty() {
        let at = host.time() as u64;
        record(app, host, SyncRecord::KnownExport { head: head.clone(), vv: c.vv.clone() });
        record(app, host, SyncRecord::LastExport { head, vv: c.vv, etag: String::new(), at });
    }
    collab::created(app, host);
    super::lifecycle::note_home(app, host);
    shell::signal(app, host, session::sig::STATE, "");
}

// ---- the desk -----------------------------------------------------------------------------------------------------

/// A build finished (`export::pdf`).
pub fn built(app: &mut App, host: &mut dyn Host, purpose: Purpose, out: Built) {
    match purpose {
        Purpose::Create => create_built(app, host, out),
        Purpose::Desk(job) => {
            if collab::with_sync(app, |s| s.core.export.build == Some(job) && {
                s.core.export.built = Some((job, out));
                true
            }) == Some(true)
            {
                collab::kick(app, host);
            }
        }
        Purpose::User => {}
    }
}

/// Looks at the engine's desk after every engine call and answers what is new.
pub fn desk(app: &mut App, host: &mut dyn Host) {
    let g = gen(app);
    let Some((build, merge, done)) = collab::with_sync(app, |s| (s.core.export.build, s.core.export.merge.as_ref().map(|m| m.0), s.core.export.done.clone())) else { return };
    // A foreign revision found at open (`doc::v5open`): its copy is written before anything else.
    if app.st.export_upload.recovery.is_none() {
        let doc = session::doc_id(app).to_string();
        if let Some(rec) = v5open::take_recovery(app, &doc) {
            let parents = parents(app);
            let req = revisions::recovered_copy(app, host, Mod::ExportUpload, &rec.name, &parents, rec.blob);
            host.out().obj_release(rec.blob);
            app.st.export_upload.recovery = Some((req, rec.head));
        }
    }
    if app.st.export_upload.recovery.is_some() {
        return; // the build and any merge wait for the copy
    }
    if let Some(job) = build.filter(|j| app.st.export_upload.built_job != (g, *j)) {
        app.st.export_upload.built_job = (g, job);
        pdf::start(app, host, Purpose::Desk(job));
    }
    if let Some(job) = merge.filter(|j| app.st.export_upload.merge_job != (g, *j)) {
        app.st.export_upload.merge_job = (g, job);
        let head = collab::with_sync(app, |s| s.core.export.merge.as_ref().map(|m| m.1.clone())).flatten().unwrap_or_default();
        if app.st.export_upload.preserved.contains(&head) {
            collab::with_sync(app, |s| s.core.export.merged = Some((job, Some(VersionVector::default()))));
            collab::kick(app, host);
        } else {
            start_merge(app, host, g, job);
        }
    }
    if done.is_some() && done != app.st.export_upload.done {
        app.st.export_upload.done = done;
        push_name_now(app, host);
    }
    super::user::desk(app, host);
}

fn parents(app: &App) -> Vec<String> {
    session::get(app).map_or(Vec::new(), |s| s.record.drive_meta.get("parents").items().iter().filter_map(|p| p.as_str().map(String::from)).collect())
}

/// §14.7 after an export: the register is newer than the Drive name we last observed or pushed.
fn push_name_now(app: &mut App, host: &mut dyn Host) {
    let Some(s) = session::get(app).filter(|s| s.writable()) else { return };
    let name = format!("{}.pdf", s.doc.name.v);
    let at = s.doc.name.at;
    let drive = s.record.drive_name.as_ref().map(|n| (n.v.clone(), Stamp { clock: n.observed_at.clock, actor: ihn_core::id::ActorId(n.observed_at.actor), idx: n.observed_at.idx }));
    if s.doc.name.v.is_empty() || !push_name(&name, at, &drive) || app.st.export_upload.rename_req != 0 {
        return;
    }
    let doc = s.doc_id.clone();
    let actor = s.author.as_ref().map_or(ihn_core::id::ActorId(0), |a| a.actor);
    app.st.export_upload.rename_req = revisions::rename(app, host, Mod::ExportUpload, &doc, &name);
    app.st.export_upload.log.push("name".into());
    // Recorded as pushed now, so the next poll does not take our own name for an outside rename.
    let observed_at = Stamp::new(host.time() as u64, actor, 0);
    record(app, host, SyncRecord::DriveName { v: name.clone(), observed_at });
    if let Some(s) = session::get_mut(app) {
        s.record.name = name;
    }
}

fn start_merge(app: &mut App, host: &mut dyn Host, gen: u32, job: u32) {
    let Some(bytes) = collab::with_sync(app, |s| s.core.export.merge.as_ref().map(|m| m.3.clone())).flatten() else { return };
    if let Some(old) = app.st.export_upload.merge.take() {
        end_merge(host, old);
    }
    let blob = sfx::new_obj(app);
    sfx::blob_new(host.out(), blob, "application/pdf", &bytes);
    let req = app.req(Mod::ExportUpload);
    pdfjs::open(host.out(), req, blob);
    app.st.export_upload.merge = Some(Merge { gen, job, blob, pdf_req: req, pdf: 0, inflate_req: 0, vv: None, attach: Vec::new(), attach_req: 0, attaching: None, copy_req: 0 });
}

fn end_merge(host: &mut dyn Host, m: Merge) {
    if m.pdf != 0 {
        pdfjs::close(host.out(), m.pdf);
    }
    host.out().obj_release(m.blob);
}

/// The merge ended: answer the desk (`None` = could not be read).
fn merged(app: &mut App, host: &mut dyn Host, vv: Option<VersionVector>) {
    let Some(m) = app.st.export_upload.merge.take() else { return };
    let (g, job) = (m.gen, m.job);
    end_merge(host, m);
    if gen(app) != g {
        return;
    }
    collab::with_sync(app, |s| {
        if s.core.export.merge.as_ref().is_some_and(|x| x.0 == job) {
            s.core.export.merged = Some((job, vv));
        }
    });
    collab::kick(app, host);
}

fn merge_io(app: &mut App, host: &mut dyn Host, io: &Io) -> bool {
    let Some(m) = app.st.export_upload.merge.as_mut() else { return false };
    if io.req == m.pdf_req {
        m.pdf_req = 0;
        m.pdf = io.obj;
        let blob = m.blob;
        let Some(opened) = (io.status >= 0).then(|| pdfjs::Opened::parse(io.data)).flatten() else {
            merged(app, host, None);
            return true;
        };
        let (kind, payload) = open::classify(&opened.keywords);
        app.st.export_upload.log.push(format!("merge:{kind:?}"));
        match kind {
            Kind::V6 => {
                let Some(bytes) = b64_decode(payload.as_bytes()) else {
                    merged(app, host, None);
                    return true;
                };
                let names: Vec<String> = opened.attachments.iter().map(|a| a.0.clone()).collect();
                let req = app.req(Mod::ExportUpload);
                let m = app.st.export_upload.merge.as_mut().unwrap();
                m.inflate_req = req;
                m.attach = names.into_iter().map(|n| ([0; 32], n)).collect();
                sfx::deflate(host.out(), req, deflate_flag::INFLATE | deflate_flag::ZLIB, Src::Bytes(&bytes));
            }
            Kind::Plain if session::get(app).is_some_and(|s| !s.doc.order().is_empty()) => {
                // SYNC §15.1: keep the v6 state; save the foreign revision next to the document first.
                let name = recovered_name(&session::get(app).map_or(String::new(), |s| s.record.name.clone()));
                let parents = parents(app);
                let req = revisions::recovered_copy(app, host, Mod::ExportUpload, &name, &parents, blob);
                app.st.export_upload.merge.as_mut().unwrap().copy_req = req;
                collab::kick(app, host);
            }
            // A 5.x revision written during this session is merged by the next open of the document
            // (`doc::v5open`; NOTES-W5.4 deviation): the export retries with backoff meanwhile.
            _ => merged(app, host, None),
        }
    } else if io.req == m.inflate_req {
        m.inflate_req = 0;
        let snap = EncodedSnapshot(io.data.to_vec());
        if io.status < 0 || snap.decode().is_err() {
            merged(app, host, None);
            return true;
        }
        let vv = snap.version_vector();
        let missing = session::with_replica(app, host, |r| {
            r.load_snapshot(&snap, PeerRef::Export);
            r.missing_assets()
        })
        .unwrap_or_default();
        let m = app.st.export_upload.merge.as_mut().unwrap();
        let names = std::mem::take(&mut m.attach);
        m.attach = missing.into_iter().filter_map(|h| Some((h, names.iter().find(|n| n.1.strip_prefix("IHN6-") == Some(&hex(&h)[..]))?.1.clone()))).collect();
        m.vv = vv.or(Some(VersionVector::default()));
        next_attachment(app, host);
    } else if io.req == m.attach_req {
        m.attach_req = 0;
        if let (true, Some(hash)) = (io.status >= 0 && io.obj != 0, m.attaching.take()) {
            let size = Reader::new(io.data).f64().unwrap_or(-1.0);
            session::put_asset(app, host, hash, io.obj, "", size);
        }
        next_attachment(app, host);
    } else if io.req == m.copy_req {
        m.copy_req = 0;
        revisions::release_body(app, host, io.req);
        let ok = (200..300).contains(&io.status);
        app.st.export_upload.log.push(format!("copy:{}", io.status));
        if let (true, Some(head)) = (ok, collab::with_sync(app, |s| s.core.export.merge.as_ref().map(|m| m.1.clone())).flatten()) {
            app.st.export_upload.preserved.push(head);
        }
        merged(app, host, ok.then(VersionVector::default));
    } else {
        return false;
    }
    true
}

fn next_attachment(app: &mut App, host: &mut dyn Host) {
    let Some(m) = app.st.export_upload.merge.as_mut() else { return };
    let Some((hash, name)) = m.attach.pop() else {
        let vv = m.vv.take();
        return merged(app, host, vv);
    };
    m.attaching = Some(hash);
    let pdf = m.pdf;
    let req = app.req(Mod::ExportUpload);
    app.st.export_upload.merge.as_mut().unwrap().attach_req = req;
    pdfjs::attachment(host.out(), req, pdf, &name);
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(_app: &mut App, _host: &mut dyn Host) {}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Io(io) => {
            let st = &app.st.export_upload;
            if st.create.as_ref().is_some_and(|c| c.stage == Stage::Uploading(io.req)) {
                create_answer(app, host, &io);
            } else if st.recovery.as_ref().is_some_and(|r| r.0 == io.req) {
                let (_, head) = app.st.export_upload.recovery.take().unwrap();
                revisions::release_body(app, host, io.req);
                app.st.export_upload.log.push(format!("copy:{}", io.status));
                if (200..300).contains(&io.status) {
                    app.st.export_upload.preserved.push(head);
                }
                collab::kick(app, host);
            } else if io.req == st.rename_req {
                app.st.export_upload.rename_req = 0;
            } else if !merge_io(app, host, &io) {
                host.out().obj_release(io.obj);
            }
        }
        Event::Timer(T_RETRY) => maybe_create(app, host),
        Event::Msg(m) => match m.chan {
            session::sig::OPENED | session::sig::COMMITTED => maybe_create(app, host),
            shell::sig::NETWORK if m.data == b"1" => {
                app.st.export_upload.retry_at = 0.0;
                maybe_create(app, host);
            }
            session::sig::CLOSED => {
                if let Some(m) = app.st.export_upload.merge.take() {
                    end_merge(host, m);
                }
            }
            _ => {}
        },
        Event::Life(l) if l.code == crate::abi::life::ONLINE => {
            app.st.export_upload.retry_at = 0.0;
            maybe_create(app, host);
        }
        _ => {}
    }
    0
}

/// `__IHN_TEST_API__.cmd('export.upload.<name>', args)`: `state` → `{creating, log}`.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    let st = &app.st.export_upload;
    match name {
        "state" => {
            let log: Vec<String> = st.log.iter().map(|l| crate::testapi::json_str(l)).collect();
            Some(format!("{{\"creating\":{},\"log\":[{}]}}", st.create.is_some(), log.join(",")))
        }
        _ => None,
    }
}
