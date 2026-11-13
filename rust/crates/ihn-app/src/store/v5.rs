//! Read-only dumper of the 5.x IndexedDB databases (ARCHITECTURE §10.1), executed by `host/idb.js`.
//!
//! The host never creates, upgrades or writes a 5.x database: it opens the existing version without a version
//! number (an open that would create the database is aborted), reads through read-only transactions and closes
//! the connection after every request, so a 5.x tab is never blocked.
//!
//! * [`info`] → [`Info`]: per store the record count, the greatest numeric key and the greatest value of the
//!   [`STAMP_FIELDS`] — the input of the migration [`Fingerprint`] (ARCHITECTURE §10.2).
//! * [`read`] → [`Page`]: records of one store in key order, in pages bounded by count and bytes. Values are
//!   structured values ([`super::sv`]); a `Blob` arrives as a JS object handle with its size and type, never as
//!   bytes — release the handles of a page with [`Page::release`] or keep the ones to read.
//!
//! The records are decoded by `ihn-core::v5::idb`.

use super::sv::{Value, R};
use super::{fx, status};
use crate::abi::{fam, Io, OutBuf};
use crate::Host;

/// The 5.x databases (persistence spec §1.1–1.3).
pub const OFFLINE_DB: &str = "inhouse-offline-v1";
pub const NOTEBOOK_DB: &str = "notebook-data-db-v1";
pub const BACKGROUND_NOTEBOOK_DB: &str = "background-notebook-data-db-v1";
pub const ORIGINALS_DB: &str = "inhouse-original-pdfs-v1";
pub const DATABASES: [&str; 4] = [OFFLINE_DB, NOTEBOOK_DB, BACKGROUND_NOTEBOOK_DB, ORIGINALS_DB];

/// Top-level numeric record fields that 5.x raises whenever it writes user data: `savedAt` (working cache),
/// `updatedAt`/`generation`/`contentVersion` (offline documents), `createdAt` (outbox, recoveries), `cachedAt`.
pub const STAMP_FIELDS: &str = "savedAt,updatedAt,createdAt,cachedAt,generation,contentVersion";

/// Stores that 5.x rewrites without any user edit (lease heartbeats, the fast-open cache): not part of a
/// fingerprint, never migrated (ARCHITECTURE §10.3).
pub const VOLATILE_STORES: [&str; 2] = ["leases", "drive-open-cache"];

/// `Io::status` of [`info`] and [`read`] when the database (or store) does not exist.
pub const MISSING: i32 = 1;

/// Asks for the [`Info`] of a 5.x database. Completion: [`status::OK`] with the info, [`MISSING`], or a
/// negative [`status`].
pub fn info(host: &mut dyn Host, req: u32, db: &str) {
    let out = host.out();
    let m = out.w.rec(fam::FX, fx::V5_INFO);
    out.w.u32(req);
    out.w.str(db);
    out.w.str(STAMP_FIELDS);
    out.w.end(m);
}

/// Reads records of `store` with a key greater than `after` (`None`: from the start), at most `limit` records
/// (0 = no limit) and, after the first record, at most about `max_bytes` of encoded data. Completion:
/// [`status::OK`] with a [`Page`], [`MISSING`], or a negative [`status`].
pub fn read(host: &mut dyn Host, req: u32, db: &str, store: &str, after: Option<&Value>, limit: u32, max_bytes: f64) {
    let out = host.out();
    let m = out.w.rec(fam::FX, fx::V5_READ);
    out.w.u32(req);
    out.w.str(db);
    out.w.str(store);
    super::sv::W(&mut out.w.buf).value(after.unwrap_or(&Value::Undef));
    out.w.u32(limit);
    out.w.f64(max_bytes);
    out.w.end(m);
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoreInfo {
    pub name: String,
    pub count: f64,
    /// The greatest numeric key (auto-increment stores), 0 when there is none.
    pub max_key: f64,
    /// The greatest value of any of [`STAMP_FIELDS`] in any record, 0 when there is none.
    pub stamp: f64,
}

/// What a 5.x database holds, without its records.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub version: u32,
    /// Sorted by name.
    pub stores: Vec<StoreInfo>,
}

