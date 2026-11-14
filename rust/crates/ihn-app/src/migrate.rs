//! One-time migration of 5.x local data (ARCHITECTURE §10, SYNC §15.4). Owned by W5.6.
//!
//! Once per account, behind Web Lock `ihn6-migrate:<account>`, in the foreground only:
//!
//! 1. **Dump**: the fingerprints of the four 5.x databases (`store::v5::info`), then the stores that hold user data,
//!    read-only, and the 5.x localStorage keys. The settings and the Drive entries become `kv`/`catalog` records
//!    (once), the recoveries an asset each plus the `kv.recoveries` list.
//! 2. **Plan** ([`plan`]): every document with unsynced data (a dirty offline row, an outbox row, a `local-`
//!    document, the working copy of the last open document) with its sources (`V5LocalDoc`s), and the pending local
//!    folders.
//! 3. **Write**, one document at a time through the document session (nothing else may hold it): the Drive base —
//!    the cached PDF when it is still the head, else a download, offline the cached PDF or nothing — imported with
//!    `doc::v5open::import`; then `v5::import::from_local` for each source over the result, its batches of the
//!    migration actor received into the session (journalled), and the `docs` record written with `seq[migration
//!    actor]`, `relay_pending` and `flags.migrated`. A `local-` document takes a pool id first (recorded in
//!    `kv.migration` before anything is written for it) and stays `created = false`. Pending folders are created in
//!    Drive first. Progress and the dump's fingerprints are stored in `kv.migration`; all writes are id-addressed,
//!    so a crash resumes idempotently (a second `from_local` pass emits nothing).
//! 4. **Seal**: `export::headless` relays, creates and exports the documents. A document is done when its record says
//!    `created` and `relay.sealed[migration actor] >= seq[migration actor]`.
//! 5. **Delete**, only when every document is done, a fresh dump has the migrated fingerprints (otherwise step 1
//!    runs again on the difference) and the service worker reports that every open window is v6: the 5.x databases,
//!    the `inhouse-media-*` caches and the read localStorage keys. A blocked deletion is retried at the next boot.

use crate::abi::{life, Event, Io};
use crate::app::Mod;
use crate::doc::open::{self, classify, pdf, File, Foreign, Kind, Meta};
use crate::doc::{session, v5open};
use crate::drive::client::{self, Body, Req};
use crate::host::LsOp;
use crate::shell::fx::{self, deflate_flag, Response, Src};
use crate::shell::json::Json;
use crate::shell::View;
use crate::store::schema::{kv, DocRecord};
use crate::store::sv::{hex, unhex16, Value};
use crate::store::v5::{self as dumper, Fingerprint, Info, Page, BACKGROUND_NOTEBOOK_DB, DATABASES, NOTEBOOK_DB, OFFLINE_DB, ORIGINALS_DB};
use crate::store::{self, locks, status, Reply, Tx as StoreTx};
use crate::testapi::json_str;
use crate::{auth, export, shell, tag, App, Host};
use ihn_core::codec::b64_decode;
use ihn_core::doc::PeerRef;
use ihn_core::id::{migration_source, ActorId, Hash};
use ihn_core::snapshot::{encode_batch, EncodedSnapshot};
use ihn_core::v5::idb::{self, BaseMeta};
use ihn_core::v5::import::{from_local, source_hash, Source, V5LocalDoc};
use ihn_core::v5::json::Value as J;

const T_START: u32 = tag(Mod::Migrate, 1);
const T_RETRY: u32 = tag(Mod::Migrate, 2);
/// Effects of `host/main.js`: the service worker's window census and the deletion of Cache Storage caches.
const FX_SW_CLIENTS: u8 = 0x73;
const FX_CACHES_DELETE: u8 = 0x74;
/// Retry period while documents are not sealed yet or a 5.x window is open.
const RETRY_MS: f64 = 4_000.0;
/// The runner is started again at most this often while documents are not done.
const RUNNER_EVERY_MS: f64 = 20_000.0;
const MEDIA_CACHES: &str = "inhouse-media-";
const META_FIELDS: &str = "capabilities(canEdit),ownedByMe,owners(displayName,emailAddress,photoLink),resourceKey,headRevisionId,modifiedTime,parents";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

/// Stores read from each 5.x database (ARCHITECTURE §10.1). Leases, the fast-open cache, ink journals and live
/// state are not migrated (§10.3); timeline rows are not converted (see NOTES-W5.6).
const READS: [(&str, &str); 11] = [
    (OFFLINE_DB, "documents"),
    (OFFLINE_DB, "outbox"),
    (OFFLINE_DB, "entries"),
    (OFFLINE_DB, "settings"),
    (OFFLINE_DB, "recoveries"),
    (NOTEBOOK_DB, "snapshots"),
    (NOTEBOOK_DB, "pages"),
    (NOTEBOOK_DB, "stroke-ops"),
    (BACKGROUND_NOTEBOOK_DB, "snapshots"),
    (BACKGROUND_NOTEBOOK_DB, "pages"),
    (BACKGROUND_NOTEBOOK_DB, "stroke-ops"),
];
/// 5.x localStorage keys: read by the migration, removed with the databases (ARCHITECTURE §7.2).
pub const LS_KEYS: [&str; 7] = ["drive-session-v1", "drive-pending-exit-upload-v1", "notebook-data-v3", "background-notebook-data-v3", "notebook-view-mode-v1", "presence-client-id-v1", "background-notebook-data-v3-session"];

// ---- the dump and the plan (pure) -----------------------------------------------------------------------------

/// What was read from 5.x: records per `(database, store)` as `v5::json` values, and the localStorage keys.
#[derive(Default)]
pub struct Dump {
    pub stores: Vec<(String, String, Vec<J>)>,
    pub ls: Vec<(String, String)>,
}

impl Dump {
    pub fn store(&self, db: &str, store: &str) -> &[J] { self.stores.iter().find(|s| s.0 == db && s.1 == store).map_or(&[], |s| &s.2[..]) }
    pub fn ls(&self, key: &str) -> Option<&str> { self.ls.iter().find(|l| l.0 == key).map(|l| &l.1[..]) }
}

