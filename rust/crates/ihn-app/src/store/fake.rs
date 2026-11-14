//! `MemDb`: an in-memory executor of the store effects (families 3.2 and 3.9) for cargo tests — the same
//! semantics as `host/idb.js` over sorted vectors. Tests of any module that persists can use it:
//!
//! ```ignore
//! let (mut app, mut host, mut db) = store::fake::boot();
//! …                                   // code under test calls store::tx / locks::request / v5::read
//! db.pump(&mut app, &mut host);       // runs the effects, delivers the completions (and the store's timers)
//! ```
//! Records of other families stay in `host.out`. Completions for request ids owned by `Mod::Store` that the
//! store itself does not consume are collected in `done` (read with [`MemDb::take`]).

use super::sv::{Value, R, W};
use super::{fx, status};
use crate::abi::{self, fam, kind, mode, tag_mod, Config};
use crate::app::Mod;
use crate::host::FakeHost;
use crate::App;
use ihn_core::codec::{Reader, Writer};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, VecDeque};

type Rows = Vec<(Value, Value)>;

/// A booted `App` with a `FakeHost` and an empty `MemDb`.
pub fn boot() -> (App, FakeHost, MemDb) {
    let mut app = App::new(Config {
        version: "6.0.0".into(),
        url: "http://127.0.0.1:4300/?e2e=1".into(),
        user_agent: "test".into(),
        language: "en".into(),
        flags: mode::E2E | mode::ONLINE,
        viewport: (1280.0, 800.0),
        dpr: 1.0,
        max_touch_points: 0,
    });
    let mut host = FakeHost::default();
    app.init(&mut host);
    host.out.w.buf.clear();
    (app, host, MemDb::default())
}

#[derive(Default)]
pub struct MemDb {
    /// Databases by name: one row vector per store, sorted by key.
    pub dbs: HashMap<String, Vec<Rows>>,
    /// Open connections: host handle → database name.
    conns: HashMap<u32, String>,
    /// Every open: `(name, version, blocked_ms, timeout_ms, stores)`.
    pub opened: Vec<(String, u32, f64, f64, Vec<String>)>,
    /// The status the next opens complete with instead of succeeding.
    pub fail_open: Option<i32>,
    /// Statuses the next write transactions fail with (one each), instead of running.
    pub fail_writes: VecDeque<i32>,
    /// Transactions attempted (failed ones included).
    pub transactions: u32,
    /// Held locks: name → request id (0 = another tab).
    pub held: BTreeMap<String, u32>,
    waiting: Vec<(String, u32)>,
    /// No Web Locks in this context.
    pub no_locks: bool,
    /// 5.x databases: name → `(version, [(store, rows sorted by key)])`.
    pub v5: HashMap<String, (u32, Vec<(String, Rows)>)>,
    timers: Vec<(u32, f64)>,
    /// Operations of `TX_PART` records waiting for their `TX`.
    parts: Vec<u8>,
    part_ops: u32,
    /// Transactions that arrived in `TX_PART` pieces.
    pub split_transactions: u32,
    /// Completions collected from the store's test inbox: `(request, status, data)`.
    pub done: Vec<(u32, i32, Vec<u8>)>,
    /// The request ids of `done` with a non-negative status, in completion order.
    pub order: Vec<u32>,
}

/// IndexedDB key order: number < date < string < binary < array.
pub fn cmp_key(a: &Value, b: &Value) -> Ordering {
    fn rank(v: &Value) -> u8 {
        match v {
            Value::Num(_) => 0,
            Value::Date(_) => 1,
            Value::Str(_) => 2,
            Value::Bytes(_) | Value::Typed(..) => 3,
            _ => 4,
        }
    }
    match (a, b) {
        (Value::Num(x), Value::Num(y)) | (Value::Date(x), Value::Date(y)) => x.total_cmp(y),
        (Value::Str(x), Value::Str(y)) => x.encode_utf16().cmp(y.encode_utf16()),
        (Value::Arr(x), Value::Arr(y)) => {
            x.iter().zip(y).map(|(p, q)| cmp_key(p, q)).find(|o| o.is_ne()).unwrap_or(x.len().cmp(&y.len()))
        }
        _ if rank(a) == 3 && rank(b) == 3 => a.as_bytes().cmp(&b.as_bytes()),
        _ => rank(a).cmp(&rank(b)),
    }
}

