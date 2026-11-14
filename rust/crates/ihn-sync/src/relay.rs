//! The Drive relay (SYNC §8): relay/archive/record codecs, the writer (epochs, archive rotation, gen persistence,
//! parallel file + record upload, seal, retries), the reader (digest check, doc check, gaps through archives) and the
//! garbage collection of own files and stale records. Owner: W2.7.
//!
//! One [`Relay`] part per session. Only the network-leader tab of an editor writes; every leader (viewers too)
//! reads. Asset files are listed in the header but not uploaded here: the pure crate cannot read blob bytes
//! (`Replica` exposes assets as host handles), see NOTES-W2.7.

use crate::*;
use ihn_core::codec::b64_encode;
use ihn_core::id::{fnv1a64, inst13};
use ihn_core::snapshot::{get_bytes, get_hash, put_bytes, put_hash};
use mode::WRITE_CAP_MS;
use std::collections::BTreeSet;

/// Deflated tail cap (§8.1, §8.5 step 3).
pub const TAIL_MAX: usize = 48 << 10;
pub const READS: usize = 4;
pub const DIGEST_RETRY_MS: f64 = 300.0;
pub const DIGEST_RETRIES: u32 = 10;
pub const OLD_FILE_DELETE_MS: f64 = 10_000.0;
pub const DAY_MS: f64 = 86_400_000.0;
pub const POOL: u32 = 20;
pub const POOL_LOW: usize = 10;
pub const MAX_FILE_ID: usize = 44;
pub const RECORD_GC_COUNT: usize = 24;
pub const RECORD_GC_AGE_MS: f64 = 30.0 * DAY_MS;
const RELAY_MAGIC: &[u8; 5] = b"IHN6R";
const ARCHIVE_MAGIC: &[u8; 5] = b"IHN6A";

/// Test command hook: `__IHN_TEST_API__.cmd('sync.<module>.<name>', args)` (JSON text in, JSON text out).
pub fn test_cmd(_name: &str, _args: &str) -> Option<String> { None }

/// This module's protocol part for a session.
pub fn part() -> Option<Box<dyn crate::Part>> { Some(Box::new(Relay::default())) }

// ------------------------------------------------------------------------------------------------ codecs

/// A count whose items take at least one byte each (safe to allocate).
fn count(r: &mut Reader) -> Option<usize> { usize::try_from(r.leb()?).ok().filter(|&n| n <= r.buf.len()) }

fn put_range(w: &mut Writer, r: &Range) {
    w.u64(r.actor.0);
    w.leb(r.first as u64);
    w.leb(r.last as u64);
}
fn get_range(r: &mut Reader) -> Option<Range> {
    let actor = ActorId(r.u64()?);
    let first = u32::try_from(r.leb()?).ok().filter(|&f| f >= 1)?;
    let last = u32::try_from(r.leb()?).ok().filter(|&l| l >= first)?;
    Some(Range { actor, first, last })
}
fn put_ranges(w: &mut Writer, ranges: &[Range]) {
    w.leb(ranges.len() as u64);
    ranges.iter().for_each(|r| put_range(w, r));
}
fn get_ranges(r: &mut Reader) -> Option<Vec<Range>> { (0..count(r)?).map(|_| get_range(r)).collect() }
fn put_batches(w: &mut Writer, batches: &[EncodedBatch]) {
    w.leb(batches.len() as u64);
    batches.iter().for_each(|b| put_bytes(w, &b.0));
}
fn get_batches(r: &mut Reader) -> Option<Vec<EncodedBatch>> {
    (0..count(r)?).map(|_| get_bytes(r).filter(|b| !b.is_empty()).map(|b| EncodedBatch(b.to_vec()))).collect()
}

/// `(actor, first, last)` runs of the given batches (sorted by actor, then seq).
pub fn ranges_of(batches: &[EncodedBatch]) -> Vec<Range> {
    let mut keys: Vec<(ActorId, u32)> = batches.iter().filter_map(|b| b.header()).map(|(a, s, _)| (a, s)).collect();
    keys.sort();
    keys.dedup();
    let mut out: Vec<Range> = Vec::new();
    for (actor, seq) in keys {
        match out.last_mut() {
            Some(r) if r.actor == actor && r.last + 1 == seq => r.last = seq,
            _ => out.push(Range { actor, first: seq, last: seq }),
        }
    }
    out
}

/// SYNC §8.3 `Payload` of a relay file.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct RelayFile {
    pub doc_id: String,
    pub inst: u64,
    pub gen: u64,
    pub written_at: u64,
    pub profile: Profile,
    pub mail: Vec<Mail>,
    /// Last export known to the writer (§14.4).
    pub export: Option<(String, VersionVector)>,
    pub assets: Vec<AssetRef>,
    pub archives: Vec<ArchiveRef>,
    pub ranges: Vec<Range>,
    pub batches: Vec<EncodedBatch>,
}

impl RelayFile {
    pub fn payload(&self) -> Vec<u8> {
        let mut w = Writer::default();
        put_str(&mut w, &self.doc_id);
        w.u64(self.inst);
        w.leb(self.gen);
        w.leb(self.written_at);
        self.profile.encode(&mut w);
        w.leb(self.mail.len() as u64);
        self.mail.iter().for_each(|m| m.encode(&mut w));
        match &self.export {
            Some((head, vv)) => {
                w.u8(1);
                put_str(&mut w, head);
                vv.encode(&mut w);
            }
            None => w.u8(0),
        }
        w.leb(self.assets.len() as u64);
        for a in &self.assets {
            put_hash(&mut w, &a.hash);
            put_str(&mut w, &a.file_id);
            w.leb(a.size);
        }
        w.leb(self.archives.len() as u64);
        for a in &self.archives {
            put_str(&mut w, &a.file_id);
            put_bytes(&mut w, &a.digest);
            put_ranges(&mut w, &a.ranges);
        }
        put_ranges(&mut w, &self.ranges);
        put_batches(&mut w, &self.batches);
        w.buf
    }