/// One document to migrate.
#[derive(Clone, Debug, Default)]
pub struct DocPlan {
    /// Drive id or `local-<uuid>`.
    pub v5_id: String,
    pub local: bool,
    /// The Drive file name (`<name>.pdf`).
    pub file_name: String,
    pub parents: Vec<String>,
    /// `docs.driveMeta` from the 5.x details (replaced by a fresh `files.get` when online).
    pub details: Meta,
    /// The Drive revision the local edits were based on.
    pub base: BaseMeta,
    /// The cached PDF of that revision, and the clean original (`originalBlob`).
    pub blob: Option<(u32, f64)>,
    pub original: Option<(u32, f64)>,
    pub sources: Vec<V5LocalDoc>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FolderPlan {
    pub local_id: String,
    pub name: String,
    pub parents: Vec<String>,
    pub color: String,
}

#[derive(Default)]
pub struct Plan {
    pub docs: Vec<DocPlan>,
    pub folders: Vec<FolderPlan>,
}

fn blob_of(v: &J) -> Option<(u32, f64)> {
    match v {
        J::Blob { obj, size } if *size > 0.0 => Some((*obj, *size)),
        _ => None,
    }
}

fn strings(v: &J) -> Vec<String> { v.arr().iter().map(J::text).filter(|s| !s.is_empty()).collect() }

/// The working copy of one notebook database (`snapshots/latest` or its localStorage backup, `pages`, `stroke-ops`).
fn working(dump: &Dump, db: &str, backup: &str, doc_id: &str, name: &str) -> Option<V5LocalDoc> {
    let meta = match dump.store(db, "snapshots").first() {
        Some(r) => idb::meta_payload(r).ok()??,
        None => idb::meta_payload(&J::Str(dump.ls(backup)?.into())).ok()??,
    };
    let rows = dump.store(db, "pages").iter().map(|r| idb::page_row(r, meta.session_id).ok().flatten()).collect();
    let ops = idb::stroke_ops(dump.store(db, "stroke-ops"), meta.session_id);
    Some(V5LocalDoc::from_working(doc_id, &meta, idb::working_copy(&meta, rows, &ops), name))
}

/// The documents and folders of `account` that hold data 5.x has not synced (ARCHITECTURE §10.1).
pub fn plan(account: &str, dump: &Dump) -> Plan {
    let mine = |a: &str| a.is_empty() || a == account;
    let session = dump.ls("drive-session-v1").and_then(|t| ihn_core::v5::json::parse(t.as_bytes())).unwrap_or_default();
    let exit = dump.ls("drive-pending-exit-upload-v1").and_then(|t| ihn_core::v5::json::parse(t.as_bytes())).unwrap_or_default();
    let outbox = idb::outbox(dump.store(OFFLINE_DB, "outbox"));
    let raw = dump.store(OFFLINE_DB, "documents");
    let mut out = Plan::default();
    let add = |out: &mut Plan, id: &str| -> usize {
        if let Some(i) = out.docs.iter().position(|d| d.v5_id == id) {
            return i;
        }
        out.docs.push(DocPlan { v5_id: id.into(), local: id.starts_with("local-"), details: Meta { can_edit: true, owned_by_me: true, ..Meta::default() }, ..DocPlan::default() });
        out.docs.len() - 1
    };
    for r in raw {
        let Ok(Some(o)) = idb::offline_document(r) else { continue };
        if !mine(&o.account) || !(o.dirty || o.is_local() || o.offline_edited || outbox.iter().any(|x| !x.folder && x.id == o.id && mine(&x.account))) {
            continue;
        }
        let i = add(&mut out, &o.id);
        let d = &mut out.docs[i];
        let details = r.get("details");
        d.details = Meta {
            can_edit: o.can_edit,
            owned_by_me: !matches!(details.get("ownedByMe"), J::Bool(false)),
            owner_name: details.get("ownerName").text(),
            owner_email: details.get("ownerEmail").text(),
            owner_photo: details.get("ownerPhoto").text(),
            resource_key: details.get("resourceKey").text(),
            head: o.base.head_revision_id.clone(),
            modified: r.get("baseMeta").get("modifiedTime").text(),
            parents: o.parent_ids.clone(),
            size: 0.0,
        };
        (d.file_name, d.parents, d.base, d.blob, d.original) = (o.file_name.clone(), o.parent_ids.clone(), o.base.clone(), o.blob, o.original_blob);
        if let Some(s) = &o.snapshot {
            if d.file_name.is_empty() && !s.export_name.is_empty() {
                d.file_name = format!("{}.pdf", s.export_name);
            }
            d.sources.push(V5LocalDoc::from_snapshot(&o.id, s, o.updated_at, o.base.modified_ms, None));
        }
    }
    // The working copy belongs to the document of the stored editor session (or the pending exit upload).
    let current = [session.get("offlineLocalId"), session.get("fileId"), exit.get("fileId")].into_iter().map(J::text).find(|s| !s.is_empty());
    // (It has no account of its own: it is another account's when that account's offline row holds the document.)
    let foreign = |id: &str| raw.iter().any(|r| r.get("id").text() == id && !mine(&r.get("account").text()));
    if let Some(id) = current.filter(|id| !foreign(id)) {
        let name = session.get("exportName").text();
        let sources: Vec<V5LocalDoc> = [(NOTEBOOK_DB, "notebook-data-v3"), (BACKGROUND_NOTEBOOK_DB, "background-notebook-data-v3")].iter().filter_map(|(db, backup)| working(dump, db, backup, &id, &name)).collect();
        if !sources.is_empty() {
            let i = add(&mut out, &id);
            let d = &mut out.docs[i];
            // The working copy first: the offline snapshot, merged after it, only adds what it lacks.
            d.sources.splice(0..0, sources);
            if d.file_name.is_empty() {
                d.file_name = session.get("fileName").text();
            }
            if d.parents.is_empty() {
                d.parents = strings(session.get("parentIds"));
                d.details.parents = d.parents.clone();
            }
            if d.details.resource_key.is_empty() {
                d.details.resource_key = session.get("resourceKey").text();
            }
        }
    }
    for d in &mut out.docs {
        if d.file_name.is_empty() {
            d.file_name = "Untitled.pdf".into();
        }
        if d.original.is_none() {
            let row = dump.store(ORIGINALS_DB, "originals").iter().find(|r| [r.get("fileId"), r.get("id")].iter().any(|k| k.text() == d.v5_id));
            d.original = row.and_then(|r| blob_of(r.get("blob")));
        }
    }
    out.docs.retain(|d| !d.sources.is_empty() || d.local);
    for op in outbox.iter().filter(|o| o.folder && mine(&o.account)) {
        let Some(e) = dump.store(OFFLINE_DB, "entries").iter().find(|e| e.get("id").text() == op.id) else { continue };
        out.folders.push(FolderPlan { local_id: op.id.clone(), name: e.get("name").text(), parents: strings(e.get("parents")), color: e.get("folderColorRgb").text() });
    }
    out
}

/// A migrated document is done: created in Drive and every batch of its migration actor sealed in the relay.
pub fn doc_done(r: &DocRecord, migration: ActorId) -> bool {
    let seq = r.seq.get(migration.0).copied().unwrap_or(0);
    r.flags.created && r.relay.sealed.get(migration.0).copied().unwrap_or(0) >= seq
}

/// `store::sv` → `v5::json` (the input of `ihn-core::v5::idb`).
pub fn to_json(v: &Value) -> J {
    match v {
        Value::Undef | Value::Map(_) | Value::Set(_) | Value::Big(_) => J::Undef,
        Value::Null => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Num(n) | Value::Date(n) => J::Num(*n),
        Value::Str(s) => J::Str(s.as_str().into()),
        Value::Bytes(b) | Value::Typed(_, b) => J::Bytes(b.clone()),
        Value::Arr(a) => J::Arr(a.iter().map(to_json).collect()),
        Value::Obj(f) => J::Obj(f.iter().map(|(k, v)| (k.as_str().into(), to_json(v))).collect()),
        Value::Blob { obj, size, .. } => J::Blob { obj: *obj, size: *size },
    }
}

// ---- progress (`kv.migration`) ----------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    /// Fingerprints of the dump whose documents are all written (`None` = database missing).
    pub fp: Vec<(String, Option<Fingerprint>)>,
    /// `(5.x id, v6 id)` of every document written or being written.
    pub docs: Vec<(String, String)>,
    /// `(local folder id, Drive id)`.
    pub folders: Vec<(String, String)>,
    pub written: bool,
    pub catalog: bool,
    pub deleted: bool,
}

fn pairs(v: &[(String, String)]) -> Value { Value::Arr(v.iter().map(|(a, b)| Value::Arr(vec![a.as_str().into(), b.as_str().into()])).collect()) }
fn unpairs(v: &Value) -> Vec<(String, String)> { v.items().iter().map(|p| (p.items().first().map_or(String::new(), Value::text), p.items().get(1).map_or(String::new(), Value::text))).collect() }

