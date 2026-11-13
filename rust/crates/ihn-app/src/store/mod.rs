//! Store: IndexedDB schema, transactions as effects, kv, assets, Web Locks, retention, 5.x dumper (ARCHITECTURE §7).
//!
//! # Using it
//! ```ignore
//! let mut t = store::Tx::new();
//! t.ops_append(doc, actor, seq, &batch);            // + docs.seq[actor], same transaction
//! let req = app.req(Mod::DocSession);
//! store::tx(app, host, req, t);                     // → Event::Io { req, status, data } at DocSession::event
//! // …
//! let mut reply = store::Reply::new(&io);           // one result per operation, in order
//! ```
//! Every transaction gets exactly one completion, never inside the `store::tx` call. The database
//! (`inhouse-notes-v6`, [`schema`]) is opened by the first transaction; transactions issued while it opens wait.
//!
//! # Degraded mode (ARCHITECTURE §4.4, SYNC §7.2)
//! The store is **degraded** when the open is blocked for 1.5 s, takes more than 5 s or fails, or when a write
//! still hits the quota after 3 retries (250 ms × 2 backoff; later writes wait behind the retry, so the journal
//! order is kept). From then on [`health`] reports `Degraded` and every write completes with
//! [`status::DEGRADED`] without being attempted (reads still run when the database is open). The session keeps
//! such batches in memory; [`reopen`] tries again (document open, user retry).
//!
//! Wire formats: [`tx`] (operations), [`sv`] (keys and values); executor `host/idb.js`.

pub mod locks;
pub mod schema;
mod selftest;
pub mod sv;
pub mod tx;
pub mod v5;

#[cfg(test)]
pub mod fake;

pub use schema::{DocRecord, Record};
pub use sv::Value;
pub use tx::{status, Key, Put, Range, Reply, Store, Tx, STORES};

use crate::abi::{fam, tag, tag_mod, Io};
use crate::app::{Mod, MODULES};
use crate::{App, Event, Host};
use std::collections::VecDeque;

pub const DB_NAME: &str = "inhouse-notes-v6";
pub const DB_VERSION: u32 = 1;
/// 5.18.0 `INDEXED_DB_BLOCKED_TIMEOUT_MS` and its open timeout.
// v5: app-v5.js:INDEXED_DB_BLOCKED_TIMEOUT_MS, INDEXED_DB_OPEN_TIMEOUT_MS
pub const BLOCKED_MS: f64 = 1500.0;
pub const OPEN_TIMEOUT_MS: f64 = 5000.0;
/// Retries of a failed write (quota) and the first backoff, doubled each time (SYNC §7.2).
pub const RETRIES: u32 = 3;
pub const RETRY_MS: f64 = 250.0;

/// Effect ops of families 3.2 (IndexedDB) and 3.9 (Web Locks); payloads are documented where they are written.
pub(crate) mod fx {
    /// `u32 req, u32 db, str name, u32 version, f64 blocked_ms, f64 timeout_ms, u32 n, n × str store`.
    pub const OPEN: u8 = 0x20;
    /// `u32 db`.
    pub const CLOSE: u8 = 0x21;
    /// `u32 req, u32 db, u8 flags (1 write, 2 strict), u32 n, n × operation`.
    pub const TX: u8 = 0x22;
    /// `u32 req, str name, f64 blocked_ms`.
    pub const DELETE_DB: u8 = 0x23;
    /// `u32 req, str name, str fields` (comma-separated).
    pub const V5_INFO: u8 = 0x24;
    /// `u32 req, str name, str store, value after, u32 limit, f64 max_bytes`.
    pub const V5_READ: u8 = 0x25;
    /// `u32 req, str name, u8 flags (1 ifAvailable, 2 shared)`.
    pub const LOCK_REQUEST: u8 = 0x80;
    /// `u32 req`.
    pub const LOCK_RELEASE: u8 = 0x81;
    /// `u32 req, str prefix`.
    pub const LOCK_QUERY: u8 = 0x82;
}

/// Why the store is degraded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cause {
    /// Another connection blocked the open for [`BLOCKED_MS`].
    Blocked,
    /// The open did not finish within [`OPEN_TIMEOUT_MS`].
    Timeout,
    /// No IndexedDB in this context.
    Unavailable,
    /// The open failed.
    Error,
    /// Writes keep failing with `QuotaExceededError`; the database is open and readable.
    Quota,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Health {
    /// Not opened yet (the first transaction opens it) or the connection was lost.
    #[default]
    Closed,
    Opening,
    Ready,
    Degraded(Cause),
}

impl Health {
    pub fn name(self) -> &'static str {
        match self {
            Health::Closed => "closed",
            Health::Opening => "opening",
            Health::Ready => "ready",
            Health::Degraded(Cause::Blocked) => "degraded:blocked",
            Health::Degraded(Cause::Timeout) => "degraded:timeout",
            Health::Degraded(Cause::Unavailable) => "degraded:unavailable",
            Health::Degraded(Cause::Error) => "degraded:error",
            Health::Degraded(Cause::Quota) => "degraded:quota",
        }
    }
}

