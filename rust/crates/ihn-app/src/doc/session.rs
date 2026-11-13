//! The open document session: model, persistence queue, the `Replica` side of the sync binding
//! (ARCHITECTURE §4.4, §7.1; SYNC §7). Owned by W3.1.
//!
//! * [`load`] reads the whole document (header chunk, page chunks, journal) and builds the writer
//!   (`Author::resume`); its `notify` request completes with [`LOADED_EMPTY`] or [`LOADED_LOCAL`].
//! * [`edit`] is the write path: build the batch, apply it, append it to `ops` with `docs.seq` in one transaction;
//!   only after the commit does the batch enter the outbox the sync engine drains ([`drain_local`], announced with
//!   [`sig::COMMITTED`]). In degraded storage the batch stays in the in-memory journal and enters the outbox at once.
//! * [`with_replica`] lends the session to `ihn-sync` as a `Replica`: remote batches are deduplicated, applied
//!   (budgeted while a pointer is in contact), journalled and reported with [`sig::CHANGED`].
//! * Compaction (the `ihn6-net` holder, pen idle) folds dirty pages into chunks and trims the journal by the
//!   retention rule (covered, or older than 24 h, within 8 MiB).

use crate::abi::{tag, Event};
use crate::app::Mod;
use crate::store::schema::{kv, DocRecord, Record};
use crate::store::sv::{hex16, unhex16, Value};
use crate::store::{self, locks, Reply, Tx as StoreTx};
use crate::testapi::json_str;
use crate::{auth, shell, App, Host};
use ihn_core::doc::{Author, BlobHandle, Changes, Compacted, Doc, PeerRef, ReceiveResult, Replica, SyncStatus, Tx};
use ihn_core::id::{ActorId, DocId, Hash, Id, IdGen};
use ihn_core::op::{Background, Batch, Body, Op};
use ihn_core::snapshot::{self, BatchDecoder, EncodedBatch, EncodedSnapshot, Loader};
use ihn_core::stamp::VersionVector;
use std::collections::VecDeque;

/// Broadcasts of the document modules (`Event::Msg`, sent with `shell::signal`).
pub mod sig {
    /// A document finished opening (data = its id): the editor builds its view.
    pub const OPENED: u8 = 0x40;
    /// Remote batches or a snapshot changed the document: read [`super::take_changes`].
    pub const CHANGED: u8 = 0x41;
    /// Local batches are durable (or the storage is degraded): the sync engine reads [`super::drain_local`].
    pub const COMMITTED: u8 = 0x42;
    /// An input of the save presentation changed (`doc::save_state` re-derives it).
    pub const STATE: u8 = 0x43;
    /// The session ended (data = the document id).
    pub const CLOSED: u8 = 0x44;
    /// `doc::open` holds a PDF that is not a v6 container (data = `v5` | `plain`); the importer claims it by
    /// returning `ret::PREVENT_DEFAULT` (see `doc::open::take_foreign`).
    pub const FOREIGN: u8 = 0x45;
}

/// `notify` status of [`load`]: nothing stored for this document.
pub const LOADED_EMPTY: i32 = 0;
/// `notify` status of [`load`]: chunks or journal entries were loaded.
pub const LOADED_LOCAL: i32 = 1;

/// Journal retention (SYNC §7.3).
pub const JOURNAL_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1000;
pub const JOURNAL_MAX_BYTES: usize = 8 << 20;
/// Remote-apply budget per frame while a pointer is in contact, and per idle slice otherwise (ARCHITECTURE §4.4).
pub const CONTACT_BUDGET_MS: f64 = 0.5;
pub const IDLE_BUDGET_MS: f64 = 4.0;
/// Ops decoded or applied between two looks at the clock.
const SLICE_OPS: usize = 256;
/// Quiet time before compaction (5.18.0's burst gating of local checkpoints).
const COMPACT_QUIET_MS: f64 = 1200.0;

const T_LATER: u32 = tag(Mod::DocSession, 1);
const W_COMPACT: u32 = tag(Mod::DocSession, 2);
const W_DRAIN: u32 = tag(Mod::DocSession, 3);

struct Entry {
    actor: u64,
    seq: u32,
    clock: u64,
    bytes: Vec<u8>,
}

/// The journal in memory, sorted by `(actor, seq)` (sorted vectors, not maps: every map type costs wasm bytes).
#[derive(Default)]
struct Journal {
    e: Vec<Entry>,
    bytes: usize,
}

impl Journal {
    fn find(&self, actor: u64, seq: u32) -> Result<usize, usize> { self.e.binary_search_by(|e| (e.actor, e.seq).cmp(&(actor, seq))) }
    fn has(&self, actor: u64, seq: u32) -> bool { self.find(actor, seq).is_ok() }
    fn put(&mut self, actor: u64, seq: u32, clock: u64, bytes: Vec<u8>) {
        self.bytes += bytes.len();
        let entry = Entry { actor, seq, clock, bytes };
        match self.find(actor, seq) {
            Ok(i) => self.bytes -= std::mem::replace(&mut self.e[i], entry).bytes.len(),
            Err(i) => self.e.insert(i, entry),
        }
    }
}

/// `actor → seq` pairs sorted by actor.
type Through = Vec<(u64, u32)>;
fn through_of(t: &Through, actor: u64) -> Option<u32> { t.binary_search_by_key(&actor, |e| e.0).ok().map(|i| t[i].1) }
fn through_set(t: &mut Through, actor: u64, seq: u32) {
    match t.binary_search_by_key(&actor, |e| e.0) {
        Ok(i) => t[i].1 = seq,
        Err(i) => t.insert(i, (actor, seq)),
    }
}

/// A remote batch being decoded and applied in slices.
struct Partial {
    bytes: Vec<u8>,
    dec: Option<BatchDecoder>,
    batch: Option<Batch>,
    at: usize,
}

/// One open document.
pub struct Session {
    pub doc_id: DocId,
    pub account: String,
    pub doc: Doc,
    /// The writer; `None` when the tab has no actor slot (B-Y7: read-only).
    pub author: Option<Author>,
    pub record: DocRecord,
    /// The Drive capability (`capabilities.canEdit`).
    pub can_edit: bool,
    /// False: nothing is written to IndexedDB (no account, or the store could not be read).
    pub persistent: bool,
    /// This tab holds `ihn6-net:<account>:<docId>`.
    pub net_holder: bool,
    /// Size of the Drive file last downloaded (5.18.0 `state.lastExportSize`), 0 = unknown.
    pub export_size: f64,
    record_stored: bool,
    journal: Journal,
    uncommitted: u32,
    failed: bool,
    saved_at: f64,
    outbox: Vec<EncodedBatch>,
    queue: VecDeque<Vec<u8>>,
    queued_keys: Vec<(u64, u32)>,
    partial: Option<Partial>,
    covered: VersionVector,
    /// Page → chunk number.
    chunks: Vec<(Id, u32)>,
    next_chunk: u32,
    full_compact: bool,
    assets: Vec<(Hash, BlobHandle)>,
    new_assets: Vec<(Hash, BlobHandle, String, f64)>,
    pending: StoreTx,
    changes: Changes,
    /// `sig::CHANGED` was sent for what `changes` holds.
    announced: bool,
    sync: Option<SyncStatus>,
}

struct Load {
    account: String,
    doc_id: String,
    notify: u32,
    slot: Option<u8>,
    slot_asked: bool,
    req: u32,
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    s: Option<Session>,
    load: Option<Load>,
    later: Vec<(u32, i32)>,
    /// Local appends in flight: request → the batches they carry.
    appends: Vec<(u32, Vec<EncodedBatch>, bool)>,
    compact_req: u32,
    asset_req: u32,
    net_req: u32,
    /// Hashes of the asset read in flight, in request order.
    asset_wanted: Vec<Hash>,
}