impl Progress {
    pub fn to_value(&self) -> Value {
        let fp = Value::Arr(self.fp.iter().map(|(db, f)| Value::Arr(vec![db.as_str().into(), f.map_or(Value::Null, Fingerprint::to_value)])).collect());
        Value::obj([("fp", fp), ("docs", pairs(&self.docs)), ("folders", pairs(&self.folders)), ("written", self.written.into()), ("catalog", self.catalog.into()), ("deleted", self.deleted.into())])
    }
    pub fn from_value(v: &Value) -> Progress {
        Progress {
            fp: v.get("fp").items().iter().map(|p| (p.items().first().map_or(String::new(), Value::text), p.items().get(1).and_then(Fingerprint::from_value))).collect(),
            docs: unpairs(v.get("docs")),
            folders: unpairs(v.get("folders")),
            written: v.get("written").flag(),
            catalog: v.get("catalog").flag(),
            deleted: v.get("deleted").flag(),
        }
    }
    fn v6(&self, v5: &str) -> Option<&str> { self.docs.iter().find(|d| d.0 == v5).map(|d| &d.1[..]) }
}

// ---- state machine ----------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Phase {
    #[default]
    Idle,
    Lock,
    Kv,
    Info,
    Read,
    Folders,
    Docs,
    Check,
    Census,
    Delete,
    /// A deletion was blocked by a 5.x connection: retried at the next boot.
    Blocked,
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Step {
    #[default]
    Ids,
    Load,
    Meta,
    Media,
    Pdf,
    Import,
    Inflate,
    Original,
}

struct Cur {
    at: usize,
    v6: String,
    step: Step,
    req: u32,
    meta: Option<Meta>,
    /// Blob being imported (owned until it is handed to the importer).
    blob: u32,
    size: f64,
    pdf: u32,
    base_vv: Option<Vec<u8>>,
    source: Option<Hash>,
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    phase: Phase,
    account: String,
    req: u32,
    lock_req: u32,
    pending: Vec<u32>,
    progress: Progress,
    pool: Vec<String>,
    /// Fingerprints of the dump being migrated.
    fp: Vec<(String, Option<Fingerprint>)>,
    reading: usize,
    after: Option<Value>,
    rows: Vec<(Value, Value)>,
    sv: Vec<(String, String, Vec<(Value, Value)>)>,
    plan: Plan,
    cur: Option<Cur>,
    /// The current run could not write every document (offline, empty pool): `written` stays false.
    deferred: bool,
    runner_at: f64,
    errors: Vec<String>,
    runs: u32,
    deletes: u32,
    blocked: bool,
}

/// The migration holds the document session (the headless runner waits).
pub fn busy(app: &App) -> bool { app.st.migrate.cur.is_some() }

fn st(app: &mut App) -> &mut State { &mut app.st.migrate }

fn retry(app: &mut App, host: &mut dyn Host, ms: f64) { app.sched.timer(host.out(), T_RETRY, ms); }

fn begin(app: &mut App, host: &mut dyn Host) {
    let account = auth::known_account(host);
    if app.cfg.has(crate::abi::mode::HEADLESS) || account.is_empty() || !matches!(st(app).phase, Phase::Idle | Phase::Blocked) || st(app).progress.deleted && st(app).account == account {
        return;
    }
    let s = st(app);
    (s.account, s.phase, s.runs, s.errors) = (account.clone(), Phase::Lock, s.runs + 1, Vec::new());
    let req = app.req(Mod::Migrate);
    st(app).lock_req = req;
    locks::request(host, req, &locks::migrate_name(&account), true);
}

/// Ends this run; the lock is kept only while the migration still has work.
fn stop(app: &mut App, host: &mut dyn Host, phase: Phase) {
    let s = st(app);
    s.phase = phase;
    s.cur = None;
    if matches!(phase, Phase::Done | Phase::Blocked | Phase::Idle) && s.lock_req != 0 {
        locks::release(host, std::mem::take(&mut s.lock_req));
    }
}

fn save(app: &mut App, host: &mut dyn Host, extra: impl FnOnce(&mut StoreTx, &str)) {
    let (account, value) = (st(app).account.clone(), st(app).progress.to_value());
    let mut t = StoreTx::new();
    t.kv_put(&account, kv::MIGRATION, &value);
    extra(&mut t, &account);
    let req = app.req(Mod::Migrate);
    store::tx(app, host, req, t);
}

fn locked(app: &mut App, host: &mut dyn Host, io: &Io) {
    if io.status != locks::GRANTED {
        // Another tab migrates; this one looks again at its next boot.
        st(app).lock_req = 0;
        return stop(app, host, Phase::Idle);
    }
    let account = st(app).account.clone();
    let mut t = StoreTx::new();
    t.kv_get(&account, kv::MIGRATION);
    t.kv_get(&account, kv::IDPOOL);
    let req = app.req(Mod::Migrate);
    (st(app).phase, st(app).req) = (Phase::Kv, req);
    store::tx(app, host, req, t);
}

fn kv_read(app: &mut App, host: &mut dyn Host, io: &Io) {
    let mut reply = Reply::new(io);
    if !reply.ok() {
        return stop(app, host, Phase::Idle);
    }
    let progress = Progress::from_value(&reply.value());
    let pool = reply.value().items().iter().map(Value::text).collect();
    let s = st(app);
    (s.progress, s.pool) = (progress, pool);
    if s.progress.deleted {
        return stop(app, host, Phase::Done);
    }
    info(app, host);
}

fn info(app: &mut App, host: &mut dyn Host) {
    st(app).phase = Phase::Info;
    st(app).fp.clear();
    let reqs: Vec<u32> = DATABASES.iter().map(|db| {
        let req = app.req(Mod::Migrate);
        dumper::info(host, req, db);
        req
    }).collect();
    st(app).pending = reqs;
}

fn info_read(app: &mut App, host: &mut dyn Host, io: &Io) {
    let s = st(app);
    let Some(i) = s.pending.iter().position(|r| *r == io.req) else { return };
    s.pending[i] = 0;
    s.fp.push((DATABASES[i].to_string(), Info::parse(io).map(|i| i.fingerprint())));
    if s.pending.iter().any(|r| *r != 0) {
        return;
    }
    s.fp.sort_by(|a, b| a.0.cmp(&b.0));
    let none = s.fp.iter().all(|f| f.1.is_none()) && LS_KEYS.iter().all(|k| host.ls(LsOp::Get, k, None).is_none());
    let s = st(app);
    if none {
        s.progress.deleted = true;
        save(app, host, |_, _| {});
        return stop(app, host, Phase::Done);
    }
    if s.phase == Phase::Census {
        // The re-dump before deleting: unchanged → ask the service worker; changed → migrate the difference.
        if s.fp == s.progress.fp {
            let req = app.req(Mod::Migrate);
            st(app).req = req;
            host.out().fx_id(FX_SW_CLIENTS, req);
        } else {
            st(app).progress.written = false;
            read(app, host);
        }
        return;
    }
    if s.progress.written && s.fp == s.progress.fp {
        return check(app, host);
    }
    read(app, host);
}

// ---- reading the dump -------------------------------------------------------------------------------------------

fn read(app: &mut App, host: &mut dyn Host) {
    let s = st(app);
    (s.phase, s.reading, s.after, s.rows) = (Phase::Read, 0, None, Vec::new());
    release_dump(app, host);
    read_next(app, host);
}

fn read_next(app: &mut App, host: &mut dyn Host) {
    let s = st(app);
    let Some(&(db, store)) = READS.get(s.reading) else { return dumped(app, host) };
    if s.fp.iter().any(|f| f.0 == db && f.1.is_none()) {
        s.reading += 1;
        return read_next(app, host);
    }
    let after = s.after.clone();
    let req = app.req(Mod::Migrate);
    st(app).req = req;
    dumper::read(host, req, db, store, after.as_ref(), 200, 8.0 * 1024.0 * 1024.0);
}

fn page_read(app: &mut App, host: &mut dyn Host, io: &Io) {
    let page = Page::parse(io).unwrap_or(Page { done: true, rows: Vec::new() });
    let s = st(app);
    s.after = page.rows.last().map(|r| r.0.clone());
    s.rows.extend(page.rows);
    if !page.done {
        return read_next(app, host);
    }
    let (db, store) = READS[s.reading];
    let rows = std::mem::take(&mut s.rows);
    s.sv.push((db.into(), store.into(), rows));
    (s.reading, s.after) = (s.reading + 1, None);
    read_next(app, host);
}