impl Info {
    pub fn parse(io: &Io) -> Option<Info> {
        if io.status != status::OK {
            return None;
        }
        let v = Value::decode(io.data)?;
        Some(Info {
            version: v.get("version").u32(),
            stores: v
                .get("stores")
                .items()
                .iter()
                .map(|s| StoreInfo {
                    name: s.get("name").text(),
                    count: s.get("count").as_f64().unwrap_or(0.0),
                    max_key: s.get("maxKey").as_f64().unwrap_or(0.0),
                    stamp: s.get("stamp").as_f64().unwrap_or(0.0),
                })
                .collect(),
        })
    }

    /// The fingerprint of this database: equal fingerprints of two dumps mean no 5.x tab wrote user data in
    /// between (ARCHITECTURE §10.2 step 3). [`VOLATILE_STORES`] are left out.
    pub fn fingerprint(&self) -> Fingerprint {
        let (mut records, mut stamp, mut hash) = (0.0, 0.0f64, 0xcbf2_9ce4_8422_2325u64);
        let mut mix = |bytes: &[u8]| bytes.iter().for_each(|b| hash = (hash ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3));
        mix(&self.version.to_le_bytes());
        for s in self.stores.iter().filter(|s| !VOLATILE_STORES.contains(&s.name.as_str())) {
            mix(s.name.as_bytes());
            for n in [s.count, s.max_key, s.stamp] {
                mix(&n.to_le_bytes());
            }
            records += s.count;
            stamp = stamp.max(s.stamp).max(s.max_key);
        }
        Fingerprint { records, stamp, hash }
    }
}

/// Per database: the record count, the greatest `savedAt`-like stamp or auto-increment id, and a hash over the
/// per-store numbers (FNV-1a 64) so that changes that cancel out in the totals are still seen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fingerprint {
    pub records: f64,
    pub stamp: f64,
    pub hash: u64,
}

impl Fingerprint {
    /// The form stored in `kv.migration`.
    pub fn to_value(self) -> Value {
        Value::obj([("records", self.records.into()), ("stamp", self.stamp.into()), ("hash", super::sv::hex16(self.hash).into())])
    }
    pub fn from_value(v: &Value) -> Option<Fingerprint> {
        Some(Fingerprint {
            records: v.get("records").as_f64()?,
            stamp: v.get("stamp").as_f64()?,
            hash: super::sv::unhex16(v.get("hash").as_str()?)?,
        })
    }
}

/// One page of records.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Page {
    /// No record follows the last one of this page.
    pub done: bool,
    /// `(key, value)` in key order. Continue with `after = rows.last().0`.
    pub rows: Vec<(Value, Value)>,
}