/// Local inputs of the save presentation (`doc::save_state`).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Local {
    /// A local batch is not committed yet.
    pub pending: bool,
    /// A local commit failed or the storage is degraded.
    pub failed: bool,
    /// `Date.now()` of the last local commit (0 = none in this session).
    pub saved_at: f64,
}

/// The result of [`edit`].
pub struct Edit {
    pub seq: u32,
    /// The ops that undo the batch (`Doc::inverse`, taken before it was applied).
    pub inverse: Vec<Op>,
    pub changes: Changes,
}

// ---- accessors ----------------------------------------------------------------------------------------------------

pub fn get(app: &App) -> Option<&Session> { app.st.doc_session.s.as_ref() }
pub fn get_mut(app: &mut App) -> Option<&mut Session> { app.st.doc_session.s.as_mut() }
/// The open document's id, or "".
pub fn doc_id(app: &App) -> &str { get(app).map_or("", |s| &s.doc_id) }
pub fn local(app: &App) -> Local {
    get(app).map_or(Local::default(), |s| Local { pending: s.uncommitted > 0, failed: s.failed, saved_at: s.saved_at })
}

impl Session {
    fn new(account: &str, doc_id: &str) -> Session {
        Session {
            doc_id: doc_id.into(),
            account: account.into(),
            doc: Doc::default(),
            author: None,
            record: DocRecord { account: account.into(), ..DocRecord::default() },
            can_edit: true,
            persistent: false,
            net_holder: false,
            export_size: 0.0,
            record_stored: false,
            journal: Journal::default(),
            uncommitted: 0,
            failed: false,
            saved_at: 0.0,
            outbox: Vec::new(),
            queue: VecDeque::new(),
            queued_keys: Vec::new(),
            partial: None,
            covered: VersionVector::default(),
            chunks: Vec::new(),
            next_chunk: 1,
            full_compact: false,
            assets: Vec::new(),
            new_assets: Vec::new(),
            pending: StoreTx::new(),
            changes: Changes::default(),
            announced: false,
            sync: None,
        }
    }

    /// May this tab write ops? (Drive capability and an actor slot.)
    pub fn writable(&self) -> bool { self.can_edit && self.author.is_some() }

    /// Own batches not sealed in the relay, by the persisted seal (used until a sync engine reports its own count).
    pub fn unsealed_local(&self) -> u32 {
        let inst = self.author.as_ref().map(|a| a.actor.inst());
        self.journal.e.iter().filter(|e| Some(ActorId(e.actor).inst()) == inst && self.record.relay.sealed.get(e.actor).is_none_or(|&sealed| e.seq > sealed)).count() as u32
    }

    fn journal_put(&mut self, actor: u64, seq: u32, clock: u64, bytes: Vec<u8>) { self.journal.put(actor, seq, clock, bytes) }

    fn note(&mut self, c: Changes) {
        let m = &mut self.changes;
        m.pages.extend(c.pages);
        m.items.extend(c.items);
        m.page_props.extend(c.page_props);
        m.order |= c.order;
        m.name |= c.name;
        m.versions |= c.versions;
        m.applied |= c.applied;
        // Nobody read the changes for a long time: keep the lists bounded by the document's size.
        if m.items.len() > 8192 {
            for list in [&mut m.pages, &mut m.items, &mut m.page_props] {
                list.sort_unstable();
                list.dedup();
            }
        }
    }

    /// Asset hashes the document refers to: page backgrounds and images that are not deleted.
    pub fn referenced_assets(&self) -> Vec<Hash> {
        let mut out: Vec<Hash> = Vec::new();
        let mut add = |h: Hash| {
            if !out.contains(&h) {
                out.push(h);
            }
        };
        for page in self.doc.pages.values() {
            match page.background.v {
                Background::Pdf { asset, .. } | Background::Custom { asset } => add(asset),
                Background::Template => {}
            }
        }
        for item in self.doc.items.values() {
            if let (false, Body::Image { asset, .. }) = (item.deleted.v, &item.body.v) {
                add(*asset);
            }
        }
        out
    }

    /// The blob handle of a stored asset.
    pub fn asset_blob(&self, h: &Hash) -> Option<BlobHandle> { self.assets.iter().find(|a| a.0 == *h).map(|a| a.1) }

    /// Remembers the blob of an asset; false when it was known.
    fn asset_add(&mut self, h: Hash, blob: BlobHandle) -> bool {
        let new = self.asset_blob(&h).is_none();
        if new {
            self.assets.push((h, blob));
        }
        new
    }

    /// Decodes and applies queued remote batches until `budget_ms` of `now()` time is used. Returns true when the
    /// queue is empty.
    fn drain(&mut self, host: &mut dyn Host, budget_ms: f64) -> bool {
        let start = host.now();
        loop {
            if self.partial.is_none() {
                let Some(bytes) = self.queue.pop_front() else { return true };
                self.partial = Some(Partial { dec: BatchDecoder::new(&bytes).ok(), bytes, batch: None, at: 0 });
                if self.partial.as_ref().is_some_and(|p| p.dec.is_none()) {
                    self.partial = None; // damaged: dropped whole, the sender will offer it again
                    continue;
                }
            }
            let p = self.partial.as_mut().unwrap();
            if let Some(dec) = &mut p.dec {
                match dec.step(&p.bytes, SLICE_OPS) {
                    Ok(true) => p.batch = p.dec.take().and_then(BatchDecoder::finish),
                    Ok(false) => {}
                    Err(_) => self.partial = None,
                }
            } else if let Some(batch) = &mut p.batch {
                let end = (p.at + SLICE_OPS).min(batch.ops.len());
                let changes = self.doc.apply_part(batch, p.at..end);
                p.at = end;
                let done = end == batch.ops.len();
                self.note(changes);
                if done {
                    let p = self.partial.take().unwrap();
                    let b = p.batch.unwrap();
                    let wall = host.time() as u64;
                    if let Some(author) = &mut self.author {
                        author.clock.observe(b.max_clock(), wall);
                    }
                    self.queued_keys.retain(|k| *k != (b.actor.0, b.seq));
                    if self.persistent {
                        self.pending.ops_put(&self.doc_id, b.actor.0, b.seq, &p.bytes);
                    }
                    self.journal_put(b.actor.0, b.seq, b.clock, p.bytes);
                }
            } else {
                self.partial = None;
            }
            if host.now() - start >= budget_ms {
                return self.partial.is_none() && self.queue.is_empty();
            }
        }
    }

    /// The journal prefixes compaction may delete: `actor → through`.
    fn trimmable(&self, wall: u64) -> Through {
        let own = self.author.as_ref().map(|a| a.actor.inst());
        let (mut through, mut stopped, mut left) = (Through::new(), None, self.journal.bytes);
        for e in &self.journal.e {
            if stopped == Some(e.actor) {
                continue;
            }
            let old = e.clock.saturating_add(JOURNAL_MAX_AGE_MS) < wall;
            if self.doc.vv.contains(ActorId(e.actor), e.seq) && (old || self.covered.contains(ActorId(e.actor), e.seq)) {
                through_set(&mut through, e.actor, e.seq);
                left -= e.bytes.len();
            } else {
                stopped = Some(e.actor);
            }
        }
        // The 8 MiB cap: the oldest received batches go first, own ones last.
        while left > JOURNAL_MAX_BYTES {
            let (mut best, mut seen): (Option<(bool, u64, usize)>, Option<u64>) = (None, None);
            for (i, e) in self.journal.e.iter().enumerate() {
                // The head of each actor: its first entry beyond what is already trimmed.
                if seen == Some(e.actor) || through_of(&through, e.actor).is_some_and(|t| e.seq <= t) {
                    continue;
                }
                seen = Some(e.actor);
                let key = (Some(ActorId(e.actor).inst()) == own, e.clock);
                if best.is_none_or(|b| key < (b.0, b.1)) {
                    best = Some((key.0, key.1, i));
                }
            }
            let Some((_, _, i)) = best else { break };
            let e = &self.journal.e[i];
            through_set(&mut through, e.actor, e.seq);
            left -= e.bytes.len();
        }
        through
    }
}