/// Releases the blob handles of the dump that no plan keeps.
fn release_dump(app: &mut App, host: &mut dyn Host) {
    let mut handles = Vec::new();
    for (_, _, rows) in std::mem::take(&mut st(app).sv) {
        rows.iter().for_each(|(_, v)| v.handles(&mut handles));
    }
    for d in std::mem::take(&mut st(app).plan.docs) {
        handles.retain(|h| d.blob.is_none_or(|b| b.0 != *h) && d.original.is_none_or(|b| b.0 != *h));
        d.blob.iter().chain(d.original.iter()).for_each(|b| host.out().obj_release(b.0));
    }
    handles.into_iter().for_each(|h| host.out().obj_release(h));
}

fn dumped(app: &mut App, host: &mut dyn Host) {
    let account = st(app).account.clone();
    let ls: Vec<(String, String)> = LS_KEYS.iter().filter_map(|k| Some((k.to_string(), host.ls(LsOp::Get, k, None)?))).collect();
    let dump = Dump { stores: st(app).sv.iter().map(|(db, s, rows)| (db.clone(), s.clone(), rows.iter().map(|r| to_json(&r.1)).collect())).collect(), ls };
    let plan = plan(&account, &dump);
    // Keep the handles the plan needs; the rest of the dump's blobs are released now (recoveries keep theirs below).
    let keep: Vec<u32> = plan.docs.iter().flat_map(|d| d.blob.iter().chain(d.original.iter()).map(|b| b.0)).collect();
    let recoveries: Vec<Value> = st(app).sv.iter().filter(|s| s.0 == OFFLINE_DB && s.1 == "recoveries").flat_map(|s| s.2.iter().map(|r| r.1.clone())).filter(|r| r.get("account").text() == account).collect();
    let mut handles = Vec::new();
    for (_, _, rows) in &st(app).sv {
        rows.iter().for_each(|(_, v)| v.handles(&mut handles));
    }
    let rec_blobs: Vec<u32> = recoveries.iter().filter_map(|r| match r.get("previousBlob") {
        Value::Blob { obj, .. } => Some(*obj),
        _ => None,
    }).collect();
    handles.retain(|h| !keep.contains(h) && !rec_blobs.contains(h));
    handles.into_iter().for_each(|h| host.out().obj_release(h));
    let sv = std::mem::take(&mut st(app).sv);
    st(app).plan = plan;
    if !st(app).progress.catalog {
        settings(app, host, &sv, &recoveries);
    } else {
        rec_blobs.into_iter().for_each(|h| host.out().obj_release(h));
    }
    st(app).deferred = false;
    folders(app, host);
}

/// Settings, Drive entries and recoveries → `kv`/`catalog`/`assets` (once; Home paints offline right after).
fn settings(app: &mut App, host: &mut dyn Host, sv: &[(String, String, Vec<(Value, Value)>)], recoveries: &[Value]) {
    let account = st(app).account.clone();
    let rows = |store: &str| sv.iter().filter(|s| s.0 == OFFLINE_DB && s.1 == store).flat_map(|s| s.2.iter().map(|r| &r.1)).filter(|r| r.get("account").text() == account).collect::<Vec<_>>();
    let mut t = StoreTx::new();
    for r in rows("settings") {
        let pick = |names: &[&str]| Value::Obj(names.iter().filter(|n| !r.get(n).is_undef()).map(|n| (n.to_string(), r.get(n).clone())).collect());
        match &r.get("id").text()[..] {
            "preferences" => t.kv_put_if_absent(&account, kv::PREFERENCES, &pick(&["retentionDays"])),
            "home-views" => t.kv_put_if_absent(&account, kv::HOME_VIEWS, &pick(&["recents", "starred", "shared"])),
            "drive-root" => t.kv_put_if_absent(&account, kv::DRIVE_ROOT, &pick(&["remoteId"])),
            _ => {}
        }
    }
    for r in rows("entries") {
        let e = crate::drive::home::entry_from_value(r);
        if e.id.is_empty() || e.id.starts_with("local-") {
            continue;
        }
        let Value::Obj(fields) = crate::drive::home::entry_value(&e) else { continue };
        let patch: Vec<(Vec<&str>, Value)> = fields.iter().map(|(k, v)| (vec![k.as_str()], v.clone())).collect();
        let patch: Vec<(&[&str], Value)> = patch.iter().map(|(k, v)| (k.as_slice(), v.clone())).collect();
        t.catalog_patch(&account, &e.id, &patch);
    }
    // Recoveries: the PDF as an asset (keyed like a saved source), listed in `kv.recoveries`.
    let mut list = Vec::new();
    for r in recoveries {
        let mut entry = vec![("id".to_string(), r.get("id").clone()), ("fileId".to_string(), r.get("fileId").clone()), ("generation".to_string(), r.get("generation").clone()), ("createdAt".to_string(), r.get("createdAt").clone()), ("exportName".to_string(), r.get("snapshot").get("exportName").clone())];
        if let Value::Blob { obj, size, mime } = r.get("previousBlob") {
            // The bytes are not hashed here (no digest round trip): the key is derived from the recovery id.
            let hash = ihn_core::v5::sha256::sha256(format!("ihn6-recovery:{}", r.get("id").text()).as_bytes());
            t.asset_put(&hash, *obj, if mime.is_empty() { "application/pdf" } else { mime }, *size);
            t.asset_ref(&hash, 1, false);
            entry.push(("asset".into(), Value::Str(hex(&hash))));
        }
        list.push(Value::Obj(entry));
    }
    if !list.is_empty() {
        t.kv_put(&account, kv::RECOVERIES, &Value::Arr(list));
    }
    st(app).progress.catalog = true;
    t.kv_put(&account, kv::MIGRATION, &st(app).progress.to_value());
    let req = app.req(Mod::Migrate);
    store::tx(app, host, req, t);
    // `asset_put` kept the caller's handles; they are not needed any more.
    for r in recoveries {
        if let Value::Blob { obj, .. } = r.get("previousBlob") {
            host.out().obj_release(*obj);
        }
    }
}

// ---- folders ----------------------------------------------------------------------------------------------------

fn online(app: &App) -> bool { !shell::offline(app) && auth::token(app).is_some() }

fn folders(app: &mut App, host: &mut dyn Host) {
    st(app).phase = Phase::Folders;
    let s = st(app);
    let Some(f) = s.plan.folders.iter().find(|f| !s.progress.folders.iter().any(|p| p.0 == f.local_id)).cloned() else {
        return docs(app, host);
    };
    if !online(app) {
        // Folders wait for the network; their documents wait with them.
        st(app).plan.folders.retain(|x| x.local_id != f.local_id);
        st(app).deferred = true;
        return folders(app, host);
    }
    let parents: Vec<String> = f.parents.iter().map(|p| folder_id(app, p)).collect();
    let mut body = format!("{{\"name\":{},\"mimeType\":\"{FOLDER_MIME}\",\"parents\":[{}]", json_str(&f.name), parents.iter().map(|p| json_str(p)).collect::<Vec<_>>().join(","));
    if !f.color.is_empty() {
        body.push_str(&format!(",\"folderColorRgb\":{}", json_str(&f.color)));
    }
    body.push('}');
    let req = Req { method: "POST", url: "https://www.googleapis.com/drive/v3/files?fields=id&supportsAllDrives=true".into(), headers: "Content-Type: application/json\n".into(), body: Body::Bytes(body.into_bytes()), ..Req::default() };
    let id = client::request(app, host, Mod::Migrate, req);
    st(app).req = id;
}

