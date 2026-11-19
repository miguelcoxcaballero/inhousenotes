//! The frozen schema of IndexedDB `inhouse-notes-v6` version 1 (ARCHITECTURE §7.1): the `docs` record, the `kv`
//! keys, the mapping of `SyncEffect::Persist(SyncRecord)` to stores, and the retention rule.
//!
//! Stored form of a `docs` record (a plain object, so that tabs patch single fields inside a transaction):
//!
//! ```text
//! { account, name, drive_name: {v, observed_at: bytes(20)} | absent, driveMeta: {resourceKey, parents, canEdit,
//!   ownedByMe, owner…}, vv: bytes, seq: {actor16 → n},
//!   relay: { file_id, epoch_day, gen, archives: [{file_id, digest: bytes, ranges: bytes}],
//!            assets: [{hash: bytes(32), file_id, size}], sealed: {actor16 → n}, perm: bytes,
//!            applied: {inst16 → [gen, d16: bytes(16)]} },
//!   exports: [[headRevisionId, vv: bytes]…] (oldest first, 8 newest kept),
//!   last_export: {headRevisionId, vv: bytes, etag, at} | absent,
//!   flags: {created, relay_pending, export_pending, migrated}, lastOpenedAt, pinned }
//! ```
//!
//! `actor16`/`inst16` are 16 hex digits. `vv`, `ranges`, `perm` and `digest` are opaque to the store (encoded by
//! `ihn-core`/`ihn-sync`). A stamp is `u64 clock, u64 actor, u32 idx`, little-endian.
//!
//! Writers: every tab patches `seq[its actor]`, `lastOpenedAt`, `pinned`, `name`; the `ihn6-net` lock holder of the
//! document owns `relay`, `exports`, `last_export`, `drive_name` and the flags.

use super::sv::{hex16, unhex16, Value, W};
use super::tx::{Key, Put, Store, Tx};

/// `kv` keys (per account).
pub mod kv {
    /// The install id: 16 hex digits of a random u64 whose low 8 bits are zero.
    pub const INST: &str = "inst";
    /// Prefix of the id-counter high-water mark of an actor: `ctr:<actor16>` → number.
    pub const CTR: &str = "ctr:";
    /// Reserved Drive file ids for documents created on this device: array of strings.
    pub const IDPOOL: &str = "idpool";
    /// The relay folder id: string.
    pub const RELAY_FOLDER: &str = "relay-folder";
    /// Link sharing of the relay folder is not possible: boolean.
    pub const RELAY_NOLINK: &str = "relay-nolink";
    /// Relay file ids to delete from Drive: array of strings.
    pub const RELAY_GC: &str = "relay-gc";
    /// `{retentionDays}`.
    pub const PREFERENCES: &str = "preferences";
    /// `{recents?, starred?, shared?}`: the last Drive order per Home row.
    pub const HOME_VIEWS: &str = "home-views";
    /// `{remoteId}`: the Drive root id.
    pub const DRIVE_ROOT: &str = "drive-root";
    /// Migration progress per 5.x source, with the dump fingerprints (ARCHITECTURE §10.2).
    pub const MIGRATION: &str = "migration";
    /// The 5.x recoveries kept for the Offline documents dialog.
    pub const RECOVERIES: &str = "recoveries";

    pub fn ctr(actor: u64) -> String { format!("{CTR}{}", super::hex16(actor)) }
}

/// Exports remembered per document (`docs.exports`).
pub const KNOWN_EXPORTS: usize = 8;
/// Default of `preferences.retentionDays` (5.18.0).
pub const RETENTION_DAYS: f64 = 14.0;

/// A small map keyed by a 64-bit id (actor, install), kept sorted by key.
#[derive(Clone, Debug, PartialEq)]
pub struct IdMap<V>(Vec<(u64, V)>);

impl<V> Default for IdMap<V> {
    fn default() -> Self { IdMap(Vec::new()) }
}