// ---- Replica ------------------------------------------------------------------------------------------------------

/// The session as `ihn-sync` sees it.
pub struct Rep<'a> {
    s: &'a mut Session,
    host: &'a mut dyn Host,
    contact: bool,
}

impl Replica for Rep<'_> {
    fn doc_id(&self) -> &DocId { &self.s.doc_id }
    fn version_vector(&self) -> &VersionVector { &self.s.doc.vv }

    fn batches_since(&self, have: &VersionVector, max_bytes: usize) -> Result<Vec<EncodedBatch>, Compacted> {
        let (s, mut out, mut size) = (&*self.s, Vec::new(), 0);
        let pairs = s.doc.vv.iter().flat_map(|(a, top)| (have.get(a) + 1..=top).map(move |seq| (a, seq))).chain(s.doc.vv.extras());
        if pairs.filter(|&(a, seq)| !have.contains(a, seq)).any(|(a, seq)| !s.journal.has(a.0, seq)) {
            return Err(Compacted);
        }
        for e in &s.journal.e {
            if have.contains(ActorId(e.actor), e.seq) || !s.doc.vv.contains(ActorId(e.actor), e.seq) {
                continue;
            }
            if !out.is_empty() && size + e.bytes.len() > max_bytes {
                break;
            }
            size += e.bytes.len();
            out.push(EncodedBatch(e.bytes.clone()));
        }
        Ok(out)
    }

    fn receive(&mut self, b: &[EncodedBatch], _from: PeerRef) -> ReceiveResult {
        let (s, mut r) = (&mut *self.s, ReceiveResult::default());
        for eb in b {
            match eb.header() {
                Some((actor, seq, _)) if seq > 0 && !s.doc.vv.contains(actor, seq) && !s.queued_keys.contains(&(actor.0, seq)) => {
                    s.queued_keys.push((actor.0, seq));
                    s.queue.push_back(eb.0.clone());
                }
                _ => r.duplicate += 1,
            }
        }
        let mark = (s.changes.pages.len(), b.len() as u32 - r.duplicate);
        s.drain(self.host, if self.contact { CONTACT_BUDGET_MS } else { f64::INFINITY });
        r.queued = s.queue.len() as u32 + s.partial.is_some() as u32;
        r.applied = mark.1.saturating_sub(r.queued);
        for page in &s.changes.pages[mark.0..] {
            if !r.changed_pages.contains(page) {
                r.changed_pages.push(*page);
            }
        }
        r
    }

    fn snapshot(&self) -> EncodedSnapshot { snapshot::encode(&self.s.doc) }

    fn load_snapshot(&mut self, snap: &EncodedSnapshot, _from: PeerRef) -> ReceiveResult {
        let Ok(other) = snap.decode() else { return ReceiveResult::default() };
        let (s, wall) = (&mut *self.s, self.host.time() as u64);
        if let Some(author) = &mut s.author {
            author.clock.observe(other.max_clock(wall), wall);
        }
        let changes = s.doc.merge(&other);
        let pages = changes.pages.clone();
        s.note(Changes { applied: true, ..changes });
        ReceiveResult { applied: 1, duplicate: 0, queued: 0, changed_pages: pages }
    }

    fn missing_assets(&self) -> Vec<Hash> { self.s.referenced_assets().into_iter().filter(|h| self.s.asset_blob(h).is_none()).collect() }

    fn put_asset(&mut self, h: Hash, bytes: BlobHandle) {
        if self.s.asset_add(h, bytes) {
            self.s.new_assets.push((h, bytes, String::new(), -1.0));
        }
    }

    fn asset(&self, h: &Hash) -> Option<BlobHandle> { self.s.asset_blob(h) }
    fn can_edit(&self) -> bool { self.s.writable() }
    fn retain(&mut self, covered: &VersionVector) { self.s.covered.merge(covered) }
}

/// Lends the open session to the sync engine (or to `doc::open`) as a `Replica`, then writes what the calls
/// produced (journal entries, assets), announces changes and schedules the queue and compaction.
pub fn with_replica<R>(app: &mut App, host: &mut dyn Host, f: impl FnOnce(&mut dyn Replica) -> R) -> Option<R> {
    let mut s = app.st.doc_session.s.take()?;
    let contact = app.sched.is_busy();
    let r = f(&mut Rep { s: &mut s, host, contact });
    app.st.doc_session.s = Some(s);
    after(app, host);
    Some(r)
}

/// What remote batches and snapshots changed since the last call (the reader of `sig::CHANGED`).
pub fn take_changes(app: &mut App) -> Changes {
    let mut c = get_mut(app).map(|s| (std::mem::take(&mut s.announced), std::mem::take(&mut s.changes)).1).unwrap_or_default();
    for list in [&mut c.pages, &mut c.items, &mut c.page_props] {
        list.sort_unstable();
        list.dedup();
    }
    c
}

/// The local batches that became durable since the last call, in commit order (`SyncEngine::local_batch`).
pub fn drain_local(app: &mut App) -> Vec<EncodedBatch> { get_mut(app).map(|s| std::mem::take(&mut s.outbox)).unwrap_or_default() }

/// The sync engine's status (`collab` calls this whenever it changes); `None` detaches the engine.
pub fn set_sync_status(app: &mut App, host: &mut dyn Host, status: Option<SyncStatus>) {
    let Some(s) = get_mut(app) else { return };
    // Degraded storage: the error state lasts until every own batch is sealed (ARCHITECTURE §4.4).
    if status.as_ref().is_some_and(|st| st.unsealed_local == 0) {
        s.failed = false;
    }
    if s.sync != status {
        s.sync = status;
        shell::signal(app, host, sig::STATE, "");
    }
}

/// The status the save presentation uses: the engine's, or one derived from the persisted record.
pub fn sync_status(app: &App) -> SyncStatus {
    let Some(s) = get(app) else { return SyncStatus::default() };
    s.sync.clone().unwrap_or_else(|| SyncStatus {
        unsealed_local: s.unsealed_local(),
        not_created: !s.record.flags.created,
        export_pending: s.record.flags.export_pending,
        online: !shell::offline(app),
        auth_paused: auth::reconnecting(app),
        ..SyncStatus::default()
    })
}

fn after(app: &mut App, host: &mut dyn Host) {
    let Some(s) = get_mut(app) else { return };
    let mut tx = std::mem::take(&mut s.pending);
    for (hash, blob, mime, size) in std::mem::take(&mut s.new_assets) {
        if s.persistent {
            tx.asset_put(&hash, blob, &mime, size);
            tx.asset_ref(&hash, 1, false);
        }
    }
    // One signal per batch of changes: the next one follows `take_changes`.
    let changed = s.changes != Changes::default() && !std::mem::replace(&mut s.announced, true);
    let queued = !s.queue.is_empty() || s.partial.is_some();
    if !tx.is_empty() {
        let req = app.req(Mod::DocSession);
        store::tx(app, host, req, tx);
    }
    if queued {
        if app.sched.is_busy() {
            app.sched.frame(host.out(), Mod::DocSession);
        } else {
            app.sched.defer(W_DRAIN, 0.0, true);
        }
    }
    if changed {
        app.sched.defer(W_COMPACT, COMPACT_QUIET_MS, true);
        shell::signal(app, host, sig::CHANGED, "");
    }
}

