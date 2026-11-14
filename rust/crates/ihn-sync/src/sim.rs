//! Deterministic simulator for the sync tests (SYNC §19.2); compiled for tests only. Owner: W2.6.
//!
//! One [`Sim`] holds a fake Drive, any number of devices (one tab with one open document each), the data
//! channels and BroadcastChannels between them and a virtual clock. Every device sees its own clock (`now + skew`).
//! It executes the [`SyncEffect`]s of each device's [`Sync`] and feeds the answers back as [`SyncIo`], in a single
//! ordered event queue: the same seed gives the same run.
//!
//! ```ignore
//! let mut sim = Sim::new(1);
//! let doc = sim.drive.create_doc("ann@x", "Notes");
//! sim.drive.share(&doc, "bob@x", true);
//! let a = sim.add_dev("ann@x", 0x1000, 0.0, my_parts);       // fn() -> Vec<Box<dyn Part>>
//! let b = sim.add_dev("bob@x", 0x2000, 600_000.0, my_parts);  // clock 10 min ahead
//! sim.open(a, &doc);  sim.open(b, &doc);
//! sim.edit(a);  sim.run(4000.0);
//! assert!(sim.converged(&[a, b]));
//! ```
//! Crash points: [`Sim::crash`] at any time, or [`Sim::crash_on`] at a chosen effect; `open` again restarts the
//! device from what it persisted. Failures: [`Drive::inject`], [`Drive::rate_limit`], [`Sim::set_online`],
//! [`Sim::block_p2p`].

use crate::*;
use ihn_core::doc::{Author, BlobHandle, Compacted, Doc, ReceiveResult, Tx};
use ihn_core::id::{inst13, IdGen};
use ihn_core::op::{Background, Body, Color, Template, Tool};
use ihn_core::snapshot::{self, EncodedSnapshot};
use ihn_core::testutil::Rng;
use std::collections::BTreeSet;

/// Test command hook kept for the module table.
pub fn test_cmd(_name: &str, _args: &str) -> Option<String> { None }

/// Simulated epoch of the true clock (ms): 2026-11-13T00:00:00Z plus a bit, so UTC-day logic has a real date.
pub const T0: f64 = 1_794_528_000_000.0 + 3_600_000.0;

// ------------------------------------------------------------------------------------------------ helpers

pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be,
        0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa,
        0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85,
        0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
        0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f,
        0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&(data.len() as u64 * 8).to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes(word.try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let t1 = v[7].wrapping_add(s1).wrapping_add((v[4] & v[5]) ^ (!v[4] & v[6])).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let t2 = s0.wrapping_add((v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]));
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// Valid deflate-raw made of stored blocks (no compression: sizes in the sim are upper bounds).
pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 5);
    let mut chunks = data.chunks(0xffff).peekable();
    if data.is_empty() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(chunk) = chunks.next() {
        out.push(chunks.peek().is_none() as u8);
        out.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(chunk.len() as u16)).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out
}

/// Inverse of [`deflate`]; `None` for anything else.
pub fn inflate(mut data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let (head, rest) = data.split_at_checked(5)?;
        let len = u16::from_le_bytes([head[1], head[2]]);
        if head[0] > 1 || !len != u16::from_le_bytes([head[3], head[4]]) {
            return None;
        }
        let (chunk, rest) = rest.split_at_checked(len as usize)?;
        out.extend_from_slice(chunk);
        data = rest;
        if head[0] == 1 {
            return data.is_empty().then_some(out);
        }
    }
}

// ------------------------------------------------------------------------------------------------ replica

/// A replica as `doc::session` will be: model + journal + assets. Everything in it is "in IndexedDB": it survives
/// a crash.
pub struct SimReplica {
    pub doc_id: DocId,
    pub doc: Doc,
    pub author: Author,
    pub journal: BTreeMap<(ActorId, u32), EncodedBatch>,
    pub assets: BTreeMap<Hash, BlobHandle>,
    pub can_edit: bool,
    /// Every source a batch or snapshot was received from, in order.
    pub received: Vec<PeerRef>,
    wall: u64,
}

impl SimReplica {
    pub fn new(doc_id: &str, actor: ActorId) -> SimReplica {
        SimReplica {
            doc_id: doc_id.into(),
            doc: Doc::default(),
            author: Author::new(IdGen::new(actor, 0), 0),
            journal: BTreeMap::new(),
            assets: BTreeMap::new(),
            can_edit: true,
            received: Vec::new(),
            wall: 0,
        }
    }
    /// One local edit (a stroke; the first edit also creates the page), committed to the journal.
    pub fn edit(&mut self, wall_ms: u64, rng: &mut Rng) -> EncodedBatch {
        self.wall = wall_ms;
        let mut tx = Tx::new(&self.doc, &mut self.author, wall_ms);
        let page = match self.doc.order().first() {
            Some(&page) => page,
            None => tx.page_new(None, Template::Default, (794, 1123), Background::Template),
        };
        let xy = (0..8).map(|_| rng.range(0.0, 700.0)).collect();
        tx.add_stroke(page, Body::Stroke { tool: Tool::Pen, color: Color::Hex(rng.below(1 << 24) as u32), width: 2.0, xy, p: None });
        let batch = tx.commit();
        let bytes = snapshot::encode_batch(&batch);
        self.doc.apply(&batch);
        self.journal.insert((batch.actor, batch.seq), bytes.clone());
        bytes
    }
    /// Canonical state bytes: equal on converged replicas.
    pub fn state(&self) -> Vec<u8> { snapshot::encode(&self.doc).0 }
}

impl Replica for SimReplica {
    fn doc_id(&self) -> &DocId { &self.doc_id }
    fn version_vector(&self) -> &VersionVector { &self.doc.vv }
    fn batches_since(&self, have: &VersionVector, max_bytes: usize) -> Result<Vec<EncodedBatch>, Compacted> {
        let (mut out, mut size) = (Vec::new(), 0);
        let wanted = self.doc.vv.iter().flat_map(|(a, top)| (have.get(a) + 1..=top).map(move |s| (a, s))).chain(self.doc.vv.extras());
        for key in wanted.filter(|&(a, s)| !have.contains(a, s)) {
            let b = self.journal.get(&key).ok_or(Compacted)?;
            if !out.is_empty() && size + b.0.len() > max_bytes {
                break;
            }
            size += b.0.len();
            out.push(b.clone());
        }
        Ok(out)
    }
    fn receive(&mut self, batches: &[EncodedBatch], from: PeerRef) -> ReceiveResult {
        let mut res = ReceiveResult::default();
        for bytes in batches {
            let Ok(batch) = bytes.decode() else { continue };
            if self.doc.vv.contains(batch.actor, batch.seq) {
                res.duplicate += 1;
                continue;
            }
            self.author.clock.observe(batch.max_clock(), self.wall);
            self.journal.insert((batch.actor, batch.seq), bytes.clone());
            res.changed_pages.extend(self.doc.apply_owned(batch).pages);
            res.applied += 1;
        }
        self.received.push(from);
        res
    }
    fn snapshot(&self) -> EncodedSnapshot { snapshot::encode(&self.doc) }
    fn load_snapshot(&mut self, s: &EncodedSnapshot, from: PeerRef) -> ReceiveResult {
        let Ok(other) = s.decode() else { return ReceiveResult::default() };
        self.author.clock.observe(other.max_clock(self.wall), self.wall);
        let changes = self.doc.merge(&other);
        self.received.push(from);
        ReceiveResult { applied: 1, changed_pages: changes.pages, ..Default::default() }
    }
    fn missing_assets(&self) -> Vec<Hash> { Vec::new() }
    fn put_asset(&mut self, h: Hash, bytes: BlobHandle) { self.assets.insert(h, bytes); }
    fn asset(&self, h: &Hash) -> Option<BlobHandle> { self.assets.get(h).copied() }
    fn can_edit(&self) -> bool { self.can_edit }
    /// Drops every covered entry at once (the real session also waits for compaction): exercises `Compacted`.
    fn retain(&mut self, covered: &VersionVector) { self.journal.retain(|&(a, s), _| s > covered.get(a)) }
}

// ------------------------------------------------------------------------------------------------ fake Drive

#[derive(Clone, Default, Debug)]
pub struct DFile {
    pub name: String,
    pub bytes: Vec<u8>,
    pub props: BTreeMap<String, String>,
    pub app: BTreeMap<String, String>,
    /// Head revision counter: changes on content writes only.
    pub head: u64,
    /// v2 etag counter: changes on any write, metadata included.
    pub etag: u64,
    pub version: u64,
    pub modified_ms: u64,
    pub trashed: bool,
    pub owner: String,
    pub editors: BTreeSet<String>,
    pub readers: BTreeSet<String>,
    /// Readable by anyone who knows the id (inherited from the sync folder).
    pub link: bool,
}

impl DFile {
    pub fn can_edit(&self, account: &str) -> bool { self.owner == account || self.editors.contains(account) }
    pub fn can_read(&self, account: &str) -> bool { self.link || self.can_edit(account) || self.readers.contains(account) }
    pub fn permission_ids(&self) -> Vec<String> { [&self.owner].into_iter().chain(&self.editors).chain(&self.readers).map(|a| format!("perm-{a}")).collect() }
    fn touch(&mut self, now: f64, content: bool) {
        self.etag += 1;
        self.version += 1;
        self.head += content as u64;
        self.modified_ms = now as u64;
    }
    pub fn head_id(&self) -> String { format!("rev-{}", self.head) }
    pub fn etag_id(&self) -> String { format!("\"e{}\"", self.etag) }
}

