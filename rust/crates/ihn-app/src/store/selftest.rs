//! `store.*` test commands (e2e only): the self-test that runs the store against the real `host/idb.js`
//! (`tests/v6/e2e/internal-store.spec.mjs`). Rust only issues the requests of a scenario and records every
//! completion as base64 of its bytes; the spec decodes the structured values and asserts on them, so neither the
//! expectations nor a JSON writer cost wasm bytes.
//!
//! * `store.health` → `{"health", "slot"}` of the application database; `store.open` opens it.
//! * `store.slot` `{account}` → takes the tab's actor slot.
//! * `store.selftest` `{run, db, …}` starts a scenario on a scratch database; `store.selftest_done` tells when it
//!   finished and `store.selftest_result` →
//!   `{done, health, pages, steps: [{status, data}…], dump: […]}`:
//!   * `basic` — kv, the atomic batch + seq append, chunks, catalog, `SyncRecord` writes, document removal,
//!     locks, the actor slot; deletes the scratch database at the end.
//!   * `degraded` `{version, blockedMs, timeoutMs}` — opens `db` (expected to fail) and issues two writes;
//!     `reopen` then retries on the same connection and deletes the database.
//!   * `dump` `{dbs: "a,b"}` — reads 5.x databases through the read-only dumper, three records per page:
//!     `dump` holds `{db, status, fingerprint: [records, stamp, hash], data}` per database and
//!     `{db, store, data}` per page; then stores the first `Blob` it met as an asset of the scratch database.

use super::locks;
use super::schema::{kv, Archive, DocRecord, Flags, LastExport, Record, RelayAsset};
use super::sv::Value;
use super::{conn, reopen_conn, start_open, submit, v5, Conn, Health, Job, Key, Put, Range, Reply, Store, Tx};
use ihn_core::codec::b64_encode;
use crate::abi::Io;
use crate::app::Mod;
use crate::testapi::json_str;
use crate::{App, Host};
use std::fmt::Write;

const ACCOUNT: &str = "selftest@example.test";
const DOC: &str = "selftest-doc";
const ACTOR: u64 = 0x5e1f_7e57_0000_0100;
const REMOTE: u64 = 0x0bad_cafe_0000_0200;
const HASH: [u8; 32] = [0x5e; 32];

#[derive(Clone, Copy, PartialEq)]
enum Run {
    Basic,
    Degraded,
    Reopen,
    Dump,
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Start,
    Info,
    Page,
    Asset,
    Delete,
}

pub(super) struct SelfTest {
    run: Run,
    conn: usize,
    step: u32,
    /// The request whose completion advances the scenario.
    req: u32,
    locks: [u32; 3],
    lock_name: String,
    done: bool,
    /// The in-memory record kept current by `Tx::persist`.
    record: DocRecord,
    /// One JSON object per completion.
    steps: String,
    // dump
    phase: Phase,
    dbs: Vec<String>,
    stores: Vec<String>,
    after: Option<Value>,
    pages: u32,
    dump: String,
    blob: Option<(u32, f64, String)>,
}

fn arg<'a>(args: &'a str, key: &str) -> Option<&'a str> {
    let mut rest = args;
    loop {
        rest = rest.split_once('"')?.1;
        if let Some(value) = rest.strip_prefix(key).and_then(|r| r.strip_prefix("\":")) {
            return match value.strip_prefix('"') {
                Some(text) => text.split('"').next(),
                None => value.split([',', '}']).next(),
            };
        }
    }
}