    pub fn decode_payload(bytes: &[u8]) -> Option<RelayFile> {
        let r = &mut Reader::new(bytes);
        let text = |r: &mut Reader| get_str(r).map(str::to_string);
        let (doc_id, inst, gen, written_at) = (text(r)?, r.u64()?, r.leb()?, r.leb()?);
        let profile = Profile::decode(r)?;
        let mail = (0..count(r)?).map(|_| Mail::decode(r)).collect::<Option<Vec<_>>>()?;
        let export = match r.u8()? {
            0 => None,
            1 => Some((text(r)?, VersionVector::decode(r)?)),
            _ => return None,
        };
        let assets = (0..count(r)?).map(|_| Some(AssetRef { hash: get_hash(r)?, file_id: text(r)?, size: r.leb()? })).collect::<Option<Vec<_>>>()?;
        let archives = (0..count(r)?)
            .map(|_| Some(ArchiveRef { file_id: text(r)?, digest: get_bytes(r)?.try_into().ok()?, ranges: get_ranges(r)? }))
            .collect::<Option<Vec<_>>>()?;
        let (ranges, batches) = (get_ranges(r)?, get_batches(r)?);
        r.is_empty().then_some(RelayFile { doc_id, inst, gen, written_at, profile, mail, export, assets, archives, ranges, batches })
    }
}

/// SYNC §8.3 `ArchiveFile` body (after the `IHN6A` header).
pub fn archive_payload(doc_id: &str, inst: u64, batches: &[EncodedBatch]) -> Vec<u8> {
    let mut w = Writer::default();
    put_str(&mut w, doc_id);
    w.u64(inst);
    put_batches(&mut w, batches);
    w.buf
}
pub fn decode_archive(bytes: &[u8]) -> Option<(String, u64, Vec<EncodedBatch>)> {
    let r = &mut Reader::new(bytes);
    let out = (get_str(r)?.to_string(), r.u64()?, get_batches(r)?);
    r.is_empty().then_some(out)
}

/// `magic ‖ u8 ver=1 ‖ u8 comp ‖ body`.
pub fn wrap(archive: bool, comp: bool, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 7);
    out.extend_from_slice(if archive { ARCHIVE_MAGIC } else { RELAY_MAGIC });
    out.extend_from_slice(&[1, comp as u8]);
    out.extend_from_slice(body);
    out
}
/// `(compressed, body)` of a wrapped file.
pub fn unwrap(archive: bool, bytes: &[u8]) -> Option<(bool, &[u8])> {
    let magic = if archive { ARCHIVE_MAGIC } else { RELAY_MAGIC };
    let rest = bytes.strip_prefix(&magic[..])?;
    match rest {
        [1, comp @ (0 | 1), body @ ..] => Some((*comp == 1, body)),
        _ => None,
    }
}

/// `d16`: base64url (no padding) of the first 12 bytes of a SHA-256.
pub fn d16(sha: &[u8]) -> String { b64_encode(&sha[..sha.len().min(12)], true) }

fn b36(mut v: u64) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(v % 36) as usize]);
        v /= 36;
        if v == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// A Drive file id that is safe to put in a request path.
pub fn valid_file_id(id: &str) -> bool { !id.is_empty() && id.len() <= MAX_FILE_ID && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') }

/// SYNC §8.4 record value `"1|" gen36 "|" n36 "|" fileId "|" d16 "|" flags`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Record {
    pub gen: u64,
    pub nonce: u64,
    pub file_id: String,
    pub d16: String,
    /// Subset of `c`, `l`, `x`, in that order.
    pub flags: String,
}

impl Record {
    pub fn parse(value: &str) -> Option<Record> {
        mode::record_flags(value)?;
        let p: Vec<&str> = value.split('|').collect();
        let num = |s: &str| u64::from_str_radix(s, 36).ok();
        let rec = Record { gen: num(p[1])?, nonce: num(p[2])?, file_id: p[3].into(), d16: p[4].into(), flags: p[5].into() };
        valid_file_id(&rec.file_id).then_some(rec)
    }
    pub fn value(&self) -> String { format!("1|{}|{}|{}|{}|{}", b36(self.gen), b36(self.nonce), self.file_id, self.d16, self.flags) }
}

pub fn record_key(inst: u64) -> String { format!("ihn6_{}", inst13(inst)) }

/// 5.x sync keys nulled by an installation's first record write (§8.4): `(properties, appProperties)`.
pub fn legacy_keys(meta: &DriveMeta) -> (Vec<String>, Vec<String>) {
    let props = meta.props.iter().map(|(k, _)| k).filter(|k| k.starts_with("p_") || *k == "ihn_live_key_v1" || k.starts_with("ihn_ink_") || k.starts_with("ihn_live_"));
    let app = meta.app.iter().map(|(k, _)| k).filter(|k| k.starts_with("ihn_lm1_"));
    (props.cloned().collect(), app.cloned().collect())
}

/// Hash of the document's `permissionIds` (epoch rotation, §8.5 step 1); 0 when unknown.
pub fn perm_hash(ids: &[String]) -> u64 {
    if ids.is_empty() {
        return 0;
    }
    let mut ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    ids.sort_unstable();
    fnv1a64(ids.join(",").as_bytes()) | 1
}

/// Whether a batch may travel in installation `inst`'s relay file of `doc_id`: its tabs and its migration actor.
pub fn relayed_by(actor: ActorId, inst: u64, doc_id: &str) -> bool {
    (!actor.is_import() && actor.inst() == inst) || actor == ActorId::import(&migration_source(inst, doc_id))
}

/// Stale sync-folder files for housekeeping (§8.7): `files` are `(file id, ihn6Doc)`; a file is stale when its
/// document is queued in `relay-gc` or no longer exists / is trashed (`gone`).
pub fn stale_files<'a>(files: &'a [(String, String)], relay_gc: &[String], gone: &dyn Fn(&str) -> bool) -> Vec<&'a str> {
    files.iter().filter(|(_, doc)| relay_gc.contains(doc) || gone(doc)).map(|(id, _)| id.as_str()).collect()
}

// ------------------------------------------------------------------------------------------------ writer

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WStage {
    /// Deflating the candidate tail.
    Deflate,
    /// Deflating / digesting / creating the archive of the old tail.
    ArchDeflate,
    ArchDigest,
    ArchUpload,
    /// Digest of the relay file bytes.
    Digest,
    /// File and record PATCHes in flight or waiting for a retry.
    Upload,
}

struct Write {
    stage: WStage,
    req: Option<u32>,
    keepalive: bool,
    left: bool,
    tail: Vec<EncodedBatch>,
    /// Old tail batches being moved to an archive.
    arch: Vec<EncodedBatch>,
    arch_id: String,
    arch_bytes: Vec<u8>,
    arch_digest: [u8; 12],
    archives: Vec<ArchiveRef>,
    /// Archives covered by the newest export: deleted after the seal.
    drop_archives: Vec<String>,
    rotate: bool,
    gen: u64,
    file_id: String,
    create: bool,
    bytes: Vec<u8>,
    record: String,
    perm: u64,
    sealed: Vec<(ActorId, u32)>,
    nulls: Vec<(String, Option<String>)>,
    app_nulls: Vec<(String, Option<String>)>,
    file_req: Option<u32>,
    rec_req: Option<u32>,
    file_ok: bool,
    rec_ok: bool,
}