/// A scripted failure: the next `times` requests matching `when` (after skipping `skip`) answer `status`/`body`.
/// With `lost`, the request is executed and only its answer is lost (status 0).
pub struct Inject {
    pub when: fn(&HttpReq) -> bool,
    pub status: u16,
    pub body: &'static str,
    pub skip: u32,
    pub times: u32,
    pub lost: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Logged {
    /// True time of arrival at Drive (ms since [`T0`]).
    pub at: f64,
    pub dev: usize,
    pub kind: &'static str,
    pub file: String,
    pub status: u16,
    pub keepalive: bool,
}

#[derive(Default)]
pub struct Drive {
    pub files: BTreeMap<String, DFile>,
    /// Accounts whose policy refuses link sharing (record flag `x`).
    pub no_link: BTreeSet<String>,
    pub injected: Vec<Inject>,
    /// `Props` writes allowed per file per window: `(count, window ms)`; more answer 403 `rateLimitExceeded`.
    pub rate_limit: Option<(usize, f64)>,
    pub log: Vec<Logged>,
    reserved: BTreeSet<String>,
    folders: BTreeMap<String, String>,
    next: u64,
}

pub const PROP_BYTES: usize = 124;
pub const PROP_KEYS: usize = 30;

pub fn kind(req: &HttpReq) -> &'static str {
    match req {
        HttpReq::Meta { .. } => "meta",
        HttpReq::Media { .. } => "media",
        HttpReq::Create { .. } => "create",
        HttpReq::Update { .. } => "update",
        HttpReq::Props { .. } => "props",
        HttpReq::Put { .. } => "put",
        HttpReq::Delete { .. } => "delete",
        HttpReq::GenerateIds { .. } => "ids",
        HttpReq::SyncFolder => "folder",
    }
}

fn file_of(req: &HttpReq) -> &str {
    match req {
        HttpReq::Meta { file } | HttpReq::Media { file } | HttpReq::Create { file, .. } | HttpReq::Update { file, .. } | HttpReq::Props { file, .. } | HttpReq::Put { file, .. } | HttpReq::Delete { file } => file,
        _ => "",
    }
}

fn keepalive(req: &HttpReq) -> bool {
    matches!(req, HttpReq::Create { keepalive: true, .. } | HttpReq::Update { keepalive: true, .. } | HttpReq::Props { keepalive: true, .. })
}

type Answer = (u16, String, Vec<u8>);

impl Drive {
    fn id(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{:0>28}", self.next)
    }
    /// A document owned by `owner` with one revision.
    pub fn create_doc(&mut self, owner: &str, name: &str) -> String {
        let id = self.id("doc");
        self.files.insert(id.clone(), DFile { name: name.into(), bytes: b"%PDF-".to_vec(), owner: owner.into(), head: 1, etag: 1, version: 1, ..Default::default() });
        id
    }
    pub fn share(&mut self, file: &str, account: &str, edit: bool) {
        let f = self.files.get_mut(file).expect("file");
        f.editors.remove(account);
        f.readers.remove(account);
        [&mut f.readers, &mut f.editors][edit as usize].insert(account.into());
        f.touch(0.0, false);
    }
    pub fn unshare(&mut self, file: &str, account: &str) {
        let f = self.files.get_mut(file).expect("file");
        f.editors.remove(account);
        f.readers.remove(account);
        f.touch(0.0, false);
    }
    /// A Drive-side copy: content and public properties are copied, permissions are not (SYNC §8.6 step 3).
    pub fn copy(&mut self, file: &str, owner: &str) -> String {
        let src = &self.files[file];
        let copy = DFile { name: format!("Copy of {}", src.name), bytes: src.bytes.clone(), props: src.props.clone(), owner: owner.into(), head: 1, etag: 1, version: 1, ..Default::default() };
        let id = self.id("doc");
        self.files.insert(id.clone(), copy);
        id
    }
    pub fn inject(&mut self, when: fn(&HttpReq) -> bool, status: u16, body: &'static str, times: u32) {
        self.injected.push(Inject { when, status, body, skip: 0, times, lost: false });
    }
    /// Requests of `kind` logged for `dev` with true arrival time in `[from, to)` (ms since [`T0`]).
    pub fn count(&self, dev: usize, kind: &str, from: f64, to: f64) -> usize {
        self.log.iter().filter(|l| l.dev == dev && l.kind == kind && l.at >= from && l.at < to).count()
    }