fn folder_id(app: &App, id: &str) -> String { app.st.migrate.progress.folders.iter().find(|p| p.0 == id).map_or(id.to_string(), |p| p.1.clone()) }

fn folder_made(app: &mut App, host: &mut dyn Host, io: &Io) {
    let made = (io.status >= 200 && io.status < 300).then(|| Json::parse(Response::parse(io.data).text()).map(|j| j.text("id").to_string())).flatten().filter(|id| !id.is_empty());
    let s = st(app);
    let Some(f) = s.plan.folders.iter().find(|f| !s.progress.folders.iter().any(|p| p.0 == f.local_id)).cloned() else { return docs(app, host) };
    match made {
        Some(id) => {
            st(app).progress.folders.push((f.local_id.clone(), id.clone()));
            let (name, parents, color) = (f.name.clone(), f.parents.iter().map(|p| folder_id(app, p)).collect::<Vec<_>>(), f.color.clone());
            save(app, host, |t, account| {
                let parents = Value::Arr(parents.iter().map(|p| Value::from(&p[..])).collect());
                t.catalog_patch(account, &id, &[(&["id"], Value::from(&id[..])), (&["name"], Value::from(&name[..])), (&["mimeType"], Value::from(FOLDER_MIME)), (&["parents"], parents), (&["folderColorRgb"], Value::from(&color[..]))]);
            });
        }
        None => {
            st(app).errors.push(format!("folder {}: {}", f.local_id, io.status));
            st(app).plan.folders.retain(|x| x.local_id != f.local_id);
            st(app).deferred = true;
        }
    }
    folders(app, host);
}

// ---- documents --------------------------------------------------------------------------------------------------

/// The session may be used: Home is shown and nothing else holds it.
fn session_free(app: &App) -> bool { shell::view(app) != View::Editor && session::get(app).is_none() && !open::busy(app) && !export::headless::running(app) }

fn docs(app: &mut App, host: &mut dyn Host) {
    st(app).phase = Phase::Docs;
    next_doc(app, host, 0);
}

fn next_doc(app: &mut App, host: &mut dyn Host, from: usize) {
    st(app).cur = None;
    let Some(d) = st(app).plan.docs.get(from).cloned() else { return written(app, host) };
    // A local document inside a folder that is not in Drive yet waits for it.
    if d.local && d.parents.iter().any(|p| p.starts_with("local-") && !st(app).progress.folders.iter().any(|f| f.0 == *p)) {
        st(app).deferred = true;
        return next_doc(app, host, from + 1);
    }
    if !session_free(app) {
        // Retried by T_RETRY (or when the session closes).
        st(app).cur = None;
        st(app).reading = from;
        return retry(app, host, 1_000.0);
    }
    let v6 = if d.local { st(app).progress.v6(&d.v5_id).map(String::from) } else { Some(d.v5_id.clone()) };
    st(app).cur = Some(Cur { at: from, v6: v6.clone().unwrap_or_default(), step: Step::Ids, req: 0, meta: None, blob: 0, size: 0.0, pdf: 0, base_vv: None, source: None });
    match v6 {
        Some(_) => load(app, host),
        None => take_id(app, host),
    }
}

/// A `local-` document takes a pool id (asking Drive for ids when the pool is empty), recorded before any write.
fn take_id(app: &mut App, host: &mut dyn Host) {
    if st(app).pool.is_empty() {
        if !online(app) {
            st(app).deferred = true;
            let at = st(app).cur.as_ref().map_or(0, |c| c.at);
            return next_doc(app, host, at + 1);
        }
        let url = format!("https://www.googleapis.com/drive/v3/files/generateIds?count={}&space=drive&type=files", crate::doc::create::POOL_SIZE);
        let req = client::request(app, host, Mod::Migrate, Req::get(&url));
        if let Some(c) = st(app).cur.as_mut() {
            c.req = req;
        }
        return;
    }
    let id = st(app).pool.remove(0);
    let at = st(app).cur.as_ref().map_or(0, |c| c.at);
    let v5 = st(app).plan.docs[at].v5_id.clone();
    st(app).progress.docs.push((v5, id.clone()));
    if let Some(c) = st(app).cur.as_mut() {
        c.v6 = id;
    }
    let pool: Vec<Value> = st(app).pool.iter().map(|i| Value::from(&i[..])).collect();
    save(app, host, |t, account| t.kv_put(account, kv::IDPOOL, &Value::Arr(pool)));
    load(app, host);
}

fn ids_answer(app: &mut App, host: &mut dyn Host, io: &Io) {
    let body = if io.status >= 0 { Response::parse(io.data).text() } else { "" };
    let ids: Vec<String> = Json::parse(body).as_ref().and_then(|j| j.get("ids")).map(|ids| ids.items().iter().filter_map(|i| i.as_str().map(String::from)).collect()).unwrap_or_default();
    if ids.is_empty() {
        st(app).deferred = true;
        let at = st(app).cur.as_ref().map_or(0, |c| c.at);
        return next_doc(app, host, at + 1);
    }
    st(app).pool = ids;
    take_id(app, host);
}

fn cur(app: &mut App) -> Option<&mut Cur> { app.st.migrate.cur.as_mut() }
fn plan_doc(app: &App) -> Option<&DocPlan> { app.st.migrate.cur.as_ref().and_then(|c| app.st.migrate.plan.docs.get(c.at)) }

fn load(app: &mut App, host: &mut dyn Host) {
    let req = app.req(Mod::Migrate);
    let account = st(app).account.clone();
    let Some(c) = cur(app) else { return };
    (c.step, c.req) = (Step::Load, req);
    let v6 = c.v6.clone();
    session::load(app, host, &account, &v6, req);
}

fn loaded(app: &mut App, host: &mut dyn Host) {
    let Some(d) = plan_doc(app).cloned() else { return };
    let has_base = session::get(app).is_some_and(|s| !s.doc.order().is_empty());
    if d.local || has_base {
        return original(app, host);
    }
    if online(app) {
        let url = format!("https://www.googleapis.com/drive/v3/files/{}?fields={}&supportsAllDrives=true", d.v5_id, fx::url_encode(META_FIELDS));
        let req = client::request(app, host, Mod::Migrate, Req { accept: &[404], ..Req::get(&url) });
        if let Some(c) = cur(app) {
            (c.step, c.req) = (Step::Meta, req);
        }
        return;
    }
    // Offline: the cached PDF of the base revision, or the local snapshot alone.
    base_blob(app, host, d.blob.map(|b| (b, true)));
}

fn meta_answer(app: &mut App, host: &mut dyn Host, io: &Io) {
    let Some(d) = plan_doc(app).cloned() else { return };
    let meta = if io.status >= 200 && io.status < 300 { Meta::parse(Response::parse(io.data).text(), &d.details.resource_key) } else { None };
    let Some(meta) = meta else {
        // Not readable in Drive (deleted, no access, failure): the cached base if there is one.
        return base_blob(app, host, d.blob.map(|b| (b, true)));
    };
    let same = !meta.head.is_empty() && meta.head == d.base.head_revision_id;
    if let Some(c) = cur(app) {
        c.meta = Some(meta);
    }
    if same && d.blob.is_some() {
        return base_blob(app, host, d.blob.map(|b| (b, true)));
    }
    let mut url = format!("https://www.googleapis.com/drive/v3/files/{}?alt=media", d.v5_id);
    let key = cur(app).and_then(|c| c.meta.as_ref()).map_or(String::new(), |m| m.resource_key.clone());
    if !key.is_empty() {
        url.push_str(&format!("&resourceKey={}", fx::url_encode(&key)));
    }
    let req = client::request(app, host, Mod::Migrate, Req { blob: true, timeout_ms: 120_000.0, ..Req::get(&url) });
    if let Some(c) = cur(app) {
        (c.step, c.req) = (Step::Media, req);
    }
}