// ------------------------------------------------------------------------------------------------ reader

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RStage {
    Wait,
    Fetch,
    Digest,
    Inflate,
    ArchFetch,
    /// A failed archive read waits for its retry.
    ArchWait,
    ArchDigest,
    ArchInflate,
}

struct Read {
    rec: Record,
    stage: RStage,
    at: f64,
    req: Option<u32>,
    tries: u32,
    raw: Vec<u8>,
    file: Option<RelayFile>,
    todo: Vec<ArchiveRef>,
    got: Vec<ArchiveRef>,
    batches: Vec<EncodedBatch>,
}

impl Read {
    fn new(rec: Record, at: f64) -> Read { Read { rec, stage: RStage::Wait, at, req: None, tries: 0, raw: Vec::new(), file: None, todo: Vec::new(), got: Vec::new(), batches: Vec::new() } }
}

// ------------------------------------------------------------------------------------------------ part

/// The relay part (writer + reader + GC).
pub struct Relay {
    w: Option<Write>,
    /// Local time of the previous relay write (2 s cap).
    wrote_at: f64,
    fails: u32,
    retry_at: f64,
    pending_since: Option<f64>,
    last_batch_at: f64,
    ids_req: Option<u32>,
    folder_req: Option<u32>,
    /// `(file id, delete at)`.
    to_delete: Vec<(String, f64)>,
    /// The own record carries `l`; no more writes until visible again.
    left: bool,
    /// A suspend/leave asked for an `l` write.
    want_left: bool,
    /// The record write was refused for lack of rights: no writes until the poll says otherwise.
    demoted: bool,
    /// A file answered 404 or an id was refused: the next write starts a new epoch.
    force_rotate: bool,
    legacy_done: bool,
    /// Records whose relay file names another document (a Drive copy, §8.4): nulled by the next write.
    foreign: BTreeSet<u64>,
    /// Records chosen by record GC (§8.4).
    gc: Vec<u64>,
    gc_done: bool,
    reads: BTreeMap<u64, Read>,
    /// Record values given up on (digest never matched, file gone, foreign).
    stale: BTreeMap<u64, String>,
    /// Per remote installation: `written_at` and ranges of its last applied file (record GC).
    seen: BTreeMap<u64, (u64, Vec<Range>)>,
}

impl Default for Relay {
    fn default() -> Relay {
        Relay {
            w: None,
            wrote_at: f64::NEG_INFINITY,
            fails: 0,
            retry_at: f64::NEG_INFINITY,
            pending_since: None,
            last_batch_at: f64::NEG_INFINITY,
            ids_req: None,
            folder_req: None,
            to_delete: Vec::new(),
            left: false,
            want_left: false,
            demoted: false,
            force_rotate: false,
            legacy_done: false,
            foreign: BTreeSet::new(),
            gc: Vec::new(),
            gc_done: false,
            reads: BTreeMap::new(),
            stale: BTreeMap::new(),
            seen: BTreeMap::new(),
        }
    }
}

/// Highest archived seq per actor.
fn archived(stored: &Stored) -> BTreeMap<ActorId, u32> {
    let mut out = BTreeMap::new();
    for r in stored.archives.iter().flat_map(|a| &a.ranges) {
        let e = out.entry(r.actor).or_insert(0);
        *e = (*e).max(r.last);
    }
    out
}

fn ok(status: u16) -> bool { (200..300).contains(&status) }
fn rate_limited(status: u16, body: &[u8]) -> bool { status == 429 || (status == 403 && String::from_utf8_lossy(body).contains("ateLimitExceeded")) }

impl Relay {
    /// Own (relayed) batches the replica holds above `floor` (per actor; other actors excluded).
    fn own_batches(cx: &Core, r: &dyn Replica, floor: &BTreeMap<ActorId, u32>) -> Option<Vec<EncodedBatch>> {
        let rv = r.version_vector();
        let mut have = VersionVector::default();
        for (a, s) in rv.iter() {
            have.raise(a, if cx.relays(a) { floor.get(&a).copied().unwrap_or(0).min(s) } else { s });
        }
        for (a, s) in rv.extras() {
            if !cx.relays(a) || s <= floor.get(&a).copied().unwrap_or(0) {
                have.add(a, s);
            }
        }
        let mut out = r.batches_since(&have, usize::MAX).ok()?;
        out.retain(|b| b.header().is_some_and(|(a, _, _)| cx.relays(a)));
        out.sort_by_key(|b| b.header().map(|(a, s, _)| (a, s)));
        Some(out)
    }

    fn unsealed(cx: &Core, r: &dyn Replica) -> bool {
        let rv = r.version_vector();
        rv.actors().into_iter().filter(|&a| cx.relays(a)).any(|a| {
            let sealed = cx.stored.sealed.get(&a).copied().unwrap_or(0);
            rv.get(a) > sealed || rv.extras().any(|(x, s)| x == a && s > sealed)
        })
    }

    fn own_record(cx: &Core) -> Option<Record> {
        let key = record_key(cx.inst);
        cx.meta.as_ref()?.0.props.iter().find(|(k, _)| *k == key).and_then(|(_, v)| Record::parse(v))
    }

    fn may_write(&self, cx: &Core) -> bool {
        cx.networking() && cx.can_edit && cx.mode != Mode::Viewer && !self.demoted && cx.polled && !self.reads.contains_key(&cx.inst)
    }

    fn flags(cx: &Core, left: bool) -> String {
        let live = matches!(cx.mode, Mode::Company | Mode::Linked | Mode::RelayOnly) && !left;
        [(live, 'c'), (left, 'l'), (cx.stored.no_link, 'x')].iter().filter(|f| f.0).map(|f| f.1).collect()
    }

    /// Takes a pooled id (≤ 44 chars).
    fn take_id(cx: &mut Core) -> Option<String> {
        let mut pool = cx.stored.id_pool.clone();
        pool.retain(|id| valid_file_id(id));
        let id = (!pool.is_empty()).then(|| pool.remove(0))?;
        cx.persist(SyncRecord::IdPool(pool));
        Some(id)
    }