pub(super) fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, args: &str) -> Option<String> {
    Some(match name {
        "health" => {
            let slot = match locks::slot_state(app) {
                locks::SlotState::Held(n) => n.to_string(),
                _ => "null".into(),
            };
            format!("{{\"health\":\"{}\",\"slot\":{slot}}}", super::health(app).name())
        }
        "open" => {
            super::open(app, host);
            "true".into()
        }
        "slot" => {
            let notify = app.req(Mod::Store);
            locks::acquire_slot(app, host, arg(args, "account").unwrap_or(ACCOUNT), notify);
            "true".into()
        }
        "selftest" => {
            let run = match arg(args, "run") {
                Some("degraded") => Run::Degraded,
                Some("reopen") => Run::Reopen,
                Some("dump") => Run::Dump,
                _ => Run::Basic,
            };
            let number = |key: &str, default: f64| arg(args, key).and_then(|v| v.parse::<u32>().ok()).map_or(default, f64::from);
            conn(app, super::MAIN);
            let previous = app.st.store.selftest.take();
            let index = match (run, &previous) {
                (Run::Reopen, Some(p)) => p.conn,
                _ => {
                    let conns = &mut app.st.store.conns;
                    conns.push(Conn::new(
                        conns.len() as u32 + 1,
                        arg(args, "db").unwrap_or("ihn6-selftest"),
                        number("version", 1.0) as u32,
                        number("blockedMs", super::BLOCKED_MS),
                        number("timeoutMs", super::OPEN_TIMEOUT_MS),
                    ));
                    conns.len() - 1
                }
            };
            let mut t = Box::new(SelfTest {
                run,
                conn: index,
                step: 0,
                req: 0,
                locks: [0; 3],
                lock_name: format!("ihn6-selftest:{}", app.st.store.conns[index].name),
                done: false,
                record: DocRecord {
                    account: ACCOUNT.into(),
                    name: "Self-test".into(),
                    vv: vec![1, 2, 3],
                    flags: Flags { created: true, ..Flags::default() },
                    last_opened_at: 1.0,
                    ..DocRecord::default()
                },
                steps: String::new(),
                phase: Phase::Start,
                dbs: arg(args, "dbs").unwrap_or("").split(',').filter(|s| !s.is_empty()).rev().map(String::from).collect(),
                stores: Vec::new(),
                after: None,
                pages: 0,
                dump: String::new(),
                blob: None,
            });
            t.advance(app, host, None);
            app.st.store.selftest = Some(t);
            "true".into()
        }
        "selftest_done" => app.st.store.selftest.as_ref().is_some_and(|t| t.done).to_string(),
        "selftest_result" => {
            let Some(t) = &app.st.store.selftest else { return Some("{\"done\":false}".into()) };
            format!(
                "{{\"done\":{},\"health\":\"{}\",\"pages\":{},\"steps\":[{}],\"dump\":[{}]}}",
                t.done,
                app.st.store.conns[t.conn].health.name(),
                t.pages,
                t.steps.trim_end_matches(','),
                t.dump.trim_end_matches(',')
            )
        }
        _ => return None,
    })
}

/// Advances the running scenario when `io` is the completion it waits for.
pub(super) fn on_io(app: &mut App, host: &mut dyn Host, io: &Io) -> bool {
    match app.st.store.selftest.take() {
        Some(mut t) if t.req == io.req && !t.done => {
            t.advance(app, host, Some(io));
            app.st.store.selftest = Some(t);
            true
        }
        other => {
            app.st.store.selftest = other;
            false
        }
    }
}

impl SelfTest {
    fn tx(&mut self, app: &mut App, host: &mut dyn Host, tx: Tx) {
        self.req = app.req(Mod::Store);
        submit(app, host, self.conn, Job { caller: self.req, tx, attempt: 0 });
    }

    /// Records a completion: its status and its bytes (the failure message when the status is negative).
    fn note(&mut self, io: &Io) {
        let _ = write!(self.steps, "{{\"status\":{},\"data\":\"{}\"}},", io.status, b64_encode(io.data, false));
    }

    fn advance(&mut self, app: &mut App, host: &mut dyn Host, io: Option<&Io>) {
        if let Some(io) = io.filter(|_| self.run != Run::Dump) {
            self.note(io);
        }
        let io = io.copied().unwrap_or(Io { req: 0, status: 0, obj: 0, data: &[] });
        match self.run {
            Run::Basic => self.basic(app, host),
            Run::Degraded | Run::Reopen => self.degraded(app, host),
            Run::Dump => self.dump(app, host, &io),
        }
        self.step += 1;
    }