impl<V> IdMap<V> {
    pub fn get(&self, id: u64) -> Option<&V> { self.0.binary_search_by_key(&id, |e| e.0).ok().map(|i| &self.0[i].1) }
    pub fn insert(&mut self, id: u64, value: V) {
        match self.0.binary_search_by_key(&id, |e| e.0) {
            Ok(i) => self.0[i].1 = value,
            Err(i) => self.0.insert(i, (id, value)),
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = (u64, &V)> { self.0.iter().map(|e| (e.0, &e.1)) }
    pub fn len(&self) -> usize { self.0.len() }
    pub fn is_empty(&self) -> bool { self.0.is_empty() }
}

impl<V> FromIterator<(u64, V)> for IdMap<V> {
    fn from_iter<I: IntoIterator<Item = (u64, V)>>(entries: I) -> Self {
        let mut map = IdMap::default();
        entries.into_iter().for_each(|(id, value)| map.insert(id, value));
        map
    }
}

impl<V, const N: usize> From<[(u64, V); N]> for IdMap<V> {
    fn from(entries: [(u64, V); N]) -> Self { entries.into_iter().collect() }
}

/// A hybrid logical clock stamp (ARCHITECTURE §4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Stamp {
    pub clock: u64,
    pub actor: u64,
    pub idx: u32,
}

impl Stamp {
    pub fn to_bytes(self) -> Vec<u8> {
        let mut b = Vec::with_capacity(20);
        b.extend_from_slice(&self.clock.to_le_bytes());
        b.extend_from_slice(&self.actor.to_le_bytes());
        b.extend_from_slice(&self.idx.to_le_bytes());
        b
    }
    pub fn from_bytes(b: &[u8]) -> Option<Stamp> {
        Some(Stamp {
            clock: u64::from_le_bytes(b.get(0..8)?.try_into().ok()?),
            actor: u64::from_le_bytes(b.get(8..16)?.try_into().ok()?),
            idx: u32::from_le_bytes(b.get(16..20)?.try_into().ok()?),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DriveName {
    pub v: String,
    pub observed_at: Stamp,
}

/// A sealed relay archive of an earlier epoch.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Archive {
    pub file_id: String,
    pub digest: Vec<u8>,
    pub ranges: Vec<u8>,
}

/// An asset uploaded as a relay file.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RelayAsset {
    pub hash: [u8; 32],
    pub file_id: String,
    pub size: f64,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Relay {
    /// The relay file of the current epoch.
    pub file_id: String,
    pub epoch_day: u32,
    pub gen: u32,
    pub archives: Vec<Archive>,
    pub assets: Vec<RelayAsset>,
    /// Per actor, the seq sealed in the relay.
    pub sealed: IdMap<u32>,
    /// Digest of the permission ids the relay files were shared with.
    pub perm: Vec<u8>,
    /// Per remote install, the relay generation applied and its digest prefix.
    pub applied: IdMap<(u32, [u8; 16])>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct LastExport {
    pub head: String,
    pub vv: Vec<u8>,
    pub etag: String,
    pub at: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Flags {
    /// The Drive file exists (false between local creation and the first export).
    pub created: bool,
    pub relay_pending: bool,
    pub export_pending: bool,
    /// Created by the 5.x migration.
    pub migrated: bool,
}

/// A `docs` record.
#[derive(Clone, Debug, PartialEq)]
pub struct DocRecord {
    pub account: String,
    pub name: String,
    pub drive_name: Option<DriveName>,
    /// `{resourceKey, parents, canEdit, ownedByMe, owner…}`: the Drive facts 5.18.0 keeps per document.
    pub drive_meta: Value,
    pub vv: Vec<u8>,
    /// Per local actor, the last seq appended to the journal.
    pub seq: IdMap<u32>,
    pub relay: Relay,
    /// `(headRevisionId, vv)` of known exports, oldest first.
    pub exports: Vec<(String, Vec<u8>)>,
    pub last_export: Option<LastExport>,
    pub flags: Flags,
    pub last_opened_at: f64,
    pub pinned: bool,
}

impl Default for DocRecord {
    fn default() -> DocRecord {
        DocRecord {
            account: String::new(),
            name: String::new(),
            drive_name: None,
            drive_meta: Value::Obj(Vec::new()),
            vv: Vec::new(),
            seq: IdMap::default(),
            relay: Relay::default(),
            exports: Vec::new(),
            last_export: None,
            flags: Flags::default(),
            last_opened_at: 0.0,
            pinned: false,
        }
    }
}

fn write_actor_map(w: &mut W, map: &IdMap<u32>) {
    w.obj(map.len());
    for (actor, seq) in map.iter() {
        w.key(&hex16(actor));
        w.num(*seq as f64);
    }
}

fn read_actor_map(v: &Value) -> IdMap<u32> {
    v.fields().iter().filter_map(|(k, v)| Some((unhex16(k)?, v.as_f64()? as u32))).collect()
}

fn write_archives(w: &mut W, archives: &[Archive]) {
    w.arr(archives.len());
    for a in archives {
        w.obj(3);
        w.key("file_id");
        w.str(&a.file_id);
        w.key("digest");
        w.bytes(&a.digest);
        w.key("ranges");
        w.bytes(&a.ranges);
    }
}

fn write_assets(w: &mut W, assets: &[RelayAsset]) {
    w.arr(assets.len());
    for a in assets {
        w.obj(3);
        w.key("hash");
        w.bytes(&a.hash);
        w.key("file_id");
        w.str(&a.file_id);
        w.key("size");
        w.num(a.size);
    }
}

fn write_applied(w: &mut W, gen: u32, d16: &[u8; 16]) {
    w.arr(2);
    w.num(gen as f64);
    w.bytes(d16);
}

fn write_exports(w: &mut W, exports: &[(String, Vec<u8>)]) {
    w.arr(exports.len());
    for (head, vv) in exports {
        w.arr(2);
        w.str(head);
        w.bytes(vv);
    }
}

fn write_strings(w: &mut W, items: &[String]) {
    w.arr(items.len());
    items.iter().for_each(|s| w.str(s));
}

impl Relay {
    fn write(&self, w: &mut W) {
        w.obj(8);
        w.key("file_id");
        w.str(&self.file_id);
        w.key("epoch_day");
        w.num(self.epoch_day as f64);
        w.key("gen");
        w.num(self.gen as f64);
        w.key("archives");
        write_archives(w, &self.archives);
        w.key("assets");
        write_assets(w, &self.assets);
        w.key("sealed");
        write_actor_map(w, &self.sealed);
        w.key("perm");
        w.bytes(&self.perm);
        w.key("applied");
        w.obj(self.applied.len());
        for (inst, (gen, d16)) in self.applied.iter() {
            w.key(&hex16(inst));
            write_applied(w, *gen, d16);
        }
    }

    pub fn from_value(v: &Value) -> Relay {
        Relay {
            file_id: v.get("file_id").text(),
            epoch_day: v.get("epoch_day").u32(),
            gen: v.get("gen").u32(),
            archives: v
                .get("archives")
                .items()
                .iter()
                .map(|a| Archive { file_id: a.get("file_id").text(), digest: a.get("digest").blob(), ranges: a.get("ranges").blob() })
                .collect(),
            assets: v
                .get("assets")
                .items()
                .iter()
                .filter_map(|a| {
                    Some(RelayAsset {
                        hash: a.get("hash").as_bytes()?.try_into().ok()?,
                        file_id: a.get("file_id").text(),
                        size: a.get("size").as_f64().unwrap_or(0.0),
                    })
                })
                .collect(),
            sealed: read_actor_map(v.get("sealed")),
            perm: v.get("perm").blob(),
            applied: v
                .get("applied")
                .fields()
                .iter()
                .filter_map(|(inst, entry)| {
                    let entry = entry.items();
                    Some((unhex16(inst)?, (entry.first()?.u32(), entry.get(1)?.as_bytes()?.try_into().ok()?)))
                })
                .collect(),
        }
    }
}

impl LastExport {
    fn write(&self, w: &mut W) {
        w.obj(4);
        w.key("headRevisionId");
        w.str(&self.head);
        w.key("vv");
        w.bytes(&self.vv);
        w.key("etag");
        w.str(&self.etag);
        w.key("at");
        w.num(self.at);
    }
}

impl DriveName {
    fn write(&self, w: &mut W) {
        w.obj(2);
        w.key("v");
        w.str(&self.v);
        w.key("observed_at");
        w.bytes(&self.observed_at.to_bytes());
    }
}

impl DocRecord {
    /// Writes the stored form (one structured value).
    pub fn write(&self, w: &mut W) {
        w.obj(10 + self.drive_name.is_some() as usize + self.last_export.is_some() as usize);
        w.key("account");
        w.str(&self.account);
        w.key("name");
        w.str(&self.name);
        if let Some(name) = &self.drive_name {
            w.key("drive_name");
            name.write(w);
        }
        w.key("driveMeta");
        match &self.drive_meta {
            Value::Undef => w.obj(0),
            meta => w.value(meta),
        }
        w.key("vv");
        w.bytes(&self.vv);
        w.key("seq");
        write_actor_map(w, &self.seq);
        w.key("relay");
        self.relay.write(w);
        w.key("exports");
        write_exports(w, &self.exports);
        if let Some(last) = &self.last_export {
            w.key("last_export");
            last.write(w);
        }
        w.key("flags");
        w.obj(4);
        for (name, on) in [
            ("created", self.flags.created),
            ("relay_pending", self.flags.relay_pending),
            ("export_pending", self.flags.export_pending),
            ("migrated", self.flags.migrated),
        ] {
            w.key(name);
            w.bool(on);
        }
        w.key("lastOpenedAt");
        w.num(self.last_opened_at);
        w.key("pinned");
        w.bool(self.pinned);
    }

    /// The stored form as a tree.
    pub fn to_value(&self) -> Value {
        let mut bytes = Vec::new();
        self.write(&mut W(&mut bytes));
        Value::decode(&bytes).unwrap_or_default()
    }

    /// Lenient: missing or mistyped fields take their defaults, unknown fields are ignored.
    pub fn from_value(v: &Value) -> DocRecord {
        let (name, last, flags) = (v.get("drive_name"), v.get("last_export"), v.get("flags"));
        DocRecord {
            account: v.get("account").text(),
            name: v.get("name").text(),
            drive_name: name.get("v").as_str().map(|text| DriveName {
                v: text.to_string(),
                observed_at: name.get("observed_at").as_bytes().and_then(Stamp::from_bytes).unwrap_or_default(),
            }),
            drive_meta: match v.get("driveMeta") {
                Value::Undef => Value::Obj(Vec::new()),
                meta => meta.clone(),
            },
            vv: v.get("vv").blob(),
            seq: read_actor_map(v.get("seq")),
            relay: Relay::from_value(v.get("relay")),
            exports: v
                .get("exports")
                .items()
                .iter()
                .filter_map(|e| Some((e.items().first()?.as_str()?.to_string(), e.items().get(1)?.blob())))
                .collect(),
            last_export: last.get("headRevisionId").as_str().map(|head| LastExport {
                head: head.to_string(),
                vv: last.get("vv").blob(),
                etag: last.get("etag").text(),
                at: last.get("at").as_f64().unwrap_or(0.0),
            }),
            flags: Flags {
                created: flags.get("created").flag(),
                relay_pending: flags.get("relay_pending").flag(),
                export_pending: flags.get("export_pending").flag(),
                migrated: flags.get("migrated").flag(),
            },
            last_opened_at: v.get("lastOpenedAt").as_f64().unwrap_or(0.0),
            pinned: v.get("pinned").flag(),
        }
    }

    /// 5.18.0's retention rule (`offline-store-v5.js:prune`) in v6 terms: the local copy of a document may be
    /// dropped when it is not pinned, not open, everything local is sealed and exported, the Drive file exists,
    /// and it was last opened more than `retention_days` ago.
    ///
    /// "Sealed" is read from the journal position itself, not only from the flags: every actor of `seq` must be
    /// covered by `relay.sealed`. `seq` is written in the transaction that appends the batch, the flags in a later
    /// one (and only by the `ihn6-net` holder), so after a crash between the two — or an append by a tab that
    /// does not hold the lock — the flags can still be clear while batches wait to be sealed.
    // v5: offline-store-v5.js:prune — !pinned && !dirty && !protected && lastOpenedAt < cutoff && no outbox row
    pub fn prunable(&self, open: bool, now_ms: f64, retention_days: f64) -> bool {
        !self.pinned
            && !open
            && self.flags.created
            && !self.flags.relay_pending
            && !self.flags.export_pending
            && self.seq.0.iter().all(|&(actor, seq)| self.relay.sealed.get(actor).is_some_and(|&sealed| seq <= sealed))
            && self.last_opened_at < now_ms - retention_days * 86_400_000.0
    }
}

/// `preferences.retentionDays`: the stored value when it is what 5.18.0 accepts (an integer 1..=3650), else 14.
// v5: offline-store-v5.js:settings, setRetention
pub fn retention_days(preferences: &Value) -> f64 {
    match preferences.get("retentionDays").as_f64() {
        Some(days) if days.fract() == 0.0 && (1.0..=3650.0).contains(&days) => days,
        _ => RETENTION_DAYS,
    }
}

/// What the sync engine asks to persist: the `SyncRecord` of SYNC §18.2, with the field layout of the
/// ARCHITECTURE §7.1 mapping table.
#[derive(Clone, Debug, PartialEq)]
pub enum Record {
    /// `docs.seq` — with the `ops` append ([`Tx::ops_append`] writes it; listed for completeness).
    Seq { actor: u64, seq: u32 },
    /// `docs.relay.gen/file_id/epoch_day` — before the upload.
    RelayPlan { gen: u32, file_id: String, epoch_day: u32 },
    /// `docs.relay.*` + `flags.relay_pending` — after both PATCHes succeed.
    RelaySealed { sealed: IdMap<u32>, archives: Vec<Archive>, assets: Vec<RelayAsset>, perm: Vec<u8>, relay_pending: bool },
    /// `docs.relay.applied` — with the received batches' append.
    RelayApplied { inst: u64, gen: u32, d16: [u8; 16] },
    /// `docs.exports` (the 8 newest).
    KnownExport { head: String, vv: Vec<u8> },
    /// `docs.last_export`.
    LastExport(LastExport),
    /// `docs.flags`.
    Flags { relay_pending: Option<bool>, export_pending: Option<bool>, created: Option<bool> },
    /// `docs.drive_name`.
    DriveName(DriveName),
    /// `kv` `idpool`.
    IdPool(Vec<String>),
    /// `kv` `relay-folder`.
    RelayFolder(String),
    /// `kv` `relay-nolink`.
    NoLink(bool),
    /// `kv` `relay-gc`.
    RelayGc(Vec<String>),
}

impl Tx {
    /// Adds the writes of one `SyncRecord` to this transaction and applies it to the in-memory `record` of the
    /// open document, so both stay equal. Document records are patched field by field (never resurrected when
    /// the document was deleted meanwhile); the `kv` records belong to `account`.
    pub fn persist(&mut self, account: &str, doc: &str, record: &mut DocRecord, r: Record) {
        let kv = |tx: &mut Tx, key: &str, value: &dyn Fn(&mut W)| tx.put_with(Store::Kv, Put::Replace, Key::Pair(account, key), value);
        match r {
            Record::Seq { actor, seq } => {
                self.doc_seq(doc, actor, seq);
                let seq = record.seq.get(actor).map_or(seq, |&old| old.max(seq));
                record.seq.insert(actor, seq);
            }
            Record::RelayPlan { gen, file_id, epoch_day } => {
                self.doc_patch_with(doc, 3, |w| {
                    w.path(&["relay", "gen"]);
                    w.num(gen as f64);
                    w.path(&["relay", "file_id"]);
                    w.str(&file_id);
                    w.path(&["relay", "epoch_day"]);
                    w.num(epoch_day as f64);
                });
                record.relay.gen = gen;
                record.relay.file_id = file_id;
                record.relay.epoch_day = epoch_day;
            }
            Record::RelaySealed { sealed, archives, assets, perm, relay_pending } => {
                self.doc_patch_with(doc, 5, |w| {
                    w.path(&["relay", "sealed"]);
                    write_actor_map(w, &sealed);
                    w.path(&["relay", "archives"]);
                    write_archives(w, &archives);
                    w.path(&["relay", "assets"]);
                    write_assets(w, &assets);
                    w.path(&["relay", "perm"]);
                    w.bytes(&perm);
                    w.path(&["flags", "relay_pending"]);
                    w.bool(relay_pending);
                });
                record.relay.sealed = sealed;
                record.relay.archives = archives;
                record.relay.assets = assets;
                record.relay.perm = perm;
                record.flags.relay_pending = relay_pending;
            }
            Record::RelayApplied { inst, gen, d16 } => {
                self.doc_patch_with(doc, 1, |w| {
                    w.path(&["relay", "applied", &hex16(inst)]);
                    write_applied(w, gen, &d16);
                });
                record.relay.applied.insert(inst, (gen, d16));
            }
            Record::KnownExport { head, vv } => {
                record.exports.retain(|e| e.0 != head);
                record.exports.push((head, vv));
                let extra = record.exports.len().saturating_sub(KNOWN_EXPORTS);
                record.exports.drain(..extra);
                self.doc_patch_with(doc, 1, |w| {
                    w.path(&["exports"]);
                    write_exports(w, &record.exports);
                });
            }
            Record::LastExport(last) => {
                self.doc_patch_with(doc, 1, |w| {
                    w.path(&["last_export"]);
                    last.write(w);
                });
                record.last_export = Some(last);
            }
            Record::Flags { relay_pending, export_pending, created } => {
                let flags = &mut record.flags;
                let changes = [
                    ("relay_pending", relay_pending, &mut flags.relay_pending),
                    ("export_pending", export_pending, &mut flags.export_pending),
                    ("created", created, &mut flags.created),
                ];
                let n = changes.iter().filter(|c| c.1.is_some()).count() as u32;
                if n > 0 {
                    self.doc_patch_with(doc, n, |w| {
                        for (name, change, flag) in changes {
                            if let Some(on) = change {
                                *flag = on;
                                w.path(&["flags", name]);
                                w.bool(on);
                            }
                        }
                    });
                }
            }
            Record::DriveName(name) => {
                self.doc_patch_with(doc, 1, |w| {
                    w.path(&["drive_name"]);
                    name.write(w);
                });
                record.drive_name = Some(name);
            }
            Record::IdPool(ids) => kv(self, kv::IDPOOL, &|w| write_strings(w, &ids)),
            Record::RelayFolder(id) => kv(self, kv::RELAY_FOLDER, &|w| w.str(&id)),
            Record::NoLink(on) => kv(self, kv::RELAY_NOLINK, &|w| w.bool(on)),
            Record::RelayGc(ids) => kv(self, kv::RELAY_GC, &|w| write_strings(w, &ids)),
        }
    }
}

#[cfg(test)]
pub(crate) fn sample_record() -> DocRecord {
    DocRecord {
        account: "ana@example.test".into(),
        name: "Cuaderno".into(),
        drive_name: Some(DriveName { v: "Cuaderno.pdf".into(), observed_at: Stamp { clock: 1 << 60, actor: u64::MAX, idx: 7 } }),
        drive_meta: Value::obj([("resourceKey", Value::Null), ("parents", vec!["root".into()].into()), ("canEdit", true.into())]),
        vv: vec![1, 2, 3],
        seq: [(0xab00, 4), (0xab01, 9)].into(),
        relay: Relay {
            file_id: "relay-1".into(),
            epoch_day: 20_400,
            gen: 3,
            archives: vec![Archive { file_id: "arch".into(), digest: vec![9; 32], ranges: vec![1, 0] }],
            assets: vec![RelayAsset { hash: [7; 32], file_id: "asset".into(), size: 1234.0 }],
            sealed: [(0xab00, 4)].into(),
            perm: vec![5; 32],
            applied: [(0xcd00, (2, [8; 16]))].into(),
        },
        exports: vec![("rev-1".into(), vec![1]), ("rev-2".into(), vec![2])],
        last_export: Some(LastExport { head: "rev-2".into(), vv: vec![2], etag: "\"e\"".into(), at: 1_793_354_400_000.0 }),
        flags: Flags { created: true, relay_pending: true, export_pending: false, migrated: true },
        last_opened_at: 1_793_354_400_000.0,
        pinned: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stored_form_is_the_frozen_layout() {
        let record = sample_record();
        let value = record.to_value();
        let names: Vec<&str> = value.fields().iter().map(|f| f.0.as_str()).collect();
        assert_eq!(
            names,
            ["account", "name", "drive_name", "driveMeta", "vv", "seq", "relay", "exports", "last_export", "flags", "lastOpenedAt", "pinned"]
        );
        let names = |v: &Value| v.fields().iter().map(|f| f.0.clone()).collect::<Vec<_>>();
        assert_eq!(names(value.get("relay")), ["file_id", "epoch_day", "gen", "archives", "assets", "sealed", "perm", "applied"]);
        assert_eq!(names(value.get("flags")), ["created", "relay_pending", "export_pending", "migrated"]);
        assert_eq!(names(value.get("last_export")), ["headRevisionId", "vv", "etag", "at"]);
        assert_eq!(names(value.get("seq")), ["000000000000ab00", "000000000000ab01"]);
        assert_eq!(value.get("drive_name").get("observed_at").as_bytes().map(<[u8]>::len), Some(20));
        assert_eq!(DocRecord::from_value(&Value::decode(&value.encode()).unwrap()), record);
        // A record without the optional parts, and garbage.
        let bare = DocRecord::default();
        assert_eq!(DocRecord::from_value(&bare.to_value()), bare);
        assert_eq!(DocRecord::from_value(&Value::Num(1.0)), bare);
    }

    #[test]
    fn retention_keeps_5_18_0_rules() {
        let day = 86_400_000.0;
        let now = 100.0 * day;
        let mut record = DocRecord { flags: Flags { created: true, ..Flags::default() }, last_opened_at: now - 15.0 * day, ..DocRecord::default() };
        assert!(record.prunable(false, now, 14.0));
        assert!(!record.prunable(true, now, 14.0), "open");
        assert!(!record.prunable(false, now, 30.0), "younger than the retention");
        for change in [
            (|r: &mut DocRecord| r.pinned = true) as fn(&mut DocRecord),
            |r| r.flags.relay_pending = true,
            |r| r.flags.export_pending = true,
            |r| r.flags.created = false,
        ] {
            let mut other = record.clone();
            change(&mut other);
            assert!(!other.prunable(false, now, 14.0));
        }
        // Unsealed own batches keep the document even when no flag says so (the flags are written later than seq).
        let actor = 0xab00;
        for (seq, sealed, prunable) in [(5, Some(3), false), (5, None, false), (5, Some(5), true), (5, Some(6), true)] {
            let mut other = DocRecord { seq: [(actor, seq), (actor + 1, 2)].into(), ..record.clone() };
            other.relay.sealed = [(actor + 1, 2)].into_iter().chain(sealed.map(|n| (actor, n))).collect();
            assert_eq!(other.prunable(false, now, 14.0), prunable, "seq {seq}, sealed {sealed:?}");
        }
        record.last_opened_at = now - 14.0 * day;
        assert!(!record.prunable(false, now, 14.0), "exactly at the cutoff stays");
        assert_eq!(retention_days(&Value::Undef), 14.0);
        assert_eq!(retention_days(&Value::obj([("retentionDays", 30.0.into())])), 30.0);
        assert_eq!(retention_days(&Value::obj([("retentionDays", 0.0.into())])), 14.0);
        assert_eq!(retention_days(&Value::obj([("retentionDays", 2.5.into())])), 14.0);
        assert_eq!(retention_days(&Value::obj([("retentionDays", 99999.0.into())])), 14.0);
        assert_eq!(retention_days(&Value::obj([("retentionDays", "7".into())])), 14.0);
    }
}
