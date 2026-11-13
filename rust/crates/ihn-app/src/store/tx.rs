//! Transactions as effects: [`Tx`] collects the operations of one IndexedDB transaction, `store::tx` sends it, and
//! the completion (`Event::Io` at the module that owns the request id) is read with [`Reply`]. All operations of a
//! `Tx` commit together or not at all.
//!
//! Wire form of an operation (`readOp` in `host/idb.js`): `u8 op, u8 store, …`; keys and values are structured
//! values ([`super::sv`]). A reply holds one value per operation, in order.
//!
//! | Op | Payload | Result |
//! |---|---|---|
//! | 1 `get` | key | the value or `undefined` |
//! | 2 `put` | u8 mode (0 put, 1 add: abort when the key exists, 2 only when absent), key, value | mode 2: the value already there, else `undefined` |
//! | 3 `del` | range | `undefined` |
//! | 4 `scan` | range, u32 limit (0 = none), u8 flags (1 = keys only) | array `[key, value, key, value…]` or `[key…]`, in key order |
//! | 5 `count` | range | number |
//! | 6 `patch` | key, u8 flags (1 = only when the record exists), u32 n, n × (path, value) | whether the record existed |
//! | 7 `add` | key, path, f64 delta, u8 flags (1 = delete the record when the sum is ≤ 0) | the sum |
//!
//! `patch` sets fields of an object record (`undefined` deletes the field; missing parents are created); `add` adds
//! to a numeric field, or to the record itself when the path is empty. Both read and write inside the transaction,
//! so concurrent tabs never lose each other's fields.
//!
//! Range: `u8 kind` — 0 all, 1 only(key), 2 bound(lower, upper, u8 flags: 1 lower open, 2 upper open),
//! 3 lower bound(key, u8 open), 4 upper bound(key, u8 open).

use super::schema::DocRecord;
use super::sv::{self, Value, R, W};
use crate::abi::Io;

/// Object stores of `inhouse-notes-v6` (ARCHITECTURE §7.1); the discriminant is the index sent in operations.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Store {
    /// `docId` → [`DocRecord`].
    Docs = 0,
    /// `[docId, chunkId]` → bytes (chunk 0 = document header, then one chunk per page).
    Chunks = 1,
    /// `[docId, actor, seq]` → encoded `Batch` (the journal).
    Ops = 2,
    /// `hash` → `{blob, mime, size, refs, remap?}`.
    Assets = 3,
    /// `[account, fileId]` → Drive entry cache.
    Catalog = 4,
    /// `[account, key]` → value.
    Kv = 5,
}

/// Store names, indexed by [`Store`]. All stores use out-of-line keys and have no indexes.
pub const STORES: [&str; 6] = ["docs", "chunks", "ops", "assets", "catalog", "kv"];

/// Completion statuses of the store effects (`Io::status`).
pub mod status {
    pub const OK: i32 = 0;
    /// Any other failure; `Io::data` is the message.
    pub const ERROR: i32 = -1;
    /// The open (or delete) is blocked by another connection.
    pub const BLOCKED: i32 = -2;
    pub const TIMEOUT: i32 = -3;
    /// No IndexedDB / Web Locks in this context.
    pub const UNAVAILABLE: i32 = -4;
    /// The connection was closed (version change by a newer tab, storage eviction).
    pub const CLOSED: i32 = -5;
    pub const QUOTA: i32 = -6;
    /// `add` found the key: nothing of the transaction was written.
    pub const CONSTRAINT: i32 = -7;
    /// The store is in degraded mode (ARCHITECTURE §4.4): the transaction was not attempted.
    pub const DEGRADED: i32 = -16;
}