    /// Id pool and sync folder (§8.2), whenever online and leader.
    fn housekeeping(&mut self, cx: &mut Core) {
        if !cx.networking() || !cx.can_edit {
            return;
        }
        if cx.stored.relay_folder.is_none() && self.folder_req.is_none() {
            self.folder_req = Some(cx.http(HttpReq::SyncFolder));
        }
        if cx.stored.id_pool.len() < POOL_LOW && self.ids_req.is_none() {
            self.ids_req = Some(cx.http(HttpReq::GenerateIds { count: POOL }));
        }
        let now = cx.now;
        for (file, _) in self.to_delete.iter().filter(|(_, at)| *at <= now) {
            cx.http(HttpReq::Delete { file: file.clone() });
        }
        self.to_delete.retain(|(_, at)| *at > now);
        if let Some(at) = self.to_delete.iter().map(|d| d.1).reduce(f64::min) {
            cx.wake(at);
        }
    }

    /// When the next write is due (`None`: nothing to write).
    fn due(&self, cx: &Core, r: &dyn Replica) -> Option<f64> {
        let cap = self.wrote_at + WRITE_CAP_MS;
        let keepalive = cx.flush == Flush::Keepalive;
        if self.want_left {
            return Some(cx.now);
        }
        if self.left {
            return None;
        }
        let cadence = cx.cadence();
        let mut due = f64::INFINITY;
        if Self::unsealed(cx, r) {
            let first = self.pending_since.unwrap_or(cx.now);
            due = if keepalive { cx.now } else { cadence.flush_at(first, self.last_batch_at.max(first), self.wrote_at) };
        }
        if cx.mail_changed || cx.record_due || cx.flush == Flush::Soon || self.force_rotate || !self.foreign.is_empty() {
            due = due.min(if keepalive { cx.now } else { cap });
        }
        if let (Some(every), true, true) = (cadence.refresh, self.wrote_at.is_finite(), cx.visible) {
            due = due.min((self.wrote_at + every).max(cap));
        }
        due.is_finite().then_some(due.max(self.retry_at))
    }

    /// §8.5 steps 1–5: plan a write and start deflating its tail.
    fn start(&mut self, cx: &mut Core, r: &mut dyn Replica, left: bool) {
        let keepalive = cx.flush == Flush::Keepalive || left;
        let floor = archived(&cx.stored);
        let Some(tail) = Self::own_batches(cx, r, &floor).or_else(|| {
            let mut floor = floor.clone();
            for (&a, &s) in &cx.stored.sealed {
                let e = floor.entry(a).or_insert(0);
                *e = (*e).max(s);
            }
            Self::own_batches(cx, r, &floor)
        }) else {
            return;
        };
        // Archives an export covers are dropped from the header and deleted after the seal (§8.7).
        let (mut archives, mut drop_archives) = (Vec::new(), Vec::new());
        for a in cx.stored.archives.clone() {
            if cx.stored.export_vv().is_some_and(|vv| a.ranges.iter().all(|g| vv.get(g.actor) >= g.last)) {
                drop_archives.push(a.file_id);
            } else {
                archives.push(a);
            }
        }
        cx.flush = Flush::None;
        cx.record_due = false;
        cx.mail_changed = false;
        self.want_left = false;
        self.left = left;
        self.wrote_at = cx.now;
        cx.status.relay_writing = true;
        let perm = cx.meta.as_ref().map_or(0, |m| perm_hash(&m.0.permission_ids));
        let perm = if perm == 0 { cx.stored.perm } else { perm };
        let rotate = std::mem::take(&mut self.force_rotate);
        self.w = Some(Write {
            stage: WStage::Deflate,
            req: None,
            keepalive,
            left,
            tail,
            arch: Vec::new(),
            arch_id: String::new(),
            arch_bytes: Vec::new(),
            arch_digest: [0; 12],
            archives,
            drop_archives,
            rotate,
            gen: 0,
            file_id: String::new(),
            create: false,
            bytes: Vec::new(),
            record: String::new(),
            perm,
            sealed: Vec::new(),
            nulls: Vec::new(),
            app_nulls: Vec::new(),
            file_req: None,
            rec_req: None,
            file_ok: false,
            rec_ok: false,
        });
        self.deflate_tail(cx);
    }

    fn file_of(&self, cx: &Core) -> RelayFile {
        let w = self.w.as_ref().expect("write");
        RelayFile {
            doc_id: cx.doc_id.clone(),
            inst: cx.inst,
            gen: w.gen,
            written_at: cx.now.max(0.0) as u64,
            profile: cx.profile.clone(),
            mail: cx.mail_out.clone(),
            export: cx.stored.exports.last().cloned(),
            assets: cx.stored.assets.clone(),
            archives: w.archives.clone(),
            ranges: ranges_of(&w.tail),
            batches: w.tail.clone(),
        }
    }

    fn deflate_tail(&mut self, cx: &mut Core) {
        let payload = self.file_of(cx).payload();
        let w = self.w.as_mut().expect("write");
        w.stage = WStage::Deflate;
        w.req = Some(cx.deflate(false, payload));
    }

    /// The deflated candidate is known: rotate an archive out, or plan and digest.
    fn tail_deflated(&mut self, cx: &mut Core, out: &[u8]) {
        let w = self.w.as_mut().expect("write");
        if out.len() > TAIL_MAX && !w.tail.is_empty() {
            // §8.5 step 3: the old (already sealed) tail becomes an archive; a new batch too big alone goes too.
            let sealed = |b: &EncodedBatch| b.header().is_some_and(|(a, s, _)| s <= cx.stored.sealed.get(&a).copied().unwrap_or(0));
            let (old, new): (Vec<_>, Vec<_>) = w.tail.drain(..).partition(sealed);
            (w.arch, w.tail) = if old.is_empty() { (new, Vec::new()) } else { (old, new) };
            w.rotate = true;
            w.stage = WStage::ArchDeflate;
            let body = archive_payload(&cx.doc_id, cx.inst, &w.arch);
            w.req = Some(cx.deflate(false, body));
            return;
        }
        // Step 1 and 4: epoch and generation, persisted before any upload.
        let today = (cx.now / DAY_MS).floor().max(0.0) as u32;
        let own = Self::own_record(cx);
        let mut rotate = w.rotate || cx.stored.relay_file.is_empty() || cx.stored.relay_epoch_day != today;
        rotate |= w.perm != 0 && cx.stored.perm != 0 && w.perm != cx.stored.perm;
        let prev = cx.stored.relay_file.clone();
        if rotate {
            let Some(id) = Self::take_id(cx) else {
                // No reserved id yet: the pool refill answers and the write starts again.
                return self.abort(cx, false);
            };
            let w = self.w.as_mut().expect("write");
            (w.file_id, w.create) = (id, true);
            for old in [Some(prev), own.as_ref().map(|o| o.file_id.clone())].into_iter().flatten() {
                if !old.is_empty() && old != w.file_id && !self.to_delete.iter().any(|d| d.0 == old) {
                    w.drop_archives.push(old);
                }
            }
        } else {
            w.file_id = prev;
        }
        let w = self.w.as_mut().expect("write");
        w.gen = cx.stored.relay_gen.max(own.map_or(0, |o| o.gen)) + 1;
        cx.persist(SyncRecord::RelayPlan { gen: w.gen, file_id: w.file_id.clone(), epoch_day: today });
        // The gen is in the payload: build again with it (sizes do not change by more than a few bytes).
        let payload = self.file_of(cx).payload();
        let w = self.w.as_mut().expect("write");
        let mut sealed: BTreeMap<ActorId, u32> = BTreeMap::new();
        for g in w.archives.iter().flat_map(|a| &a.ranges).chain(&ranges_of(&w.tail)) {
            let e = sealed.entry(g.actor).or_insert(0);
            *e = (*e).max(g.last);
        }
        w.sealed = sealed.into_iter().collect();
        w.bytes = wrap(false, false, &payload);
        // Compress when it pays (the sim's deflate stores; a real one shrinks).
        if out.len() + 7 < w.bytes.len() {
            w.stage = WStage::Digest;
            w.req = Some(cx.deflate(false, payload));
            w.bytes.clear();
        } else {
            w.stage = WStage::Digest;
            w.req = Some(cx.digest(w.bytes.clone()));
        }
    }