enum Bounds {
    All,
    Only(Value),
    Bound(Value, Value, bool, bool),
    From(Value, bool),
    To(Value, bool),
}

impl Bounds {
    fn read(r: &mut R) -> Option<Bounds> {
        Some(match r.0.u8()? {
            0 => Bounds::All,
            1 => Bounds::Only(r.value()?),
            2 => {
                let (lo, hi, flags) = (r.value()?, r.value()?, r.0.u8()?);
                Bounds::Bound(lo, hi, flags & 1 != 0, flags & 2 != 0)
            }
            3 => Bounds::From(r.value()?, r.0.u8()? != 0),
            _ => Bounds::To(r.value()?, r.0.u8()? != 0),
        })
    }
    fn has(&self, key: &Value) -> bool {
        let above = |lo: &Value, open: bool| cmp_key(key, lo).is_gt() || (!open && cmp_key(key, lo).is_eq());
        let below = |hi: &Value, open: bool| cmp_key(key, hi).is_lt() || (!open && cmp_key(key, hi).is_eq());
        match self {
            Bounds::All => true,
            Bounds::Only(k) => cmp_key(key, k).is_eq(),
            Bounds::Bound(lo, hi, lo_open, hi_open) => above(lo, *lo_open) && below(hi, *hi_open),
            Bounds::From(lo, open) => above(lo, *open),
            Bounds::To(hi, open) => below(hi, *open),
        }
    }
}

fn find(rows: &Rows, key: &Value) -> Result<usize, usize> { rows.binary_search_by(|row| cmp_key(&row.0, key)) }

fn path(r: &mut R) -> Option<Vec<String>> {
    let n = r.0.u32()?;
    (0..n).map(|_| r.0.str().map(String::from)).collect()
}

/// The object at `path` inside `v`, created (replacing non-objects) on the way.
fn descend<'a>(mut v: &'a mut Value, path: &[String]) -> &'a mut Value {
    for name in path {
        if !matches!(v.get(name), Value::Obj(_)) {
            v.set(name, Value::Obj(Vec::new()));
        }
        let Value::Obj(fields) = v else { unreachable!() };
        v = &mut fields.iter_mut().find(|f| f.0 == *name).unwrap().1;
    }
    v
}