    fn basic(&mut self, app: &mut App, host: &mut dyn Host) {
        let mut t = Tx::new();
        match self.step {
            0 => {
                t.kv_put_if_absent(ACCOUNT, kv::INST, &"aaaa".into());
                t.kv_put_if_absent(ACCOUNT, kv::INST, &"bbbb".into());
                t.kv_add(ACCOUNT, &kv::ctr(ACTOR), 65536.0);
                t.kv_add(ACCOUNT, &kv::ctr(ACTOR), 65536.0);
                t.doc_put(DOC, &self.record);
                t.ops_append(DOC, ACTOR, 1, b"one");
                for chunk in [0, 10, 2] {
                    t.chunk_put(DOC, chunk, &[chunk as u8; 5]);
                }
                t.catalog_put(ACCOUNT, DOC, &Value::obj([("name", "Self-test".into())]));
                t.catalog_patch(ACCOUNT, DOC, &[(&["starred"], true.into()), (&["name"], Value::Undef)]);
                t.catalog_get(ACCOUNT, DOC);
            }
            // seq 2 is new but seq 1 exists: the whole transaction must abort.
            1 => {
                t.ops_append(DOC, ACTOR, 2, b"two");
                t.ops_append(DOC, ACTOR, 1, b"dup");
            }
            2 => {
                t.doc_get(DOC);
                t.ops_scan(DOC);
                t.chunk_scan(DOC);
            }
            3 => {
                self.record.seq.insert(ACTOR, 1);
                // One record per kind of write (nested patch, whole field, kv); the full mapping is a cargo test.
                let records = [
                    Record::RelaySealed {
                        sealed: [(ACTOR, 1)].into(),
                        archives: vec![Archive { file_id: "old".into(), digest: vec![1; 2], ranges: vec![2, 3] }],
                        assets: vec![RelayAsset { hash: [4; 32], file_id: "asset".into(), size: 9.0 }],
                        perm: vec![5; 2],
                        relay_pending: true,
                    },
                    Record::RelayApplied { inst: REMOTE, gen: 2, d16: [6; 16] },
                    Record::LastExport(LastExport { head: "rev-1".into(), vv: vec![7], etag: "e".into(), at: 2.0 }),
                    Record::IdPool(vec!["id-1".into()]),
                ];
                for r in records {
                    t.persist(ACCOUNT, DOC, &mut self.record, r);
                }
                t.ops_put(DOC, REMOTE, 5, b"remote");
                t.ops_append(DOC, ACTOR, 2, b"two");
                t.ops_trim(DOC, ACTOR, 1);
                self.record.seq.insert(ACTOR, 2);
                // A late seq 1 does not take the stored seq back; a reference counted before the asset's bytes
                // arrive is kept when they do (bytes stand in for the Blob here).
                t.doc_seq(DOC, ACTOR, 1);
                t.asset_ref(&HASH, 1, true);
                let asset = Value::obj([("blob", (&b"\x09"[..]).into()), ("mime", "x/y".into()), ("size", 1.0.into()), ("refs", 0.0.into())]);
                t.put(Store::Assets, Put::Fill, Key::Str(&super::sv::hex(&HASH)), &asset);
                t.strict = true;
            }
            4 => {
                t.doc_get(DOC);
                t.ops_scan(DOC);
                t.kv_get(ACCOUNT, kv::IDPOOL);
                t.ops_scan_actor(DOC, ACTOR, 1, 1);
                t.asset_get(&HASH);
            }
            5 => {
                t.doc_delete(DOC);
                for store in [Store::Docs, Store::Ops, Store::Chunks] {
                    t.count(store, Range::All);
                }
                t.asset_ref(&HASH, -1, true);
                t.count(Store::Assets, Range::All);
            }
            6 => {
                self.locks = [app.req(Mod::Store), app.req(Mod::Store), app.req(Mod::Store)];
                self.req = self.locks[0];
                locks::request(host, self.req, &self.lock_name, true);
            }
            // Held: a second request is refused, a waiting one is listed as pending…
            7 => {
                self.req = self.locks[1];
                locks::request(host, self.req, &self.lock_name, true);
            }
            8 => {
                locks::request(host, self.locks[2], &self.lock_name, false);
                self.req = app.req(Mod::Store);
                locks::query(host, self.req, &self.lock_name);
            }
            // …and granted when the holder releases.
            9 => {
                locks::release(host, self.locks[0]);
                self.req = self.locks[2];
            }
            // A release takes effect asynchronously: the proof is that a new waiting request gets the lock.
            10 => {
                locks::release(host, self.locks[2]);
                self.req = self.locks[0];
                locks::request(host, self.req, &self.lock_name, false);
            }
            11 => {
                locks::release(host, self.locks[0]);
                self.req = app.req(Mod::Store);
                locks::acquire_slot(app, host, ACCOUNT, self.req);
            }
            12 => {
                locks::release_slot(app, host);
                self.finish(app, host);
            }
            _ => self.done = true,
        }
        if !t.is_empty() {
            self.tx(app, host, t);
        }
    }