    fn abort(&mut self, cx: &mut Core, failed: bool) {
        self.w = None;
        cx.status.relay_writing = false;
        if failed {
            self.fails += 1;
            self.retry_at = cx.now + cx.backoff(500.0, 30_000.0, self.fails);
        }
    }

    fn upload(&mut self, cx: &mut Core) {
        let doc = cx.doc_id.clone();
        let w = self.w.as_mut().expect("write");
        if !w.file_ok && w.file_req.is_none() {
            let (file, bytes, keepalive) = (w.file_id.clone(), w.bytes.clone(), w.keepalive);
            w.file_req = Some(cx.http(if w.create {
                let app = vec![("ihn6Kind".into(), "relay".into()), ("ihn6Doc".into(), doc.clone()), ("ihn6Inst".into(), inst13(cx.inst))];
                HttpReq::Create { file, name: format!("ihn6-{doc}-{}.bin", inst13(cx.inst)), app, bytes, keepalive }
            } else {
                HttpReq::Update { file, bytes, keepalive }
            }));
        }
        if !w.rec_ok && w.rec_req.is_none() {
            let mut set = vec![(record_key(cx.inst), Some(w.record.clone()))];
            set.extend(w.nulls.iter().cloned());
            let req = HttpReq::Props { file: doc, set, app: w.app_nulls.clone(), keepalive: w.keepalive };
            w.rec_req = Some(cx.http(req));
        }
    }

    /// The relay bytes and their digest are final: build the record and upload both (§8.5 step 6).
    fn ready(&mut self, cx: &mut Core, digest: &[u8]) {
        let flags = Self::flags(cx, self.w.as_ref().is_some_and(|w| w.left));
        let mut nulls: Vec<(String, Option<String>)> = Vec::new();
        let mut app_nulls = Vec::new();
        if let Some((meta, _)) = &cx.meta {
            if !self.legacy_done {
                let (p, a) = legacy_keys(meta);
                nulls.extend(p.into_iter().map(|k| (k, None)));
                app_nulls.extend(a.into_iter().map(|k| (k, None)));
            }
            let present: BTreeSet<&str> = meta.props.iter().map(|(k, _)| k.as_str()).collect();
            for inst in self.foreign.iter().chain(&self.gc) {
                let key = record_key(*inst);
                if present.contains(key.as_str()) && *inst != cx.inst {
                    nulls.push((key, None));
                }
            }
        }
        let w = self.w.as_mut().expect("write");
        w.record = Record { gen: w.gen, nonce: cx.now.max(0.0) as u64, file_id: w.file_id.clone(), d16: d16(digest), flags }.value();
        (w.nulls, w.app_nulls) = (nulls, app_nulls);
        w.stage = WStage::Upload;
        self.upload(cx);
    }

    /// Step 7: both PATCHes succeeded.
    fn seal(&mut self, cx: &mut Core, r: &dyn Replica) {
        let w = self.w.take().expect("write");
        cx.persist(SyncRecord::RelaySealed { sealed: w.sealed.clone(), archives: w.archives.clone(), assets: cx.stored.assets.clone(), perm: w.perm });
        cx.status.relay_writing = false;
        self.legacy_done = true;
        self.foreign.clear();
        self.gc.clear();
        self.fails = 0;
        let at = cx.now + OLD_FILE_DELETE_MS;
        self.to_delete.extend(w.drop_archives.into_iter().map(|f| (f, at)));
        cx.wake(at);
        if Self::unsealed(cx, r) {
            cx.persist(SyncRecord::Flags { relay_pending: Some(true), export_pending: None, created: None });
            self.pending_since = Some(self.last_batch_at.min(cx.now));
        } else {
            self.pending_since = None;
        }
    }

    fn write_answer(&mut self, cx: &mut Core, id: u32, status: u16, body: &[u8], r: &dyn Replica) -> bool {
        let Some(w) = self.w.as_mut() else { return false };
        if w.req == Some(id) && w.stage == WStage::ArchUpload {
            w.req = None;
            match status {
                s if ok(s) || s == 409 => {
                    let ranges = ranges_of(&w.arch);
                    w.archives.push(ArchiveRef { file_id: std::mem::take(&mut w.arch_id), digest: w.arch_digest, ranges });
                    w.arch.clear();
                    self.deflate_tail(cx);
                }
                400 => match Self::take_id(cx) {
                    Some(id) => {
                        let w = self.w.as_mut().expect("write");
                        w.arch_id = id;
                        self.create_archive(cx);
                    }
                    None => self.abort(cx, true),
                },
                _ => self.abort(cx, true),
            }
            return true;
        }
        let file = w.file_req == Some(id);
        if !file && w.rec_req != Some(id) {
            return false;
        }
        if file {
            w.file_req = None;
            match status {
                s if ok(s) => w.file_ok = true,
                409 if w.create => {
                    // Created by a lost response: continue with an update of the same bytes.
                    w.create = false;
                    self.upload(cx);
                    return true;
                }
                // The file is gone or the id was refused: a new epoch, new gen.
                404 | 400 => {
                    self.force_rotate = true;
                    self.abort(cx, false);
                    return true;
                }
                _ => return self.failed(cx, status, body),
            }
        } else {
            w.rec_req = None;
            match status {
                s if ok(s) => w.rec_ok = true,
                403 if !rate_limited(status, body) => {
                    // §12.3: no right to write properties any more; the next poll makes the session a viewer.
                    self.demoted = true;
                    self.abort(cx, false);
                    return true;
                }
                _ => return self.failed(cx, status, body),
            }
        }
        let w = self.w.as_ref().expect("write");
        if w.file_ok && w.rec_ok {
            self.seal(cx, r);
        }
        true
    }