/// Runs the operations of one transaction on `stores`; the error is the transaction's failure status.
fn exec(stores: &mut [Rows], r: &mut R, ops: u32) -> Result<Vec<u8>, i32> {
    let bad = status::ERROR;
    let mut out = Vec::new();
    for _ in 0..ops {
        let (op, store) = (r.0.u8().ok_or(bad)?, r.0.u8().ok_or(bad)? as usize);
        let rows = stores.get_mut(store).ok_or(bad)?;
        let mut w = W(&mut out);
        match op {
            1 => {
                let key = r.value().ok_or(bad)?;
                w.value(find(rows, &key).map_or(&Value::Undef, |i| &rows[i].1));
            }
            2 => {
                let (mode, key, value) = (r.0.u8().ok_or(bad)?, r.value().ok_or(bad)?, r.value().ok_or(bad)?);
                match (find(rows, &key), mode) {
                    (Ok(_), 1) => return Err(status::CONSTRAINT),
                    (Ok(i), 2) => w.value(&rows[i].1),
                    (Ok(i), 3) => {
                        w.value(&rows[i].1);
                        if let (Value::Obj(old), Value::Obj(new)) = (&mut rows[i].1, value) {
                            let missing: Vec<_> = new.into_iter().filter(|f| old.iter().all(|o| o.0 != f.0)).collect();
                            old.extend(missing);
                        }
                    }
                    (Ok(i), _) => {
                        rows[i].1 = value;
                        w.undef();
                    }
                    (Err(i), _) => {
                        rows.insert(i, (key, value));
                        w.undef();
                    }
                }
            }
            3 => {
                let range = Bounds::read(r).ok_or(bad)?;
                rows.retain(|row| !range.has(&row.0));
                w.undef();
            }
            4 => {
                let (range, limit, flags) = (Bounds::read(r).ok_or(bad)?, r.0.u32().ok_or(bad)?, r.0.u8().ok_or(bad)?);
                let hits: Vec<&(Value, Value)> =
                    rows.iter().filter(|row| range.has(&row.0)).take(if limit == 0 { usize::MAX } else { limit as usize }).collect();
                w.arr(hits.len() * if flags & 1 != 0 { 1 } else { 2 });
                for (key, value) in hits {
                    w.value(key);
                    if flags & 1 == 0 {
                        w.value(value);
                    }
                }
            }
            5 => {
                let range = Bounds::read(r).ok_or(bad)?;
                w.num(rows.iter().filter(|row| range.has(&row.0)).count() as f64);
            }
            6 => {
                let (key, flags, n) = (r.value().ok_or(bad)?, r.0.u8().ok_or(bad)?, r.0.u32().ok_or(bad)?);
                let at = find(rows, &key);
                w.bool(at.is_ok());
                let mut record = at.map_or(Value::Obj(Vec::new()), |i| rows[i].1.clone());
                for _ in 0..n {
                    let (path, value) = (path(r).ok_or(bad)?, r.value().ok_or(bad)?);
                    let (last, parents) = path.split_last().ok_or(bad)?;
                    let parent = descend(&mut record, parents);
                    // Flag 2 (max): a number never replaces a greater (or equal) number.
                    let keep = flags & 2 != 0 && matches!((parent.get(last), &value), (Value::Num(now), Value::Num(new)) if now >= new);
                    if !keep {
                        parent.set(last, value);
                    }
                }
                match at {
                    Ok(i) => rows[i].1 = record,
                    Err(i) if flags & 1 == 0 => rows.insert(i, (key, record)),
                    Err(_) => {}
                }
            }
            7 => {
                let key = r.value().ok_or(bad)?;
                let (path, delta, flags) = (path(r).ok_or(bad)?, r.0.f64().ok_or(bad)?, r.0.u8().ok_or(bad)?);
                let at = find(rows, &key);
                let mut record = at.map_or(Value::Undef, |i| rows[i].1.clone());
                let sum = match path.split_last() {
                    None => {
                        let sum = record.as_f64().unwrap_or(0.0) + delta;
                        record = sum.into();
                        sum
                    }
                    Some((last, parents)) => {
                        let parent = descend(&mut record, parents);
                        let sum = parent.get(last).as_f64().unwrap_or(0.0) + delta;
                        parent.set(last, sum.into());
                        sum
                    }
                };
                w.num(sum);
                match (at, flags & 1 != 0 && sum <= 0.0) {
                    (Ok(i), true) => drop(rows.remove(i)),
                    (Ok(i), false) => rows[i].1 = record,
                    (Err(i), false) => rows.insert(i, (key, record)),
                    (Err(_), true) => {}
                }
            }
            _ => return Err(bad),
        }
    }
    Ok(out)
}

impl MemDb {
    /// The completion collected for `req`, if any: `(status, data)`.
    pub fn take(&mut self, req: u32) -> Option<(i32, Vec<u8>)> {
        let i = self.done.iter().position(|d| d.0 == req)?;
        let (_, status, data) = self.done.remove(i);
        Some((status, data))
    }

    /// Drops every connection, as a version change or an eviction does: transactions on them fail `CLOSED`.
    pub fn close_all(&mut self) { self.conns.clear(); }

    /// Runs effects and the store's timers until nothing is left. Timers advance the host clocks.
    pub fn pump(&mut self, app: &mut App, host: &mut FakeHost) {
        loop {
            self.pump_io(app, host);
            self.timers.sort_by(|a, b| a.1.total_cmp(&b.1));
            if self.timers.is_empty() {
                return;
            }
            let (id, ms) = self.timers.remove(0);
            host.advance(ms);
            self.timers.iter_mut().for_each(|t| t.1 -= ms);
            let mut w = Writer::default();
            w.u32(id);
            app.event(host, kind::TIMER, &w.buf);
        }
    }

    /// Runs effects until none is left, without firing timers.
    pub fn pump_io(&mut self, app: &mut App, host: &mut FakeHost) {
        while self.step(app, host) > 0 {}
    }