struct Job {
    /// The caller's request id: where the completion goes.
    caller: u32,
    tx: Tx,
    /// Times this job was sent again (quota backoff, lost connection).
    attempt: u32,
}

struct Conn {
    /// Host handle of the connection.
    id: u32,
    name: String,
    version: u32,
    blocked_ms: f64,
    timeout_ms: f64,
    health: Health,
    open_req: u32,
    /// Jobs waiting for the open, or (writes) for a retry ahead of them.
    queue: VecDeque<Job>,
    /// Jobs put back at the head of `queue` after the connection was lost.
    requeued: usize,
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    /// `[0]` is the application database; the self-test adds its own.
    conns: Vec<Conn>,
    /// Transactions in flight: `(wire request id, connection, job)`.
    pending: Vec<(u32, usize, Job)>,
    /// Writes waiting for their backoff: `(due, connection, job)` in order.
    retry: Vec<(f64, usize, Job)>,
    /// Completions to deliver from a timer: `(caller, status)`.
    later: Vec<(u32, i32)>,
    slot: locks::Slot,
    selftest: Option<Box<selftest::SelfTest>>,
    /// Completions for the store's own request ids that nothing consumed (read by `fake::MemDb`).
    #[cfg(test)]
    pub(crate) inbox: Vec<(u32, i32, Vec<u8>)>,
}

const MAIN: usize = 0;
const T_LATER: u32 = tag(Mod::Store, 1);
const T_RETRY: u32 = tag(Mod::Store, 2);

fn conn(app: &mut App, c: usize) -> &mut Conn {
    let conns = &mut app.st.store.conns;
    if conns.is_empty() {
        conns.push(Conn::new(1, DB_NAME, DB_VERSION, BLOCKED_MS, OPEN_TIMEOUT_MS));
    }
    &mut conns[c]
}

impl Conn {
    fn new(id: u32, name: &str, version: u32, blocked_ms: f64, timeout_ms: f64) -> Conn {
        Conn { id, name: name.into(), version, blocked_ms, timeout_ms, health: Health::Closed, open_req: 0, queue: VecDeque::new(), requeued: 0 }
    }
}

/// The state of the application database.
pub fn health(app: &App) -> Health { app.st.store.conns.first().map_or(Health::Closed, |c| c.health) }

/// The degraded-mode signal: local persistence is not available; batches must be kept in memory and the relay
/// seal is the durability point.
pub fn degraded(app: &App) -> bool { matches!(health(app), Health::Degraded(_)) }

/// Opens the database now (boot of a signed-in session) instead of with the first transaction.
pub fn open(app: &mut App, host: &mut dyn Host) {
    if conn(app, MAIN).health == Health::Closed {
        start_open(app, host, MAIN);
    }
}

/// Leaves degraded mode and tries again: opens the database if it is not open, lets writes through again after
/// a quota failure. Does nothing while healthy.
pub fn reopen(app: &mut App, host: &mut dyn Host) { reopen_conn(app, host, MAIN) }

/// Runs a transaction on the application database. The completion is an `Event::Io` with `req` at the module
/// that owns `req`: `status` is one of [`status`], `data` the results ([`Reply`]) or the failure message.
pub fn tx(app: &mut App, host: &mut dyn Host, req: u32, tx: Tx) { submit(app, host, MAIN, Job { caller: req, tx, attempt: 0 }) }

/// Deletes a whole database (the migration's last step, tests). Completion: [`status::OK`],
/// [`status::BLOCKED`] when another connection still holds it after `BLOCKED_MS` (the deletion then happens when
/// that connection closes), or [`status::ERROR`].
pub fn delete_db(host: &mut dyn Host, req: u32, name: &str) {
    let out = host.out();
    let m = out.w.rec(fam::FX, fx::DELETE_DB);
    out.w.u32(req);
    out.w.str(name);
    out.w.f64(BLOCKED_MS);
    out.w.end(m);
}

fn reopen_conn(app: &mut App, host: &mut dyn Host, c: usize) {
    match conn(app, c).health {
        Health::Degraded(Cause::Quota) => {
            conn(app, c).health = Health::Ready;
            flush(app, host, c);
        }
        Health::Degraded(_) => start_open(app, host, c),
        _ => {}
    }
}

fn start_open(app: &mut App, host: &mut dyn Host, c: usize) {
    let req = app.req(Mod::Store);
    let conn = conn(app, c);
    conn.health = Health::Opening;
    conn.open_req = req;
    let out = host.out();
    let m = out.w.rec(fam::FX, fx::OPEN);
    out.w.u32(req);
    out.w.u32(conn.id);
    out.w.str(&conn.name);
    out.w.u32(conn.version);
    out.w.f64(conn.blocked_ms);
    out.w.f64(conn.timeout_ms);
    out.w.u32(STORES.len() as u32);
    STORES.iter().for_each(|s| out.w.str(s));
    out.w.end(m);
}