    /// A failed upload: retry the same bytes and gen after the backoff (§8.5 "Fail").
    fn failed(&mut self, cx: &mut Core, _status: u16, _body: &[u8]) -> bool {
        self.fails += 1;
        self.retry_at = cx.now + cx.backoff(500.0, 30_000.0, self.fails);
        cx.wake(self.retry_at);
        true
    }

    fn create_archive(&mut self, cx: &mut Core) {
        let doc = cx.doc_id.clone();
        let w = self.w.as_mut().expect("write");
        let gen = cx.stored.relay_gen + 1;
        let app = vec![("ihn6Kind".into(), "archive".into()), ("ihn6Doc".into(), doc.clone()), ("ihn6Inst".into(), inst13(cx.inst))];
        let name = format!("ihn6-{doc}-{}-a{}.bin", inst13(cx.inst), b36(gen));
        w.stage = WStage::ArchUpload;
        w.req = Some(cx.http(HttpReq::Create { file: w.arch_id.clone(), name, app, bytes: w.arch_bytes.clone(), keepalive: false }));
    }

    fn write_tick(&mut self, cx: &mut Core, r: &mut dyn Replica) {
        if let Some(w) = &self.w {
            // Retry what failed, once the backoff passed and we are online.
            if w.stage == WStage::Upload && (w.file_req.is_none() && !w.file_ok || w.rec_req.is_none() && !w.rec_ok) {
                if cx.now >= self.retry_at && cx.online {
                    self.upload(cx);
                } else {
                    cx.wake(self.retry_at);
                }
            }
            return;
        }
        if !self.may_write(cx) {
            if cx.flush == Flush::Keepalive && !cx.leader {
                cx.flush = Flush::None;
            }
            return;
        }
        let Some(due) = self.due(cx, r) else {
            if cx.flush != Flush::None && !Self::unsealed(cx, r) && !cx.mail_changed {
                cx.flush = Flush::None;
            }
            return;
        };
        if cx.now >= due {
            let left = self.want_left;
            self.start(cx, r, left);
        } else {
            cx.wake(due);
        }
    }

    // -------------------------------------------------------------------------------------------- reader

    fn read_tick(&mut self, cx: &mut Core) {
        if !cx.networking() {
            return;
        }
        let active = self.reads.values().filter(|j| j.stage != RStage::Wait).count();
        let mut free = READS.saturating_sub(active);
        let now = cx.now;
        for j in self.reads.values_mut().filter(|j| matches!(j.stage, RStage::Wait | RStage::ArchWait)) {
            if j.at > now {
                cx.wake(j.at);
            } else if j.stage == RStage::ArchWait {
                j.stage = RStage::ArchFetch;
                j.req = Some(cx.http(HttpReq::Media { file: j.todo[0].file_id.clone() }));
            } else if free > 0 {
                free -= 1;
                j.stage = RStage::Fetch;
                j.req = Some(cx.http(HttpReq::Media { file: j.rec.file_id.clone() }));
            }
        }
    }

    fn give_up(&mut self, inst: u64) {
        if let Some(j) = self.reads.remove(&inst) {
            self.stale.insert(inst, j.rec.value());
        }
    }

    fn retry_read(&mut self, cx: &mut Core, inst: u64, delay: f64) {
        let j = self.reads.get_mut(&inst).expect("read");
        j.tries += 1;
        if j.tries >= DIGEST_RETRIES {
            return self.give_up(inst);
        }
        (j.stage, j.at, j.req) = (RStage::Wait, cx.now + delay, None);
        cx.wake(j.at);
    }

    fn read_answer(&mut self, cx: &mut Core, id: u32, status: u16, body: &[u8], r: &mut dyn Replica) -> bool {
        let Some((&inst, _)) = self.reads.iter().find(|(_, j)| j.req == Some(id)) else { return false };
        let j = self.reads.get_mut(&inst).expect("read");
        j.req = None;
        match (j.stage, status) {
            (RStage::Fetch, 200) => {
                j.raw = body.to_vec();
                j.stage = RStage::Digest;
                j.req = Some(cx.digest(body.to_vec()));
            }
            (RStage::Fetch, 404) => self.give_up(inst),
            (RStage::Fetch, _) => self.retry_read(cx, inst, 1000.0),
            (RStage::ArchFetch, 200) => {
                j.raw = body.to_vec();
                j.stage = RStage::ArchDigest;
                j.req = Some(cx.digest(body.to_vec()));
            }
            (RStage::ArchFetch, 404) => {
                // GC'd: the export covers it (§8.6 step 3).
                j.todo.remove(0);
                self.next_archive(cx, inst, r);
            }
            (RStage::ArchFetch, _) => {
                j.tries += 1;
                if j.tries >= DIGEST_RETRIES {
                    j.todo.remove(0);
                    self.next_archive(cx, inst, r);
                } else {
                    (j.stage, j.at) = (RStage::ArchWait, cx.now + 1000.0);
                    cx.wake(j.at);
                }
            }
            _ => {}
        }
        true
    }