/// The base PDF to import: `(handle, cached)` (a cached blob belongs to the plan until handed over).
fn base_blob(app: &mut App, host: &mut dyn Host, blob: Option<((u32, f64), bool)>) {
    let Some(((obj, size), cached)) = blob else { return original(app, host) };
    if cached {
        let at = cur(app).map_or(0, |c| c.at);
        st(app).plan.docs[at].blob = None;
        // The cached copy is the base revision: its facts are the 5.x base, not a newer head.
        let d = st(app).plan.docs[at].clone();
        if let Some(c) = cur(app) {
            if c.meta.as_ref().is_none_or(|m| m.head != d.base.head_revision_id) {
                c.meta = Some(Meta { head: d.base.head_revision_id.clone(), modified: d.details.modified.clone(), ..c.meta.clone().unwrap_or(d.details.clone()) });
            }
        }
    }
    let req = app.req(Mod::Migrate);
    if let Some(c) = cur(app) {
        (c.step, c.req, c.blob, c.size) = (Step::Pdf, req, obj, size);
    }
    pdf::open(host.out(), req, obj);
}

fn pdf_opened(app: &mut App, host: &mut dyn Host, io: &Io) {
    let Some(d) = plan_doc(app).cloned() else { return };
    let req = app.req(Mod::Migrate);
    let Some(c) = cur(app) else { return };
    let (blob, size) = (std::mem::take(&mut c.blob), c.size);
    let Some(opened) = (io.status >= 0).then(|| pdf::Opened::parse(io.data)).flatten() else {
        host.out().obj_release(blob);
        st(app).errors.push(format!("{}: base not readable", d.v5_id));
        return original(app, host);
    };
    let meta = c.meta.clone().unwrap_or(d.details.clone());
    let (kind, payload) = classify(&opened.keywords);
    if kind == Kind::V6 {
        // Another device already wrote a v6 container: its snapshot is the base.
        c.pdf = io.obj;
        let bytes = b64_decode(payload.as_bytes()).unwrap_or_default();
        (c.step, c.req) = (Step::Inflate, req);
        host.out().obj_release(blob);
        return fx::deflate(host.out(), req, deflate_flag::INFLATE | deflate_flag::ZLIB, Src::Bytes(&bytes));
    }
    let f = Foreign { file: File { id: c.v6.clone(), name: d.file_name.clone(), resource_key: meta.resource_key.clone() }, kind, blob, size, pdf: io.obj, opened, meta };
    (c.step, c.req) = (Step::Import, req);
    v5open::import(app, host, f, req);
}

fn inflated(app: &mut App, host: &mut dyn Host, io: &Io) {
    if let Some(c) = cur(app) {
        let p = std::mem::take(&mut c.pdf);
        pdf::close(host.out(), p);
    }
    if io.status >= 0 {
        let snap = EncodedSnapshot(io.data.to_vec());
        session::with_replica(app, host, |r| r.load_snapshot(&snap, PeerRef::Export));
    }
    base_done(app, host);
}

/// The base is in the session: remember its VV (the head becomes a known export).
fn base_done(app: &mut App, host: &mut dyn Host) {
    let vv = session::get(app).map(|s| {
        let mut w = ihn_core::codec::Writer::default();
        s.doc.vv.encode(&mut w);
        w.buf
    });
    if let Some(c) = cur(app) {
        c.base_vv = vv;
    }
    original(app, host);
}

/// A `local-` document's clean original PDF (pages with a PDF background) becomes a source asset.
fn original(app: &mut App, host: &mut dyn Host) {
    let at = cur(app).map_or(0, |c| c.at);
    let Some((obj, _)) = st(app).plan.docs.get(at).and_then(|d| d.original) else { return merge(app, host) };
    let req = app.req(Mod::Migrate);
    if let Some(c) = cur(app) {
        (c.step, c.req) = (Step::Original, req);
    }
    fx::digest(host.out(), req, Src::blob(obj));
}

fn original_digest(app: &mut App, host: &mut dyn Host, io: &Io) {
    let at = cur(app).map_or(0, |c| c.at);
    if let (Ok(sha), Some((obj, size))) = (<[u8; 32]>::try_from(io.data), st(app).plan.docs[at].original.take()) {
        let hash = source_hash(&sha, Source::Saved);
        session::put_asset(app, host, hash, obj, "application/pdf", size);
        if let Some(c) = cur(app) {
            c.source = Some(hash);
        }
    }
    merge(app, host);
}

/// `from_local` for every source over the session's document; the batches are received (journalled) and the
/// record is written whole.
fn merge(app: &mut App, host: &mut dyn Host) {
    let Some(c) = st(app).cur.as_ref() else { return };
    let (at, source, base_vv, fresh) = (c.at, c.source, c.base_vv.clone(), c.meta.clone());
    let d = st(app).plan.docs[at].clone();
    let Some((v6, inst)) = session::get(app).and_then(|s| Some((s.doc_id.clone(), s.author.as_ref()?.actor.inst()))) else {
        st(app).errors.push(format!("{}: no session", d.v5_id));
        return finish_doc(app, host, false);
    };
    let actor = ActorId::import(&migration_source(inst, &v6));
    for src in &d.sources {
        let src = V5LocalDoc { source: src.source.or(source), ..src.clone() };
        let imp = match session::get(app).map(|s| from_local(&src, actor, Some(&s.doc))) {
            Some(Ok(imp)) => imp,
            Some(Err(e)) => {
                st(app).errors.push(format!("{}: {}", d.v5_id, e.message()));
                continue;
            }
            None => break,
        };
        for a in &imp.assets {
            let obj = fx::new_obj(app);
            fx::blob_new(host.out(), obj, &a.mime, &a.bytes);
            session::put_asset(app, host, a.hash, obj, &a.mime, a.bytes.len() as f64);
        }
        let encoded: Vec<_> = imp.batches.iter().map(encode_batch).collect();
        session::with_replica(app, host, |r| r.receive(&encoded, PeerRef::Import));
    }
    let parents: Vec<String> = d.parents.iter().map(|p| folder_id(app, p)).collect();
    let Some(s) = session::get_mut(app) else { return };
    let seq = s.doc.vv.get(actor);
    let r = &mut s.record;
    if r.name.is_empty() || d.local {
        r.name = d.file_name.clone();
    }
    if d.local {
        let meta = Meta { parents: parents.clone(), ..d.details.clone() };
        r.drive_meta = Value::obj([("parents", Value::Arr(parents.iter().map(|p| Value::from(&p[..])).collect())), ("canEdit", true.into()), ("ownedByMe", true.into()), ("ownerName", Value::from(&meta.owner_name[..])), ("ownerEmail", Value::from(&meta.owner_email[..])), ("ownerPhoto", Value::from(&meta.owner_photo[..]))]);
    } else if let Some(m) = fresh.as_ref().or(Some(&d.details)).filter(|_| r.drive_meta.fields().is_empty() || fresh.is_some()) {
        r.drive_meta = m.value();
        s.can_edit = m.can_edit;
    }
    let r = &mut s.record;
    if let (Some(vv), Some(head)) = (base_vv, fresh.as_ref().map(|m| m.head.clone()).filter(|h| !h.is_empty())) {
        if !r.exports.iter().any(|e| e.0 == head) {
            r.exports.push((head, vv));
        }
    }
    r.flags.created = !d.local || r.flags.created;
    r.flags.migrated = true;
    if seq > 0 {
        r.seq.insert(actor.0, seq.max(r.seq.get(actor.0).copied().unwrap_or(0)));
        r.flags.relay_pending = true;
        r.flags.export_pending = true;
    }
    r.last_opened_at = r.last_opened_at.max(d.sources.iter().map(|s| s.saved_at).fold(0.0, f64::max));
    session::write_record(app, host);
    if d.local {
        // The Home card of a document that exists only on this device.
        let (name, account) = (d.file_name.clone(), st(app).account.clone());
        let mut t = StoreTx::new();
        let parents = Value::Arr(parents.iter().map(|p| Value::from(&p[..])).collect());
        t.catalog_patch(&account, &v6, &[(&["id"], Value::from(&v6[..])), (&["name"], Value::from(&name[..])), (&["mimeType"], Value::from("application/pdf")), (&["parents"], parents)]);
        let req = app.req(Mod::Migrate);
        store::tx(app, host, req, t);
    }
    if !st(app).progress.docs.iter().any(|p| p.0 == d.v5_id) {
        st(app).progress.docs.push((d.v5_id.clone(), v6));
        save(app, host, |_, _| {});
    }
    finish_doc(app, host, true);
}