/// A write of this connection is being retried: later writes wait so the journal order holds.
fn stalled(st: &State, c: usize) -> bool {
    st.retry.iter().any(|r| r.1 == c) || st.pending.iter().any(|(_, pc, job)| *pc == c && job.attempt > 0 && job.tx.write)
}

fn submit(app: &mut App, host: &mut dyn Host, c: usize, job: Job) {
    let health = conn(app, c).health;
    match health {
        Health::Closed => {
            conn(app, c).queue.push_back(job);
            start_open(app, host, c);
        }
        Health::Opening => conn(app, c).queue.push_back(job),
        Health::Ready if job.tx.write && stalled(&app.st.store, c) => conn(app, c).queue.push_back(job),
        _ => dispatch(app, host, c, job),
    }
}

/// Sends or rejects a job according to the health of an opened (or given up) connection.
fn dispatch(app: &mut App, host: &mut dyn Host, c: usize, job: Job) {
    match conn(app, c).health {
        Health::Ready => send(app, host, c, job),
        Health::Degraded(Cause::Quota) if !job.tx.write => send(app, host, c, job),
        _ => later(app, host, job.caller, status::DEGRADED),
    }
}

fn send(app: &mut App, host: &mut dyn Host, c: usize, job: Job) {
    if job.tx.is_empty() {
        // IndexedDB has no transaction over zero stores: nothing to do is a success.
        return later(app, host, job.caller, status::OK);
    }
    let wire = app.req(Mod::Store);
    let id = conn(app, c).id;
    let out = host.out();
    let m = out.w.rec(fam::FX, fx::TX);
    out.w.u32(wire);
    out.w.u32(id);
    out.w.u8(job.tx.write as u8 | (job.tx.strict as u8) << 1);
    out.w.u32(job.tx.ops);
    out.w.raw(&job.tx.buf);
    out.w.end(m);
    app.st.store.pending.push((wire, c, job));
}

/// Sends what waits in the queue, in order, as far as the connection allows.
fn flush(app: &mut App, host: &mut dyn Host, c: usize) {
    loop {
        let wait = stalled(&app.st.store, c);
        let conn = conn(app, c);
        if conn.health == Health::Opening || (wait && conn.health == Health::Ready && conn.queue.front().is_some_and(|j| j.tx.write)) {
            return;
        }
        let Some(job) = conn.queue.pop_front() else { return };
        if conn.health == Health::Closed {
            conn.queue.push_front(job);
            return start_open(app, host, c);
        }
        dispatch(app, host, c, job);
    }
}

/// Delivers a completion from a timer, so that a caller never gets it inside its own `store::tx` call.
fn later(app: &mut App, host: &mut dyn Host, caller: u32, status: i32) {
    app.st.store.later.push((caller, status));
    app.sched.timer(host.out(), T_LATER, 0.0);
}

fn forward(app: &mut App, host: &mut dyn Host, io: Io) {
    if tag_mod(io.req) == Mod::Store as usize {
        own_io(app, host, &io);
    } else if let Some(module) = MODULES.get(tag_mod(io.req)) {
        (module.event)(app, host, &Event::Io(io));
    }
}

/// Completions of requests the store made for itself (slot lock, self-test).
fn own_io(app: &mut App, host: &mut dyn Host, io: &Io) {
    if !locks::on_io(app, host, io) && !selftest::on_io(app, host, io) {
        #[cfg(test)]
        app.st.store.inbox.push((io.req, io.status, io.data.to_vec()));
    }
}

fn opened(app: &mut App, host: &mut dyn Host, c: usize, io: &Io) {
    let conn = conn(app, c);
    conn.requeued = 0;
    conn.health = match io.status {
        status::OK => Health::Ready,
        status::BLOCKED => Health::Degraded(Cause::Blocked),
        status::TIMEOUT => Health::Degraded(Cause::Timeout),
        status::UNAVAILABLE => Health::Degraded(Cause::Unavailable),
        _ => Health::Degraded(Cause::Error),
    };
    if io.status != status::OK {
        let message = format!("ihn: {} could not be opened ({}): local storage is degraded", conn.name, conn.health.name());
        host.log(2, &message);
    }
    flush(app, host, c);
}