    fn exec(&mut self, account: &str, req: &HttpReq, now: f64) -> Answer {
        let ok = |body: Vec<u8>| (200, String::new(), body);
        let err = |status: u16, reason: &str| (status, String::new(), reason.as_bytes().to_vec());
        match req {
            HttpReq::GenerateIds { count } => {
                let ids: Vec<String> = (0..*count).map(|_| self.id("gen")).collect();
                self.reserved.extend(ids.iter().cloned());
                ok(ids.join("\n").into_bytes())
            }
            HttpReq::SyncFolder => {
                let id = match self.folders.get(account) {
                    Some(id) => id.clone(),
                    None => {
                        let id = self.id("folder");
                        self.folders.insert(account.into(), id.clone());
                        id
                    }
                };
                ok(if self.no_link.contains(account) { format!("{id}\n1") } else { id }.into_bytes())
            }
            HttpReq::Create { file, name, app, bytes, .. } => {
                if self.files.contains_key(file) {
                    return err(409, "conflict");
                }
                if !self.reserved.remove(file) {
                    return err(400, "invalid id");
                }
                let link = !self.no_link.contains(account);
                let mut f = DFile { name: name.clone(), bytes: bytes.clone(), app: app.iter().cloned().collect(), owner: account.into(), link, ..Default::default() };
                f.touch(now, true);
                self.files.insert(file.clone(), f);
                ok(Vec::new())
            }
            _ => {
                let Some(f) = self.files.get_mut(file_of(req)).filter(|f| f.can_read(account)) else { return err(404, "notFound") };
                let can_edit = f.can_edit(account);
                match req {
                    HttpReq::Meta { .. } => {
                        let meta = DriveMeta {
                            props: f.props.clone().into_iter().collect(),
                            app: f.app.clone().into_iter().collect(),
                            head: f.head_id(),
                            version: f.version,
                            modified_ms: f.modified_ms,
                            name: f.name.clone(),
                            trashed: f.trashed,
                            can_edit,
                            permission_ids: if can_edit { f.permission_ids() } else { Vec::new() },
                        };
                        (200, f.etag_id(), meta.encode())
                    }
                    HttpReq::Media { .. } => (200, f.etag_id(), f.bytes.clone()),
                    _ if !can_edit => err(403, "insufficientFilePermissions"),
                    HttpReq::Update { bytes, .. } => {
                        f.bytes = bytes.clone();
                        f.touch(now, true);
                        ok(Vec::new())
                    }
                    HttpReq::Props { set, app, .. } => {
                        let mut props = f.props.clone();
                        for (k, v) in set {
                            match v {
                                Some(v) if k.len() + v.len() > PROP_BYTES => return err(400, "invalid property"),
                                Some(v) => drop(props.insert(k.clone(), v.clone())),
                                None => drop(props.remove(k)),
                            }
                        }
                        if props.len() > PROP_KEYS {
                            return err(400, "too many properties");
                        }
                        if let Some((max, window)) = self.rate_limit {
                            let recent = self.log.iter().filter(|l| l.kind == "props" && l.file == *file_of(req) && l.status == 200 && l.at > now - T0 - window).count();
                            if recent >= max {
                                return err(403, "rateLimitExceeded");
                            }
                        }
                        f.props = props;
                        for (k, v) in app {
                            match v {
                                Some(v) => drop(f.app.insert(k.clone(), v.clone())),
                                None => drop(f.app.remove(k)),
                            }
                        }
                        f.touch(now, false);
                        ok(Vec::new())
                    }
                    HttpReq::Put { if_match, bytes, app, .. } => {
                        if *if_match != f.etag_id() {
                            return err(412, "conditionNotMet");
                        }
                        f.bytes = bytes.clone();
                        f.app.extend(app.iter().cloned());
                        f.touch(now, true);
                        (200, f.etag_id(), f.head_id().into_bytes())
                    }
                    HttpReq::Delete { .. } => {
                        if f.owner != account {
                            return err(403, "insufficientFilePermissions");
                        }
                        self.files.remove(file_of(req));
                        (204, String::new(), Vec::new())
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    /// One request arriving at Drive at true time `now`.
    pub fn request(&mut self, dev: usize, account: &str, req: &HttpReq, now: f64) -> Answer {
        let mut lost = false;
        let mut answer = None;
        for i in 0..self.injected.len() {
            let inj = &mut self.injected[i];
            if !(inj.when)(req) {
                continue;
            }
            if inj.skip > 0 {
                inj.skip -= 1;
                continue;
            }
            inj.times -= 1;
            if inj.lost {
                lost = true;
            } else {
                answer = Some((inj.status, String::new(), inj.body.as_bytes().to_vec()));
            }
            if inj.times == 0 {
                self.injected.remove(i);
            }
            break;
        }
        let mut answer = answer.unwrap_or_else(|| self.exec(account, req, now));
        self.log.push(Logged { at: now - T0, dev, kind: kind(req), file: file_of(req).into(), status: answer.0, keepalive: keepalive(req) });
        if lost {
            answer = (0, String::new(), Vec::new());
        }
        answer
    }
}

// ------------------------------------------------------------------------------------------------ world

pub type Parts = fn() -> Vec<Box<dyn Part>>;

pub struct Dev {
    pub account: String,
    pub inst: u64,
    /// Tabs with the same browser share the BroadcastChannel and the Web Locks.
    pub browser: usize,
    /// Local clock = true clock + skew (ms).
    pub skew: f64,
    pub online: bool,
    /// `None` while crashed / closed.
    pub sync: Option<Sync>,
    pub replica: SimReplica,
    /// What `SyncEffect::Persist` wrote: survives a crash.
    pub stored: Stored,
    pub doc: String,
    pub parts: Parts,
    /// Every effect the device emitted (tests inspect it).
    pub effects: Vec<SyncEffect>,
    boot: u32,
    timer: u32,
    crash_on: Option<(Box<dyn Fn(&SyncEffect) -> bool>, bool)>,
}

enum Ev {
    Io { dev: usize, boot: u32, io: SyncIo },
    Tick { dev: usize, boot: u32, timer: u32 },
    /// A request reaching Drive (it does even if the sender crashed meanwhile).
    Arrive { dev: usize, boot: u32, id: u32, req: HttpReq },
    /// DTLS done for the pair, if both sides still hold the same descriptions.
    Connect { a: usize, b: usize, nonce: u64 },
}

#[derive(Default)]
struct Conn {
    nonce: u64,
    remote_offer: Option<u64>,
    open: bool,
    last_delivery: f64,
}

pub struct Sim {
    /// True time (ms).
    pub now: f64,
    pub rng: Rng,
    pub drive: Drive,
    pub devs: Vec<Dev>,
    /// One-way data-channel delay (ms).
    pub link_ms: f64,
    /// Multiplies the Drive latency model of SYNC §17.
    pub latency_scale: f64,
    queue: BTreeMap<(u64, u64), Ev>,
    seq: u64,
    conns: BTreeMap<(usize, u64), Conn>,
    blocked: BTreeSet<(u64, u64)>,
    all_blocked: bool,
    locks: BTreeMap<(usize, String), Vec<usize>>,
}

impl Sim {
    pub fn new(seed: u64) -> Sim {
        Sim {
            now: T0,
            rng: Rng::new(seed),
            drive: Drive::default(),
            devs: Vec::new(),
            link_ms: 20.0,
            latency_scale: 1.0,
            queue: BTreeMap::new(),
            seq: 0,
            conns: BTreeMap::new(),
            blocked: BTreeSet::new(),
            all_blocked: false,
            locks: BTreeMap::new(),
        }
    }

    /// A device in its own browser. `inst_random` is masked to an installation id.
    pub fn add_dev(&mut self, account: &str, inst_random: u64, skew: f64, parts: Parts) -> usize {
        let browser = self.devs.len();
        self.add_tab(account, ActorId::inst_from_random(inst_random), 0, browser, skew, parts)
    }
    /// Another tab of `of`'s browser (same installation, next slot).
    pub fn add_sibling(&mut self, of: usize) -> usize {
        let d = &self.devs[of];
        let slot = self.devs.iter().filter(|x| x.browser == d.browser).count() as u8;
        self.add_tab(&d.account.clone(), d.inst, slot, d.browser, d.skew, d.parts)
    }
    fn add_tab(&mut self, account: &str, inst: u64, slot: u8, browser: usize, skew: f64, parts: Parts) -> usize {
        assert!(inst != 0 && inst & 0xff == 0);
        self.devs.push(Dev {
            account: account.into(),
            inst,
            browser,
            skew,
            online: true,
            sync: None,
            replica: SimReplica::new("", ActorId::tab(inst, slot)),
            stored: Stored { created: true, ..Default::default() },
            doc: String::new(),
            parts,
            effects: Vec::new(),
            boot: 0,
            timer: 0,
            crash_on: None,
        });
        self.devs.len() - 1
    }

    pub fn local(&self, d: usize) -> f64 { self.now + self.devs[d].skew }
    pub fn sync(&mut self, d: usize) -> &mut Sync { self.devs[d].sync.as_mut().expect("device is open") }
    pub fn core(&self, d: usize) -> &Core { &self.devs[d].sync.as_ref().expect("device is open").core }
    /// `d` currently counts installation `inst` as present / vouched (by its own clock).
    pub fn present(&self, d: usize, inst: u64) -> bool { self.core(d).presence.present(inst, self.local(d)) }
    pub fn vouched(&self, d: usize, inst: u64) -> bool { self.core(d).presence.vouched(inst, self.local(d)) }
    pub fn phase(&self, d: usize) -> Phase { self.devs[d].sync.as_ref().map_or(Phase::Closed, |s| s.phase()) }

    /// Opens (or reopens after a crash) the document: a new `Sync` from what the device persisted.
    pub fn open(&mut self, d: usize, doc: &str) {
        let seed = self.rng.next();
        let dev = &mut self.devs[d];
        if dev.doc != doc {
            dev.replica = SimReplica::new(doc, dev.replica.author.actor);
            dev.doc = doc.into();
        }
        let can_edit = self.drive.files.get(doc).is_none_or(|f| f.can_edit(&dev.account));
        dev.replica.can_edit = can_edit;
        let profile = Profile { email: dev.account.clone(), name: dev.account.split('@').next().unwrap_or("").into(), device_type: b'p', ..Default::default() };
        let cfg = Config { doc_id: doc.into(), inst: dev.inst, actor: dev.replica.author.actor, profile, can_edit, visible: true, online: dev.online, stored: dev.stored.clone(), seed };
        dev.sync = Some(Sync::with_parts(cfg, (dev.parts)()));
        self.tick(d);
    }

    /// The tab dies: in-memory state is gone, requests already sent still reach Drive, locks are released.
    pub fn crash(&mut self, d: usize) {
        let dev = &mut self.devs[d];
        dev.sync = None;
        dev.boot += 1;
        dev.crash_on = None;
        self.conns.retain(|&(owner, _), _| owner != d);
        self.unlock(d);
    }
    /// Crash when the device emits an effect matching `when`: with `after`, that effect is still executed.
    pub fn crash_on(&mut self, d: usize, when: impl Fn(&SyncEffect) -> bool + 'static, after: bool) { self.devs[d].crash_on = Some((Box::new(when), after)) }

    pub fn set_online(&mut self, d: usize, online: bool) {
        self.devs[d].online = online;
        self.io(d, SyncIo::Lifecycle(if online { Lifecycle::Online } else { Lifecycle::Offline }));
    }
    /// No data channel can be established between the two installations (or between any, with `block_all_p2p`).
    pub fn block_p2p(&mut self, a: usize, b: usize) {
        let (x, y) = (self.devs[a].inst, self.devs[b].inst);
        self.blocked.insert((x.min(y), x.max(y)));
    }
    pub fn block_all_p2p(&mut self) { self.all_blocked = true }

    /// A local edit on `d`: committed to its journal, then handed to sync (SYNC §7.2).
    pub fn edit(&mut self, d: usize) -> EncodedBatch {
        let wall = self.local(d) as u64;
        let dev = &mut self.devs[d];
        let batch = dev.replica.edit(wall, &mut self.rng);
        if let Some(sync) = dev.sync.as_mut() {
            sync.local_batch(&batch);
        }
        self.io(d, SyncIo::Lifecycle(Lifecycle::Input));
        batch
    }

    pub fn converged(&self, devs: &[usize]) -> bool { devs.windows(2).all(|w| self.devs[w[0]].replica.state() == self.devs[w[1]].replica.state()) }

    /// Delivers an input now.
    pub fn io(&mut self, d: usize, io: SyncIo) {
        let local = self.local(d);
        let dev = &mut self.devs[d];
        let Some(sync) = dev.sync.as_mut() else { return };
        let fx = sync.at(local).on_io(io, &mut dev.replica);
        self.effects(d, fx);
    }
    pub fn tick(&mut self, d: usize) {
        let local = self.local(d);
        let dev = &mut self.devs[d];
        let Some(sync) = dev.sync.as_mut() else { return };
        let fx = sync.tick(local, &mut dev.replica);
        self.effects(d, fx);
    }

    fn push(&mut self, delay: f64, ev: Ev) {
        self.seq += 1;
        self.queue.insert((((self.now + delay.max(0.0)) * 1000.0) as u64, self.seq), ev);
    }
    fn push_io(&mut self, d: usize, delay: f64, io: SyncIo) {
        let boot = self.devs[d].boot;
        self.push(delay, Ev::Io { dev: d, boot, io });
    }
    fn dev_of(&self, inst: u64, doc: &str) -> Option<usize> { self.devs.iter().position(|x| x.inst == inst && x.doc == doc && x.sync.as_ref().is_some_and(|s| s.core.leader)) }

    /// Drive latency (ms) of one request: SYNC §17's ranges, split evenly between the two directions.
    fn latency(&mut self, req: &HttpReq) -> f64 {
        let (lo, hi) = match req {
            HttpReq::Meta { .. } => (100.0, 300.0),
            HttpReq::Media { .. } => (150.0, 400.0),
            _ => (300.0, 700.0),
        };
        self.rng.range(lo, hi) * self.latency_scale / 2.0
    }

    fn unlock(&mut self, d: usize) {
        let key = (self.devs[d].browser, self.devs[d].doc.clone());
        let Some(queue) = self.locks.get_mut(&key) else { return };
        let held = queue.first() == Some(&d);
        queue.retain(|&x| x != d);
        if let (true, Some(&next)) = (held, queue.first()) {
            self.push_io(next, 0.1, SyncIo::Locks(LockEvent::Net { held: true }));
        }
    }

    fn effects(&mut self, d: usize, fx: Vec<SyncEffect>) {
        for e in fx {
            let crash = self.devs[d].crash_on.as_ref().and_then(|(when, after)| when(&e).then_some(*after));
            if crash == Some(false) {
                return self.crash(d);
            }
            self.devs[d].effects.push(e.clone());
            self.effect(d, e);
            if crash == Some(true) {
                return self.crash(d);
            }
        }
    }

    fn effect(&mut self, d: usize, e: SyncEffect) {
        let boot = self.devs[d].boot;
        match e {
            SyncEffect::Http { id, req } => {
                if self.devs[d].online {
                    let up = self.latency(&req);
                    self.push(up, Ev::Arrive { dev: d, boot, id, req });
                } else {
                    self.push_io(d, 1.0, SyncIo::Http { id, status: 0, etag: String::new(), body: Vec::new() });
                }
            }
            SyncEffect::Digest { id, bytes } => self.push_io(d, 0.2, SyncIo::Digest { id, out: sha256(&bytes).to_vec() }),
            SyncEffect::Deflate { id, inflate: inf, bytes } => {
                let out = if inf { inflate(&bytes).unwrap_or_default() } else { deflate(&bytes) };
                self.push_io(d, 0.2, SyncIo::Deflate { id, out });
            }
            SyncEffect::Persist(rec) => self.devs[d].stored.apply(&rec),
            SyncEffect::Timer { at } => {
                self.devs[d].timer += 1;
                let timer = self.devs[d].timer;
                // Never early: the queue key truncates to microseconds and `local` is a float sum, and a tick that
                // arrives before `at` would ask for the same timer again, forever.
                self.push((at - self.local(d)).max(0.0) + 0.01, Ev::Tick { dev: d, boot, timer });
            }
            SyncEffect::Lock { hold: true } => {
                let queue = self.locks.entry((self.devs[d].browser, self.devs[d].doc.clone())).or_default();
                if !queue.contains(&d) {
                    queue.push(d);
                    if queue.len() == 1 {
                        self.push_io(d, 0.1, SyncIo::Locks(LockEvent::Net { held: true }));
                    }
                }
            }
            SyncEffect::Lock { hold: false } => {
                self.unlock(d);
                self.push_io(d, 0.1, SyncIo::Locks(LockEvent::Net { held: false }));
            }
            SyncEffect::Send { to: Dest::Tabs, bytes, .. } => {
                let (browser, doc) = (self.devs[d].browser, self.devs[d].doc.clone());
                for other in 0..self.devs.len() {
                    if other != d && self.devs[other].browser == browser && self.devs[other].doc == doc {
                        self.push_io(other, 0.05, SyncIo::Tab(bytes.clone()));
                    }
                }
            }
            SyncEffect::Send { to: Dest::Peer(peer), ch, bytes } => {
                let (inst, link, now) = (self.devs[d].inst, self.link_ms, self.now);
                let Some(other) = self.dev_of(peer, &self.devs[d].doc) else { return };
                let alive = self.devs[d].online && self.devs[other].online && self.conns.get(&(other, inst)).is_some_and(|c| c.open);
                let Some(conn) = self.conns.get_mut(&(d, peer)).filter(|c| c.open && alive) else { return };
                // Ordered and reliable: never earlier than the previous message of this direction.
                conn.last_delivery = conn.last_delivery.max(now + link);
                let delay = conn.last_delivery - now;
                self.push_io(other, delay, SyncIo::Rtc(RtcEvent::Message { peer: inst, ch, bytes }));
            }
            SyncEffect::Rtc(req) => self.rtc(d, req),
        }
    }

    /// Fake WebRTC: descriptions are `o:<inst>:<nonce>` / `a:<inst>:<nonce of the offer>`; a connection opens when
    /// the offerer applies an answer to its current offer and the pair is not blocked.
    fn rtc(&mut self, d: usize, req: RtcReq) {
        let inst = self.devs[d].inst;
        let gather = self.rng.range(70.0, 650.0);
        match req {
            RtcReq::Create { peer, offer, .. } => {
                let nonce = self.rng.next();
                self.conns.insert((d, peer), Conn { nonce, ..Default::default() });
                if offer {
                    self.push_io(d, gather, SyncIo::Rtc(RtcEvent::Local { peer, kind: SdpKind::Offer, sdp: format!("o:{inst}:{nonce}") }));
                }
            }
            RtcReq::SetRemote { peer, kind, sdp } => {
                let mut parts = sdp.split(':');
                let tag = parts.next();
                let from: Option<u64> = parts.next().and_then(|p| p.parse().ok());
                let nonce: Option<u64> = parts.next().and_then(|p| p.parse().ok());
                let other = self.dev_of(peer, &self.devs[d].doc);
                let Some(conn) = self.conns.get_mut(&(d, peer)) else { return };
                let valid = from == Some(peer) && tag == Some(["o", "a"][(kind == SdpKind::Answer) as usize]);
                match (kind, valid) {
                    (SdpKind::Offer, true) => conn.remote_offer = nonce,
                    (SdpKind::Answer, true) if nonce == Some(conn.nonce) => {
                        let Some(other) = other else { return };
                        let nonce = conn.nonce;
                        self.push(300.0, Ev::Connect { a: d, b: other, nonce });
                    }
                    // A description that is not the peer's: the handshake fails.
                    _ => self.push_io(d, 1.0, SyncIo::Rtc(RtcEvent::State { peer, state: 4 })),
                }
            }
            RtcReq::Answer { peer } => {
                if let Some(nonce) = self.conns.get(&(d, peer)).and_then(|c| c.remote_offer) {
                    self.push_io(d, gather, SyncIo::Rtc(RtcEvent::Local { peer, kind: SdpKind::Answer, sdp: format!("a:{inst}:{nonce}") }));
                }
            }
            RtcReq::Close { peer } => {
                let was_open = self.conns.remove(&(d, peer)).is_some_and(|c| c.open);
                let other = self.dev_of(peer, &self.devs[d].doc);
                if let (true, Some(other)) = (was_open, other) {
                    if self.conns.remove(&(other, inst)).is_some() {
                        for ch in [Channel::Ctl, Channel::Bulk] {
                            self.push_io(other, self.link_ms, SyncIo::Rtc(RtcEvent::ChannelClosed { peer: inst, ch }));
                        }
                        self.push_io(other, self.link_ms, SyncIo::Rtc(RtcEvent::State { peer: inst, state: 5 }));
                    }
                }
            }
        }
    }

    fn event(&mut self, ev: Ev) {
        match ev {
            Ev::Io { dev, boot, io } => {
                if self.devs[dev].boot == boot {
                    self.io(dev, io);
                }
            }
            Ev::Tick { dev, boot, timer } => {
                if self.devs[dev].boot == boot && self.devs[dev].timer == timer {
                    self.tick(dev);
                }
            }
            Ev::Arrive { dev, boot, id, req } => {
                let account = self.devs[dev].account.clone();
                let (status, etag, body) = self.drive.request(dev, &account, &req, self.now);
                let down = self.latency(&req);
                self.push(down, Ev::Io { dev, boot, io: SyncIo::Http { id, status, etag, body } });
            }
            Ev::Connect { a, b, nonce } => {
                let (ia, ib) = (self.devs[a].inst, self.devs[b].inst);
                let blocked = self.all_blocked || self.blocked.contains(&(ia.min(ib), ia.max(ib))) || !self.devs[a].online || !self.devs[b].online;
                let matching = self.conns.get(&(a, ib)).is_some_and(|c| c.nonce == nonce) && self.conns.get(&(b, ia)).is_some_and(|c| c.remote_offer == Some(nonce));
                if blocked || !matching {
                    return;
                }
                for (dev, peer) in [(a, ib), (b, ia)] {
                    self.conns.get_mut(&(dev, peer)).unwrap().open = true;
                    self.push_io(dev, 0.0, SyncIo::Rtc(RtcEvent::State { peer, state: 2 }));
                    for ch in [Channel::Ctl, Channel::Bulk] {
                        self.push_io(dev, 0.0, SyncIo::Rtc(RtcEvent::ChannelOpen { peer, ch }));
                    }
                }
            }
        }
    }

    /// Runs every event up to `now + ms`.
    pub fn run(&mut self, ms: f64) {
        let end = self.now + ms;
        while let Some(entry) = self.queue.first_entry() {
            let at = entry.key().0 as f64 / 1000.0;
            if at > end {
                break;
            }
            let ev = entry.remove();
            self.now = self.now.max(at);
            self.event(ev);
        }
        self.now = end;
    }
    /// Runs in 10 ms steps until `done` holds; `false` if it does not within `max_ms`.
    pub fn run_until(&mut self, max_ms: f64, mut done: impl FnMut(&mut Sim) -> bool) -> bool {
        let end = self.now + max_ms;
        while !done(self) {
            if self.now >= end {
                return false;
            }
            self.run(10.0);
        }
        true
    }
}

/// `"ihn6_" + inst13`.
pub fn record_key(inst: u64) -> String { format!("ihn6_{}", inst13(inst)) }

// ------------------------------------------------------------------------------------------------ self-tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::{Link, PRESENCE_MS};
    use ihn_core::doc::PeerLink;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn hex(bytes: &[u8]) -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() }

    #[test]
    fn sha256_and_deflate_helpers() {
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hex(&sha256(&[b'a'; 1000])), "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3");
        let mut rng = Rng::new(5);
        for len in [0usize, 1, 0xffff, 0x10000, 200_000] {
            let data: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
            let packed = deflate(&data);
            assert_eq!(inflate(&packed).as_deref(), Some(&data[..]));
            assert!(inflate(&packed[..packed.len() - 1]).is_none());
        }
        assert!(inflate(&[4, 0, 0, 0xff, 0xff]).is_none());
    }

    const ANN: &str = "ann@example.org";
    const BOB: &str = "bob@example.org";
    const EVE: &str = "eve@example.org";

    fn props(file: &str, set: &[(&str, Option<&str>)]) -> HttpReq {
        HttpReq::Props { file: file.into(), set: set.iter().map(|(k, v)| (k.to_string(), v.map(str::to_string))).collect(), app: Vec::new(), keepalive: false }
    }
    fn meta(drive: &mut Drive, account: &str, file: &str) -> (DriveMeta, String) {
        let (status, etag, body) = drive.request(0, account, &HttpReq::Meta { file: file.into() }, T0);
        assert_eq!(status, 200);
        (DriveMeta::decode(&body).unwrap(), etag)
    }

    #[test]
    fn fake_drive_properties_etags_and_conditional_puts() {
        let mut drive = Drive::default();
        let doc = drive.create_doc(ANN, "Notes");
        drive.share(&doc, BOB, true);
        drive.share(&doc, EVE, false);
        let (m0, e0) = meta(&mut drive, ANN, &doc);
        assert!(m0.can_edit && m0.permission_ids.len() == 3 && m0.name == "Notes");
        // A record of the longest shape fits; one byte more does not; nothing of a refused write is applied.
        let key = record_key(u64::MAX & !0xff);
        let value = "v".repeat(PROP_BYTES - key.len());
        assert_eq!(drive.request(0, ANN, &props(&doc, &[(&key, Some(&value))]), T0).0, 200);
        let long = format!("{value}x");
        assert_eq!(drive.request(0, ANN, &props(&doc, &[("other", Some("1")), (&key, Some(&long))]), T0).0, 400);
        // A metadata write changes the etag and the version, not the head revision.
        let (m1, e1) = meta(&mut drive, BOB, &doc);
        assert_eq!((m1.props.len(), &m1.head, m1.version), (1, &m0.head, m0.version + 1));
        assert_ne!(e0, e1);
        // 30 public properties at most.
        let many: Vec<(String, Option<&str>)> = (0..PROP_KEYS).map(|i| (format!("k{i}"), Some("1"))).collect();
        let many: Vec<(&str, Option<&str>)> = many.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        assert_eq!(drive.request(0, ANN, &props(&doc, &many), T0).0, 400);
        assert_eq!(drive.request(0, ANN, &props(&doc, &many[1..]), T0).0, 200);
        assert_eq!(drive.request(0, ANN, &props(&doc, &[("k1", None), ("k2", None)]), T0).0, 200);
        // Readers cannot write properties (edit proof) and get no permissionIds.
        let (status, _, body) = drive.request(0, EVE, &props(&doc, &[("k", Some("v"))]), T0);
        assert_eq!((status, &body[..]), (403, &b"insufficientFilePermissions"[..]));
        let (me, _) = meta(&mut drive, EVE, &doc);
        assert!(!me.can_edit && me.permission_ids.is_empty() && me.props.len() == PROP_KEYS - 2);
        // Conditional PUT: a stale etag is refused; churn (same head, new etag) succeeds with the fresh etag.
        let put = |etag: &str| HttpReq::Put { file: doc.clone(), if_match: etag.into(), bytes: b"%PDF-2".to_vec(), app: vec![("ihn6_vv".into(), "x".into())] };
        let (m2, e2) = meta(&mut drive, BOB, &doc);
        assert_eq!(drive.request(0, ANN, &props(&doc, &[("k3", None)]), T0).0, 200);
        assert_eq!(drive.request(0, BOB, &put(&e2), T0).0, 412);
        let (m3, e3) = meta(&mut drive, BOB, &doc);
        assert_eq!(m3.head, m2.head);
        let (status, e4, head) = drive.request(0, BOB, &put(&e3), T0 + 5.0);
        assert_eq!(status, 200);
        let (m4, e5) = meta(&mut drive, ANN, &doc);
        assert_eq!((m4.head.as_bytes(), &e4, m4.modified_ms), (&head[..], &e5, T0 as u64 + 5));
        assert_ne!(m4.head, m3.head);
        assert_eq!(m4.app, [("ihn6_vv".to_string(), "x".to_string())]);
        assert_eq!(drive.request(0, EVE, &put(&e5), T0).0, 403);
        // Removing a person changes permissionIds; a stranger gets 404.
        drive.unshare(&doc, EVE);
        assert_eq!(meta(&mut drive, ANN, &doc).0.permission_ids.len(), 2);
        assert_eq!(drive.request(0, EVE, &HttpReq::Meta { file: doc.clone() }, T0).0, 404);
        assert_eq!(drive.count(0, "props", 0.0, 1.0), 7);
    }

    #[test]
    fn fake_drive_sync_folder_files_and_failures() {
        let mut drive = Drive::default();
        let doc = drive.create_doc(ANN, "Notes");
        let (_, _, ids) = drive.request(0, ANN, &HttpReq::GenerateIds { count: 3 }, T0);
        let ids: Vec<String> = String::from_utf8(ids).unwrap().lines().map(str::to_string).collect();
        assert_eq!(ids.len(), 3);
        let folder = drive.request(0, ANN, &HttpReq::SyncFolder, T0).2;
        assert_eq!(folder, drive.request(0, ANN, &HttpReq::SyncFolder, T0).2);
        let create = |id: &str| HttpReq::Create { file: id.into(), name: "ihn6-x.bin".into(), app: vec![("ihn6Kind".into(), "relay".into())], bytes: vec![1, 2], keepalive: true };
        assert_eq!(drive.request(0, ANN, &create("made-up"), T0).0, 400);
        assert_eq!(drive.request(0, ANN, &create(&ids[0]), T0).0, 200);
        // A retry after a lost response finds the file: 409 ⇒ continue with an update.
        assert_eq!(drive.request(0, ANN, &create(&ids[0]), T0).0, 409);
        let update = HttpReq::Update { file: ids[0].clone(), bytes: vec![3], keepalive: false };
        assert_eq!(drive.request(0, ANN, &update, T0).0, 200);
        // Link-readable: any account that knows the id reads it; only the owner writes or deletes.
        let media = HttpReq::Media { file: ids[0].clone() };
        assert_eq!(drive.request(1, EVE, &media, T0), (200, "\"e2\"".into(), vec![3]));
        assert_eq!(drive.request(1, EVE, &update, T0).0, 403);
        assert_eq!(drive.request(1, EVE, &HttpReq::Delete { file: ids[0].clone() }, T0).0, 403);
        assert_eq!(drive.request(0, ANN, &HttpReq::Delete { file: ids[0].clone() }, T0).0, 204);
        assert_eq!(drive.request(1, EVE, &media, T0).0, 404);
        // A policy that refuses link sharing: flag in the folder answer, files private.
        drive.no_link.insert(BOB.into());
        assert!(drive.request(2, BOB, &HttpReq::SyncFolder, T0).2.ends_with(b"\n1"));
        let (_, _, id) = drive.request(2, BOB, &HttpReq::GenerateIds { count: 1 }, T0);
        let id = String::from_utf8(id).unwrap();
        assert_eq!(drive.request(2, BOB, &create(&id), T0).0, 200);
        assert_eq!(drive.request(1, EVE, &HttpReq::Media { file: id.clone() }, T0).0, 404);
        // A copy carries the public properties but not the permissions.
        drive.share(&doc, BOB, true);
        assert_eq!(drive.request(0, ANN, &props(&doc, &[("ihn6_x", Some("1|1|a|f|AAAAAAAAAAAAAAAA|"))]), T0).0, 200);
        let copy = drive.copy(&doc, EVE);
        assert_eq!(meta(&mut drive, EVE, &copy).0.props.len(), 1);
        assert_eq!(drive.request(2, BOB, &HttpReq::Meta { file: copy.clone() }, T0).0, 404);
        // Injection, lost responses and the rate limit.
        drive.inject(|r| matches!(r, HttpReq::Props { .. }), 403, "userRateLimitExceeded", 2);
        for expect in [403, 403, 200] {
            assert_eq!(drive.request(0, ANN, &props(&doc, &[("a", Some("1"))]), T0 + 10.0).0, expect);
        }
        drive.injected.push(Inject { when: |r| matches!(r, HttpReq::Props { .. }), status: 0, body: "", skip: 1, times: 1, lost: true });
        assert_eq!(drive.request(0, ANN, &props(&doc, &[("b", Some("1"))]), T0 + 20.0).0, 200);
        assert_eq!(drive.request(0, ANN, &props(&doc, &[("c", Some("1"))]), T0 + 30.0).0, 0);
        assert!(drive.files[&doc].props.contains_key("c") && drive.injected.is_empty());
        drive.rate_limit = Some((2, 1000.0));
        let codes: Vec<u16> = (0..4).map(|i| drive.request(0, ANN, &props(&doc, &[("d", Some("1"))]), T0 + 2000.0 + i as f64 * 400.0).0).collect();
        assert_eq!(codes, [200, 200, 403, 200]);
        assert_eq!(drive.log.last().map(|l| (l.kind, l.status, l.at)), Some(("props", 200, 3200.0)));
    }

    // -- test parts ------------------------------------------------------------------------------------------

    type Trace = Rc<RefCell<Vec<String>>>;
    thread_local! {
        static TRACE: Trace = Trace::default();
    }
    fn trace() -> Vec<String> { TRACE.with(|t| std::mem::take(&mut *t.borrow_mut())) }
    fn note(cx: &Core, what: String) { TRACE.with(|t| t.borrow_mut().push(format!("{:x}:{what}", cx.inst >> 8))) }

    /// The smallest relay writer that makes devices see each other: it writes the own record when the session
    /// asks (entering company, every 60 s in company, leaving), respects the 2 s cap and vouches for every record
    /// it polls. The real one is `relay.rs` (W2.7).
    #[derive(Default)]
    struct Beacon {
        gen: u64,
        wrote_at: f64,
        writing: bool,
        left: bool,
    }

    impl Beacon {
        fn write(&mut self, cx: &mut Core, flags: &str, keepalive: bool) {
            self.gen += 1;
            (self.wrote_at, self.writing) = (cx.now, true);
            let value = format!("1|{}|{}|relay-{:x}|AAAAAAAAAAAAAAAA|{flags}", self.gen, self.gen, cx.inst);
            cx.http(HttpReq::Props { file: cx.doc_id.clone(), set: vec![(record_key(cx.inst), Some(value))], app: Vec::new(), keepalive });
        }
    }

    impl Part for Beacon {
        fn meta(&mut self, cx: &mut Core, _r: &mut dyn Replica) {
            let insts: Vec<u64> = cx.presence.remotes.keys().copied().collect();
            insts.into_iter().for_each(|i| cx.presence.verified(i));
        }
        fn http(&mut self, cx: &mut Core, _id: u32, status: u16, _etag: &str, _body: &[u8], _r: &mut dyn Replica) {
            self.writing = false;
            note(cx, format!("record {status}"));
        }
        fn change(&mut self, cx: &mut Core, c: Change, _r: &mut dyn Replica) {
            note(cx, format!("{c:?}"));
            if matches!(c, Change::Suspend | Change::Leave) && cx.networking() && cx.can_edit && !self.left && self.gen > 0 {
                self.left = true;
                self.write(cx, "l", true);
            }
            if c == Change::Visible {
                self.left = false;
            }
        }
        fn tick(&mut self, cx: &mut Core, _r: &mut dyn Replica) {
            if !cx.networking() || !cx.can_edit || !cx.visible || self.writing || self.left {
                return;
            }
            let due = match cx.cadence().refresh {
                _ if cx.record_due => cx.now,
                Some(every) if self.gen > 0 => self.wrote_at + every,
                _ => return,
            }
            .max(self.wrote_at + mode::WRITE_CAP_MS);
            if cx.now >= due {
                cx.record_due = false;
                self.write(cx, "c", false);
            } else {
                cx.wake(due);
            }
        }
    }

    fn beacon() -> Vec<Box<dyn Part>> { vec![Box::new(Beacon::default())] }
    fn none() -> Vec<Box<dyn Part>> { Vec::new() }

    /// Three editors of one document whose clocks are `skews` apart.
    fn world(skews: [f64; 3]) -> (Sim, String, [usize; 3]) {
        let mut sim = Sim::new(7);
        let doc = sim.drive.create_doc(ANN, "Notes");
        sim.drive.share(&doc, BOB, true);
        sim.drive.share(&doc, EVE, true);
        let a = sim.add_dev(ANN, 0x1000, skews[0], beacon);
        let b = sim.add_dev(BOB, 0x2000, skews[1], beacon);
        let c = sim.add_dev(EVE, 0x3000, skews[2], beacon);
        (sim, doc, [a, b, c])
    }

    fn mode_of(sim: &Sim, d: usize) -> Mode {
        match sim.phase(d) {
            Phase::Live(mode) => mode,
            other => panic!("device {d} is {other:?}"),
        }
    }

    /// The whole §9.2 story; returns the request log so runs with different skews can be compared.
    fn mode_story(skews: [f64; 3]) -> Vec<Logged> {
        let (mut sim, doc, [a, b, c]) = world(skews);
        trace();
        // Alone: first poll at once, then one GET per 20 s, no writes; 60 s cadence after 5 idle minutes.
        sim.open(a, &doc);
        assert_eq!(sim.phase(a), Phase::Loading { not_created: false });
        sim.run(1000.0);
        assert_eq!(mode_of(&sim, a), Mode::Alone);
        sim.io(a, SyncIo::Lifecycle(Lifecycle::Input));
        sim.run(59_000.0);
        assert_eq!(sim.drive.count(a, "meta", 0.0, 60_000.0), 3);
        sim.run(300_000.0);
        let t = sim.now - T0;
        sim.run(180_000.0);
        assert!((2..=3).contains(&sim.drive.count(a, "meta", t, t + 180_000.0)));
        assert!(sim.drive.log.iter().all(|l| l.kind == "meta"), "a lone idle device writes nothing");
        sim.io(a, SyncIo::Lifecycle(Lifecycle::Input));
        sim.run(500.0);
        // B opens: it polls at once, sees nobody (A wrote no record while alone and idle) and stays Alone until
        // it writes: the test marks its record due, which is what a relay flush after an edit amounts to.
        let joined = sim.now - T0;
        sim.open(b, &doc);
        sim.run(400.0);
        assert_eq!(mode_of(&sim, b), Mode::Alone);
        sim.sync(b).core.record_due = true;
        sim.tick(b);
        // A notices at its next poll (≤ 20 s), enters Company and writes its record at once; B sees it ≤ 20 s later.
        assert!(sim.run_until(21_000.0, |s| mode_of(s, a) == Mode::Company));
        assert!(sim.run_until(21_000.0, |s| mode_of(s, b) == Mode::Company && s.present(b, s.devs[a].inst)));
        assert!(sim.now - T0 - joined < 42_000.0);
        let (ia, ib, ic) = (sim.devs[a].inst, sim.devs[b].inst, sim.devs[c].inst);
        assert!(sim.vouched(a, ib));
        // Company: 500 ms polls, the record refreshed every 60 s, never two record writes within 2 s.
        let t = sim.now - T0;
        sim.run(120_000.0);
        for d in [a, b] {
            let polls = sim.drive.count(d, "meta", t, t + 120_000.0);
            assert!((130..=240).contains(&polls), "{polls} polls in 2 min of Company");
            assert_eq!(sim.drive.count(d, "props", t, t + 120_000.0), 2);
            assert_eq!(mode_of(&sim, d), Mode::Company);
        }
        // Everyone elects the same scribe: the smallest installation.
        assert!(sim.sync(a).core.is_scribe() && !sim.sync(b).core.is_scribe());
        assert_eq!(sim.sync(a).peers().iter().map(|v| (v.inst, v.link)).collect::<Vec<_>>(), [(ib, PeerLink::Relay)]);
        // Channel open to everyone present ⇒ Linked (10 s polls); an unreachable peer ⇒ Relay-only pair (1 s).
        sim.sync(a).core.presence.set_link(ib, Link::Open);
        sim.tick(a);
        assert_eq!(mode_of(&sim, a), Mode::Linked);
        assert_eq!(sim.sync(a).status().peers_connected, 1);
        let t = sim.now - T0;
        sim.run(60_000.0);
        assert!((5..=7).contains(&sim.drive.count(a, "meta", t, t + 60_000.0)));
        sim.sync(a).core.presence.set_link(ib, Link::Unreachable);
        sim.tick(a);
        assert_eq!(mode_of(&sim, a), Mode::RelayOnly);
        let t = sim.now - T0;
        sim.run(30_000.0);
        assert!((20..=31).contains(&sim.drive.count(a, "meta", t, t + 30_000.0)));
        sim.sync(a).core.presence.set_link(ib, Link::Idle);
        // A third editor joins a document in company: it sees both at its first poll.
        sim.open(c, &doc);
        assert!(sim.run_until(1000.0, |s| s.phase(c) == Phase::Live(Mode::Company)));
        assert_eq!(sim.core(c).presence.scribe(ic, sim.local(c)), ia);
        assert!(sim.run_until(5000.0, |s| s.present(a, ic) && s.present(b, ic)));
        // B leaves properly: record `l`, the others drop it at their next poll. C crashes: dropped after 120 s.
        sim.io(b, SyncIo::Lifecycle(Lifecycle::Leave));
        assert_eq!(sim.phase(b), Phase::Leaving);
        assert!(sim.run_until(2000.0, |s| s.phase(b) == Phase::Closed));
        assert!(sim.run_until(2000.0, |s| !s.present(a, ib)));
        sim.crash(c);
        let crashed = sim.now;
        assert_eq!(mode_of(&sim, a), Mode::Company);
        assert!(sim.run_until(PRESENCE_MS + 2000.0, |s| mode_of(s, a) == Mode::Alone));
        assert!(sim.now - crashed > PRESENCE_MS - 61_000.0, "C's last refresh was at most 60 s before the crash");
        assert!(sim.sync(a).core.is_scribe());
        // Back to the Alone cadence, and no more record writes.
        let t = sim.now - T0;
        sim.run(60_000.0);
        assert!((1..=3).contains(&sim.drive.count(a, "meta", t, t + 60_000.0)));
        assert_eq!(sim.drive.count(a, "props", t, t + 60_000.0), 0);
        // Records never exceed Drive's limits.
        assert!(sim.drive.log.iter().all(|l| l.status == 200));
        let file = &sim.drive.files[&doc];
        assert!(file.props.len() == 3 && file.props.iter().all(|(k, v)| k.len() + v.len() <= PROP_BYTES));
        sim.drive.log
    }

    #[test]
    fn modes_and_cadence_through_the_simulator() { mode_story([0.0; 3]); }

    #[test]
    fn clock_skew_of_ten_minutes_changes_nothing() {
        // Presence, vouching, modes, scribe and cadence use receiver-local time only: with the devices' clocks
        // 10 minutes apart in either direction the run is request-for-request the same.
        let base = mode_story([0.0; 3]);
        assert!(base.len() > 500);
        for skews in [[0.0, 600_000.0, -600_000.0], [-600_000.0, 0.0, 600_000.0], [600_000.0, -600_000.0, 0.0]] {
            let run = mode_story(skews);
            assert_eq!(run.len(), base.len(), "skews {skews:?}");
            for (x, y) in run.iter().zip(&base) {
                // Local times are sums of floats: allow a few microseconds, nothing else.
                assert!((x.at - y.at).abs() < 0.01 && Logged { at: 0.0, ..x.clone() } == Logged { at: 0.0, ..y.clone() }, "skews {skews:?}: {x:?} / {y:?}");
            }
        }
    }

    #[test]
    fn hidden_suspended_offline_and_back() {
        let (mut sim, doc, [a, b, _]) = world([0.0, 600_000.0, 0.0]);
        sim.open(a, &doc);
        sim.open(b, &doc);
        sim.run(1000.0);
        for d in [a, b] {
            sim.sync(d).core.record_due = true;
            sim.tick(d);
        }
        assert!(sim.run_until(21_000.0, |s| mode_of(s, a) == Mode::Company && mode_of(s, b) == Mode::Company));
        trace();
        // Hidden: no polls, peers kept (still leader); after 60 s: Suspended, record `l` (keepalive), lock released.
        sim.io(b, SyncIo::Lifecycle(Lifecycle::Hidden));
        assert_eq!(sim.phase(b), Phase::Background);
        assert_eq!(sim.sync(b).core.flush, Flush::Keepalive);
        sim.run(1000.0);
        let t = sim.now - T0;
        sim.run(58_000.0);
        assert_eq!((sim.phase(b), sim.sync(b).core.leader), (Phase::Background, true));
        sim.run(2000.0);
        assert_eq!((sim.phase(b), sim.sync(b).core.leader), (Phase::Suspended, false));
        assert_eq!(sim.drive.count(b, "meta", t, t + 600_000.0), 0);
        let last = sim.drive.log.iter().rev().find(|l| l.dev == b).unwrap();
        assert_eq!((last.kind, last.keepalive), ("props", true));
        assert!(sim.drive.files[&doc].props[&record_key(sim.devs[b].inst)].ends_with("|l"));
        assert!(sim.run_until(3000.0, |s| mode_of(s, a) == Mode::Alone));
        let seen = trace();
        assert!(seen.iter().any(|l| l == "20:Hidden") && seen.iter().any(|l| l == "20:Suspend") && seen.iter().any(|l| l == "20:Leader(false)"), "{seen:?}");
        // Hidden for hours: nothing at all.
        let t = sim.now - T0;
        sim.run(7_200_000.0);
        assert!(sim.drive.log.iter().all(|l| l.dev != b || l.at < t));
        // Visible: lock, poll now, live again.
        sim.io(b, SyncIo::Lifecycle(Lifecycle::Visible));
        assert!(sim.run_until(500.0, |s| s.sync(b).core.leader && matches!(s.phase(b), Phase::Live(_))));
        sim.run(400.0);
        assert_eq!(sim.drive.count(b, "meta", t, f64::MAX), 1);
        // Offline: no requests; online: catch-up poll at once.
        sim.set_online(b, false);
        assert_eq!(sim.phase(b), Phase::Offline);
        assert!(!sim.sync(b).status().online);
        let t = sim.now - T0;
        sim.run(100_000.0);
        assert_eq!(sim.drive.count(b, "meta", t, f64::MAX), 0);
        sim.set_online(b, true);
        assert_eq!(sim.phase(b), Phase::CatchingUp);
        assert!(sim.run_until(500.0, |s| matches!(s.phase(b), Phase::Live(_))));
        // A poll that fails backs off and recovers; 401 pauses.
        sim.drive.inject(|r| matches!(r, HttpReq::Meta { .. }), 500, "backendError", 3);
        sim.drive.inject(|r| matches!(r, HttpReq::Meta { .. }), 401, "authError", 1);
        let t = sim.now - T0;
        assert!(sim.run_until(25_000.0, |s| s.sync(a).core.poll_fails > 0 || s.sync(b).core.poll_fails > 0));
        assert!(sim.run_until(120_000.0, |s| s.drive.injected.is_empty()));
        sim.run(75_000.0); // the longest backoff: the idle Alone cadence (60 s) × 1.2
        assert!(sim.sync(a).core.poll_fails == 0 && sim.sync(b).core.poll_fails == 0 && !sim.sync(b).status().auth_paused);
        assert!(sim.drive.count(a, "meta", t, f64::MAX) + sim.drive.count(b, "meta", t, f64::MAX) >= 6);
    }

    #[test]
    fn viewers_demotion_trash_and_not_created() {
        let mut sim = Sim::new(3);
        let doc = sim.drive.create_doc(ANN, "Notes");
        sim.drive.share(&doc, BOB, true);
        sim.drive.share(&doc, EVE, false);
        let (a, b, v) = (sim.add_dev(ANN, 0x1000, 0.0, beacon), sim.add_dev(BOB, 0x2000, 0.0, beacon), sim.add_dev(EVE, 0x3000, 0.0, beacon));
        for d in [a, b, v] {
            sim.open(d, &doc);
        }
        sim.run(1000.0);
        for d in [a, b] {
            sim.sync(d).core.record_due = true;
            sim.tick(d);
        }
        // The viewer polls at 1 s while editor records change, sees the editors, and never writes.
        assert_eq!(sim.phase(v), Phase::Live(Mode::Viewer));
        sim.run(25_000.0);
        let t = sim.now - T0;
        sim.run(60_000.0);
        assert!((40..=60).contains(&sim.drive.count(v, "meta", t, f64::MAX)));
        assert_eq!(sim.sync(v).peers().len(), 2);
        // Editors do not count the viewer (it has no record): they are in company with each other only.
        assert_eq!(sim.sync(a).peers().len(), 1);
        // No editor record changes for 30 min ⇒ 20 s polls (the 5 s tier lies between).
        sim.crash(a);
        sim.crash(b);
        sim.run(1_800_000.0);
        let t = sim.now - T0;
        sim.run(60_000.0);
        assert!((2..=3).contains(&sim.drive.count(v, "meta", t, f64::MAX)));
        assert!(sim.drive.log.iter().all(|l| l.dev != v || l.kind == "meta"));
        // Demotion: the poll says canEdit=false ⇒ Viewer; rights back ⇒ editor again.
        sim.open(b, &doc);
        sim.run(500.0);
        trace();
        sim.drive.share(&doc, BOB, false);
        assert!(sim.run_until(21_000.0, |s| s.phase(b) == Phase::Live(Mode::Viewer)));
        assert!(trace().contains(&"20:CanEdit(false)".to_string()));
        sim.drive.share(&doc, BOB, true);
        assert!(sim.run_until(21_000.0, |s| matches!(s.phase(b), Phase::Live(m) if m != Mode::Viewer)));
        // No access at all ⇒ 404 ⇒ Closed with the error; trashed ⇒ Closed.
        sim.drive.unshare(&doc, EVE);
        assert!(sim.run_until(21_000.0, |s| s.phase(v) == Phase::Closed));
        assert_eq!(sim.sync(v).status().last_error.map(|e| e.code), Some("404".into()));
        sim.drive.files.get_mut(&doc).unwrap().trashed = true;
        assert!(sim.run_until(61_000.0, |s| s.phase(b) == Phase::Closed));
        let t = sim.now - T0;
        sim.run(100_000.0);
        assert!(sim.drive.log.iter().all(|l| l.at < t), "closed sessions are silent");
        // A document created offline: no polls until its Drive file exists.
        let n = sim.add_dev(ANN, 0x4000, 0.0, none);
        sim.devs[n].stored.created = false;
        sim.open(n, "reserved-id");
        sim.run(60_000.0);
        assert_eq!(sim.phase(n), Phase::Loading { not_created: true });
        assert!(sim.sync(n).status().not_created && sim.drive.count(n, "meta", 0.0, f64::MAX) == 0);
        sim.edit(n);
        assert_eq!(sim.sync(n).status().unsealed_local, 1);
        sim.drive.files.insert("reserved-id".into(), DFile { owner: ANN.into(), head: 1, ..Default::default() });
        sim.io(n, SyncIo::Lifecycle(Lifecycle::Created));
        assert!(sim.run_until(500.0, |s| s.phase(n) == Phase::Live(Mode::Alone)));
        assert!(sim.devs[n].stored.created);
    }

    /// Records what the session routes to a part, and drives the fake channels.
    #[derive(Default)]
    struct Probe {
        ids: Vec<u32>,
    }

    impl Part for Probe {
        fn local_batch(&mut self, cx: &mut Core, b: &EncodedBatch, _r: &mut dyn Replica) {
            note(cx, format!("batch {}", b.header().unwrap().1));
            cx.send_all(&Frame::Batches(vec![b.clone()]));
        }
        fn live(&mut self, cx: &mut Core, p: &LivePreview) {
            if let LivePreview::Cancel { live } = p {
                cx.send_all(&Frame::InkCancel { live: *live });
            }
        }
        fn presence(&mut self, cx: &mut Core, p: &PresenceState) { cx.send(Dest::Tabs, &Frame::Presence(p.clone())) }
        fn http(&mut self, cx: &mut Core, id: u32, status: u16, _etag: &str, body: &[u8], _r: &mut dyn Replica) { note(cx, format!("http {} {status} {}", self.ids.contains(&id), body.len())) }
        fn digest(&mut self, cx: &mut Core, id: u32, out: &[u8], _r: &mut dyn Replica) { note(cx, format!("digest {} {}", self.ids.contains(&id), hex(&out[..4]))) }
        fn deflate(&mut self, cx: &mut Core, id: u32, out: &[u8], _r: &mut dyn Replica) { note(cx, format!("deflate {} {}", self.ids.contains(&id), out.len())) }
        fn rtc(&mut self, cx: &mut Core, ev: &RtcEvent, _r: &mut dyn Replica) {
            match ev {
                RtcEvent::Local { peer, kind, sdp } => note(cx, format!("local {:x} {kind:?} {sdp}", peer >> 8)),
                RtcEvent::ChannelOpen { peer, ch: Channel::Bulk } => cx.presence.set_link(*peer, Link::Open),
                RtcEvent::ChannelClosed { peer, .. } | RtcEvent::Fault { peer } | RtcEvent::State { peer, state: 4 | 5 } => {
                    cx.presence.set_link(*peer, Link::Idle);
                    note(cx, format!("down {:x} {}", peer >> 8, matches!(ev, RtcEvent::Fault { .. })));
                }
                _ => {}
            }
        }
        fn change(&mut self, cx: &mut Core, c: Change, _r: &mut dyn Replica) { note(cx, format!("{c:?}")) }
        fn frame(&mut self, cx: &mut Core, from: PeerRef, f: &Frame, r: &mut dyn Replica) {
            match f {
                Frame::Batches(b) => {
                    let res = r.receive(b, from);
                    note(cx, format!("applied {} from {from:?}", res.applied));
                }
                Frame::Snapshot(s) => note(cx, format!("snapshot {} from {from:?}", s.0.len())),
                other => note(cx, format!("frame {:#04x} from {from:?}", other.encode()[0])),
            }
        }
    }
    fn probe() -> Vec<Box<dyn Part>> { vec![Box::new(Probe::default()), Box::new(Probe::default())] }
    fn probe1() -> Vec<Box<dyn Part>> { vec![Box::new(Probe::default())] }

    #[test]
    fn requests_are_routed_to_the_part_that_asked() {
        let mut sim = Sim::new(4);
        let doc = sim.drive.create_doc(ANN, "Notes");
        let a = sim.add_dev(ANN, 0x1000, 0.0, probe);
        sim.open(a, &doc);
        sim.run(500.0);
        trace();
        // Ask on behalf of the second part only (the session sets the owner around each call; here by hand).
        let core = &mut sim.sync(a).core;
        core.owner = 1;
        let ids = [core.http(HttpReq::Media { file: doc.clone() }), core.digest(b"abc".to_vec()), core.deflate(false, vec![7; 10]), core.deflate(true, vec![9])];
        core.owner = SELF;
        core.persist(SyncRecord::RelayPlan { gen: 4, file_id: "f".into(), epoch_day: 9 });
        assert_eq!(sim.sync(a).core.stored.relay_gen, 4);
        sim.sync(a).parts[1] = Box::new(Probe { ids: ids.to_vec() });
        sim.tick(a);
        sim.run(1000.0);
        assert_eq!(trace(), ["10:digest true ba7816bf", "10:deflate true 15", "10:deflate true 0", "10:http true 200 5"]);
        assert_eq!((sim.devs[a].stored.relay_gen, sim.devs[a].stored.relay_file.as_str()), (4, "f"));
        // An answer nobody waits for (the session was restarted) is ignored.
        sim.io(a, SyncIo::Http { id: 999, status: 200, etag: String::new(), body: Vec::new() });
        sim.io(a, SyncIo::Digest { id: 998, out: Vec::new() });
        assert!(trace().is_empty());
    }

    fn connect(sim: &mut Sim, a: usize, b: usize) {
        let (ia, ib) = (sim.devs[a].inst, sim.devs[b].inst);
        sim.sync(a).core.rtc(RtcReq::Create { peer: ib, ice: Vec::new(), offer: true });
        sim.tick(a);
        sim.run(700.0);
        let offer = trace().into_iter().find_map(|l| l.split("Offer ").nth(1).map(str::to_string)).expect("offer");
        sim.sync(b).core.rtc(RtcReq::Create { peer: ia, ice: Vec::new(), offer: false });
        sim.sync(b).core.rtc(RtcReq::SetRemote { peer: ia, kind: SdpKind::Offer, sdp: offer });
        sim.sync(b).core.rtc(RtcReq::Answer { peer: ia });
        sim.tick(b);
        sim.run(700.0);
        let answer = trace().into_iter().find_map(|l| l.split("Answer ").nth(1).map(str::to_string)).expect("answer");
        sim.sync(a).core.rtc(RtcReq::SetRemote { peer: ib, kind: SdpKind::Answer, sdp: answer });
        sim.tick(a);
        sim.run(400.0);
    }

    #[test]
    fn fake_channels_tabs_and_locks() {
        let mut sim = Sim::new(5);
        let doc = sim.drive.create_doc(ANN, "Notes");
        sim.drive.share(&doc, BOB, true);
        let (a, b) = (sim.add_dev(ANN, 0x1000, -600_000.0, probe1), sim.add_dev(BOB, 0x2000, 600_000.0, probe1));
        let a2 = sim.add_sibling(a);
        for d in [a, b, a2] {
            sim.open(d, &doc);
        }
        sim.run(500.0);
        // One tab per browser and document holds `ihn6-net`; only it polls.
        assert_eq!([a, b, a2].map(|d| sim.sync(d).core.leader), [true, true, false]);
        assert_eq!([a, b, a2].map(|d| sim.drive.count(d, "meta", 0.0, f64::MAX)), [1, 1, 0]);
        assert_eq!(sim.phase(a2), Phase::Loading { not_created: false });
        trace();
        let (ia, ib) = (sim.devs[a].inst, sim.devs[b].inst);
        connect(&mut sim, a, b);
        assert_eq!((sim.sync(a).core.open_peers(), sim.sync(b).core.open_peers()), (vec![ib], vec![ia]));
        assert_eq!((sim.phase(a), trace().last().map(String::as_str)), (Phase::Live(Mode::Linked), Some("20:Mode(Alone)")));
        // A batch travels to the peer (one link delay) and to the other tab; duplicates are not applied twice.
        let t = sim.now;
        sim.edit(a);
        assert!(sim.run_until(100.0, |s| s.converged(&[a, b, a2])));
        assert!((sim.link_ms..sim.link_ms + 11.0).contains(&(sim.now - t)));
        assert_eq!(trace(), ["10:batch 1", "10:applied 1 from Tab", format!("20:applied 1 from Peer({ia})").as_str()]);
        assert_eq!(sim.devs[b].replica.received, [PeerRef::Peer(ia)]);
        sim.io(b, SyncIo::Tab(Frame::Batches(vec![sim.devs[a].replica.journal.values().next().unwrap().clone()]).encode()));
        assert_eq!(trace(), ["20:applied 0 from Tab"]);
        // A frame above 60 KiB goes over the bulk channel as Parts and arrives whole; order is kept.
        let big = Frame::Snapshot(EncodedSnapshot(vec![5; 3 * wire::PART_DATA + 100]));
        sim.sync(b).core.send(Dest::Peer(ia), &big);
        sim.sync(b).core.send(Dest::Peer(ia), &Frame::Ping(1));
        sim.tick(b);
        let sent: Vec<(Channel, usize)> = sim.devs[b].effects.iter().filter_map(|e| if let SyncEffect::Send { ch, bytes, .. } = e { Some((*ch, bytes.len())) } else { None }).collect();
        let sent = &sent[sent.len() - 5..];
        assert_eq!(sent.iter().map(|s| s.0).collect::<Vec<_>>(), [Channel::Bulk, Channel::Bulk, Channel::Bulk, Channel::Bulk, Channel::Ctl]);
        assert!(sent.iter().all(|s| s.1 <= wire::MAX_MESSAGE));
        sim.run(50.0);
        let got = trace();
        assert_eq!(got[..2], [format!("10:snapshot {} from Peer({ib})", 3 * wire::PART_DATA + 100), format!("10:frame 0x20 from Peer({ib})")]);
        // Live ink and presence reach the tabs; a tab message that does not decode is dropped.
        sim.sync(a).live(LivePreview::Cancel { live: 9 });
        sim.sync(a).presence(PresenceState { mode: ihn_core::doc::PresenceMode::Editing, page: ihn_core::id::Id(1), tool_colour: Color::Hex(1), save_phase: 2 });
        sim.tick(a);
        sim.io(a2, SyncIo::Tab(vec![0xff, 1, 2]));
        sim.run(50.0);
        let got = trace();
        assert!(got.contains(&"10:frame 0x13 from Tab".to_string()) && got.contains(&"10:frame 0x18 from Tab".to_string()) && got.contains(&format!("20:frame 0x13 from Peer({ia})")), "{got:?}");
        assert!(sim.sync(a).core.own_presence.is_some());
        // Garbage from a peer closes the channel on both sides (§10.5).
        sim.io(a, SyncIo::Rtc(RtcEvent::Message { peer: ib, ch: Channel::Ctl, bytes: vec![0x03, 0xff] }));
        sim.run(50.0);
        let got = trace();
        assert!(got.contains(&"10:down 20 true".to_string()) && got.contains(&"20:down 10 false".to_string()), "{got:?}");
        assert!(sim.sync(a).core.open_peers().is_empty() && sim.sync(b).core.open_peers().is_empty());
        sim.edit(b);
        sim.run(100.0);
        assert!(!sim.converged(&[a, b]), "a closed channel carries nothing");
        // Blocked P2P: the handshake completes but no channel opens. An answer to another offer fails.
        sim.block_p2p(a, b);
        connect(&mut sim, a, b);
        assert!(sim.sync(a).core.open_peers().is_empty());
        sim.sync(a).core.rtc(RtcReq::SetRemote { peer: ib, kind: SdpKind::Answer, sdp: format!("a:{ib}:12345") });
        sim.tick(a);
        sim.run(10.0);
        assert!(trace().contains(&"10:down 20 false".to_string()));
        // The lock moves to the other tab when its holder dies, and that tab starts polling.
        sim.crash(a);
        assert!(sim.run_until(500.0, |s| s.sync(a2).core.leader && matches!(s.phase(a2), Phase::Live(_))));
        assert!(trace().contains(&"10:Leader(true)".to_string()));
    }

    #[test]
    fn crash_points_and_restart() {
        let mut sim = Sim::new(6);
        let doc = sim.drive.create_doc(ANN, "Notes");
        let a = sim.add_dev(ANN, 0x1000, 0.0, probe);
        sim.open(a, &doc);
        sim.run(500.0);
        sim.edit(a);
        sim.edit(a);
        // Crash before an effect is executed: nothing of it happens.
        sim.crash_on(a, |e| matches!(e, SyncEffect::Persist(_)), false);
        sim.sync(a).core.persist(SyncRecord::RelayPlan { gen: 1, file_id: "f1".into(), epoch_day: 1 });
        sim.tick(a);
        assert!(sim.devs[a].sync.is_none() && sim.devs[a].stored.relay_gen == 0);
        // Restart: the journal and the persisted state are back; local batches are still unsealed.
        sim.open(a, &doc);
        assert_eq!((sim.sync(a).status().unsealed_local, sim.devs[a].replica.journal.len()), (2, 2));
        // Crash right after the plan was persisted and the upload left: the request still reaches Drive.
        sim.crash_on(a, |e| matches!(e, SyncEffect::Http { req: HttpReq::Props { .. }, .. }), true);
        let core = &mut sim.sync(a).core;
        core.persist(SyncRecord::RelayPlan { gen: 1, file_id: "f1".into(), epoch_day: 1 });
        core.http(props(&doc, &[("k", Some("v"))]));
        core.persist(SyncRecord::RelaySealed { sealed: vec![(core.actor, 2)], archives: Vec::new(), assets: Vec::new(), perm: 0 });
        sim.tick(a);
        assert!(sim.devs[a].sync.is_none());
        sim.run(1000.0);
        assert_eq!(sim.drive.files[&doc].props.get("k").map(String::as_str), Some("v"));
        assert_eq!((sim.devs[a].stored.relay_gen, sim.devs[a].stored.sealed.len()), (1, 0));
        // After the restart the answer of the old incarnation is not delivered; the seal persists this time.
        sim.open(a, &doc);
        trace();
        let core = &mut sim.sync(a).core;
        assert_eq!(core.stored.relay_gen, 1);
        core.persist(SyncRecord::RelaySealed { sealed: vec![(core.actor, 2)], archives: Vec::new(), assets: Vec::new(), perm: 0 });
        sim.tick(a);
        sim.run(1000.0);
        assert!(trace().iter().all(|l| !l.contains("http")));
        let status = sim.sync(a).status();
        assert_eq!((status.unsealed_local, status.relay_durable_through.get(sim.devs[a].replica.author.actor)), (0, 2));
        // Journal retention: nothing is covered before an export is known; then seal ∧ export ∧ peers.
        let actor = sim.devs[a].replica.author.actor;
        sim.run(11_000.0);
        assert_eq!(sim.devs[a].replica.journal.len(), 2);
        let mut vv = VersionVector::default();
        vv.raise(actor, 1);
        sim.sync(a).core.persist(SyncRecord::KnownExport { head: "rev-2".into(), vv: vv.clone() });
        let all = sim.devs[a].replica.doc.vv.clone();
        sim.sync(a).core.peer_vv.insert(PeerRef::Tab, all);
        sim.run(21_000.0);
        assert_eq!(sim.devs[a].replica.journal.keys().collect::<Vec<_>>(), [&(actor, 2)]);
        assert_eq!(sim.devs[a].replica.batches_since(&VersionVector::default(), 1 << 20), Err(Compacted));
        assert_eq!(sim.devs[a].replica.batches_since(&vv, 1 << 20).map(|b| b.len()), Ok(1));
    }
}