    /// Closes the scratch connection and deletes its database.
    fn finish(&mut self, app: &mut App, host: &mut dyn Host) {
        let conn = &mut app.st.store.conns[self.conn];
        conn.health = Health::Closed;
        host.out().fx_id(super::fx::CLOSE, conn.id);
        self.req = app.req(Mod::Store);
        super::delete_db(host, self.req, &app.st.store.conns[self.conn].name);
    }

    fn degraded(&mut self, app: &mut App, host: &mut dyn Host) {
        let mut t = Tx::new();
        t.kv_put(ACCOUNT, "probe", &Value::Num(self.step as f64));
        match (self.run, self.step) {
            // A write queued behind the open, then one issued after the store gave up.
            (Run::Degraded, 0) => {
                start_open(app, host, self.conn);
                self.tx(app, host, t);
            }
            (Run::Degraded, 1) => self.tx(app, host, t),
            (Run::Reopen, 0) => {
                reopen_conn(app, host, self.conn);
                self.tx(app, host, t);
            }
            (Run::Reopen, 1) => self.finish(app, host),
            _ => self.done = true,
        }
    }

    fn dump(&mut self, app: &mut App, host: &mut dyn Host, io: &Io) {
        match self.phase {
            Phase::Start => {}
            Phase::Info => {
                let name = self.dbs.last().cloned().unwrap_or_default();
                let info = v5::Info::parse(io);
                let f = info.as_ref().map_or(v5::Fingerprint { records: 0.0, stamp: 0.0, hash: 0 }, v5::Info::fingerprint);
                let _ = write!(
                    self.dump,
                    "{{\"db\":{},\"status\":{},\"fingerprint\":[{},{},\"{}\"],\"data\":\"{}\"}},",
                    json_str(&name),
                    io.status,
                    f.records,
                    f.stamp,
                    super::sv::hex16(f.hash),
                    b64_encode(if info.is_some() { io.data } else { &[] }, false)
                );
                self.stores = info.map_or(Vec::new(), |i| i.stores.into_iter().rev().map(|s| s.name).collect());
                if self.stores.is_empty() {
                    self.dbs.pop();
                }
            }
            Phase::Page => {
                let Some(page) = v5::Page::parse(io) else {
                    self.done = true;
                    return;
                };
                self.pages += 1;
                let _ = write!(
                    self.dump,
                    "{{\"db\":{},\"store\":{},\"data\":\"{}\"}},",
                    json_str(self.dbs.last().map_or("", |s| s)),
                    json_str(self.stores.last().map_or("", |s| s)),
                    b64_encode(io.data, false)
                );
                if self.blob.is_none() {
                    if let Some(Value::Blob { obj, size, mime }) = page.rows.iter().find_map(|row| first_blob(&row.1)) {
                        self.blob = Some((*obj, *size, mime.clone()));
                    }
                }
                page.release(host.out(), &self.blob.as_ref().map_or(Vec::new(), |b| vec![b.0]));
                self.after = page.rows.last().map(|r| r.0.clone()).filter(|_| !page.done);
                if page.done {
                    self.stores.pop();
                    if self.stores.is_empty() {
                        self.dbs.pop();
                    }
                }
            }
            Phase::Asset => {
                self.note(io);
                let (obj, ..) = self.blob.take().unwrap_or_default();
                let mut handles = vec![obj];
                let mut reply = Reply::new(io);
                (0..3).for_each(|_| reply.value().handles(&mut handles));
                handles.into_iter().for_each(|h| host.out().obj_release(h));
                self.phase = Phase::Delete;
                return self.finish(app, host);
            }
            Phase::Delete => {
                self.note(io);
                self.done = true;
                return;
            }
        }
        // The next page or store of the current database, else the next database, else the asset.
        self.req = app.req(Mod::Store);
        if let (Some(db), Some(store)) = (self.dbs.last(), self.stores.last()) {
            v5::read(host, self.req, db, store, self.after.as_ref(), 3, 262_144.0);
            self.phase = Phase::Page;
        } else if let Some(db) = self.dbs.last() {
            v5::info(host, self.req, db);
            self.phase = Phase::Info;
        } else if let Some((obj, size, mime)) = self.blob.clone() {
            let mut t = Tx::new();
            t.asset_put(&HASH, obj, &mime, size);
            t.asset_ref(&HASH, 1, true);
            t.asset_get(&HASH);
            self.tx(app, host, t);
            self.phase = Phase::Asset;
        } else {
            self.done = true;
        }
    }
}