// ---- write path ---------------------------------------------------------------------------------------------------

/// One user action: `f` fills the transaction; the batch is applied, journalled and (after the commit) handed to
/// the sync engine. `None` when there is no writable session or `f` added nothing.
pub fn edit(app: &mut App, host: &mut dyn Host, f: impl FnOnce(&mut Tx)) -> Option<Edit> {
    let wall = host.time() as u64;
    let degraded = store::degraded(app);
    let s = app.st.doc_session.s.as_mut().filter(|s| s.writable())?;
    let author = s.author.as_mut()?;
    let mut tx = Tx::new(&s.doc, author, wall);
    f(&mut tx);
    if tx.is_empty() {
        return None;
    }
    let batch = tx.commit();
    let inverse = s.doc.inverse(&batch);
    let changes = s.doc.apply(&batch);
    let encoded = snapshot::encode_batch(&batch);
    let (actor, seq) = (batch.actor.0, batch.seq);
    s.journal_put(actor, seq, batch.clock, encoded.0.clone());
    s.record.seq.insert(actor, seq);
    let mark = s.author.as_mut().and_then(|a| a.ids.persist_mark());
    if s.persistent && !degraded {
        let mut t = StoreTx::new();
        let with_record = !std::mem::replace(&mut s.record_stored, true);
        if with_record {
            t.doc_put(&s.doc_id, &s.record);
        }
        t.ops_append(&s.doc_id, actor, seq, &encoded.0);
        if let Some(mark) = mark {
            t.kv_put(&s.account, &kv::ctr(actor), &Value::from(mark as f64));
        }
        s.uncommitted += 1;
        let req = app.req(Mod::DocSession);
        app.st.doc_session.appends.push((req, vec![encoded], with_record));
        store::tx(app, host, req, t);
    } else {
        // Degraded storage: the in-memory journal holds the batch and the relay seal is the durability point.
        s.failed = true;
        s.outbox.push(encoded);
        shell::signal(app, host, sig::COMMITTED, "");
    }
    app.sched.defer(W_COMPACT, COMPACT_QUIET_MS, true);
    shell::signal(app, host, sig::STATE, "");
    Some(Edit { seq, inverse, changes })
}

fn appended(app: &mut App, host: &mut dyn Host, (batches, with_record): (Vec<EncodedBatch>, bool), io: &crate::abi::Io) {
    let time = host.time();
    let Some(s) = get_mut(app) else { return };
    let mut reply = Reply::new(io);
    s.uncommitted = s.uncommitted.saturating_sub(1);
    if reply.ok() {
        s.saved_at = time;
        // The `docs` record vanished (retention in another tab): the batch is stored, write the record again.
        if with_record {
            reply.skip();
        }
        reply.skip();
        if reply.flag() == Some(false) {
            let mut t = StoreTx::new();
            t.doc_put(&s.doc_id, &s.record);
            let req = app.req(Mod::DocSession);
            store::tx(app, host, req, t);
        }
    } else {
        s.failed = true;
    }
    let Some(s) = get_mut(app) else { return };
    s.outbox.extend(batches);
    shell::signal(app, host, sig::COMMITTED, "");
    shell::signal(app, host, sig::STATE, "");
}

/// Stores an asset of the open document (image bytes, a source PDF) and keeps its blob handle.
pub fn put_asset(app: &mut App, host: &mut dyn Host, hash: Hash, blob: BlobHandle, mime: &str, size: f64) {
    let Some(s) = get_mut(app) else { return };
    if s.asset_add(hash, blob) {
        s.new_assets.push((hash, blob, mime.into(), size));
    }
    after(app, host);
}

/// Sets the Drive facts of the open document (`doc::open` after the metadata arrived) and stores the record.
pub fn set_meta(app: &mut App, host: &mut dyn Host, name: &str, can_edit: bool, meta: Value, export: Option<(String, Vec<u8>)>) {
    let time = host.time();
    let Some(s) = get_mut(app) else { return };
    s.can_edit = can_edit;
    s.record.name = name.into();
    s.record.drive_meta = meta;
    s.record.last_opened_at = time;
    // Field patches, never the whole record: `seq` and `relay` belong to the tabs that append and seal. (Until the
    // record exists the patches do nothing and the compaction below writes it whole.)
    let mut t = StoreTx::new();
    t.doc_patch(&s.doc_id, &[(&["name"], Value::from(name)), (&["driveMeta"], s.record.drive_meta.clone()), (&["lastOpenedAt"], Value::from(time))]);
    t.persist(&s.account, &s.doc_id, &mut s.record, Record::Flags { relay_pending: None, export_pending: None, created: Some(true) });
    if let Some((head, vv)) = export {
        t.persist(&s.account, &s.doc_id, &mut s.record, Record::KnownExport { head, vv });
    }
    if s.persistent && s.record_stored {
        let req = app.req(Mod::DocSession);
        store::tx(app, host, req, t);
    }
    compact(app, host, true);
    shell::signal(app, host, sig::STATE, "");
}

/// A sync record of the open document (`SyncEffect::Persist`): written and mirrored in `Session::record`.
pub fn persist(app: &mut App, host: &mut dyn Host, r: Record) {
    let Some(s) = get_mut(app) else { return };
    let mut t = StoreTx::new();
    t.persist(&s.account, &s.doc_id, &mut s.record, r);
    if s.persistent {
        let req = app.req(Mod::DocSession);
        store::tx(app, host, req, t);
    }
    shell::signal(app, host, sig::STATE, "");
}

// ---- compaction ---------------------------------------------------------------------------------------------------

/// Folds dirty pages into chunks and trims the journal. Only the `ihn6-net` holder compacts; `force` is for the
/// tab that just downloaded the document (nothing is stored yet, so there is nothing to race with).
pub fn compact(app: &mut App, host: &mut dyn Host, force: bool) {
    let (wall, degraded, busy) = (host.time() as u64, store::degraded(app), app.st.doc_session.compact_req != 0);
    let Some(s) = get_mut(app) else { return };
    if !s.persistent || degraded || busy || s.partial.is_some() || !s.queue.is_empty() || !(s.net_holder || force) {
        return;
    }
    let (mut pages, header) = s.doc.take_dirty();
    if std::mem::take(&mut s.full_compact) {
        pages = s.doc.chunk_pages();
        pages.extend(s.chunks.iter().map(|c| c.0));
        pages.sort_unstable();
        pages.dedup();
    }
    let through = s.trimmable(wall);
    if pages.is_empty() && !header && through.is_empty() && s.record_stored {
        return;
    }
    let mut t = StoreTx::new();
    let mut vv = ihn_core::codec::Writer::default();
    s.doc.vv.encode(&mut vv);
    s.record.vv = vv.buf;
    if !std::mem::replace(&mut s.record_stored, true) {
        t.doc_put(&s.doc_id, &s.record);
    } else {
        t.doc_patch(&s.doc_id, &[(&["vv"], Value::from(&s.record.vv[..]))]);
    }
    t.chunk_put(&s.doc_id, 0, &snapshot::encode_header(&s.doc));
    for page in pages {
        let at = s.chunks.iter().position(|c| c.0 == page);
        if s.doc.chunk_items(page).is_empty() {
            if let Some(at) = at {
                t.chunk_del(&s.doc_id, s.chunks.swap_remove(at).1);
            }
        } else {
            let chunk = at.map(|at| s.chunks[at].1).unwrap_or_else(|| {
                s.next_chunk += 1;
                s.chunks.push((page, s.next_chunk - 1));
                s.next_chunk - 1
            });
            t.chunk_put(&s.doc_id, chunk, &snapshot::encode_chunk(&s.doc, page));
        }
    }
    for &(actor, seq) in &through {
        t.ops_trim(&s.doc_id, actor, seq);
    }
    let journal = &mut s.journal;
    let mut freed = 0;
    journal.e.retain(|e| {
        let keep = through_of(&through, e.actor).is_none_or(|t| e.seq > t);
        if !keep {
            freed += e.bytes.len();
        }
        keep
    });
    journal.bytes -= freed;
    let req = app.req(Mod::DocSession);
    app.st.doc_session.compact_req = req;
    store::tx(app, host, req, t);
}