impl Page {
    pub fn parse(io: &Io) -> Option<Page> {
        if io.status != status::OK {
            return None;
        }
        let mut r = R::new(io.data);
        let n = r.arr()?;
        let done = r.bool()?;
        let rows = (0..n.saturating_sub(1) / 2).map(|_| Some((r.value()?, r.value()?))).collect::<Option<Vec<_>>>()?;
        Some(Page { done, rows })
    }
    /// Releases the JS handles of every `Blob` in the page except those in `keep`.
    pub fn release(&self, out: &mut OutBuf, keep: &[u32]) {
        let mut handles = Vec::new();
        for (_, value) in &self.rows {
            value.handles(&mut handles);
        }
        handles.into_iter().filter(|h| !keep.contains(h)).for_each(|h| out.obj_release(h));
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::boot;
    use super::*;
    use crate::app::Mod;

    fn rows(n: u32, saved_at: f64) -> Vec<(Value, Value)> {
        (1..=n).map(|i| (Value::Num(i as f64), Value::obj([("id", i.into()), ("savedAt", (saved_at + i as f64).into())]))).collect()
    }

    fn fingerprint(db: &mut super::super::fake::MemDb, name: &str) -> Option<Fingerprint> {
        let (mut app, mut host, _) = boot();
        let req = app.req(Mod::Store);
        info(&mut host, req, name);
        db.pump(&mut app, &mut host);
        let (status, data) = db.take(req).unwrap();
        Info::parse(&Io { req, status, obj: 0, data: &data }).map(|i| i.fingerprint())
    }

    #[test]
    fn the_fingerprint_changes_when_5x_writes_and_only_then() {
        let (_, _, mut db) = boot();
        db.v5.insert(
            NOTEBOOK_DB.into(),
            (4, vec![("snapshots".into(), vec![("latest".into(), Value::obj([("savedAt", 1000.0.into())]))]), ("stroke-ops".into(), rows(3, 900.0))]),
        );
        db.v5.insert(OFFLINE_DB.into(), (2, vec![("leases".into(), rows(1, 0.0)), ("outbox".into(), Vec::new())]));
        let base = fingerprint(&mut db, NOTEBOOK_DB).unwrap();
        assert_eq!((base.records, base.stamp), (4.0, 1000.0));
        assert_eq!(fingerprint(&mut db, NOTEBOOK_DB), Some(base), "deterministic");
        assert_eq!(Fingerprint::from_value(&base.to_value()), Some(base));
        assert_eq!(fingerprint(&mut db, BACKGROUND_NOTEBOOK_DB), None, "a missing database is not an empty one");

        // A new stroke-op (greater auto-increment id), a rewritten snapshot (greater savedAt), a replaced row.
        db.v5.get_mut(NOTEBOOK_DB).unwrap().1[1].1.push((4.0.into(), Value::obj([("savedAt", 950.0.into())])));
        let appended = fingerprint(&mut db, NOTEBOOK_DB).unwrap();
        assert_ne!(appended, base);
        assert_eq!((appended.records, appended.stamp), (5.0, 1000.0));
        db.v5.get_mut(NOTEBOOK_DB).unwrap().1[0].1[0].1 = Value::obj([("savedAt", 2000.0.into())]);
        assert_eq!(fingerprint(&mut db, NOTEBOOK_DB).unwrap().stamp, 2000.0);

        // Lease heartbeats do not count; an outbox row does.
        let offline = fingerprint(&mut db, OFFLINE_DB).unwrap();
        db.v5.get_mut(OFFLINE_DB).unwrap().1[0].1.push((9.0.into(), Value::obj([("savedAt", 5e12.into())])));
        assert_eq!(fingerprint(&mut db, OFFLINE_DB), Some(offline));
        db.v5.get_mut(OFFLINE_DB).unwrap().1[1].1.push(("a\0doc".into(), Value::obj([("generation", 3.0.into())])));
        assert_ne!(fingerprint(&mut db, OFFLINE_DB), Some(offline));
    }

    #[test]
    fn records_are_read_in_pages_and_blob_handles_released() {
        let (mut app, mut host, mut db) = boot();
        let mut records = rows(5, 0.0);
        records[1].1.set("blob", Value::Blob { obj: 31, size: 4.0, mime: "application/pdf".into() });
        records[2].1.set("blob", Value::Blob { obj: 32, size: 4.0, mime: "application/pdf".into() });
        db.v5.insert(NOTEBOOK_DB.into(), (4, vec![("pages".into(), records.clone())]));
        let (mut after, mut all, mut pages) = (None, Vec::new(), 0);
        loop {
            let req = app.req(Mod::Store);
            read(&mut host, req, NOTEBOOK_DB, "pages", after.as_ref(), 2, 1e9);
            db.pump(&mut app, &mut host);
            let (status, data) = db.take(req).unwrap();
            let page = Page::parse(&Io { req, status, obj: 0, data: &data }).unwrap();
            page.release(&mut host.out, &[32]);
            pages += 1;
            after = page.rows.last().map(|r| r.0.clone());
            all.extend(page.rows);
            if page.done {
                break;
            }
        }
        assert_eq!((pages, all), (3, records));
        assert_eq!(host.take(), "obj_release 31\n");
        for (store, database) in [("missing", NOTEBOOK_DB), ("pages", ORIGINALS_DB)] {
            let req = app.req(Mod::Store);
            read(&mut host, req, database, store, None, 0, 1e9);
            db.pump(&mut app, &mut host);
            assert_eq!(db.take(req).unwrap().0, MISSING);
        }
    }
}