fn done(app: &mut App, host: &mut dyn Host, c: usize, mut job: Job, io: &Io) {
    let retried = job.attempt > 0;
    let healthy = conn(app, c).health == Health::Ready;
    match io.status {
        status::QUOTA if job.tx.write && healthy && job.attempt < RETRIES => {
            let due = host.now() + RETRY_MS * f64::from(1u32 << job.attempt);
            job.attempt += 1;
            app.st.store.retry.push((due, c, job));
            return arm_retry(app, host);
        }
        status::QUOTA if job.tx.write && healthy => {
            conn(app, c).health = Health::Degraded(Cause::Quota);
            host.log(2, "ihn: storage quota exceeded: local storage is degraded");
        }
        // The connection went away under the transaction (a newer tab upgrading, eviction): open again and
        // run it again, ahead of what was queued meanwhile.
        status::CLOSED if job.attempt < RETRIES => {
            job.attempt += 1;
            let conn = conn(app, c);
            let at = conn.requeued.min(conn.queue.len());
            conn.queue.insert(at, job);
            conn.requeued = at + 1;
            if conn.health == Health::Ready {
                conn.health = Health::Closed;
            }
            return flush(app, host, c);
        }
        _ => {}
    }
    forward(app, host, Io { req: job.caller, ..*io });
    if retried || io.status == status::QUOTA {
        flush(app, host, c);
    }
}

fn arm_retry(app: &mut App, host: &mut dyn Host) {
    if let Some(due) = app.st.store.retry.iter().map(|r| r.0).reduce(f64::min) {
        let delay = (due - host.now()).max(0.0);
        app.sched.timer(host.out(), T_RETRY, delay);
    }
}

fn on_io(app: &mut App, host: &mut dyn Host, io: &Io) {
    let st = &mut app.st.store;
    if let Some(c) = st.conns.iter().position(|c| c.health == Health::Opening && c.open_req == io.req) {
        opened(app, host, c, io);
    } else if let Some(i) = st.pending.iter().position(|p| p.0 == io.req) {
        let (_, c, job) = st.pending.remove(i);
        done(app, host, c, job, io);
    } else {
        own_io(app, host, io);
    }
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound. The database is opened lazily
/// (a signed-out visitor never creates it).
pub fn init(_app: &mut App, _host: &mut dyn Host) {}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Io(io) => on_io(app, host, &io),
        Event::Timer(T_LATER) => {
            for (caller, status) in std::mem::take(&mut app.st.store.later) {
                forward(app, host, Io { req: caller, status, obj: 0, data: &[] });
            }
        }
        Event::Timer(T_RETRY) => {
            let now = host.now();
            let (due, wait): (Vec<_>, Vec<_>) = std::mem::take(&mut app.st.store.retry).into_iter().partition(|r| r.0 <= now);
            app.st.store.retry = wait;
            for (_, c, job) in due {
                dispatch(app, host, c, job);
            }
            arm_retry(app, host);
        }
        _ => {}
    }
    0
}

/// `__IHN_TEST_API__.cmd('<module>.<name>', args)`: JSON text in, JSON text out; `None` = unknown command.
pub fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, args: &str) -> Option<String> {
    selftest::test_cmd(app, host, name, args)
}

#[cfg(test)]
mod tests {
    use super::fake::{boot, MemDb};
    use super::schema::{kv, sample_record, Archive, RelayAsset};
    use super::*;
    use crate::app::Mod;

    const DOC: &str = "drive-file-1";
    const ACCOUNT: &str = "ana@example.test";
    const ACTOR: u64 = 0x1122_3344_5566_7700;

    /// Runs one transaction to completion and returns `(status, results)`.
    fn run(t: Tx) -> (i32, Vec<Value>) {
        let (mut app, mut host, mut db) = boot();
        run_on(&mut app, &mut host, &mut db, t)
    }

    fn run_on(app: &mut App, host: &mut crate::host::FakeHost, db: &mut MemDb, t: Tx) -> (i32, Vec<Value>) {
        let req = app.req(Mod::Store);
        let n = t.len();
        tx(app, host, req, t);
        db.pump(app, host);
        let (status, data) = db.take(req).expect("one completion");
        let mut r = sv::R::new(&data);
        let results = if status >= 0 { (0..n).map(|_| r.value().expect("a result per operation")).collect() } else { Vec::new() };
        assert!(status < 0 || r.is_empty());
        (status, results)
    }

    #[test]
    fn the_first_transaction_opens_the_database_with_the_frozen_stores() {
        let (mut app, mut host, mut db) = boot();
        assert_eq!(health(&app), Health::Closed);
        let mut t = Tx::new();
        t.kv_put(ACCOUNT, kv::RELAY_FOLDER, &"folder".into());
        let (a, b) = (app.req(Mod::Store), app.req(Mod::Store));
        tx(&mut app, &mut host, a, t);
        assert_eq!(health(&app), Health::Opening);
        let mut t = Tx::new();
        t.kv_get(ACCOUNT, kv::RELAY_FOLDER);
        tx(&mut app, &mut host, b, t);
        assert!(db.take(a).is_none(), "nothing completes inside the call");
        db.pump(&mut app, &mut host);
        assert_eq!(health(&app), Health::Ready);
        assert_eq!(db.opened, [(DB_NAME.to_string(), 1, 1500.0, 5000.0, STORES.map(String::from).to_vec())]);
        assert_eq!(db.take(a).unwrap().0, status::OK);
        assert_eq!(Value::decode(&db.take(b).unwrap().1), Some("folder".into()), "queued transactions keep their order");
        assert!(!degraded(&app));
    }