// ---- load and close -----------------------------------------------------------------------------------------------

/// Ends the open session: releases the network lock and the asset handles.
pub fn close(app: &mut App, host: &mut dyn Host) {
    let st = &mut app.st.doc_session;
    st.load = None;
    let net = std::mem::take(&mut st.net_req);
    if net != 0 {
        locks::release(host, net);
    }
    let Some(s) = st.s.take() else { return };
    for (_, blob) in s.assets {
        host.out().obj_release(blob);
    }
    app.sched.cancel(W_COMPACT);
    app.sched.cancel(W_DRAIN);
    shell::signal(app, host, sig::CLOSED, &s.doc_id);
}

/// Opens the session of `doc_id` from the store. `notify` (a request id of the caller's module) completes with
/// [`LOADED_EMPTY`] or [`LOADED_LOCAL`]; never inside this call. With an empty `account` the session is
/// memory-only.
pub fn load(app: &mut App, host: &mut dyn Host, account: &str, doc_id: &str, notify: u32) {
    // Leaving a document that still has a dirty page: one last compaction by the holder.
    compact(app, host, false);
    close(app, host);
    let mut l = Load { account: account.into(), doc_id: doc_id.into(), notify, slot: None, slot_asked: false, req: app.req(Mod::DocSession) };
    if account.is_empty() {
        l.req = T_LATER;
        app.sched.timer(host.out(), T_LATER, 0.0);
    } else {
        locks::acquire_slot(app, host, account, l.req);
    }
    app.st.doc_session.load = Some(l);
}

/// A session for a document created on this device (`doc::create`): empty, writable at once, not in Drive yet.
pub fn begin_new(app: &mut App, host: &mut dyn Host, account: &str, doc_id: &str, notify: u32) { load(app, host, account, doc_id, notify) }

fn read_store(app: &mut App, host: &mut dyn Host) {
    let req = app.req(Mod::DocSession);
    let Some(l) = app.st.doc_session.load.as_mut() else { return };
    let mut random = [0u8; 8];
    host.random(&mut random);
    let inst = ActorId::inst_from_random(u64::from_le_bytes(random));
    let mut t = StoreTx::new();
    t.kv_put_if_absent(&l.account, kv::INST, &Value::from(hex16(inst)));
    t.kv_scan(&l.account);
    t.doc_get(&l.doc_id);
    t.chunk_scan(&l.doc_id);
    t.ops_scan(&l.doc_id);
    l.req = req;
    store::tx(app, host, req, t);
}

fn loaded(app: &mut App, host: &mut dyn Host, io: Option<&crate::abi::Io>) {
    let Some(l) = app.st.doc_session.load.take() else { return };
    let (wall, mut s) = (host.time() as u64, Session::new(&l.account, &l.doc_id));
    let (mut inst, mut mark, mut local) = (0u64, 0u64, false);
    let mut reply = io.map(Reply::new).filter(Reply::ok);
    if let Some(reply) = &mut reply {
        s.persistent = true;
        reply.skip();
        let kv_rows = reply.rows();
        let find = |key: &str| kv_rows.iter().find(|(k, _)| k.items().get(1).and_then(Value::as_str) == Some(key)).map(|(_, v)| v);
        inst = find(kv::INST).and_then(Value::as_str).and_then(unhex16).unwrap_or(0);
        if let Some(slot) = l.slot {
            mark = find(&kv::ctr(locks::actor(inst, slot))).and_then(Value::as_f64).unwrap_or(0.0) as u64;
        }
        if let Some(record) = reply.doc().filter(|r| r.account == l.account) {
            s.record = record;
            s.record_stored = true;
            s.can_edit = s.record.drive_meta.get("canEdit").as_bool().unwrap_or(true);
        }
        let mut loader = Loader::default();
        let mut ok = true;
        let chunks = reply.chunks();
        for &(id, bytes) in &chunks {
            ok &= if id == 0 { loader.header(bytes).is_ok() } else { loader.chunk(bytes).is_ok() };
            if let (true, Some(page)) = (id != 0, bytes.get(1..17)) {
                s.chunks.push((Id(u128::from_le_bytes(page.try_into().unwrap())), id));
                s.next_chunk = s.next_chunk.max(id + 1);
            }
        }
        // A damaged chunk: start from the journal alone and rewrite every chunk at the next compaction.
        s.doc = if ok { loader.finish() } else { Doc::default() };
        s.full_compact = !ok;
        local = ok && !chunks.is_empty();
        for (actor, seq, bytes) in reply.ops() {
            if let Ok(b) = snapshot::decode_batch(bytes) {
                s.journal_put(actor, seq, b.clock, bytes.to_vec());
                s.doc.apply_owned(b);
                local = true;
            }
        }
        // Dirty flags of the replay stay set: the next compaction folds the journal into the chunks.
    }
    if inst == 0 {
        let mut random = [0u8; 8];
        host.random(&mut random);
        inst = ActorId::inst_from_random(u64::from_le_bytes(random));
    }
    // Without the store the session cannot know its id high-water mark or seq: it takes a fresh install id, so
    // nothing it issues can collide with an earlier session.
    let slot = if s.persistent { l.slot } else { Some(0) };
    if let Some(slot) = slot {
        let actor = ActorId::tab(inst, slot);
        let seq = s.record.seq.get(actor.0).copied().unwrap_or(0).max(s.doc.vv.get(actor));
        s.author = Some(Author::resume(IdGen::new(actor, mark), seq, &s.doc, wall));
    }
    let wanted: Vec<Hash> = s.referenced_assets().into_iter().collect();
    let (persistent, stored) = (s.persistent, s.record_stored);
    app.sched.new_session();
    app.st.doc_session.s = Some(s);
    if persistent {
        let mut t = StoreTx::new();
        if stored {
            t.doc_patch(&l.doc_id, &[(&["lastOpenedAt"], Value::from(host.time()))]);
        }
        wanted.iter().for_each(|h| t.asset_get(h));
        let req = app.req(Mod::DocSession);
        app.st.doc_session.asset_req = req;
        app.st.doc_session.asset_wanted = wanted;
        // The asset handles arrive before `notify`: the caller sees the session with its stored assets.
        app.st.doc_session.later.push((l.notify, if local { LOADED_LOCAL } else { LOADED_EMPTY }));
        store::tx(app, host, req, t);
        let net = app.req(Mod::DocSession);
        app.st.doc_session.net_req = net;
        locks::request(host, net, &locks::net_name(&l.account, &l.doc_id), false);
    } else {
        shell::deliver(app, host, l.notify, if local { LOADED_LOCAL } else { LOADED_EMPTY }, 0, &[]);
    }
}