    /// Runs the store effects currently in the out-buffer once and delivers their completions; returns how many
    /// records it consumed.
    pub fn step(&mut self, app: &mut App, host: &mut FakeHost) -> usize {
        let buf = std::mem::take(&mut host.out.w.buf);
        let mut r = Reader::new(&buf);
        let (mut events, mut consumed) = (Vec::new(), 0);
        while !r.is_empty() {
            let start = r.buf;
            let (family, op, payload) = (r.u8().unwrap(), r.u8().unwrap(), r.bytes().unwrap());
            let mut p = Reader::new(payload);
            let ours = family == fam::FX
                && match op {
                    fx::OPEN..=fx::TX_PART | fx::LOCK_REQUEST..=fx::LOCK_QUERY => true,
                    abi::fx::TIMER_SET | abi::fx::TIMER_CLEAR => tag_mod(Reader::new(payload).u32().unwrap()) == Mod::Store as usize,
                    _ => false,
                };
            if !ours {
                host.out.w.buf.extend_from_slice(&start[..start.len() - r.buf.len()]);
                continue;
            }
            consumed += 1;
            self.run(op, &mut p, &mut events);
        }
        for (req, status, data) in events {
            let mut w = Writer::default();
            w.u32(req);
            w.i32(status);
            w.u32(0);
            w.raw(&data);
            app.event(host, kind::IO, &w.buf);
        }
        for (req, status, data) in app.st.store.inbox.drain(..) {
            if status >= 0 {
                self.order.push(req);
            }
            self.done.push((req, status, data));
        }
        consumed
    }