    #[test]
    fn kv_counters_and_first_writer_wins() {
        let mut t = Tx::new();
        t.kv_put_if_absent(ACCOUNT, kv::INST, &"aaaa".into());
        t.kv_put_if_absent(ACCOUNT, kv::INST, &"bbbb".into());
        t.kv_get(ACCOUNT, kv::INST);
        t.kv_add(ACCOUNT, &kv::ctr(ACTOR), 65536.0);
        t.kv_add(ACCOUNT, &kv::ctr(ACTOR), 65536.0);
        t.kv_put("other@example.test", kv::IDPOOL, &Value::Arr(vec!["x".into()]));
        t.kv_scan(ACCOUNT);
        t.kv_del(ACCOUNT, kv::INST);
        t.kv_get(ACCOUNT, kv::INST);
        let (status, r) = run(t);
        assert_eq!(status, status::OK);
        assert_eq!(r[..5], [Value::Undef, "aaaa".into(), "aaaa".into(), 65536.0.into(), 131072.0.into()]);
        let key = |k: &str| Value::Arr(vec![ACCOUNT.into(), k.into()]);
        assert_eq!(r[6], Value::Arr(vec![key("ctr:1122334455667700"), 131072.0.into(), key("inst"), "aaaa".into()]), "one account only");
        assert_eq!(r[8], Value::Undef);
    }

    #[test]
    fn a_batch_and_its_seq_commit_together_or_not_at_all() {
        let (mut app, mut host, mut db) = boot();
        let mut t = Tx::new();
        t.doc_put(DOC, &sample_record());
        t.ops_append(DOC, ACTOR, 1, b"one");
        assert_eq!(run_on(&mut app, &mut host, &mut db, t).0, status::OK);
        // seq 2 is new, seq 1 is already in the journal: neither the batch nor the seq of this transaction stays.
        let mut t = Tx::new();
        t.ops_append(DOC, ACTOR, 2, b"two");
        t.ops_append(DOC, ACTOR, 1, b"again");
        assert_eq!(run_on(&mut app, &mut host, &mut db, t).0, status::CONSTRAINT);
        let mut t = Tx::new();
        t.doc_get(DOC);
        t.ops_scan(DOC);
        let req = app.req(Mod::Store);
        tx(&mut app, &mut host, req, t);
        db.pump(&mut app, &mut host);
        let (status, data) = db.take(req).unwrap();
        let mut reply = Reply::new(&Io { req, status, obj: 0, data: &data });
        let record = reply.doc().unwrap();
        assert_eq!(record.seq.get(ACTOR), Some(&1));
        assert_eq!(record, DocRecord { seq: [(0xab00, 4), (0xab01, 9), (ACTOR, 1)].into(), ..sample_record() });
        assert_eq!(reply.ops(), [(ACTOR, 1, &b"one"[..])]);
    }

    #[test]
    fn journal_chunks_and_document_removal() {
        let (mut app, mut host, mut db) = boot();
        let other = ACTOR + 1;
        let mut t = Tx::new();
        t.doc_put(DOC, &DocRecord::default());
        t.doc_put("drive-file-10", &DocRecord::default());
        for seq in 1..=4 {
            t.ops_append(DOC, ACTOR, seq, &[seq as u8]);
        }
        t.ops_put(DOC, other, 7, b"remote");
        t.ops_put(DOC, other, 7, b"remote");
        t.ops_put("drive-file-10", ACTOR, 1, b"other doc");
        for chunk in [0, 2, 10] {
            t.chunk_put(DOC, chunk, &[chunk as u8; 3]);
        }
        t.chunk_put("drive-file-10", 0, b"x");
        assert_eq!(run_on(&mut app, &mut host, &mut db, t).0, status::OK);

        let mut t = Tx::new();
        t.ops_trim(DOC, ACTOR, 2);
        t.chunk_del(DOC, 2);
        t.ops_scan(DOC);
        t.ops_scan_actor(DOC, ACTOR, 3, 0);
        t.chunk_scan(DOC);
        t.count(Store::Ops, Range::All);
        let req = app.req(Mod::Store);
        tx(&mut app, &mut host, req, t);
        db.pump(&mut app, &mut host);
        let (status, data) = db.take(req).unwrap();
        let mut reply = Reply::new(&Io { req, status, obj: 0, data: &data });
        reply.skip();
        reply.skip();
        assert_eq!(reply.ops(), [(ACTOR, 3, &[3u8][..]), (ACTOR, 4, &[4][..]), (other, 7, b"remote")]);
        assert_eq!(reply.ops(), [(ACTOR, 4, &[4u8][..])]);
        assert_eq!(reply.chunks(), [(0, &[0u8; 3][..]), (10, &[10; 3][..])], "numeric chunk order, this document only");
        assert_eq!(reply.num(), Some(4.0));

        let mut t = Tx::new();
        t.doc_delete(DOC);
        t.doc_get(DOC);
        t.count(Store::Ops, Range::All);
        t.count(Store::Chunks, Range::All);
        t.docs_scan();
        // A late seq bump of another tab must not bring the record back.
        t.ops_append(DOC, ACTOR, 9, b"late");
        t.doc_get(DOC);
        let (status, r) = run_on(&mut app, &mut host, &mut db, t);
        assert_eq!(status, status::OK);
        assert_eq!(r[3..6], [Value::Undef, 1.0.into(), 1.0.into()]);
        assert_eq!(r[6].items().len(), 2, "the other document stays");
        assert_eq!(r[8..], [false.into(), Value::Undef]);
    }