fn assets_read(app: &mut App, host: &mut dyn Host, io: &crate::abi::Io) {
    let st = &mut app.st.doc_session;
    let (wanted, mut reply) = (std::mem::take(&mut st.asset_wanted), Reply::new(io));
    if let (true, Some(s)) = (reply.ok(), st.s.as_mut()) {
        if s.record_stored {
            reply.skip();
        }
        for hash in wanted {
            let record = reply.value();
            let mut handles = Vec::new();
            record.get("blob").handles(&mut handles);
            if let Some(&blob) = handles.first() {
                s.asset_add(hash, blob);
            }
        }
    }
    for (notify, status) in std::mem::take(&mut app.st.doc_session.later) {
        shell::deliver(app, host, notify, status, 0, &[]);
    }
}

// ---- module contract ----------------------------------------------------------------------------------------------

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(_app: &mut App, _host: &mut dyn Host) {}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Timer(T_LATER) => loaded(app, host, None),
        Event::Work { id: W_COMPACT, stale: false } => compact(app, host, false),
        Event::Work { id: W_DRAIN, stale: false } | Event::Frame(_) => {
            let budget = if app.sched.is_busy() { CONTACT_BUDGET_MS } else { IDLE_BUDGET_MS };
            if let Some(s) = get_mut(app) {
                s.drain(host, budget);
            }
            after(app, host);
        }
        Event::Io(io) => route_io(app, host, &io),
        Event::Msg(m) if m.chan == shell::sig::SIGNED_OUT => close(app, host),
        _ => {}
    }
    0
}

fn route_io(app: &mut App, host: &mut dyn Host, io: &crate::abi::Io) {
    let st = &mut app.st.doc_session;
    if let Some(l) = st.load.as_mut().filter(|l| l.req == io.req) {
        if !std::mem::replace(&mut l.slot_asked, true) {
            // The slot answer: 0..=255, or EXHAUSTED / UNAVAILABLE (read-only session).
            l.slot = u8::try_from(io.status).ok();
            read_store(app, host);
        } else {
            loaded(app, host, Some(io));
        }
    } else if let Some(at) = st.appends.iter().position(|a| a.0 == io.req) {
        let (_, batches, with_record) = st.appends.swap_remove(at);
        appended(app, host, (batches, with_record), io);
    } else if io.req == st.compact_req {
        st.compact_req = 0;
        if io.status < 0 {
            if let Some(s) = st.s.as_mut() {
                s.full_compact = true;
            }
        }
    } else if io.req == st.asset_req {
        st.asset_req = 0;
        assets_read(app, host, io);
    } else if io.req == st.net_req && io.status == locks::GRANTED {
        if let Some(s) = st.s.as_mut() {
            s.net_holder = true;
        }
        app.sched.defer(W_COMPACT, COMPACT_QUIET_MS, true);
    }
}

/// `__IHN_TEST_API__.cmd('doc.session.<name>', args)`:
/// * `state` → `{docId, account, pages, items, live, name, vv, snapshot (FNV-1a of the canonical bytes), canEdit,
///   writable, persistent, netHolder, journal, uncommitted, failed, assets, missing}` or `{docId: ""}`;
/// * `fixture` → base64 of the snapshot of a small deterministic document (three pages: template, agenda, photo;
///   two strokes and an image) — the e2e spec wraps it into a v6 container PDF;
/// * `stroke` → appends one pen stroke to the first page (the write path); `compact` forces a compaction.
pub fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    match name {
        "state" => Some(state_json(app)),
        "fixture" => Some(json_str(&ihn_core::codec::b64_encode(&snapshot::encode(&fixture()).0, false))),
        "stroke" => {
            let page = get(app)?.doc.order().first().copied()?;
            let done = edit(app, host, |tx| _ = tx.add_stroke(page, pen(&[10.0, 10.0, 60.0, 80.0, 120.0, 40.0]))).is_some();
            Some(done.to_string())
        }
        "compact" => {
            compact(app, host, true);
            Some("true".into())
        }
        _ => None,
    }
}

fn pen(xy: &[f64]) -> Body { Body::Stroke { tool: ihn_core::op::Tool::Pen, color: ihn_core::op::Color::Hex(0x1a1a1a), width: 2.0, xy: xy.to_vec(), p: None } }

/// Hash of the fixture's photo asset (the bytes are the e2e spec's; the model only names them).
pub const FIXTURE_ASSET: Hash = [0x6a; 32];

/// The deterministic document of `doc.session.fixture`.
pub fn fixture() -> Doc {
    use ihn_core::op::{ImageRect, Template};
    let mut doc = Doc::default();
    let mut author = Author::new(IdGen::new(ActorId::tab(0x0f1e_2d3c_4b5a_6900, 0), 0), 0);
    let mut tx = Tx::new(&doc, &mut author, 1_793_354_400_000);
    let first = tx.page_new(None, Template::Default, (794, 1123), Background::Template);
    let second = tx.page_new(Some(first), Template::Agenda, (794, 1123), Background::Template);
    tx.page_new(Some(second), Template::Default, (794, 1123), Background::Custom { asset: FIXTURE_ASSET });
    tx.add_stroke(first, pen(&[100.0, 100.0, 150.0, 180.0, 220.0, 140.0, 300.0, 260.0]));
    tx.add_stroke(second, Body::Stroke { tool: ihn_core::op::Tool::Highlighter, color: ihn_core::op::Color::Hex(0xffe600), width: 16.0, xy: vec![80.0, 300.0, 400.0, 300.0], p: None });
    tx.add_image(first, FIXTURE_ASSET, ImageRect { cx: 400.0, cy: 600.0, w: 200.0, h: 120.0, rotation: 0.0 });
    tx.set_name("Fixture");
    let b = tx.commit();
    doc.apply(&b);
    doc
}