    fn run(&mut self, op: u8, p: &mut Reader, events: &mut Vec<(u32, i32, Vec<u8>)>) {
        let first = p.u32().unwrap();
        match op {
            abi::fx::TIMER_SET => {
                self.timers.retain(|t| t.0 != first);
                self.timers.push((first, p.f64().unwrap()));
            }
            abi::fx::TIMER_CLEAR => self.timers.retain(|t| t.0 != first),
            fx::OPEN => {
                let (id, name, version, blocked, timeout) = (p.u32().unwrap(), p.str().unwrap().to_string(), p.u32().unwrap(), p.f64().unwrap(), p.f64().unwrap());
                let stores: Vec<String> = (0..p.u32().unwrap()).map(|_| p.str().unwrap().to_string()).collect();
                let status = self.fail_open.unwrap_or(status::OK);
                if status == status::OK {
                    self.dbs.entry(name.clone()).or_insert_with(|| vec![Vec::new(); stores.len()]);
                    self.conns.insert(id, name.clone());
                }
                self.opened.push((name, version, blocked, timeout, stores));
                events.push((first, status, Vec::new()));
            }
            fx::CLOSE => drop(self.conns.remove(&first)),
            fx::TX_PART => {
                // `first` is the operation count; the operations follow.
                self.part_ops += first;
                self.parts.extend_from_slice(p.buf);
            }
            fx::TX => {
                let (conn, flags, mut ops) = (p.u32().unwrap(), p.u8().unwrap(), p.u32().unwrap());
                let parts = std::mem::take(&mut self.parts);
                let joined = if flags & 4 != 0 {
                    ops += std::mem::take(&mut self.part_ops);
                    self.split_transactions += 1;
                    [&parts[..], p.buf].concat()
                } else {
                    p.buf.to_vec()
                };
                let Some(stores) = self.conns.get(&conn).and_then(|name| self.dbs.get_mut(name)) else {
                    return events.push((first, status::CLOSED, b"closed".to_vec()));
                };
                self.transactions += 1;
                let failure = if flags & 1 != 0 { self.fail_writes.pop_front() } else { None };
                let mut copy = stores.clone();
                let mut r = R(Reader::new(&joined));
                match failure.map_or_else(|| exec(&mut copy, &mut r, ops), Err) {
                    Ok(results) => {
                        assert!(r.is_empty(), "trailing bytes after the last operation");
                        *stores = copy;
                        events.push((first, status::OK, results));
                    }
                    Err(status) => events.push((first, status, b"failed".to_vec())),
                }
            }
            fx::DELETE_DB => {
                let name = p.str().unwrap();
                self.dbs.remove(name);
                self.v5.remove(name);
                self.conns.retain(|_, n| n != name);
                events.push((first, status::OK, Vec::new()));
            }
            fx::V5_INFO => {
                let (name, fields) = (p.str().unwrap(), p.str().unwrap());
                let Some((version, stores)) = self.v5.get(name) else { return events.push((first, 1, Vec::new())) };
                let mut stores: Vec<&(String, Rows)> = stores.iter().collect();
                stores.sort_by(|a, b| a.0.cmp(&b.0));
                let stores = stores.iter().map(|(name, rows)| {
                    let max = |f: &dyn Fn(&(Value, Value)) -> f64| rows.iter().map(f).fold(0.0, f64::max);
                    Value::obj([
                        ("name", name.as_str().into()),
                        ("count", (rows.len() as f64).into()),
                        ("maxKey", max(&|row| row.0.as_f64().unwrap_or(0.0)).into()),
                        ("stamp", max(&|row| fields.split(',').filter_map(|f| row.1.get(f).as_f64()).fold(0.0, f64::max)).into()),
                    ])
                });
                let info = Value::obj([("version", (*version).into()), ("stores", Value::Arr(stores.collect()))]);
                events.push((first, status::OK, info.encode()));
            }
            fx::V5_READ => {
                let (name, store) = (p.str().unwrap(), p.str().unwrap());
                let mut r = R(*p);
                let (after, limit, max_bytes) = (r.value().unwrap(), r.0.u32().unwrap(), r.0.f64().unwrap());
                let Some(rows) = self.v5.get(name).and_then(|db| db.1.iter().find(|s| s.0 == store)) else {
                    return events.push((first, 1, Vec::new()));
                };
                let mut rest = rows.1.iter().filter(|row| after.is_undef() || cmp_key(&row.0, &after).is_gt()).peekable();
                let (mut body, mut n) = (Vec::new(), 0);
                while let Some(row) = rest.peek() {
                    if n > 0 && ((limit > 0 && n >= limit) || body.len() as f64 >= max_bytes) {
                        break;
                    }
                    W(&mut body).value(&row.0);
                    W(&mut body).value(&row.1);
                    n += 1;
                    rest.next();
                }
                let mut data = Vec::new();
                let mut w = W(&mut data);
                w.arr(1 + 2 * n as usize);
                w.bool(rest.peek().is_none());
                data.extend_from_slice(&body);
                events.push((first, status::OK, data));
            }
            fx::LOCK_REQUEST => {
                let (name, flags) = (p.str().unwrap().to_string(), p.u8().unwrap());
                if self.no_locks {
                    events.push((first, status::UNAVAILABLE, Vec::new()));
                } else if !self.held.contains_key(&name) {
                    self.held.insert(name, first);
                    events.push((first, 1, Vec::new()));
                } else if flags & 1 != 0 {
                    events.push((first, 0, Vec::new()));
                } else {
                    self.waiting.push((name, first));
                }
            }
            fx::LOCK_RELEASE => {
                self.waiting.retain(|w| w.1 != first);
                let Some(name) = self.held.iter().find(|h| *h.1 == first).map(|h| h.0.clone()) else { return };
                self.held.remove(&name);
                if let Some(i) = self.waiting.iter().position(|w| w.0 == name) {
                    let (name, req) = self.waiting.remove(i);
                    self.held.insert(name, req);
                    events.push((req, 1, Vec::new()));
                }
            }
            fx::LOCK_QUERY => {
                if self.no_locks {
                    return events.push((first, status::UNAVAILABLE, Vec::new()));
                }
                let prefix = p.str().unwrap();
                let pick = |names: &mut dyn Iterator<Item = &String>| {
                    let mut names: Vec<&String> = names.filter(|n| n.starts_with(prefix)).collect();
                    names.sort();
                    Value::Arr(names.into_iter().map(|n| n.as_str().into()).collect())
                };
                let v = Value::obj([("held", pick(&mut self.held.keys())), ("pending", pick(&mut self.waiting.iter().map(|w| &w.0)))]);
                events.push((first, status::OK, v.encode()));
            }
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_sort_like_indexeddb() {
        let arr = |items: Vec<Value>| Value::Arr(items);
        let sorted = [
            Value::Num(-1.0),
            Value::Num(10.0),
            Value::Date(0.0),
            "".into(),
            "a".into(),
            "b".into(),
            Value::Bytes(vec![0]),
            arr(vec![]),
            arr(vec!["doc".into()]),
            arr(vec!["doc".into(), 2.0.into()]),
            arr(vec!["doc".into(), 10.0.into()]),
            arr(vec!["doc".into(), "00ff".into(), 1.0.into()]),
            arr(vec!["doc".into(), "00ff".into(), arr(vec![])]),
            arr(vec!["doc".into(), arr(vec![])]),
            arr(vec!["doc2".into()]),
        ];
        for (i, a) in sorted.iter().enumerate() {
            for (j, b) in sorted.iter().enumerate() {
                assert_eq!(cmp_key(a, b), i.cmp(&j), "{a:?} vs {b:?}");
            }
        }
    }
}