    #[test]
    fn assets_are_content_addressed_and_reference_counted() {
        let hash = [0xabu8; 32];
        let mut t = Tx::new();
        t.asset_put(&hash, 41, "image/png", 10.0);
        t.asset_put(&hash, 42, "image/png", 10.0);
        t.asset_ref(&hash, 2, true);
        t.asset_remap(&hash, &[3, 1]);
        t.asset_get(&hash);
        t.asset_keys();
        t.asset_ref(&hash, -2, true);
        t.asset_get(&hash);
        let (status, r) = run(t);
        assert_eq!(status, status::OK);
        assert!(r[0].is_undef() && !r[1].is_undef(), "the second put finds the first");
        assert_eq!(r[2], 2.0.into());
        let blob = Value::Blob { obj: 41, size: 10.0, mime: "image/png".into() };
        assert_eq!(
            r[4],
            Value::obj([("blob", blob), ("mime", "image/png".into()), ("size", 10.0.into()), ("refs", 2.0.into()), ("remap", vec![3u32.into(), 1u32.into()].into())])
        );
        assert_eq!(r[5], Value::Arr(vec!["ab".repeat(32).into()]));
        assert_eq!(r[6..], [0.0.into(), Value::Undef], "collected at zero");
    }

    #[test]
    fn catalog_entries_merge_field_by_field() {
        let mut t = Tx::new();
        t.catalog_put(ACCOUNT, "f1", &Value::obj([("name", "A".into()), ("starred", false.into())]));
        t.catalog_patch(ACCOUNT, "f1", &[(&["starred"], true.into()), (&["localPreview"], "data:".into()), (&["name"], Value::Undef)]);
        t.catalog_patch(ACCOUNT, "f2", &[(&["name"], "B".into())]);
        t.catalog_put("zoe@example.test", "f3", &Value::Null);
        t.catalog_scan(ACCOUNT);
        t.catalog_del(ACCOUNT, "f1");
        t.catalog_get(ACCOUNT, "f1");
        let (status, r) = run(t);
        assert_eq!(status, status::OK);
        assert_eq!(r[1..3], [true.into(), false.into()]);
        let rows = r[4].items();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1], Value::obj([("starred", true.into()), ("localPreview", "data:".into())]));
        assert_eq!(rows[3], Value::obj([("name", "B".into())]));
        assert!(r[6].is_undef());
    }

    #[test]
    fn sync_records_map_to_the_stores_of_the_architecture_table() {
        let (mut app, mut host, mut db) = boot();
        let mut record = DocRecord { account: ACCOUNT.into(), ..DocRecord::default() };
        let mut t = Tx::new();
        t.doc_put(DOC, &record);
        let stamp = schema::Stamp { clock: 99, actor: ACTOR, idx: 1 };
        let last = schema::LastExport { head: "rev-9".into(), vv: vec![9], etag: "etag".into(), at: 5.0 };
        let mut records = vec![
            Record::Seq { actor: ACTOR, seq: 3 },
            Record::RelayPlan { gen: 7, file_id: "relay-file".into(), epoch_day: 20_405 },
            Record::RelaySealed {
                sealed: [(ACTOR, 3)].into(),
                archives: vec![Archive { file_id: "old".into(), digest: vec![1; 32], ranges: vec![2] }],
                assets: vec![RelayAsset { hash: [3; 32], file_id: "asset".into(), size: 77.0 }],
                perm: vec![4; 32],
                relay_pending: false,
            },
            Record::RelayApplied { inst: 0xdd00, gen: 2, d16: [5; 16] },
            Record::RelayApplied { inst: 0xee00, gen: 1, d16: [6; 16] },
            Record::LastExport(last.clone()),
            Record::Flags { relay_pending: None, export_pending: Some(true), created: Some(true) },
            Record::Flags { relay_pending: None, export_pending: None, created: None },
            Record::DriveName(schema::DriveName { v: "Renamed".into(), observed_at: stamp }),
            Record::IdPool(vec!["id-1".into(), "id-2".into()]),
            Record::RelayFolder("folder-id".into()),
            Record::NoLink(true),
            Record::RelayGc(vec!["gone".into()]),
        ];
        records.extend((0..10).map(|i| Record::KnownExport { head: format!("rev-{i}"), vv: vec![i] }));
        records.push(Record::KnownExport { head: "rev-5".into(), vv: vec![55] });
        for r in records {
            t.persist(ACCOUNT, DOC, &mut record, r);
        }
        t.doc_get(DOC);
        t.kv_scan(ACCOUNT);
        let n = t.len() as usize;
        let (status, r) = run_on(&mut app, &mut host, &mut db, t);
        assert_eq!(status, status::OK);
        // The stored record equals the in-memory one the mapping kept current.
        assert_eq!(DocRecord::from_value(&r[n - 2]), record);
        assert_eq!(record.seq, [(ACTOR, 3)].into());
        assert_eq!((record.relay.gen, record.relay.file_id.as_str(), record.relay.epoch_day), (7, "relay-file", 20_405));
        assert_eq!(record.relay.sealed, [(ACTOR, 3)].into());
        assert_eq!(record.relay.applied, [(0xdd00, (2, [5; 16])), (0xee00, (1, [6; 16]))].into());
        assert_eq!(record.last_export, Some(last));
        assert_eq!(record.flags, schema::Flags { created: true, relay_pending: false, export_pending: true, migrated: false });
        assert_eq!(record.drive_name.as_ref().map(|n| n.observed_at), Some(stamp));
        let heads: Vec<&str> = record.exports.iter().map(|e| e.0.as_str()).collect();
        assert_eq!(heads, ["rev-2", "rev-3", "rev-4", "rev-6", "rev-7", "rev-8", "rev-9", "rev-5"], "the 8 newest, re-seen last");
        assert_eq!(record.exports[7].1, [55]);
        let key = |k: &str| Value::Arr(vec![ACCOUNT.into(), k.into()]);
        assert_eq!(
            r[n - 1],
            Value::Arr(vec![
                key(kv::IDPOOL),
                vec!["id-1".into(), "id-2".into()].into(),
                key(kv::RELAY_FOLDER),
                "folder-id".into(),
                key(kv::RELAY_GC),
                vec!["gone".into()].into(),
                key(kv::RELAY_NOLINK),
                true.into(),
            ])
        );
        // A record that was deleted is not resurrected by a late patch.
        let mut t = Tx::new();
        t.doc_delete(DOC);
        t.persist(ACCOUNT, DOC, &mut record, Record::Flags { relay_pending: Some(true), export_pending: None, created: None });
        t.doc_get(DOC);
        let (_, r) = run_on(&mut app, &mut host, &mut db, t);
        assert_eq!(r[3..], [false.into(), Value::Undef]);
    }

    #[test]
    fn a_blocked_or_slow_open_is_the_degraded_signal() {
        for (failure, cause) in [
            (status::BLOCKED, Cause::Blocked),
            (status::TIMEOUT, Cause::Timeout),
            (status::UNAVAILABLE, Cause::Unavailable),
            (status::ERROR, Cause::Error),
        ] {
            let (mut app, mut host, mut db) = boot();
            db.fail_open = Some(failure);
            let (write, read) = (app.req(Mod::Store), app.req(Mod::Store));
            let mut t = Tx::new();
            t.ops_put(DOC, ACTOR, 1, b"x");
            tx(&mut app, &mut host, write, t);
            let mut t = Tx::new();
            t.doc_get(DOC);
            tx(&mut app, &mut host, read, t);
            db.pump(&mut app, &mut host);
            assert_eq!(health(&app), Health::Degraded(cause));
            assert!(degraded(&app));
            assert_eq!(db.take(write).unwrap().0, status::DEGRADED);
            assert_eq!(db.take(read).unwrap().0, status::DEGRADED, "no database: reads fail too");
            assert_eq!(host.logs.len(), 1);
            // Later transactions are refused without touching the host, from a timer.
            let again = app.req(Mod::Store);
            tx(&mut app, &mut host, again, Tx::new());
            assert!(db.take(again).is_none());
            db.pump(&mut app, &mut host);
            assert_eq!(db.take(again).unwrap().0, status::DEGRADED);
            assert_eq!(db.transactions, 0);
            // `reopen` tries again.
            db.fail_open = None;
            reopen(&mut app, &mut host);
            let mut t = Tx::new();
            t.ops_put(DOC, ACTOR, 1, b"x");
            assert_eq!(run_on(&mut app, &mut host, &mut db, t).0, status::OK);
            assert_eq!(health(&app), Health::Ready);
        }
    }

    #[test]
    fn quota_errors_are_retried_three_times_then_degrade() {
        let (mut app, mut host, mut db) = boot();
        open(&mut app, &mut host);
        db.pump(&mut app, &mut host);
        // One failure: retried after 250 ms; a write issued meanwhile waits behind it, a read does not.
        db.fail_writes.extend([status::QUOTA]);
        let (first, second, read) = (app.req(Mod::Store), app.req(Mod::Store), app.req(Mod::Store));
        let mut t = Tx::new();
        t.doc_put(DOC, &DocRecord::default());
        t.ops_append(DOC, ACTOR, 1, b"one");
        tx(&mut app, &mut host, first, t);
        db.pump_io(&mut app, &mut host);
        let mut t = Tx::new();
        t.ops_append(DOC, ACTOR, 2, b"two");
        tx(&mut app, &mut host, second, t);
        let mut t = Tx::new();
        t.count(Store::Ops, Range::All);
        tx(&mut app, &mut host, read, t);
        db.pump_io(&mut app, &mut host);
        assert_eq!(Value::decode(&db.take(read).unwrap().1), Some(0.0.into()));
        assert!(db.take(first).is_none() && db.take(second).is_none());
        let t0 = host.now;
        db.pump(&mut app, &mut host);
        assert_eq!(host.now - t0, 250.0);
        assert_eq!((db.take(first).unwrap().0, db.take(second).unwrap().0), (status::OK, status::OK));
        assert_eq!(db.order, [read, first, second], "the journal order survives the retry");
        assert_eq!(health(&app), Health::Ready);

        // Four failures in a row: 250 + 500 + 1000 ms, then degraded; the queued write is refused.
        db.fail_writes.extend([status::QUOTA; 4]);
        let (third, fourth) = (app.req(Mod::Store), app.req(Mod::Store));
        let mut t = Tx::new();
        t.ops_append(DOC, ACTOR, 3, b"three");
        tx(&mut app, &mut host, third, t);
        db.pump_io(&mut app, &mut host);
        let mut t = Tx::new();
        t.ops_append(DOC, ACTOR, 4, b"four");
        tx(&mut app, &mut host, fourth, t);
        let t0 = host.now;
        db.pump(&mut app, &mut host);
        assert_eq!(host.now - t0, 1750.0);
        assert_eq!(db.take(third).unwrap().0, status::QUOTA);
        assert_eq!(db.take(fourth).unwrap().0, status::DEGRADED);
        assert_eq!(health(&app), Health::Degraded(Cause::Quota));
        // Reads still work; writes are refused until `reopen`.
        let mut t = Tx::new();
        t.ops_scan(DOC);
        let (status, r) = run_on(&mut app, &mut host, &mut db, t);
        assert_eq!((status, r[0].items().len()), (status::OK, 4));
        let mut t = Tx::new();
        t.ops_append(DOC, ACTOR, 3, b"three");
        assert_eq!(run_on(&mut app, &mut host, &mut db, t).0, status::DEGRADED);
        reopen(&mut app, &mut host);
        let mut t = Tx::new();
        t.ops_append(DOC, ACTOR, 3, b"three");
        assert_eq!(run_on(&mut app, &mut host, &mut db, t).0, status::OK);
    }

    #[test]
    fn a_lost_connection_is_reopened_and_the_transactions_run_again_in_order() {
        let (mut app, mut host, mut db) = boot();
        open(&mut app, &mut host);
        db.pump(&mut app, &mut host);
        let (a, b, c) = (app.req(Mod::Store), app.req(Mod::Store), app.req(Mod::Store));
        for (req, seq) in [(a, 1), (b, 2)] {
            let mut t = Tx::new();
            t.ops_put(DOC, ACTOR, seq, b"x");
            tx(&mut app, &mut host, req, t);
        }
        db.close_all();
        db.step(&mut app, &mut host);
        let mut t = Tx::new();
        t.ops_put(DOC, ACTOR, 3, b"x");
        tx(&mut app, &mut host, c, t);
        db.pump(&mut app, &mut host);
        assert_eq!(db.order, [a, b, c]);
        assert_eq!(db.opened.len(), 2);
        assert!([a, b, c].iter().all(|r| db.take(*r).unwrap().0 == status::OK));
        assert_eq!(health(&app), Health::Ready);
    }

    #[test]
    fn an_empty_transaction_succeeds_without_reaching_the_host() {
        let (mut app, mut host, mut db) = boot();
        assert_eq!(run_on(&mut app, &mut host, &mut db, Tx::new()), (status::OK, Vec::new()));
        assert_eq!((db.transactions, health(&app)), (0, Health::Ready));
    }

    #[test]
    fn other_failures_reach_the_caller_unchanged() {
        let (mut app, mut host, mut db) = boot();
        db.fail_writes.extend([status::ERROR]);
        let mut t = Tx::new();
        t.kv_put(ACCOUNT, "k", &Value::Null);
        assert_eq!(run_on(&mut app, &mut host, &mut db, t).0, status::ERROR);
        assert_eq!(health(&app), Health::Ready);
        assert_eq!(db.transactions, 1, "not retried");
    }
}