fn state_json(app: &App) -> String {
    let Some(s) = get(app) else { return "{\"docId\":\"\"}".into() };
    let snap = snapshot::encode(&s.doc).0;
    let mut vv = ihn_core::codec::Writer::default();
    s.doc.vv.encode(&mut vv);
    let missing = s.referenced_assets().iter().filter(|h| s.asset_blob(h).is_none()).count();
    format!(
        "{{\"docId\":{},\"account\":{},\"pages\":{},\"items\":{},\"live\":{},\"name\":{},\"vv\":{},\"snapshot\":\"{:032x}\",\"snapshotBytes\":{},\"canEdit\":{},\"writable\":{},\"persistent\":{},\"netHolder\":{},\"journal\":{},\"uncommitted\":{},\"failed\":{},\"assets\":{},\"missing\":{}}}",
        json_str(&s.doc_id), json_str(&s.account), s.doc.order().len(), s.doc.items.len(), s.doc.items.values().filter(|i| i.is_live()).count(), json_str(&s.doc.name.v),
        json_str(&ihn_core::codec::b64_encode(&vv.buf, false)), ihn_core::id::fnv1a128(&snap), snap.len(), s.can_edit, s.writable(), s.persistent, s.net_holder, s.journal.e.len(), s.uncommitted, s.failed, s.assets.len(), missing
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{kind, OutBuf};
    use crate::doc::save_state::{self, Phase};
    use crate::host::{Ask, FakeHost, IntlOp, LsOp, Metric, Rect};
    use crate::store::fake::{boot, MemDb};
    use crate::store::status;
    use ihn_core::codec::Writer;
    use ihn_core::op::Template;

    const ACCOUNT: &str = "ana@example.test";
    const DOC: &str = "drive-file-1";
    const NOTIFY: u32 = tag(Mod::DocOpen, 0x77);
    const A4: (u32, u32) = (794, 1123);

    /// A host whose `performance.now()` advances by `step` at every reading (the remote-apply budget needs time
    /// to pass while Rust runs).
    struct Ticking<'a> {
        inner: &'a mut FakeHost,
        step: f64,
        clock: std::cell::Cell<f64>,
    }
    impl Host for Ticking<'_> {
        fn out(&mut self) -> &mut OutBuf { &mut self.inner.out }
        fn flush(&mut self) {}
        fn rect(&mut self, h: crate::abi::Node) -> Rect { self.inner.rect(h) }
        fn metric(&mut self, h: crate::abi::Node, m: Metric) -> f64 { self.inner.metric(h, m) }
        fn now(&self) -> f64 {
            self.clock.set(self.clock.get() + self.step);
            self.clock.get()
        }
        fn time(&self) -> f64 { self.inner.time }
        fn random(&mut self, buf: &mut [u8]) { self.inner.random(buf) }
        fn ls(&mut self, op: LsOp, key: &str, val: Option<&str>) -> Option<String> { self.inner.ls(op, key, val) }
        fn intl(&mut self, op: IntlOp, input: &str) -> String { self.inner.intl(op, input) }
        fn log(&mut self, level: u32, msg: &str) { self.inner.log(level, msg) }
        fn ask(&mut self, q: Ask, input: &str) -> String { self.inner.ask(q, input) }
    }

    fn open(app: &mut App, host: &mut FakeHost, db: &mut MemDb) {
        load(app, host, ACCOUNT, DOC, NOTIFY);
        db.pump(app, host);
        assert!(get(app).is_some(), "the session is loaded");
    }
    fn stroke(app: &mut App, host: &mut dyn Host, x: f64) -> Edit {
        let page = get(app).unwrap().doc.order()[0];
        edit(app, host, |tx| _ = tx.add_stroke(page, pen(&[x, 10.0, x + 40.0, 80.0]))).expect("a writable session")
    }
    fn first_page(app: &mut App, host: &mut FakeHost) {
        edit(app, host, |tx| _ = tx.page_new(None, Template::Default, A4, Background::Template)).expect("a writable session");
    }
    fn snap(app: &App) -> Vec<u8> { snapshot::encode(&get(app).unwrap().doc).0 }
    fn work(app: &mut App, host: &mut FakeHost) {
        host.advance(5000.0);
        let mut w = Writer::default();
        w.u32(0xff00_0000);
        app.event(host, kind::TIMER, &w.buf);
    }
    /// `n` batches of another device: a page, then strokes on it.
    fn remote(n: usize) -> Vec<EncodedBatch> {
        let mut doc = Doc::default();
        let mut author = Author::new(IdGen::new(ActorId::tab(0x0bad_cafe_0000_0200, 3), 0), 0);
        let mut out = Vec::new();
        let mut page = Id::NONE;
        for i in 0..n {
            let mut tx = Tx::new(&doc, &mut author, 1_793_354_400_000 + i as u64);
            if i == 0 {
                page = tx.page_new(None, Template::Default, A4, Background::Template);
            } else {
                for k in 0..40 {
                    tx.add_stroke(page, pen(&[i as f64, k as f64, 50.0, 60.0, 70.0, 80.0]));
                }
            }
            let b = tx.commit();
            doc.apply(&b);
            out.push(snapshot::encode_batch(&b));
        }
        out
    }

    #[test]
    fn a_local_batch_is_journalled_before_the_sync_engine_sees_it() {
        let (mut app, mut host, mut db) = boot();
        open(&mut app, &mut host, &mut db);
        let s = get(&app).unwrap();
        assert!(s.persistent && s.writable() && s.net_holder && s.doc.order().is_empty());
        host.out.w.buf.clear();
        first_page(&mut app, &mut host);
        // Applied at once (the tool renders in the same frame), not yet durable, not yet for the network.
        assert_eq!(get(&app).unwrap().doc.order().len(), 1);
        assert!(local(&app).pending && drain_local(&mut app).is_empty());
        assert_eq!(save_state::phase(&app), Phase::Saving);
        assert_eq!(db.dbs.get(store::DB_NAME).map_or(0, |stores| stores[2].len()), 0, "nothing in `ops` before the transaction ran");
        db.pump(&mut app, &mut host);
        let stores = &db.dbs[store::DB_NAME];
        assert_eq!((stores[0].len(), stores[2].len()), (1, 1), "the record and the batch, one transaction");
        let record = DocRecord::from_value(&stores[0][0].1);
        let actor = get(&app).unwrap().author.as_ref().unwrap().actor.0;
        assert_eq!((record.account.as_str(), record.seq.get(actor)), (ACCOUNT, Some(&1)));
        let out = drain_local(&mut app);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].header().map(|h| (h.0 .0, h.1)), Some((actor, 1)));
        assert!(!local(&app).pending && local(&app).saved_at > 0.0);
        // Not sealed in the relay yet: "waiting to upload", never "saved".
        assert_eq!(save_state::phase(&app), Phase::Paused);
        // Several edits keep their order in the outbox.
        stroke(&mut app, &mut host, 1.0);
        stroke(&mut app, &mut host, 2.0);
        db.pump(&mut app, &mut host);
        assert_eq!(drain_local(&mut app).iter().map(|b| b.header().unwrap().1).collect::<Vec<_>>(), [2, 3]);
    }

    #[test]
    fn a_reload_restores_the_document_from_chunks_and_journal_and_resumes_the_writer() {
        let (mut app, mut host, mut db) = boot();
        open(&mut app, &mut host, &mut db);
        first_page(&mut app, &mut host);
        let first = stroke(&mut app, &mut host, 1.0);
        db.pump(&mut app, &mut host);
        compact(&mut app, &mut host, false);
        db.pump(&mut app, &mut host);
        assert!(db.dbs[store::DB_NAME][1].len() >= 2, "header chunk and one page chunk");
        assert_eq!(db.dbs[store::DB_NAME][2].len(), 2, "nothing covers the journal yet: entries stay");
        stroke(&mut app, &mut host, 2.0); // journal only
        db.pump(&mut app, &mut host);
        let (before, ids): (Vec<u8>, usize) = (snap(&app), get(&app).unwrap().doc.items.len());
        close(&mut app, &mut host);
        db.pump(&mut app, &mut host);
        assert!(get(&app).is_none());
        open(&mut app, &mut host, &mut db);
        assert_eq!(snap(&app), before, "chunks + journal replay give the same canonical bytes");
        let s = get(&app).unwrap();
        assert_eq!((s.author.as_ref().unwrap().seq, s.journal.e.len()), (3, 3));
        // The next batch continues seq, ids and the clock.
        let next = stroke(&mut app, &mut host, 3.0);
        assert_eq!(next.seq, 4);
        assert!(next.seq > first.seq && get(&app).unwrap().doc.items.len() == ids + 1, "a fresh id, not a reissued one");
        db.pump(&mut app, &mut host);
        // Retention: covered entries leave the journal at the next compaction.
        let vv = get(&app).unwrap().doc.vv.clone();
        with_replica(&mut app, &mut host, |r| r.retain(&vv));
        compact(&mut app, &mut host, false);
        db.pump(&mut app, &mut host);
        assert_eq!((db.dbs[store::DB_NAME][2].len(), get(&app).unwrap().journal.e.len()), (0, 0));
        let have = VersionVector::default();
        assert_eq!(with_replica(&mut app, &mut host, |r| r.batches_since(&have, 1 << 20)), Some(Err(Compacted)));
        assert_eq!(with_replica(&mut app, &mut host, |r| r.batches_since(&vv, 1 << 20)), Some(Ok(Vec::new())));
        // And the compacted state alone reloads.
        let before = snap(&app);
        close(&mut app, &mut host);
        open(&mut app, &mut host, &mut db);
        assert_eq!(snap(&app), before);
    }

    #[test]
    fn degraded_storage_keeps_batches_in_memory_and_shows_the_error_until_they_are_sealed() {
        // The database cannot be opened at all.
        let (mut app, mut host, mut db) = boot();
        db.fail_open = Some(status::BLOCKED);
        open(&mut app, &mut host, &mut db);
        assert!(store::degraded(&app) && !get(&app).unwrap().persistent && get(&app).unwrap().writable());
        first_page(&mut app, &mut host);
        assert_eq!(drain_local(&mut app).len(), 1, "sent at once: the relay seal is the durability point");
        assert_eq!((local(&app).failed, save_state::phase(&app)), (true, Phase::Error));
        let have = VersionVector::default();
        assert_eq!(with_replica(&mut app, &mut host, |r| r.batches_since(&have, 1 << 20).map(|b| b.len())), Some(Ok(1)), "the in-memory journal serves peers");
        set_sync_status(&mut app, &mut host, Some(SyncStatus { unsealed_local: 1, online: true, ..SyncStatus::default() }));
        assert_eq!(save_state::phase(&app), Phase::Error);
        set_sync_status(&mut app, &mut host, Some(SyncStatus { online: true, ..SyncStatus::default() }));
        assert_eq!(save_state::phase(&app), Phase::Saved);

        // The database works, then a write keeps failing with the quota (the store retried three times).
        let (mut app, mut host, mut db) = boot();
        open(&mut app, &mut host, &mut db);
        first_page(&mut app, &mut host);
        db.pump(&mut app, &mut host);
        drain_local(&mut app);
        db.fail_writes.extend([status::QUOTA; 4]);
        stroke(&mut app, &mut host, 1.0);
        db.pump(&mut app, &mut host);
        assert!(store::degraded(&app));
        assert_eq!((drain_local(&mut app).len(), local(&app).failed, save_state::phase(&app)), (1, true, Phase::Error));
        // While degraded nothing is attempted: the next batch goes straight to the engine.
        let transactions = db.transactions;
        stroke(&mut app, &mut host, 2.0);
        assert_eq!((drain_local(&mut app).len(), db.transactions), (1, transactions));
        assert_eq!(get(&app).unwrap().journal.e.len(), 3);
    }

    #[test]
    fn remote_batches_are_applied_at_once_but_budgeted_while_a_pointer_is_in_contact() {
        let (mut app, mut host, mut db) = boot();
        open(&mut app, &mut host, &mut db);
        let batches = remote(30);
        // No contact: everything is decoded and applied in the call.
        let r = with_replica(&mut app, &mut host, |r| r.receive(&batches[..10], PeerRef::Tab)).unwrap();
        assert_eq!((r.applied, r.duplicate, r.queued, r.changed_pages.len()), (10, 0, 0, 1));
        assert_eq!(take_changes(&mut app).pages.len(), 1);
        db.pump(&mut app, &mut host);
        assert_eq!(db.dbs[store::DB_NAME][2].len(), 10, "received batches are journalled");
        // Contact: half a millisecond per call, the rest waits.
        app.sched.set_busy(Mod::EditorInput, true);
        let mut ticking = Ticking { inner: &mut host, step: 0.2, clock: Default::default() };
        let r = with_replica(&mut app, &mut ticking, |r| r.receive(&batches[8..], PeerRef::Peer(7))).unwrap();
        assert_eq!(r.duplicate, 2);
        assert!(r.queued >= 15 && r.applied + r.queued == 20, "{r:?}");
        let applied = get(&app).unwrap().doc.vv.get(ActorId::tab(0x0bad_cafe_0000_0200, 3));
        assert_eq!(applied, 10 + r.applied);
        // A redelivery of queued batches is a duplicate, not a second queue entry.
        let again = with_replica(&mut app, &mut ticking, |r| r.receive(&batches[25..], PeerRef::Peer(7))).unwrap();
        assert_eq!(again.duplicate, 5);
        // Each frame applies a little more.
        let before = get(&app).unwrap().queue.len();
        app.frame(&mut ticking, 16.0);
        let after = get(&app).unwrap().queue.len();
        assert!(after < before && after > 0, "{before} → {after}");
        // Pen up: the queue drains in idle slices of the scheduler.
        app.sched.set_busy(Mod::EditorInput, false);
        app.frame(&mut ticking, 32.0);
        assert!(get(&app).unwrap().queue.len() < after, "the frame after pen-up applies an idle slice");
        for _ in 0..40 {
            work(&mut app, &mut host);
        }
        let s = get(&app).unwrap();
        assert!(s.queue.is_empty() && s.partial.is_none());
        assert_eq!(s.doc.vv.get(ActorId::tab(0x0bad_cafe_0000_0200, 3)), 30);
        assert_eq!(s.doc.items.len(), 29 * 40);
        db.pump(&mut app, &mut host);
        assert_eq!(db.dbs[store::DB_NAME][2].len(), 30);
        // The journal answers a peer that has the first twenty.
        let mut have = VersionVector::default();
        have.raise(ActorId::tab(0x0bad_cafe_0000_0200, 3), 20);
        assert_eq!(with_replica(&mut app, &mut host, |r| r.batches_since(&have, 1 << 24).map(|b| b.len())), Some(Ok(10)));
        // A damaged batch is dropped whole.
        let r = with_replica(&mut app, &mut host, |r| r.receive(&[EncodedBatch(vec![1, 2, 3])], PeerRef::Tab)).unwrap();
        assert_eq!((r.applied, r.duplicate), (0, 1));
    }

    #[test]
    fn snapshots_merge_and_a_second_tab_does_not_compact() {
        let (mut app, mut host, mut db) = boot();
        // Another tab holds the document's network lock.
        db.held.insert(locks::net_name(ACCOUNT, DOC), 0);
        open(&mut app, &mut host, &mut db);
        assert!(!get(&app).unwrap().net_holder);
        first_page(&mut app, &mut host);
        let snapshot = snapshot::encode(&fixture());
        let r = with_replica(&mut app, &mut host, |r| (r.load_snapshot(&snapshot, PeerRef::Export), r.missing_assets())).unwrap();
        assert_eq!((r.0.applied, r.1), (1, vec![FIXTURE_ASSET]));
        assert_eq!(get(&app).unwrap().doc.order().len(), 4, "merged, never replaced");
        db.pump(&mut app, &mut host);
        compact(&mut app, &mut host, false);
        db.pump(&mut app, &mut host);
        assert_eq!(db.dbs[store::DB_NAME][1].len(), 0, "only the lock holder writes chunks");
        // The journal still holds the own batch; the next holder folds it.
        assert_eq!(db.dbs[store::DB_NAME][2].len(), 1);
    }

    #[test]
    fn the_journal_cap_drops_the_oldest_received_batches_first() {
        let mut s = Session::new(ACCOUNT, DOC);
        s.author = Some(Author::new(IdGen::new(ActorId::tab(0x1100, 1), 0), 0));
        let big = vec![0u8; 3 << 20];
        for (actor, seq, clock) in [(0x1101u64, 1u32, 50u64), (0x2200, 1, 10), (0x2200, 2, 30), (0x3300, 1, 20)] {
            s.doc.vv.add(ActorId(actor), seq);
            s.journal_put(actor, seq, clock, big.clone());
        }
        // 12 MiB with a cap of 8: the two oldest received batches go, the own one stays.
        let t = s.trimmable(60);
        assert_eq!(t, [(0x2200, 1), (0x3300, 1)]);
        // Age: everything older than a day goes, whoever wrote it.
        let t = s.trimmable(40 + JOURNAL_MAX_AGE_MS);
        assert_eq!(t, [(0x2200, 2), (0x3300, 1)]);
    }
}