fn finish_doc(app: &mut App, host: &mut dyn Host, ok: bool) {
    let at = cur(app).map_or(0, |c| c.at);
    if let Some(c) = st(app).cur.take() {
        for h in [c.blob] {
            if h != 0 {
                host.out().obj_release(h);
            }
        }
        if c.pdf != 0 {
            pdf::close(host.out(), c.pdf);
        }
    }
    if !ok {
        st(app).deferred = true;
    }
    if session::get(app).is_some() {
        session::close(app, host);
    }
    next_doc(app, host, at + 1);
}

/// Every document of the plan was handled.
fn written(app: &mut App, host: &mut dyn Host) {
    let s = st(app);
    s.cur = None;
    if !s.deferred {
        s.progress.fp = s.fp.clone();
        s.progress.written = true;
    }
    save(app, host, |_, _| {});
    release_dump(app, host);
    kick_runner(app, host, true);
    check(app, host);
}

fn kick_runner(app: &mut App, host: &mut dyn Host, now: bool) {
    let t = host.now();
    if (now || t - st(app).runner_at > RUNNER_EVERY_MS) && session_free(app) && online(app) && shell::view(app) == View::Home {
        st(app).runner_at = t;
        export::headless::run(app, host);
    }
}

// ---- done, census, deletion -------------------------------------------------------------------------------------

fn check(app: &mut App, host: &mut dyn Host) {
    st(app).phase = Phase::Check;
    let account = st(app).account.clone();
    let mut t = StoreTx::new();
    t.kv_get(&account, kv::INST);
    t.docs_scan();
    let req = app.req(Mod::Migrate);
    st(app).req = req;
    store::tx(app, host, req, t);
}

fn checked(app: &mut App, host: &mut dyn Host, io: &Io) {
    let mut reply = Reply::new(io);
    if !reply.ok() {
        return retry(app, host, RETRY_MS);
    }
    let inst = unhex16(&reply.value().text()).unwrap_or(0);
    let rows = reply.rows();
    let mut handles = Vec::new();
    rows.iter().for_each(|(_, v)| v.handles(&mut handles));
    handles.into_iter().for_each(|h| host.out().obj_release(h));
    let s = st(app);
    let done = s.progress.written && s.progress.docs.iter().all(|(_, v6)| {
        let actor = ActorId::import(&migration_source(inst, v6));
        rows.iter().find(|(k, _)| k.text() == *v6).is_some_and(|(_, v)| doc_done(&DocRecord::from_value(v), actor))
    });
    if !done {
        kick_runner(app, host, false);
        // A run that could not write everything (offline, empty pool) starts over when it can.
        let next = if st(app).progress.written { RETRY_MS } else { 2.0 * RETRY_MS };
        return retry(app, host, next);
    }
    st(app).phase = Phase::Census;
    info(app, host);
    st(app).phase = Phase::Census;
}

fn census(app: &mut App, host: &mut dyn Host, io: &Io) {
    let answer = if io.status >= 0 { Json::parse(std::str::from_utf8(io.data).unwrap_or("")) } else { None };
    let (windows, v6) = answer.as_ref().map_or((0.0, -1.0), |j| (j.get("windows").and_then(Json::as_f64).unwrap_or(0.0), j.get("v6").and_then(Json::as_f64).unwrap_or(-1.0)));
    if windows < 1.0 || v6 != windows {
        // A 5.x window is open (or the service worker cannot tell): next visibility change or retry.
        st(app).phase = Phase::Check;
        return retry(app, host, RETRY_MS);
    }
    delete(app, host);
}

fn delete(app: &mut App, host: &mut dyn Host) {
    st(app).phase = Phase::Delete;
    st(app).deletes += 1;
    st(app).blocked = false;
    let reqs: Vec<u32> = DATABASES
        .iter()
        .map(|db| {
            let req = app.req(Mod::Migrate);
            store::delete_db(host, req, db);
            req
        })
        .collect();
    let caches = app.req(Mod::Migrate);
    let out = host.out();
    let m = out.w.rec(crate::abi::fam::FX, FX_CACHES_DELETE);
    out.w.u32(caches);
    out.w.str(MEDIA_CACHES);
    out.w.end(m);
    st(app).pending = reqs.into_iter().chain([caches]).collect();
}

fn deleted(app: &mut App, host: &mut dyn Host, io: &Io) {
    let s = st(app);
    let Some(i) = s.pending.iter().position(|r| *r == io.req) else { return };
    s.pending[i] = 0;
    if io.status != status::OK {
        s.blocked = true;
    }
    if s.pending.iter().any(|r| *r != 0) {
        return;
    }
    if st(app).blocked {
        st(app).errors.push("a 5.x database is still open".into());
        return stop(app, host, Phase::Blocked);
    }
    for k in LS_KEYS {
        host.ls(LsOp::Remove, k, None);
    }
    st(app).progress.deleted = true;
    save(app, host, |_, _| {});
    stop(app, host, Phase::Done);
}

// ---- module contract --------------------------------------------------------------------------------------------

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(app: &mut App, host: &mut dyn Host) {
    if !app.cfg.has(crate::abi::mode::HEADLESS) {
        app.sched.timer(host.out(), T_START, 0.0);
    }
}

fn on_io(app: &mut App, host: &mut dyn Host, io: &Io) {
    let s = st(app);
    if io.req == s.lock_req && s.phase == Phase::Lock {
        return locked(app, host, io);
    }
    if s.pending.contains(&io.req) {
        return match s.phase {
            Phase::Info | Phase::Census => info_read(app, host, io),
            Phase::Delete => deleted(app, host, io),
            _ => {}
        };
    }
    if let Some(c) = s.cur.as_ref().filter(|c| c.req == io.req) {
        return match c.step {
            Step::Ids => ids_answer(app, host, io),
            Step::Load => loaded(app, host),
            Step::Meta => meta_answer(app, host, io),
            Step::Media => {
                if io.status >= 200 && io.status < 300 && io.obj != 0 {
                    base_blob(app, host, Some(((io.obj, Response::parse(io.data).blob_size()), false)));
                } else {
                    if io.obj != 0 {
                        host.out().obj_release(io.obj);
                    }
                    let d = plan_doc(app).cloned();
                    base_blob(app, host, d.and_then(|d| d.blob).map(|b| (b, true)));
                }
            }
            Step::Pdf => pdf_opened(app, host, io),
            Step::Import => {
                if io.status >= 0 {
                    base_done(app, host);
                } else {
                    let id = plan_doc(app).map_or(String::new(), |d| d.v5_id.clone());
                    st(app).errors.push(format!("{id}: base import failed"));
                    original(app, host);
                }
            }
            Step::Inflate => inflated(app, host, io),
            Step::Original => original_digest(app, host, io),
        };
    }
    if io.req != s.req || io.req == 0 {
        if io.obj != 0 {
            host.out().obj_release(io.obj);
        }
        return;
    }
    match s.phase {
        Phase::Kv => kv_read(app, host, io),
        Phase::Read => page_read(app, host, io),
        Phase::Folders => folder_made(app, host, io),
        Phase::Check => checked(app, host, io),
        Phase::Census => census(app, host, io),
        _ => {}
    }
}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Timer(T_START) => begin(app, host),
        Event::Timer(T_RETRY) => match app.st.migrate.phase {
            Phase::Docs if app.st.migrate.cur.is_none() => {
                let from = st(app).reading;
                next_doc(app, host, from)
            }
            Phase::Check => check(app, host),
            _ => {}
        },
        Event::Io(io) => on_io(app, host, &io),
        // The session was taken over (a document opened from Home): this document is written again later.
        Event::Msg(m) if m.chan == session::sig::CLOSED && session::get(app).is_none() && app.st.migrate.cur.as_ref().is_some_and(|c| c.step != Step::Ids && c.v6.as_bytes() == m.data) => {
            let at = cur(app).map_or(0, |c| c.at);
            st(app).cur = None;
            st(app).reading = at;
            retry(app, host, 1_000.0);
        }
        Event::Msg(m) if m.chan == shell::sig::AUTH && st(app).phase == Phase::Idle => begin(app, host),
        Event::Life(l) if l.code == life::VISIBILITY && l.a == 0.0 => match st(app).phase {
            Phase::Check => check(app, host),
            Phase::Idle | Phase::Blocked => begin(app, host),
            _ => {}
        },
        _ => {}
    }
    0
}