fn first_blob(v: &Value) -> Option<&Value> {
    match v {
        Value::Blob { .. } => Some(v),
        Value::Arr(items) => items.iter().find_map(first_blob),
        Value::Obj(fields) => fields.iter().find_map(|f| first_blob(&f.1)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::boot;
    use super::super::status;
    use super::super::sv::R;
    use super::*;
    use ihn_core::codec::b64_decode;

    /// The `(status, values)` of every `"status":…,"data":"…"` entry of a result, in order.
    fn entries(json: &str) -> Vec<(i32, Vec<Value>)> {
        json.split("\"data\":\"")
            .collect::<Vec<_>>()
            .windows(2)
            .map(|w| {
                let status = w[0].rsplit("\"status\":").next().filter(|_| w[0].contains("\"status\":")).map_or(0, |s| s.split(',').next().unwrap().parse().unwrap());
                let bytes = b64_decode(w[1].split('"').next().unwrap().as_bytes()).unwrap();
                let mut r = R::new(if status >= 0 { &bytes } else { &[] });
                (status, std::iter::from_fn(|| r.value()).collect())
            })
            .collect()
    }

    fn run(app: &mut App, host: &mut crate::host::FakeHost, db: &mut super::super::fake::MemDb, args: &str) -> String {
        assert_eq!(test_cmd(app, host, "selftest", args).unwrap(), "true");
        db.pump(app, host);
        let json = test_cmd(app, host, "selftest_result", "null").unwrap();
        assert!(json.starts_with("{\"done\":true,"), "{json}");
        json
    }

    #[test]
    fn arguments_are_read_from_the_json_text() {
        let args = "{\"run\":\"degraded\",\"db\":\"x-y\",\"version\":2,\"blockedMs\":300}";
        assert_eq!((arg(args, "run"), arg(args, "db"), arg(args, "version"), arg(args, "blockedMs")), (Some("degraded"), Some("x-y"), Some("2"), Some("300")));
        assert_eq!((arg(args, "missing"), arg(args, "d"), arg("null", "db")), (None, None, None));
    }

    /// The transcript the e2e spec expects from the real host, here from the reference executor.
    #[test]
    fn the_basic_scenario_matches_the_reference_executor() {
        let (mut app, mut host, mut db) = boot();
        let json = run(&mut app, &mut host, &mut db, "{\"run\":\"basic\",\"db\":\"scratch\"}");
        let steps = entries(&json);
        assert_eq!(steps.iter().map(|s| s.0).collect::<Vec<_>>(), [0, -7, 0, 0, 0, 0, 1, 0, 0, 1, 1, 0, 0]);
        let u = Value::Undef;
        let key = |k: Value| Value::Arr(vec![DOC.into(), k]);
        assert_eq!(
            steps[0].1,
            [u.clone(), "aaaa".into(), 65536.0.into(), 131072.0.into(), u.clone(), u.clone(), true.into(), u.clone(), u.clone(), u.clone(), u.clone(), true.into(), Value::obj([("starred", true.into())])]
        );
        // After the aborted transaction: seq 1 and one batch; chunks in numeric order.
        let stored = DocRecord::from_value(&steps[2].1[0]);
        assert_eq!(stored.seq, [(ACTOR, 1)].into());
        assert_eq!(steps[2].1[1], Value::Arr(vec![Value::Arr(vec![DOC.into(), "5e1f7e5700000100".into(), 1.0.into()]), (&b"one"[..]).into()]));
        assert_eq!(steps[2].1[2].items().iter().step_by(2).cloned().collect::<Vec<_>>(), [key(0.0.into()), key(2.0.into()), key(10.0.into())]);
        // The stored record equals the one `persist` kept in memory.
        let record = app.st.store.selftest.as_ref().unwrap().record.clone();
        assert_eq!(DocRecord::from_value(&steps[4].1[0]), record);
        assert_eq!((record.seq.get(ACTOR), record.relay.applied.get(REMOTE), record.flags.relay_pending), (Some(&2), Some(&(2, [6; 16])), true));
        assert_eq!(steps[4].1[1].items().len(), 4, "remote 5 and own 2; own 1 trimmed");
        assert_eq!(steps[4].1[2], Value::Arr(vec!["id-1".into()]));
        assert_eq!(steps[3].1[8..], [true.into(), 1.0.into(), Value::obj([("refs", 1.0.into())])]);
        assert_eq!(steps[4].1[4], Value::obj([("refs", 1.0.into()), ("blob", (&b"\x09"[..]).into()), ("mime", "x/y".into()), ("size", 1.0.into())]));
        assert_eq!(steps[5].1[3..], vec![Value::Num(0.0); 5]);
        let name: Value = "ihn6-selftest:scratch".into();
        assert_eq!(steps[8].1, [Value::obj([("held", vec![name.clone()].into()), ("pending", vec![name].into())])]);
        assert!(!db.dbs.contains_key("scratch") && db.held.is_empty());
        assert_eq!(test_cmd(&mut app, &mut host, "health", "null").unwrap(), "{\"health\":\"closed\",\"slot\":null}");
    }

    #[test]
    fn the_degraded_scenario_reports_the_signal_and_recovers() {
        let (mut app, mut host, mut db) = boot();
        db.fail_open = Some(status::BLOCKED);
        let json = run(&mut app, &mut host, &mut db, "{\"run\":\"degraded\",\"db\":\"held\",\"version\":2,\"blockedMs\":300}");
        assert_eq!(db.opened, [("held".to_string(), 2, 300.0, 5000.0, super::super::STORES.map(String::from).to_vec())]);
        assert!(json.contains("\"health\":\"degraded:blocked\""), "{json}");
        assert_eq!(entries(&json).iter().map(|s| s.0).collect::<Vec<_>>(), [status::DEGRADED, status::DEGRADED]);
        db.fail_open = None;
        let json = run(&mut app, &mut host, &mut db, "{\"run\":\"reopen\"}");
        assert_eq!(entries(&json), [(0, vec![Value::Undef]), (0, vec![])]);
    }

    #[test]
    fn the_dump_scenario_reads_every_record_in_pages() {
        let (mut app, mut host, mut db) = boot();
        let rows = |n: u32| (1..=n).map(|i| (Value::Num(i as f64), Value::obj([("savedAt", (i * 10).into())]))).collect::<Vec<_>>();
        let mut pages = rows(7);
        let blob = Value::Blob { obj: 77, size: 3.0, mime: "application/pdf".into() };
        pages[4].1.set("blob", blob.clone());
        db.v5.insert("notebook".into(), (4, vec![("pages".into(), pages.clone()), ("empty".into(), Vec::new())]));
        db.v5.insert("offline".into(), (2, vec![("outbox".into(), rows(1))]));
        let json = run(&mut app, &mut host, &mut db, "{\"run\":\"dump\",\"db\":\"assets\",\"dbs\":\"notebook,absent,offline\"}");
        assert!(json.contains("\"pages\":5,"), "{json}");
        let (steps, dump) = json.split_at(json.find("\"dump\":").unwrap());
        // The asset written from the dumped Blob, then the scratch database deleted.
        let stored = Value::obj([("blob", blob), ("mime", "application/pdf".into()), ("size", 3.0.into()), ("refs", 1.0.into())]);
        assert_eq!(entries(steps), [(0, vec![Value::Undef, 1.0.into(), stored]), (0, vec![])]);
        let order: Vec<&str> = dump.split("{\"db\":").skip(1).map(|e| &e[..e.find(",\"data\"").unwrap().min(e.find(",\"fingerprint\"").unwrap_or(usize::MAX))]).collect();
        assert_eq!(
            order,
            [
                "\"notebook\",\"status\":0",
                "\"notebook\",\"store\":\"empty\"",
                "\"notebook\",\"store\":\"pages\"",
                "\"notebook\",\"store\":\"pages\"",
                "\"notebook\",\"store\":\"pages\"",
                "\"absent\",\"status\":1",
                "\"offline\",\"status\":0",
                "\"offline\",\"store\":\"outbox\"",
            ]
        );
        assert!(dump.contains("\"fingerprint\":[7,70,\"") && dump.contains("\"fingerprint\":[0,0,\"0000000000000000\"]") && dump.contains("\"fingerprint\":[1,10,\""), "{dump}");
        let data = entries(dump);
        let read: Vec<Value> = data[2..5].iter().flat_map(|page| page.1[0].items()[1..].to_vec()).collect();
        assert_eq!(read, pages.into_iter().flat_map(|row| [row.0, row.1]).collect::<Vec<_>>());
        assert_eq!(data[1].1, [Value::Arr(vec![true.into()])], "an empty store is one finished page");
        assert!(!db.dbs.contains_key("assets"));
        let released = host.take();
        assert_eq!(released.matches("obj_release 77").count(), 2, "the dump's handle and the stored copy's: {released}");
    }
}