    fn read_digest(&mut self, cx: &mut Core, id: u32, out: &[u8], r: &mut dyn Replica) -> bool {
        let Some((&inst, _)) = self.reads.iter().find(|(_, j)| j.req == Some(id)) else { return false };
        let j = self.reads.get_mut(&inst).expect("read");
        j.req = None;
        if j.stage == RStage::Digest {
            if d16(out) != j.rec.d16 {
                // Seal ahead of the upload (or a tampered file): try again shortly (§8.6 step 2).
                self.retry_read(cx, inst, DIGEST_RETRY_MS);
                return true;
            }
            match unwrap(false, &j.raw) {
                Some((true, body)) => {
                    j.stage = RStage::Inflate;
                    j.req = Some(cx.deflate(true, body.to_vec()));
                }
                Some((false, body)) => {
                    let body = body.to_vec();
                    self.file_read(cx, inst, &body, r);
                }
                None => self.give_up(inst),
            }
        } else if j.stage == RStage::ArchDigest {
            let want = j.todo[0].digest;
            if out.get(..12) != Some(&want[..]) {
                j.todo.remove(0);
                self.next_archive(cx, inst, r);
                return true;
            }
            match unwrap(true, &j.raw) {
                Some((true, body)) => {
                    j.stage = RStage::ArchInflate;
                    j.req = Some(cx.deflate(true, body.to_vec()));
                }
                Some((false, body)) => {
                    let body = body.to_vec();
                    self.archive_read(cx, inst, &body, r);
                }
                None => {
                    j.todo.remove(0);
                    self.next_archive(cx, inst, r);
                }
            }
        }
        true
    }

    fn read_inflate(&mut self, cx: &mut Core, id: u32, out: &[u8], r: &mut dyn Replica) -> bool {
        let Some((&inst, _)) = self.reads.iter().find(|(_, j)| j.req == Some(id)) else { return false };
        let j = self.reads.get_mut(&inst).expect("read");
        j.req = None;
        match j.stage {
            RStage::Inflate => self.file_read(cx, inst, out, r),
            RStage::ArchInflate => self.archive_read(cx, inst, out, r),
            _ => {}
        }
        true
    }

    /// Step 3: the relay file passed the digest check.
    fn file_read(&mut self, cx: &mut Core, inst: u64, payload: &[u8], r: &mut dyn Replica) {
        let Some(file) = RelayFile::decode_payload(payload) else { return self.give_up(inst) };
        if file.doc_id != cx.doc_id || file.inst != inst {
            // A Drive copy carries the original's records: never apply them; the next write nulls them.
            if file.doc_id != cx.doc_id {
                self.foreign.insert(inst);
            }
            return self.give_up(inst);
        }
        let rv = r.version_vector();
        let mut todo = Vec::new();
        for g in &file.ranges {
            let have = rv.get(g.actor);
            if have + 1 < g.first {
                for a in &file.archives {
                    if a.ranges.iter().any(|x| x.actor == g.actor && x.last > have && x.first < g.first) && !todo.contains(a) && valid_file_id(&a.file_id) {
                        todo.push(a.clone());
                    }
                }
            }
        }
        // Actors only in archives (the whole tail rotated out).
        for a in &file.archives {
            let only = a.ranges.iter().any(|x| !file.ranges.iter().any(|g| g.actor == x.actor) && x.last > rv.get(x.actor));
            if only && !todo.contains(a) && valid_file_id(&a.file_id) {
                todo.push(a.clone());
            }
        }
        let j = self.reads.get_mut(&inst).expect("read");
        j.batches = file.batches.clone();
        j.file = Some(file);
        j.todo = todo;
        self.next_archive(cx, inst, r);
    }

    fn archive_read(&mut self, cx: &mut Core, inst: u64, payload: &[u8], r: &mut dyn Replica) {
        let j = self.reads.get_mut(&inst).expect("read");
        let a = j.todo.remove(0);
        if let Some((doc, from, batches)) = decode_archive(payload) {
            if doc == cx.doc_id && from == inst {
                j.batches.extend(batches);
                j.got.push(a);
            }
        }
        self.next_archive(cx, inst, r);
    }

    fn next_archive(&mut self, cx: &mut Core, inst: u64, r: &mut dyn Replica) {
        let j = self.reads.get_mut(&inst).expect("read");
        if let Some(a) = j.todo.first() {
            j.stage = RStage::ArchFetch;
            j.req = Some(cx.http(HttpReq::Media { file: a.file_id.clone() }));
            return;
        }
        let j = self.reads.remove(&inst).expect("read");
        self.apply(cx, inst, j, r);
    }

    /// Steps 4–7.
    fn apply(&mut self, cx: &mut Core, inst: u64, j: Read, r: &mut dyn Replica) {
        let Some(file) = j.file else { return };
        let own = inst == cx.inst;
        let mut batches: Vec<EncodedBatch> = j.batches.into_iter().filter(|b| b.header().is_some_and(|(a, _, _)| relayed_by(a, inst, &cx.doc_id))).collect();
        batches.sort_by_key(|b| b.header().map(|(a, s, _)| (a, s)));
        // A gap the archives could not fill: the export must be downloaded (§8.6 step 3, §8.8).
        let rv = r.version_vector().clone();
        let mut covered = rv.clone();
        batches.iter().filter_map(|b| b.header()).for_each(|(a, s, _)| { covered.add(a, s); });
        if file.ranges.iter().chain(file.archives.iter().flat_map(|a| &a.ranges)).any(|g| covered.get(g.actor) < g.last && !(g.first..=g.last).all(|s| covered.contains(g.actor, s))) {
            cx.need_export = true;
        }
        if !batches.is_empty() {
            r.receive(&batches, PeerRef::Relay(inst));
        }
        let mut sealed: BTreeMap<ActorId, u32> = BTreeMap::new();
        for g in file.ranges.iter().chain(file.archives.iter().flat_map(|a| &a.ranges)) {
            let e = sealed.entry(g.actor).or_insert(0);
            *e = (*e).max(g.last);
        }
        if own {
            // Recovery of an own relay file written by an incarnation whose state is lost (degraded storage, §7.2):
            // adopt its gen, file (rotated away by the next write), archives and seal.
            cx.persist(SyncRecord::RelayPlan { gen: file.gen.max(cx.stored.relay_gen), file_id: j.rec.file_id.clone(), epoch_day: 0 });
            cx.persist(SyncRecord::RelaySealed { sealed: sealed.into_iter().collect(), archives: file.archives.clone(), assets: file.assets.clone(), perm: cx.stored.perm });
            return;
        }
        let current = cx.presence.remotes.get(&inst).is_some_and(|x| Record::parse(&x.value).is_some_and(|v| v.gen == j.rec.gen && v.d16 == j.rec.d16));
        if current {
            cx.presence.verified(inst);
            if let Some(x) = cx.presence.remotes.get_mut(&inst) {
                x.profile = Some(file.profile.clone());
            }
            let me = cx.inst;
            cx.mail_in.extend(file.mail.iter().filter(|m| m.to == me).map(|m| (inst, m.clone())));
        }
        if let Some((head, vv)) = &file.export {
            if !cx.stored.exports.iter().any(|(h, v)| h == head && v == vv) && !head.is_empty() {
                cx.persist(SyncRecord::KnownExport { head: head.clone(), vv: vv.clone() });
            }
        }
        let all: Vec<Range> = file.archives.iter().flat_map(|a| a.ranges.clone()).chain(file.ranges.clone()).collect();
        self.seen.insert(inst, (file.written_at, all));
        cx.persist(SyncRecord::RelayApplied { inst, gen: j.rec.gen, d16: j.rec.d16.clone() });
    }