/// `__IHN_TEST_API__.cmd('migrate.<name>', args)`: `state` → `{phase, runs, deletes, docs, folders, written,
/// deleted, errors}`; `run` starts a run now (as a new boot would); `has {ids}` → the 5.x stroke ids missing from
/// the open document.
pub fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    match name {
        "state" => {
            let s = &app.st.migrate;
            let list = |v: &[(String, String)]| v.iter().map(|(a, b)| format!("[{},{}]", json_str(a), json_str(b))).collect::<Vec<_>>().join(",");
            let errors: Vec<String> = s.errors.iter().map(|e| json_str(e)).collect();
            Some(format!(
                "{{\"phase\":\"{:?}\",\"runs\":{},\"deletes\":{},\"busy\":{},\"docs\":[{}],\"folders\":[{}],\"written\":{},\"deleted\":{},\"errors\":[{}]}}",
                s.phase,
                s.runs,
                s.deletes,
                s.cur.is_some(),
                list(&s.progress.docs),
                list(&s.progress.folders),
                s.progress.written,
                s.progress.deleted,
                errors.join(",")
            ))
        }
        "run" => {
            begin(app, host);
            Some("true".into())
        }
        "has" => {
            // `{ids}` (5.x stroke ids) → those that are not live items of the open document.
            let json = Json::parse(_args);
            let ids: Vec<String> = json.as_ref().and_then(|j| j.get("ids")).map(|l| l.items().iter().filter_map(|i| i.as_str().map(String::from)).collect()).unwrap_or_default();
            let s = session::get(app);
            let missing: Vec<String> = ids.iter().filter(|id| !s.is_some_and(|s| s.doc.items.get(&ihn_core::id::Id::legacy(id)).is_some_and(|i| i.is_live()))).map(|i| json_str(i)).collect();
            Some(format!("[{}]", missing.join(",")))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ihn_core::v5::json::parse;

    fn dir() -> std::path::PathBuf { std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/v6/fixtures/v5/storage") }

    fn revive(v: J, n: &mut u32) -> J {
        match v {
            J::Obj(f) => match f.iter().find(|(k, _)| &**k == "$").and_then(|(_, t)| t.as_str().map(str::to_string)).as_deref() {
                Some("undef") => J::Undef,
                Some("u8") => J::Bytes(b64_decode(f.iter().find(|(k, _)| &**k == "b").and_then(|(_, b)| b.as_str()).unwrap_or("").as_bytes()).unwrap_or_default()),
                Some("blob") => {
                    *n += 1;
                    J::Blob { obj: *n, size: 1.0 }
                }
                _ => J::Obj(f.into_iter().map(|(k, v)| (k, revive(v, n))).collect()),
            },
            J::Arr(a) => J::Arr(a.into_iter().map(|v| revive(v, n)).collect()),
            o => o,
        }
    }

    fn dump(name: &str) -> Dump {
        let root = revive(parse(&std::fs::read(dir().join(format!("{name}.dump.json"))).unwrap()).unwrap(), &mut 0);
        let mut d = Dump::default();
        for db in root.get("indexedDB").arr() {
            for s in db.get("stores").arr() {
                d.stores.push((db.get("name").text(), s.get("name").text(), s.get("records").arr().iter().map(|r| r.get("value").clone()).collect()));
            }
        }
        if let J::Obj(f) = root.get("localStorage") {
            d.ls = f.iter().filter_map(|(k, v)| Some((k.to_string(), v.as_str()?.to_string()))).collect();
        }
        d
    }

    #[test]
    fn every_storage_fixture_plans_its_unsynced_document() {
        for name in ["background-worker", "local-doc", "offline-dirty", "pending-exit", "recoveries", "recovery-conflict", "stroke-ops-pending", "xor-packed", "local-folder"] {
            let e = parse(&std::fs::read(dir().join(format!("{name}.expect.json"))).unwrap()).unwrap();
            let p = plan(&e.get("account").text(), &dump(name));
            match e.get("document").get("id").as_str() {
                Some(id) => {
                    assert_eq!(p.docs.iter().map(|d| d.v5_id.as_str()).collect::<Vec<_>>(), [id], "{name}");
                    let d = &p.docs[0];
                    assert_eq!(d.local, id.starts_with("local-"), "{name}");
                    assert!(!d.sources.is_empty() && d.file_name.ends_with(".pdf"), "{name}");
                    // Every unsynced stroke is in one of the sources.
                    for s in e.get("unsyncedStrokeIds").arr() {
                        let s = s.text();
                        assert!(d.sources.iter().any(|src| src.pages.iter().any(|p| p.strokes.iter().any(|x| x.id == s))), "{name}: {s}");
                    }
                }
                None => assert!(p.docs.is_empty(), "{name}"),
            }
            assert_eq!(p.folders.len(), (name == "local-folder") as usize, "{name}");
        }
        let p = plan("ana@example.test", &dump("local-folder"));
        assert_eq!((p.folders[0].name.as_str(), p.folders[0].parents.as_slice()), ("Offline folder", &["root".to_string()][..]));
        // Another account's data is not planned.
        assert!(plan("ben@example.test", &dump("offline-dirty")).docs.is_empty());
    }

    #[test]
    fn progress_round_trips_and_the_done_rule_needs_the_seal() {
        let fp = Fingerprint { records: 3.0, stamp: 9.0, hash: 7 };
        let p = Progress { fp: vec![("a".into(), Some(fp)), ("b".into(), None)], docs: vec![("local-1".into(), "x".into())], folders: vec![("local-f".into(), "F".into())], written: true, catalog: true, deleted: false };
        assert_eq!(Progress::from_value(&Value::decode(&p.to_value().encode()).unwrap()), p);
        assert_eq!(Progress::from_value(&Value::Undef), Progress::default());
        let actor = ActorId::import("migrate:x");
        let mut r = DocRecord { flags: crate::store::schema::Flags { created: true, ..Default::default() }, ..DocRecord::default() };
        assert!(doc_done(&r, actor), "nothing migrated");
        r.seq.insert(actor.0, 3);
        assert!(!doc_done(&r, actor));
        r.relay.sealed.insert(actor.0, 2);
        assert!(!doc_done(&r, actor));
        r.relay.sealed.insert(actor.0, 3);
        assert!(doc_done(&r, actor));
        r.flags.created = false;
        assert!(!doc_done(&r, actor), "a local document must be created");
    }
}