/// An IndexedDB key.
#[derive(Clone, Copy, Debug)]
pub enum Key<'a> {
    Str(&'a str),
    /// `[a, b]`.
    Pair(&'a str, &'a str),
    /// `[docId, chunkId]`.
    Chunk(&'a str, u32),
    /// `[docId, actor, seq]`.
    Op(&'a str, u64, u32),
    /// `[a]`: sorts before every key that starts with `a`.
    Prefix(&'a str),
    /// `[a, []]`: sorts after every `[a, number | string…]`.
    PrefixEnd(&'a str),
    /// `[docId, actor]` and `[docId, actor, []]`: the bounds of one actor's journal.
    Actor(&'a str, u64),
    ActorEnd(&'a str, u64),
    Value(&'a Value),
}

impl Key<'_> {
    fn write(self, w: &mut W) {
        match self {
            Key::Str(s) => w.str(s),
            Key::Pair(a, b) => {
                w.arr(2);
                w.str(a);
                w.str(b);
            }
            Key::Chunk(doc, id) => {
                w.arr(2);
                w.str(doc);
                w.num(id as f64);
            }
            Key::Op(doc, actor, seq) => {
                w.arr(3);
                w.str(doc);
                w.hex16(actor);
                w.num(seq as f64);
            }
            Key::Prefix(a) => {
                w.arr(1);
                w.str(a);
            }
            Key::PrefixEnd(a) => {
                w.arr(2);
                w.str(a);
                w.arr(0);
            }
            Key::Actor(doc, actor) => {
                w.arr(2);
                w.str(doc);
                w.hex16(actor);
            }
            Key::ActorEnd(doc, actor) => {
                w.arr(3);
                w.str(doc);
                w.hex16(actor);
                w.arr(0);
            }
            Key::Value(v) => w.value(v),
        }
    }
}

/// A key range.
#[derive(Clone, Copy, Debug)]
pub enum Range<'a> {
    All,
    Only(Key<'a>),
    /// Lower, upper, lower open, upper open.
    Bound(Key<'a>, Key<'a>, bool, bool),
    /// Keys from (after, when open) this one.
    From(Key<'a>, bool),
    To(Key<'a>, bool),
}

impl<'a> Range<'a> {
    /// Every key `[a, …]`.
    pub fn prefix(a: &'a str) -> Range<'a> { Range::Bound(Key::Prefix(a), Key::PrefixEnd(a), false, false) }
    fn write(self, w: &mut W) {
        match self {
            Range::All => w.0.push(0),
            Range::Only(k) => {
                w.0.push(1);
                k.write(w);
            }
            Range::Bound(lo, hi, lo_open, hi_open) => {
                w.0.push(2);
                lo.write(w);
                hi.write(w);
                w.0.push(lo_open as u8 | (hi_open as u8) << 1);
            }
            Range::From(k, open) | Range::To(k, open) => {
                w.0.push(if matches!(self, Range::From(..)) { 3 } else { 4 });
                k.write(w);
                w.0.push(open as u8);
            }
        }
    }
}

/// `put` modes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Put {
    /// Insert or replace.
    Replace = 0,
    /// Insert; when the key exists the whole transaction aborts with [`status::CONSTRAINT`].
    Add = 1,
    /// Insert when absent; the result is the value already there (`undefined` when this one was written).
    IfAbsent = 2,
}

/// The operations of one transaction.
#[derive(Default, Clone)]
pub struct Tx {
    pub(crate) buf: Vec<u8>,
    pub(crate) ops: u32,
    pub(crate) write: bool,
    /// `{durability: 'strict'}` (fsync before the completion). The default is the browser's relaxed durability,
    /// which survives a tab or browser crash.
    pub strict: bool,
}

impl Tx {
    pub fn new() -> Tx { Tx::default() }
    /// Number of operations (= number of results in the reply).
    pub fn len(&self) -> u32 { self.ops }
    pub fn is_empty(&self) -> bool { self.ops == 0 }
    pub fn is_write(&self) -> bool { self.write }

    fn op(&mut self, op: u8, store: Store, write: bool) -> W<'_> {
        self.ops += 1;
        self.write |= write;
        self.buf.extend_from_slice(&[op, store as u8]);
        W(&mut self.buf)
    }

    // ---- generic operations ----

    pub fn get(&mut self, store: Store, key: Key) { key.write(&mut self.op(1, store, false)); }
    /// `put` whose value is written by `value` (exactly one structured value).
    pub fn put_with(&mut self, store: Store, mode: Put, key: Key, value: impl FnOnce(&mut W)) {
        let mut w = self.op(2, store, true);
        w.0.push(mode as u8);
        key.write(&mut w);
        value(&mut w);
    }
    pub fn put(&mut self, store: Store, mode: Put, key: Key, value: &Value) {
        self.put_with(store, mode, key, |w| w.value(value));
    }
    pub fn del(&mut self, store: Store, range: Range) { range.write(&mut self.op(3, store, true)); }
    /// Keys and values (or only keys) of a range in key order; `limit` 0 = all.
    pub fn scan(&mut self, store: Store, range: Range, limit: u32, keys_only: bool) {
        let mut w = self.op(4, store, false);
        range.write(&mut w);
        w.0.extend_from_slice(&limit.to_le_bytes());
        w.0.push(keys_only as u8);
    }
    pub fn count(&mut self, store: Store, range: Range) { range.write(&mut self.op(5, store, false)); }
    /// Sets fields of an object record, creating the record (unless `only_existing`) and missing parents. A field
    /// set to `Value::Undef` is deleted.
    pub fn patch(&mut self, store: Store, key: Key, only_existing: bool, fields: &[(&[&str], Value)]) {
        self.patch_with(store, key, only_existing, fields.len() as u32, |w| {
            for (path, value) in fields {
                w.path(path);
                w.value(value);
            }
        });
    }
    /// `patch` whose `n` fields are written by `fields`: each one [`W::path`] followed by one value.
    pub fn patch_with(&mut self, store: Store, key: Key, only_existing: bool, n: u32, fields: impl FnOnce(&mut W)) {
        let mut w = self.op(6, store, true);
        key.write(&mut w);
        w.0.push(only_existing as u8);
        w.0.extend_from_slice(&n.to_le_bytes());
        fields(&mut w);
    }
    /// Adds `delta` to the number at `path` (the record itself when the path is empty; missing = 0); with
    /// `delete_at_zero` the record is deleted when the sum is ≤ 0. The result is the sum.
    pub fn add(&mut self, store: Store, key: Key, path: &[&str], delta: f64, delete_at_zero: bool) {
        let mut w = self.op(7, store, true);
        key.write(&mut w);
        w.path(path);
        w.0.extend_from_slice(&delta.to_le_bytes());
        w.0.push(delete_at_zero as u8);
    }

    // ---- kv: [account, key] ----

    pub fn kv_get(&mut self, account: &str, key: &str) { self.get(Store::Kv, Key::Pair(account, key)); }
    pub fn kv_put(&mut self, account: &str, key: &str, value: &Value) {
        self.put(Store::Kv, Put::Replace, Key::Pair(account, key), value);
    }
    /// Writes the value unless one exists; the result is the existing one (how `inst` is created exactly once
    /// among racing tabs).
    pub fn kv_put_if_absent(&mut self, account: &str, key: &str, value: &Value) {
        self.put(Store::Kv, Put::IfAbsent, Key::Pair(account, key), value);
    }
    pub fn kv_del(&mut self, account: &str, key: &str) { self.del(Store::Kv, Range::Only(Key::Pair(account, key))); }
    /// Atomic counter (`ctr:<actor>` id blocks): the result is the new high-water mark.
    pub fn kv_add(&mut self, account: &str, key: &str, delta: f64) {
        self.add(Store::Kv, Key::Pair(account, key), &[], delta, false);
    }
    /// Every `[key, value]` of an account.
    pub fn kv_scan(&mut self, account: &str) { self.scan(Store::Kv, Range::prefix(account), 0, false); }

    // ---- docs ----

    pub fn doc_get(&mut self, doc: &str) { self.get(Store::Docs, Key::Str(doc)); }
    /// Writes the whole record: only when creating a document (or migrating one). Later changes are patches.
    pub fn doc_put(&mut self, doc: &str, record: &DocRecord) {
        self.put_with(Store::Docs, Put::Replace, Key::Str(doc), |w| record.write(w));
    }
    /// Sets fields of an existing document record; a record deleted meanwhile is not resurrected.
    pub fn doc_patch(&mut self, doc: &str, fields: &[(&[&str], Value)]) {
        self.patch(Store::Docs, Key::Str(doc), true, fields);
    }
    pub fn doc_patch_with(&mut self, doc: &str, n: u32, fields: impl FnOnce(&mut W)) {
        self.patch_with(Store::Docs, Key::Str(doc), true, n, fields);
    }
    /// Every document record (all accounts; filter by `DocRecord::account`).
    pub fn docs_scan(&mut self) { self.scan(Store::Docs, Range::All, 0, false); }
    /// Removes a document with its chunks and journal (retention, delete forever). Asset references are the
    /// caller's ([`Tx::asset_ref`]).
    pub fn doc_delete(&mut self, doc: &str) {
        self.del(Store::Docs, Range::Only(Key::Str(doc)));
        self.del(Store::Chunks, Range::prefix(doc));
        self.del(Store::Ops, Range::prefix(doc));
    }

    // ---- chunks: [docId, chunkId] ----

    pub fn chunk_put(&mut self, doc: &str, chunk: u32, bytes: &[u8]) {
        self.put_with(Store::Chunks, Put::Replace, Key::Chunk(doc, chunk), |w| w.bytes(bytes));
    }
    pub fn chunk_del(&mut self, doc: &str, chunk: u32) { self.del(Store::Chunks, Range::Only(Key::Chunk(doc, chunk))); }
    /// All chunks of a document ([`Reply::chunks`]).
    pub fn chunk_scan(&mut self, doc: &str) { self.scan(Store::Chunks, Range::prefix(doc), 0, false); }

    // ---- ops (journal): [docId, actor, seq] ----

    /// Appends an own batch and bumps `docs.seq[actor]` — one transaction (SYNC §7.2). A `seq` that is already in
    /// the journal aborts everything with [`status::CONSTRAINT`]. Two results: `undefined`, then whether the
    /// document record existed.
    pub fn ops_append(&mut self, doc: &str, actor: u64, seq: u32, batch: &[u8]) {
        self.put_with(Store::Ops, Put::Add, Key::Op(doc, actor, seq), |w| w.bytes(batch));
        self.doc_patch_with(doc, 1, |w| {
            w.path(&["seq", &sv::hex16(actor)]);
            w.num(seq as f64);
        });
    }
    /// Stores a received batch (idempotent: a duplicate replaces identical bytes).
    pub fn ops_put(&mut self, doc: &str, actor: u64, seq: u32, batch: &[u8]) {
        self.put_with(Store::Ops, Put::Replace, Key::Op(doc, actor, seq), |w| w.bytes(batch));
    }
    /// The whole journal of a document in `(actor, seq)` order ([`Reply::ops`]).
    pub fn ops_scan(&mut self, doc: &str) { self.scan(Store::Ops, Range::prefix(doc), 0, false); }
    /// One actor's batches with `seq > after`, at most `limit` (0 = all).
    pub fn ops_scan_actor(&mut self, doc: &str, actor: u64, after: u32, limit: u32) {
        self.scan(Store::Ops, Range::Bound(Key::Op(doc, actor, after), Key::ActorEnd(doc, actor), true, false), limit, false);
    }
    /// Drops one actor's batches with `seq <= through` (journal retention, ARCHITECTURE §4.4).
    pub fn ops_trim(&mut self, doc: &str, actor: u64, through: u32) {
        self.del(Store::Ops, Range::Bound(Key::Actor(doc, actor), Key::Op(doc, actor, through), false, false));
    }

    // ---- assets: hash → {blob, mime, size, refs, remap?} ----

    pub fn asset_get(&mut self, hash: &[u8; 32]) { self.get(Store::Assets, Key::Str(&sv::hex(hash))); }
    /// Stores an asset unless it is already there (content-addressed: same hash, same bytes). `blob` is the JS
    /// object handle of the `Blob`; the caller keeps owning the handle. New assets start with `refs` 0.
    pub fn asset_put(&mut self, hash: &[u8; 32], blob: u32, mime: &str, size: f64) {
        self.put_with(Store::Assets, Put::IfAbsent, Key::Str(&sv::hex(hash)), |w| {
            w.obj(4);
            w.key("blob");
            w.blob(blob, size, mime);
            w.key("mime");
            w.str(mime);
            w.key("size");
            w.num(size);
            w.key("refs");
            w.num(0.0);
        });
    }
    /// Adds to the reference count; with `collect` the asset is deleted when the count reaches 0. The result is
    /// the new count.
    pub fn asset_ref(&mut self, hash: &[u8; 32], delta: i32, collect: bool) {
        self.add(Store::Assets, Key::Str(&sv::hex(hash)), &["refs"], delta as f64, collect);
    }
    /// Page remap of a source PDF rebuilt from an export (ARCHITECTURE §4.5): source page → page in this blob.
    pub fn asset_remap(&mut self, hash: &[u8; 32], remap: &[u32]) {
        self.patch_with(Store::Assets, Key::Str(&sv::hex(hash)), true, 1, |w| {
            w.path(&["remap"]);
            w.arr(remap.len());
            remap.iter().for_each(|&page| w.num(page as f64));
        });
    }
    /// The hashes of all stored assets (keys only: no `Blob` handle is created).
    pub fn asset_keys(&mut self) { self.scan(Store::Assets, Range::All, 0, true); }

    // ---- catalog: [account, fileId] ----

    pub fn catalog_get(&mut self, account: &str, file: &str) { self.get(Store::Catalog, Key::Pair(account, file)); }
    pub fn catalog_put(&mut self, account: &str, file: &str, entry: &Value) {
        self.put(Store::Catalog, Put::Replace, Key::Pair(account, file), entry);
    }
    /// Merges fields into an entry (5.18.0 `cacheEntries` merges field by field), creating it when missing.
    pub fn catalog_patch(&mut self, account: &str, file: &str, fields: &[(&[&str], Value)]) {
        self.patch(Store::Catalog, Key::Pair(account, file), false, fields);
    }
    pub fn catalog_del(&mut self, account: &str, file: &str) {
        self.del(Store::Catalog, Range::Only(Key::Pair(account, file)));
    }
    /// Every entry of an account ([`Reply::rows`]; the key is `[account, fileId]`).
    pub fn catalog_scan(&mut self, account: &str) { self.scan(Store::Catalog, Range::prefix(account), 0, false); }
}

/// The completion of a transaction: one result per operation, read in order.
pub struct Reply<'a> {
    pub status: i32,
    /// The failure message when `status < 0`.
    pub message: &'a str,
    r: R<'a>,
}

impl<'a> Reply<'a> {
    pub fn new(io: &Io<'a>) -> Reply<'a> {
        let ok = io.status >= 0;
        Reply {
            status: io.status,
            message: if ok { "" } else { std::str::from_utf8(io.data).unwrap_or("") },
            r: R::new(if ok { io.data } else { &[] }),
        }
    }
    pub fn ok(&self) -> bool { self.status >= 0 }
    /// The raw cursor (zero-copy access to the next results).
    pub fn reader(&mut self) -> &mut R<'a> { &mut self.r }
    /// The next result as a tree (`Undef` when there is none).
    pub fn value(&mut self) -> Value { self.r.value().unwrap_or_default() }
    pub fn skip(&mut self) { self.r.skip(); }
    /// The next result as a number (`count`, `add`).
    pub fn num(&mut self) -> Option<f64> { self.r.num() }
    /// The next result as a boolean (`patch`: the record existed).
    pub fn flag(&mut self) -> Option<bool> { self.r.bool() }
    /// The next result as bytes (`get` of a chunk or batch).
    pub fn bytes(&mut self) -> Option<&'a [u8]> { self.r.bytes() }
    /// The next result as a document record (`doc_get`).
    pub fn doc(&mut self) -> Option<DocRecord> {
        match self.value() {
            v @ Value::Obj(_) => Some(DocRecord::from_value(&v)),
            _ => None,
        }
    }
    /// A `scan` result as `(key, value)` rows.
    pub fn rows(&mut self) -> Vec<(Value, Value)> {
        let n = self.r.arr().unwrap_or(0) / 2;
        (0..n).map_while(|_| Some((self.r.value()?, self.r.value()?))).collect()
    }
    /// A keys-only `scan` result.
    pub fn keys(&mut self) -> Vec<Value> {
        let n = self.r.arr().unwrap_or(0);
        (0..n).map_while(|_| self.r.value()).collect()
    }
    /// `ops_scan*`: `(actor, seq, batch)` in key order, borrowing the batches from the event payload.
    pub fn ops(&mut self) -> Vec<(u64, u32, &'a [u8])> {
        let n = self.r.arr().unwrap_or(0) / 2;
        (0..n)
            .map_while(|_| {
                let key = self.r.value()?;
                let bytes = self.r.bytes();
                Some((key, bytes))
            })
            .filter_map(|(key, bytes)| {
                let k = key.items();
                Some((sv::unhex16(k.get(1)?.as_str()?)?, k.get(2)?.as_f64()? as u32, bytes?))
            })
            .collect()
    }
    /// `chunk_scan`: `(chunkId, bytes)` in id order.
    pub fn chunks(&mut self) -> Vec<(u32, &'a [u8])> {
        let n = self.r.arr().unwrap_or(0) / 2;
        (0..n)
            .map_while(|_| {
                let key = self.r.value()?;
                let bytes = self.r.bytes();
                Some((key, bytes))
            })
            .filter_map(|(key, bytes)| Some((key.items().get(1)?.as_f64()? as u32, bytes?)))
            .collect()
    }
}