    /// A poll answer: schedule reads of changed records, own-file recovery and record GC.
    fn observe(&mut self, cx: &mut Core) {
        let Some((meta, _)) = cx.meta.clone() else { return };
        let mut records = 0;
        for (key, value) in &meta.props {
            let Some(inst) = mode::record_inst(key) else { continue };
            records += 1;
            let Some(rec) = Record::parse(value) else { continue };
            let own = inst == cx.inst;
            if own && (rec.gen <= cx.stored.relay_gen || !cx.can_edit) {
                continue;
            }
            let applied = cx.stored.applied.get(&inst).is_some_and(|(g, d)| *g == rec.gen && *d == rec.d16);
            if applied && !own {
                // Read and checked in an earlier session: still vouched for (§10.1 b).
                cx.presence.verified(inst);
                continue;
            }
            if self.stale.get(&inst) == Some(value) || self.foreign.contains(&inst) {
                continue;
            }
            match self.reads.get(&inst) {
                Some(j) if j.rec == rec => {}
                Some(j) if j.stage != RStage::Wait => {} // finishes; the new value is read on the next poll
                _ => drop(self.reads.insert(inst, Read::new(rec, cx.now))),
            }
        }
        // Record GC (§8.4), at most once per session: records unchanged for 30 days (by their files' write time)
        // whose ranges the newest export covers.
        if records > RECORD_GC_COUNT && !self.gc_done && cx.can_edit {
            self.gc_done = true;
            let export = cx.stored.export_vv().cloned().unwrap_or_default();
            self.gc = self.seen.iter().filter(|(_, (at, ranges))| cx.now - *at as f64 >= RECORD_GC_AGE_MS && ranges.iter().all(|g| export.get(g.actor) >= g.last)).map(|(&i, _)| i).collect();
        }
    }
}

impl Part for Relay {
    fn local_batch(&mut self, cx: &mut Core, b: &EncodedBatch, _r: &mut dyn Replica) {
        if !b.header().is_some_and(|(a, _, _)| cx.relays(a)) {
            return;
        }
        self.last_batch_at = cx.now;
        if self.pending_since.is_none() {
            self.pending_since = Some(cx.now);
            if !cx.stored.relay_pending {
                cx.persist(SyncRecord::Flags { relay_pending: Some(true), export_pending: None, created: None });
            }
        }
    }

    fn meta(&mut self, cx: &mut Core, _r: &mut dyn Replica) {
        if cx.can_edit {
            self.demoted = false;
        }
        self.observe(cx);
    }

    fn http(&mut self, cx: &mut Core, id: u32, status: u16, _etag: &str, body: &[u8], r: &mut dyn Replica) {
        if self.folder_req == Some(id) {
            self.folder_req = None;
            if status == 200 {
                let text = String::from_utf8_lossy(body);
                let mut lines = text.lines();
                if let Some(folder) = lines.next().filter(|f| !f.is_empty()) {
                    cx.persist(SyncRecord::RelayFolder(folder.into()));
                    cx.persist(SyncRecord::NoLink(lines.next() == Some("1")));
                }
            }
            return;
        }
        if self.ids_req == Some(id) {
            self.ids_req = None;
            if status == 200 {
                let mut pool = cx.stored.id_pool.clone();
                pool.extend(String::from_utf8_lossy(body).lines().map(str::to_string).filter(|i| valid_file_id(i)));
                cx.persist(SyncRecord::IdPool(pool));
            }
            return;
        }
        if !self.write_answer(cx, id, status, body, r) {
            self.read_answer(cx, id, status, body, r);
        }
    }

    fn digest(&mut self, cx: &mut Core, id: u32, out: &[u8], r: &mut dyn Replica) {
        if let Some(w) = self.w.as_mut().filter(|w| w.req == Some(id)) {
            w.req = None;
            match w.stage {
                WStage::ArchDigest => {
                    w.arch_digest.copy_from_slice(&out[..12]);
                    match Self::take_id(cx) {
                        Some(id) => {
                            self.w.as_mut().expect("write").arch_id = id;
                            self.create_archive(cx);
                        }
                        None => self.abort(cx, false),
                    }
                }
                WStage::Digest => self.ready(cx, out),
                _ => {}
            }
            return;
        }
        self.read_digest(cx, id, out, r);
    }

    fn deflate(&mut self, cx: &mut Core, id: u32, out: &[u8], r: &mut dyn Replica) {
        if let Some(w) = self.w.as_mut().filter(|w| w.req == Some(id)) {
            w.req = None;
            match w.stage {
                WStage::Deflate => self.tail_deflated(cx, out),
                WStage::ArchDeflate => {
                    w.arch_bytes = wrap(true, true, out);
                    w.stage = WStage::ArchDigest;
                    w.req = Some(cx.digest(w.arch_bytes.clone()));
                }
                WStage::Digest => {
                    w.bytes = wrap(false, true, out);
                    w.req = Some(cx.digest(w.bytes.clone()));
                }
                _ => {}
            }
            return;
        }
        self.read_inflate(cx, id, out, r);
    }

    fn change(&mut self, cx: &mut Core, c: Change, _r: &mut dyn Replica) {
        match c {
            Change::Suspend | Change::Leave if cx.can_edit && !self.left => {
                // Record `l` (keepalive) if this installation has a record or has written one in this session.
                if Self::own_record(cx).is_some_and(|o| !o.flags.contains('l')) || self.wrote_at.is_finite() {
                    self.want_left = true;
                }
            }
            Change::Visible => self.left = false,
            Change::Leader(true) => {
                // Resume from `ops`/`docs.relay` (§11): unsealed batches of the other tabs are written by us now.
                self.pending_since = Some(cx.now);
            }
            Change::Leader(false) => {
                self.reads.retain(|_, j| j.stage != RStage::Wait);
            }
            Change::CanEdit(true) => self.demoted = false,
            _ => {}
        }
    }

    fn tick(&mut self, cx: &mut Core, r: &mut dyn Replica) {
        self.housekeeping(cx);
        self.write_tick(cx, r);
        self.read_tick(cx);
    }
}

#[cfg(test)]
#[path = "relay_tests.rs"]
mod tests;
