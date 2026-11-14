//! `ihn-sync` — pure sync protocol state machines (SYNC.md §18.1). No I/O; depends on `ihn-core` only.
//!
//! Owner of this file: W2.6. It holds the session machine (SYNC §16.1), the effect/input types (§18.2) and the
//! shared state ([`Core`]) the protocol parts work on. A part ([`Part`]) is one of `relay`, `export`, `peer`,
//! `entropy`, `live`: it gets every input the session routes to it and answers with effects through `Core`.
//! Parts are listed in [`parts`]; the session itself owns the poll (§8.6), presence and modes (§9), the phase,
//! the `ihn6-net` lock, request routing, frame decoding and `Part` reassembly.

pub mod entropy;
pub mod export;
pub mod live;
pub mod mode;
pub mod peer;
pub mod relay;
#[cfg(test)]
pub mod sim;
pub mod wire;

use ihn_core::codec::{Reader, Writer};
use ihn_core::doc::{LivePreview, PeerView, PresenceState, Replica, SyncEngine, SyncError, SyncStatus};
use ihn_core::id::{migration_source, ActorId, DocId, Hash};
use ihn_core::snapshot::{get_str, put_str, EncodedBatch};
use ihn_core::stamp::{Stamp, VersionVector};
use mode::{Cadence, Mode, Presence, HIDDEN_SUSPEND_MS};
use std::collections::{BTreeMap, HashMap};
use wire::{Assembled, Frame, Mail, Profile, Reassembly, PART_DATA};

pub use ihn_core::doc::PeerRef;

/// Routes `sync.<module>.<name>` test commands to `<module>::test_cmd` (called by `ihn-app::testapi`).
pub fn test_cmd(module: &str, name: &str, args: &str) -> Option<String> {
    let f: fn(&str, &str) -> Option<String> = match module {
        "entropy" => entropy::test_cmd,
        "export" => export::test_cmd,
        "live" => live::test_cmd,
        "mode" => mode::test_cmd,
        "peer" => peer::test_cmd,
        "relay" => relay::test_cmd,
        "wire" => wire::test_cmd,
        _ => return None,
    };
    f(name, args)
}

/// The protocol parts of a session, in dispatch order. Each module answers with its part from `pub fn part()`
/// (`None` until its unit has built it), so W2.7 (`relay`, `export`) and W2.8 (`peer`, `entropy`, `live`) only edit
/// their own files.
fn parts() -> Vec<Box<dyn Part>> {
    [relay::part(), export::part(), peer::part(), entropy::part(), live::part()].into_iter().flatten().collect()
}

// ------------------------------------------------------------------------------------------------ effects and inputs

/// A Drive request, executed by `ihn-app::drive` (auth, deadlines, 401 retry, JSON). Sync never sees JSON: the
/// executor answers [`HttpReq::Meta`] with an encoded [`DriveMeta`] and everything else with raw bytes.
#[derive(Clone, PartialEq, Debug)]
pub enum HttpReq {
    /// `GET files/{file}?fields=properties,appProperties,headRevisionId,version,modifiedTime,name,trashed,
    /// capabilities(canEdit),permissionIds`, `Cache-Control: no-cache` (§8.6 poll, §14.2 CheckHead). The answer's
    /// `etag` is the v2 file etag.
    Meta { file: String },
    /// `GET files/{file}?alt=media`.
    Media { file: String },
    /// Multipart create with a reserved id in the sync folder (§8.3), `application/octet-stream`.
    Create { file: String, name: String, app: Vec<(String, String)>, bytes: Vec<u8>, keepalive: bool },
    /// Media `PATCH` of a relay file (single writer, no `If-Match`).
    Update { file: String, bytes: Vec<u8>, keepalive: bool },
    /// `PATCH files/{file}?fields=id` merging `properties` / `appProperties`; `None` deletes a key (§8.4).
    Props { file: String, set: Vec<(String, Option<String>)>, app: Vec<(String, Option<String>)>, keepalive: bool },
    /// The export upload: media `PUT` with `If-Match` (§14.2); 412 when the etag moved.
    Put { file: String, if_match: String, bytes: Vec<u8>, app: Vec<(String, String)> },
    Delete { file: String },
    /// `files/generateIds`; the answer is the ids, one per line.
    GenerateIds { count: u32 },
    /// Find or create the account's sync folder and share it by link (§8.2). The answer is the folder id followed
    /// by `\n1` when link sharing was refused.
    SyncFolder,
}

/// `HttpReq::Meta` answer, re-encoded from Drive's JSON by the executor.
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct DriveMeta {
    pub props: Vec<(String, String)>,
    pub app: Vec<(String, String)>,
    pub head: String,
    pub version: u64,
    pub modified_ms: u64,
    pub name: String,
    pub trashed: bool,
    pub can_edit: bool,
    pub permission_ids: Vec<String>,
}

impl DriveMeta {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        for map in [&self.props, &self.app] {
            w.leb(map.len() as u64);
            for (k, v) in map {
                put_str(&mut w, k);
                put_str(&mut w, v);
            }
        }
        put_str(&mut w, &self.head);
        w.leb(self.version);
        w.leb(self.modified_ms);
        put_str(&mut w, &self.name);
        w.u8(self.trashed as u8 | (self.can_edit as u8) << 1);
        w.leb(self.permission_ids.len() as u64);
        self.permission_ids.iter().for_each(|p| put_str(&mut w, p));
        w.buf
    }
    pub fn decode(bytes: &[u8]) -> Option<DriveMeta> {
        let r = &mut Reader::new(bytes);
        let text = |r: &mut Reader| get_str(r).map(str::to_string);
        let mut maps = [Vec::new(), Vec::new()];
        for map in &mut maps {
            for _ in 0..r.leb()? {
                map.push((text(r)?, text(r)?));
            }
        }
        let [props, app] = maps;
        let (head, version, modified_ms, name) = (text(r)?, r.leb()?, r.leb()?, text(r)?);
        let bits = r.u8().filter(|b| *b < 4)?;
        let mut permission_ids = Vec::new();
        for _ in 0..r.leb()? {
            permission_ids.push(text(r)?);
        }
        r.is_empty().then_some(DriveMeta { props, app, head, version, modified_ms, name, trashed: bits & 1 != 0, can_edit: bits & 2 != 0, permission_ids })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dest {
    Peer(u64),
    /// The document's BroadcastChannel.
    Tabs,
}

/// Negotiated data channels (§10.3): 0 `ihn6`, 1 `ihn6-bulk`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Channel {
    Ctl,
    Bulk,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SdpKind {
    Offer = 1,
    Answer = 2,
}

/// Executed by `host/peer.js`; `peer` is the remote installation.
#[derive(Clone, PartialEq, Debug)]
pub enum RtcReq {
    /// New connection with both channels; `offer` ⇒ create the offer and report it (`RtcEvent::Local`).
    Create { peer: u64, ice: Vec<String>, offer: bool },
    SetRemote { peer: u64, kind: SdpKind, sdp: String },
    /// Create the answer and report it.
    Answer { peer: u64 },
    Close { peer: u64 },
}

#[derive(Clone, PartialEq, Debug)]
pub enum RtcEvent {
    /// Local description after the gathering rule (§10.4).
    Local { peer: u64, kind: SdpKind, sdp: String },
    ChannelOpen { peer: u64, ch: Channel },
    ChannelClosed { peer: u64, ch: Channel },
    Message { peer: u64, ch: Channel, bytes: Vec<u8> },
    /// `bufferedamountlow` of the bulk channel.
    Drained { peer: u64 },
    /// `connectionState`: 0 new, 1 connecting, 2 connected, 3 disconnected, 4 failed, 5 closed.
    State { peer: u64, state: u8 },
    /// Produced by the session, never by the host: a message from `peer` failed to decode or broke a reassembly
    /// limit; the session already asked for `RtcReq::Close`.
    Fault { peer: u64 },
}

/// Page and network lifecycle, as the host reports it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lifecycle {
    Hidden,
    Visible,
    Online,
    Offline,
    /// Connection change (§9.3): poll now, probe channels.
    NetChange,
    /// `pagehide`: only keepalive requests can still leave.
    PageHide,
    /// Home / close.
    Leave,
    /// The Drive file of a document created offline now exists (§16.1 `NotCreated`).
    Created,
    /// Local input on the document (Alone cadence, export idleness).
    Input,
    /// Pen contact began / ended (an accepted contact aborts an export, §14.1).
    Contact(bool),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LockEvent {
    /// `ihn6-net:<account>:<docId>` acquired / lost.
    Net { held: bool },
}

#[derive(Clone, PartialEq, Debug)]
pub enum SyncEffect {
    Http { id: u32, req: HttpReq },
    Rtc(RtcReq),
    Send { to: Dest, ch: Channel, bytes: Vec<u8> },
    /// SHA-256 (effect family 3.8).
    Digest { id: u32, bytes: Vec<u8> },
    /// deflate-raw (effect family 3.10).
    Deflate { id: u32, inflate: bool, bytes: Vec<u8> },
    Persist(SyncRecord),
    /// Call `tick` at `at` (host time, ms). Replaces the previous timer.
    Timer { at: f64 },
    /// Request (`hold`) or release the `ihn6-net` Web Lock; the answer is `LockEvent::Net`.
    Lock { hold: bool },
}

#[derive(Clone, PartialEq, Debug)]
pub enum SyncIo {
    /// `status` 0 = no answer (network failure, deadline, aborted).
    Http { id: u32, status: u16, etag: String, body: Vec<u8> },
    Rtc(RtcEvent),
    /// A BroadcastChannel message.
    Tab(Vec<u8>),
    Digest { id: u32, out: Vec<u8> },
    /// `out` is empty when inflating failed.
    Deflate { id: u32, out: Vec<u8> },
    Lifecycle(Lifecycle),
    Locks(LockEvent),
}

/// SYNC §4.1 `Range`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Range {
    pub actor: ActorId,
    pub first: u32,
    pub last: u32,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ArchiveRef {
    pub file_id: String,
    pub digest: [u8; 12],
    pub ranges: Vec<Range>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AssetRef {
    pub hash: Hash,
    pub file_id: String,
    pub size: u64,
}

/// What sync persists; the mapping to stores is ARCHITECTURE §7.1 (table "SyncRecord").
#[derive(Clone, PartialEq, Debug)]
pub enum SyncRecord {
    Seq { actor: ActorId, seq: u32 },
    /// Before the upload (§8.5 step 4).
    RelayPlan { gen: u64, file_id: String, epoch_day: u32 },
    /// After both PATCHes succeeded; clears `flags.relay_pending`.
    RelaySealed { sealed: Vec<(ActorId, u32)>, archives: Vec<ArchiveRef>, assets: Vec<AssetRef>, perm: u64 },
    RelayApplied { inst: u64, gen: u64, d16: String },
    KnownExport { head: String, vv: VersionVector },
    LastExport { head: String, vv: VersionVector, etag: String, at: u64 },
    Flags { relay_pending: Option<bool>, export_pending: Option<bool>, created: Option<bool> },
    DriveName { v: String, observed_at: Stamp },
    IdPool(Vec<String>),
    RelayFolder(String),
    NoLink(bool),
    RelayGc(Vec<String>),
}

/// The persisted sync state of a document (`docs` row and `kv` keys), folded from [`SyncRecord`]s. The session is
/// opened with it and keeps it current, so a part reads its resume state here.
#[derive(Clone, PartialEq, Default, Debug)]
pub struct Stored {
    pub seq: BTreeMap<ActorId, u32>,
    pub relay_file: String,
    pub relay_epoch_day: u32,
    pub relay_gen: u64,
    pub sealed: BTreeMap<ActorId, u32>,
    pub archives: Vec<ArchiveRef>,
    pub assets: Vec<AssetRef>,
    pub perm: u64,
    pub applied: BTreeMap<u64, (u64, String)>,
    /// 8 newest known exports, oldest first.
    pub exports: Vec<(String, VersionVector)>,
    pub last_export: Option<(String, VersionVector, String, u64)>,
    pub relay_pending: bool,
    pub export_pending: bool,
    pub created: bool,
    pub drive_name: Option<(String, Stamp)>,
    pub id_pool: Vec<String>,
    pub relay_folder: Option<String>,
    pub no_link: bool,
    pub relay_gc: Vec<String>,
}

impl Stored {
    pub fn apply(&mut self, rec: &SyncRecord) {
        match rec.clone() {
            SyncRecord::Seq { actor, seq } => drop(self.seq.insert(actor, seq)),
            SyncRecord::RelayPlan { gen, file_id, epoch_day } => (self.relay_gen, self.relay_file, self.relay_epoch_day) = (gen, file_id, epoch_day),
            SyncRecord::RelaySealed { sealed, archives, assets, perm } => {
                self.sealed.extend(sealed);
                (self.archives, self.assets, self.perm, self.relay_pending) = (archives, assets, perm, false);
            }
            SyncRecord::RelayApplied { inst, gen, d16 } => drop(self.applied.insert(inst, (gen, d16))),
            SyncRecord::KnownExport { head, vv } => {
                self.exports.retain(|(h, _)| *h != head);
                self.exports.push((head, vv));
                let extra = self.exports.len().saturating_sub(8);
                self.exports.drain(..extra);
            }
            SyncRecord::LastExport { head, vv, etag, at } => self.last_export = Some((head, vv, etag, at)),
            SyncRecord::Flags { relay_pending, export_pending, created } => {
                self.relay_pending = relay_pending.unwrap_or(self.relay_pending);
                self.export_pending = export_pending.unwrap_or(self.export_pending);
                self.created = created.unwrap_or(self.created);
            }
            SyncRecord::DriveName { v, observed_at } => self.drive_name = Some((v, observed_at)),
            SyncRecord::IdPool(ids) => self.id_pool = ids,
            SyncRecord::RelayFolder(id) => self.relay_folder = Some(id),
            SyncRecord::NoLink(v) => self.no_link = v,
            SyncRecord::RelayGc(ids) => self.relay_gc = ids,
        }
    }
    /// The VV of the newest export this device knows.
    pub fn export_vv(&self) -> Option<&VersionVector> { self.exports.last().map(|(_, vv)| vv) }
}

// ------------------------------------------------------------------------------------------------ session

/// SYNC §16.1.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Closed,
    /// Before the first poll; `not_created`: the Drive file does not exist yet (no polls, no relay).
    Loading { not_created: bool },
    Live(Mode),
    /// Hidden: no poll, relay flushed, peers kept.
    Background,
    /// Hidden 60 s: peers closed, record `l`, lock released.
    Suspended,
    Offline,
    /// Back online, first poll pending (§8.8).
    CatchingUp,
    Leaving,
}

/// What a part is told besides its own answers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    /// Flush the relay now (keepalive rules); keep peers.
    Hidden,
    /// Hidden for 60 s: `Bye`, record `l`. The lock is released right after this call.
    Suspend,
    Visible,
    Online,
    Offline,
    NetChange,
    PageHide,
    /// `Bye`, record `l`, relay flush, export if alone. The session closes when no request is in flight.
    Leave,
    Created,
    /// Pen contact began.
    Contact,
    /// `ihn6-net` acquired (resume from `Core::stored`) or lost (stop all network work).
    Leader(bool),
    /// Own capability changed (§12.3): `false` ⇒ close peers, stop relay writes.
    CanEdit(bool),
    /// The mode changed; the previous one is given.
    Mode(Mode),
}

/// Requested relay flush urgency (set by the session, cleared by the relay writer).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum Flush {
    #[default]
    None,
    /// As soon as the 2 s cap allows.
    Soon,
    /// Hide/exit: at once, `keepalive`, ignoring the cap (§8.5).
    Keepalive,
}

pub struct Config {
    pub doc_id: DocId,
    pub inst: u64,
    pub actor: ActorId,
    pub profile: Profile,
    pub can_edit: bool,
    pub visible: bool,
    pub online: bool,
    /// `docs` row + `kv` state (ARCHITECTURE §7.1).
    pub stored: Stored,
    /// Any random value: seeds the backoff jitter.
    pub seed: u64,
}

/// The session state every part works on.
pub struct Core {
    /// Host time (ms) of the current call.
    pub now: f64,
    pub doc_id: DocId,
    pub inst: u64,
    pub actor: ActorId,
    pub profile: Profile,
    pub stored: Stored,
    pub presence: Presence,
    pub mode: Mode,
    /// This tab holds `ihn6-net`: only then it polls, writes the relay and holds peers (§11).
    pub leader: bool,
    pub online: bool,
    pub visible: bool,
    pub can_edit: bool,
    pub contact: bool,
    /// A poll has succeeded since open.
    pub polled: bool,
    pub last_input_at: f64,
    /// Latest poll answer and its v2 etag.
    pub meta: Option<(DriveMeta, String)>,
    pub status: SyncStatus,
    /// Latest own presence (for `Hello` and `Presence` frames).
    pub own_presence: Option<PresenceState>,
    // -- blackboard between parts
    pub flush: Flush,
    /// Entering company: write the own record at once with flag `c` (§9.2). Cleared by the relay writer.
    pub record_due: bool,
    /// Signalling mail to publish in the next relay write (owned by `peer.rs`, read by `relay.rs`);
    /// set `mail_changed` when editing it.
    pub mail_out: Vec<Mail>,
    pub mail_changed: bool,
    /// Mail addressed to us from verified relay files: `(sender, entry)` (pushed by `relay.rs`, drained by `peer.rs`).
    pub mail_in: Vec<(u64, Mail)>,
    /// Known VV of every open peer and tab (written by `entropy.rs`; feeds journal retention, §7.3).
    pub peer_vv: HashMap<PeerRef, VersionVector>,
    /// The relay cannot fill a gap: the export must be downloaded (§8.6 step 3; set by `relay.rs`, cleared by `export.rs`).
    pub need_export: bool,
    /// Remote live ink / erase previews (written by `live.rs`, drawn by the remote-ink overlay).
    pub live: live::Previews,
    /// The export desk: build / merge requests to the host and their answers (owned by `export.rs`, served by the
    /// session binding).
    pub export: export::Desk,
    // -- private
    effects: Vec<SyncEffect>,
    owner: u8,
    owners: BTreeMap<u32, u8>,
    next_id: u32,
    msg_id: u32,
    wake: f64,
    rng: u64,
    assemblies: BTreeMap<u64, Reassembly>,
    closed: bool,
    leaving_since: Option<f64>,
    suspended: bool,
    catching_up: bool,
    hidden_at: f64,
    lock_wanted: bool,
    poll: Option<u32>,
    poll_at: f64,
    polled_at: f64,
    poll_fails: u32,
    retained_at: f64,
    migration: ActorId,
}

const SELF: u8 = u8::MAX;
const LEAVE_LIMIT_MS: f64 = 30_000.0;
const RETAIN_EVERY_MS: f64 = 10_000.0;

impl Core {
    fn request(&mut self) -> u32 {
        self.next_id += 1;
        self.owners.insert(self.next_id, self.owner);
        self.next_id
    }
    /// Issues a Drive request; the answer comes to the calling part's [`Part::http`].
    pub fn http(&mut self, req: HttpReq) -> u32 {
        let id = self.request();
        self.effects.push(SyncEffect::Http { id, req });
        id
    }
    /// SHA-256; the answer comes to [`Part::digest`].
    pub fn digest(&mut self, bytes: Vec<u8>) -> u32 {
        let id = self.request();
        self.effects.push(SyncEffect::Digest { id, bytes });
        id
    }
    /// deflate-raw; the answer comes to [`Part::deflate`].
    pub fn deflate(&mut self, inflate: bool, bytes: Vec<u8>) -> u32 {
        let id = self.request();
        self.effects.push(SyncEffect::Deflate { id, inflate, bytes });
        id
    }
    pub fn rtc(&mut self, req: RtcReq) {
        if let RtcReq::Close { peer } = req {
            self.assemblies.remove(&peer);
        }
        self.effects.push(SyncEffect::Rtc(req));
    }
    /// Persists and folds into [`Core::stored`].
    pub fn persist(&mut self, rec: SyncRecord) {
        self.stored.apply(&rec);
        self.effects.push(SyncEffect::Persist(rec));
    }
    /// Ask for a `tick` at `at` at the latest.
    pub fn wake(&mut self, at: f64) { self.wake = self.wake.min(at) }
    /// `U(0.8, 1.2)`.
    pub fn jitter(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        0.8 + (self.rng >> 11) as f64 / (1u64 << 53) as f64 * 0.4
    }
    /// `min(max, base × 2^(fails−1)) × U(0.8, 1.2)` (§8.5, §10.4).
    pub fn backoff(&mut self, base: f64, max: f64, fails: u32) -> f64 { (base * (1u64 << fails.saturating_sub(1).min(30)) as f64).min(max) * self.jitter() }

    /// Sends one frame: small ones on channel 0, larger ones as `Part`s on the bulk channel. Tabs take any size.
    /// (A sender that paces bulk data by `Drained` uses [`wire::fragment`] and [`Core::send_raw`] itself.)
    pub fn send(&mut self, to: Dest, frame: &Frame) {
        let bytes = frame.encode();
        if to == Dest::Tabs || bytes.len() <= PART_DATA {
            return self.send_raw(to, Channel::Ctl, bytes);
        }
        self.msg_id = self.msg_id.wrapping_add(1);
        for part in wire::fragment(self.msg_id, &bytes).unwrap_or_default() {
            self.send_raw(to, Channel::Bulk, part);
        }
    }
    pub fn send_raw(&mut self, to: Dest, ch: Channel, bytes: Vec<u8>) { self.effects.push(SyncEffect::Send { to, ch, bytes }) }
    /// To the tabs and every open peer.
    pub fn send_all(&mut self, frame: &Frame) {
        self.send(Dest::Tabs, frame);
        for inst in self.open_peers() {
            self.send(Dest::Peer(inst), frame);
        }
    }
    pub fn open_peers(&self) -> Vec<u64> { self.presence.remotes.iter().filter(|(_, r)| r.link == mode::Link::Open).map(|(&i, _)| i).collect() }

    pub fn is_scribe(&self) -> bool { self.presence.scribe(self.inst, self.now) == self.inst }
    pub fn cadence(&self) -> Cadence {
        let since = match self.mode {
            Mode::Viewer => self.presence.last_change().unwrap_or(f64::NEG_INFINITY),
            _ => self.last_input_at,
        };
        Cadence::of(self.mode, self.now - since)
    }
    /// Batches of this actor are written to our relay file: the tabs of this installation and its migration actor.
    pub fn relays(&self, actor: ActorId) -> bool { actor == self.migration || (!actor.is_import() && actor.inst() == self.inst) }
    /// Network work is allowed at all (leader, online, created, not closed).
    pub fn networking(&self) -> bool { self.leader && self.online && self.stored.created && !self.closed }

    pub fn phase(&self) -> Phase {
        if self.closed {
            Phase::Closed
        } else if self.leaving_since.is_some() {
            Phase::Leaving
        } else if !self.stored.created {
            Phase::Loading { not_created: true }
        } else if !self.visible {
            [Phase::Background, Phase::Suspended][self.suspended as usize]
        } else if !self.online {
            Phase::Offline
        } else if self.catching_up {
            Phase::CatchingUp
        } else if !self.polled {
            Phase::Loading { not_created: false }
        } else {
            Phase::Live(self.mode)
        }
    }

    fn error(&mut self, phase: &str, code: &str, detail: &str) {
        self.status.last_error = Some(SyncError { phase: phase.into(), code: code.into(), detail: detail.into() });
    }
    fn lock(&mut self, hold: bool) {
        if self.lock_wanted != hold {
            self.lock_wanted = hold;
            self.effects.push(SyncEffect::Lock { hold });
        }
    }
    fn poll_now(&mut self) { self.poll_at = self.now }

    /// Journal entries every destination already has (§7.3).
    fn covered(&self, r: &dyn Replica) -> VersionVector {
        let mut out = VersionVector::default();
        for (actor, mut seq) in r.version_vector().iter() {
            for vv in self.peer_vv.values().chain(self.stored.export_vv()) {
                seq = seq.min(vv.get(actor));
            }
            if self.stored.exports.is_empty() {
                seq = 0;
            }
            if self.relays(actor) {
                seq = seq.min(self.stored.sealed.get(&actor).copied().unwrap_or(0));
            }
            if seq > 0 {
                out.raise(actor, seq);
            }
        }
        out
    }
}

/// One protocol part. Every method has the session state and, where the document may be touched, the replica.
#[allow(unused_variables)]
pub trait Part {
    fn local_batch(&mut self, cx: &mut Core, b: &EncodedBatch, r: &mut dyn Replica) {}
    fn live(&mut self, cx: &mut Core, p: &LivePreview) {}
    fn presence(&mut self, cx: &mut Core, p: &PresenceState) {}
    /// A successful poll; `cx.meta` holds it and `cx.presence` has already seen its records.
    fn meta(&mut self, cx: &mut Core, r: &mut dyn Replica) {}
    /// The answer to a request this part issued with [`Core::http`].
    fn http(&mut self, cx: &mut Core, id: u32, status: u16, etag: &str, body: &[u8], r: &mut dyn Replica) {}
    fn digest(&mut self, cx: &mut Core, id: u32, out: &[u8], r: &mut dyn Replica) {}
    fn deflate(&mut self, cx: &mut Core, id: u32, out: &[u8], r: &mut dyn Replica) {}
    /// Every `RtcEvent` except `Message` (those arrive decoded through [`Part::frame`]).
    fn rtc(&mut self, cx: &mut Core, ev: &RtcEvent, r: &mut dyn Replica) {}
    /// A decoded frame from a peer or a tab (`Part`s already reassembled).
    fn frame(&mut self, cx: &mut Core, from: PeerRef, f: &Frame, r: &mut dyn Replica) {}
    fn change(&mut self, cx: &mut Core, c: Change, r: &mut dyn Replica) {}
    /// Runs on every call into the session, after the inputs were dispatched: do what is due, `cx.wake` the rest.
    fn tick(&mut self, cx: &mut Core, r: &mut dyn Replica) {}
}

enum Queued {
    Batch(EncodedBatch),
    Live(LivePreview),
    Presence(PresenceState),
}

/// One per open document.
pub struct Sync {
    pub core: Core,
    parts: Vec<Box<dyn Part>>,
    queued: Vec<Queued>,
    views: Vec<PeerView>,
    timer: f64,
}

impl Sync {
    pub fn new(cfg: Config) -> Sync { Sync::with_parts(cfg, parts()) }

    pub fn with_parts(cfg: Config, parts: Vec<Box<dyn Part>>) -> Sync {
        let core = Core {
            now: 0.0,
            migration: ActorId::import(&migration_source(cfg.inst, &cfg.doc_id)),
            doc_id: cfg.doc_id,
            inst: cfg.inst,
            actor: cfg.actor,
            profile: cfg.profile,
            stored: cfg.stored,
            presence: Presence::default(),
            mode: if cfg.can_edit { Mode::Alone } else { Mode::Viewer },
            leader: false,
            online: cfg.online,
            visible: cfg.visible,
            can_edit: cfg.can_edit,
            contact: false,
            polled: false,
            last_input_at: f64::NEG_INFINITY,
            meta: None,
            status: SyncStatus::default(),
            own_presence: None,
            flush: Flush::None,
            record_due: false,
            mail_out: Vec::new(),
            mail_changed: false,
            mail_in: Vec::new(),
            peer_vv: HashMap::new(),
            need_export: false,
            live: live::Previews::default(),
            export: export::Desk::default(),
            effects: Vec::new(),
            owner: SELF,
            owners: BTreeMap::new(),
            next_id: 0,
            msg_id: 0,
            wake: f64::INFINITY,
            rng: cfg.seed | 1,
            assemblies: BTreeMap::new(),
            closed: false,
            leaving_since: None,
            suspended: false,
            catching_up: false,
            hidden_at: 0.0,
            lock_wanted: false,
            poll: None,
            poll_at: 0.0,
            polled_at: f64::NEG_INFINITY,
            poll_fails: 0,
            retained_at: 0.0,
        };
        Sync { core, parts, queued: Vec::new(), views: Vec::new(), timer: f64::INFINITY }
    }

    /// Sets the host time for the next `on_io` (which carries none). Call it before every `on_io`.
    pub fn at(&mut self, now: f64) -> &mut Self {
        self.core.now = self.core.now.max(now);
        self
    }

    pub fn phase(&self) -> Phase { self.core.phase() }

    fn each(&mut self, mut f: impl FnMut(&mut dyn Part, &mut Core)) {
        for (i, p) in self.parts.iter_mut().enumerate() {
            self.core.owner = i as u8;
            f(p.as_mut(), &mut self.core);
        }
        self.core.owner = SELF;
    }
    fn one(&mut self, owner: u8, f: impl FnOnce(&mut dyn Part, &mut Core)) {
        if let Some(p) = self.parts.get_mut(owner as usize) {
            self.core.owner = owner;
            f(p.as_mut(), &mut self.core);
            self.core.owner = SELF;
        }
    }
    fn change(&mut self, c: Change, r: &mut dyn Replica) { self.each(|p, cx| p.change(cx, c, r)) }

    fn close(&mut self) {
        self.core.closed = true;
        self.core.lock(false);
        self.core.owners.clear();
        self.core.poll = None;
    }

    fn lifecycle(&mut self, ev: Lifecycle, r: &mut dyn Replica) {
        let now = self.core.now;
        let c = &mut self.core;
        let change = match ev {
            Lifecycle::Hidden if c.visible => {
                (c.visible, c.hidden_at) = (false, now);
                c.flush = c.flush.max(Flush::Keepalive);
                Change::Hidden
            }
            Lifecycle::Visible if !c.visible => {
                (c.visible, c.suspended) = (true, false);
                c.poll_now();
                Change::Visible
            }
            Lifecycle::Online if !c.online => {
                (c.online, c.catching_up, c.poll_fails) = (true, c.polled, 0);
                c.poll_now();
                Change::Online
            }
            Lifecycle::Offline if c.online => {
                c.online = false;
                Change::Offline
            }
            Lifecycle::NetChange => {
                c.poll_now();
                Change::NetChange
            }
            Lifecycle::PageHide => {
                c.flush = c.flush.max(Flush::Keepalive);
                Change::PageHide
            }
            Lifecycle::Leave if c.leaving_since.is_none() => {
                c.leaving_since = Some(now);
                c.flush = c.flush.max(Flush::Keepalive);
                Change::Leave
            }
            Lifecycle::Created if !c.stored.created => {
                c.persist(SyncRecord::Flags { relay_pending: None, export_pending: None, created: Some(true) });
                c.poll_now();
                Change::Created
            }
            Lifecycle::Input => {
                // Back from the idle cadence (Alone, 60 s) at once, not after the long wait already scheduled.
                c.last_input_at = now;
                return c.poll_at = c.poll_at.min(c.polled_at + c.cadence().poll);
            }
            Lifecycle::Contact(on) => {
                c.contact = on;
                c.last_input_at = now;
                if !on {
                    return;
                }
                Change::Contact
            }
            _ => return,
        };
        self.change(change, r);
    }

    fn poll_answer(&mut self, status: u16, etag: String, body: &[u8], r: &mut dyn Replica) {
        let c = &mut self.core;
        c.status.auth_paused = status == 401;
        let meta = if status == 200 { DriveMeta::decode(body) } else { None };
        let Some(meta) = meta else {
            match status {
                404 => {
                    c.error("poll", "404", "The document no longer exists");
                    return self.close();
                }
                403 if c.can_edit => {
                    c.can_edit = false;
                    self.change(Change::CanEdit(false), r);
                }
                _ => {}
            }
            let c = &mut self.core;
            c.poll_fails += 1;
            let poll = c.cadence().poll;
            c.poll_at = c.now + c.backoff(poll.max(1000.0), 30_000.0f64.max(poll), c.poll_fails);
            return;
        };
        if meta.trashed {
            c.error("poll", "trashed", "The document is in the bin");
            return self.close();
        }
        let own = c.inst;
        let records = meta.props.iter().filter_map(|(k, v)| Some((mode::record_inst(k)?, v.as_str()))).filter(|(i, _)| *i != own);
        c.presence.observe(records, c.now);
        (c.polled, c.catching_up, c.poll_fails, c.polled_at) = (true, false, 0, c.now);
        let can_edit = meta.can_edit;
        c.meta = Some((meta, etag));
        if can_edit != c.can_edit {
            c.can_edit = can_edit;
            self.change(Change::CanEdit(can_edit), r);
        }
        self.update_mode(r);
        self.each(|p, cx| p.meta(cx, r));
        let c = &mut self.core;
        c.poll_at = c.now + c.cadence().poll;
    }

    fn update_mode(&mut self, r: &mut dyn Replica) {
        let c = &mut self.core;
        let new = c.presence.mode(c.now, c.can_edit);
        if new != c.mode {
            let old = std::mem::replace(&mut c.mode, new);
            c.record_due |= old == Mode::Alone && new != Mode::Viewer;
            // A faster cadence applies from the previous poll, not from the next one of the old cadence.
            c.poll_at = c.poll_at.min(c.polled_at + c.cadence().poll);
            self.change(Change::Mode(old), r);
        }
    }

    /// Everything that is due after the inputs of this call were dispatched.
    fn pump(&mut self, r: &mut dyn Replica) -> Vec<SyncEffect> {
        if self.core.closed {
            self.queued.clear();
            return std::mem::take(&mut self.core.effects);
        }
        for q in std::mem::take(&mut self.queued) {
            match &q {
                Queued::Batch(b) => self.each(|p, cx| p.local_batch(cx, b, r)),
                Queued::Live(l) => self.each(|p, cx| p.live(cx, l)),
                Queued::Presence(s) => self.each(|p, cx| p.presence(cx, s)),
            }
        }
        let now = self.core.now;
        if self.core.last_input_at == f64::NEG_INFINITY {
            self.core.last_input_at = now; // opening the document counts as input for the Alone cadence
        }
        // Hidden for 60 s ⇒ Suspended (§9.3).
        if !self.core.visible && !self.core.suspended && self.core.leaving_since.is_none() {
            if now >= self.core.hidden_at + HIDDEN_SUSPEND_MS {
                self.core.suspended = true;
                self.change(Change::Suspend, r);
                self.core.lock(false);
            } else {
                self.core.wake(self.core.hidden_at + HIDDEN_SUSPEND_MS);
            }
        }
        if self.core.visible && self.core.leaving_since.is_none() {
            self.core.lock(true);
        }
        self.update_mode(r);
        // The poll (§8.6): one metadata GET per cadence while visible, online and leader.
        let c = &mut self.core;
        if c.networking() && c.visible && c.leaving_since.is_none() && c.poll.is_none() {
            if now >= c.poll_at {
                let file = c.doc_id.clone();
                c.poll = Some(c.http(HttpReq::Meta { file }));
            } else {
                c.wake(c.poll_at);
            }
        }
        self.each(|p, cx| p.tick(cx, r));
        self.update_mode(r);
        let c = &mut self.core;
        if let Some(at) = c.presence.next_expiry(now) {
            c.wake(at + 1.0);
        }
        if c.leader && now - c.retained_at >= RETAIN_EVERY_MS {
            c.retained_at = now;
            let covered = c.covered(r);
            if !covered.is_empty() {
                r.retain(&covered);
            }
        }
        // Leaving ends when the parts' requests have been answered (or after 30 s).
        if let Some(since) = c.leaving_since {
            let host_work = c.export.build.is_some() || c.export.merge.is_some();
            if (c.owners.is_empty() && !host_work) || now >= since + LEAVE_LIMIT_MS {
                self.close();
            } else {
                c.wake(since + LEAVE_LIMIT_MS);
            }
        }
        let c = &mut self.core;
        self.views = c.presence.views(now);
        c.status.online = c.online;
        c.status.not_created = !c.stored.created;
        c.status.export_pending = c.stored.export_pending;
        c.status.peers_connected = self.views.iter().filter(|v| v.link == ihn_core::doc::PeerLink::Connected).count() as u32;
        c.status.unsealed_local = r.version_vector().iter().filter(|(a, _)| c.relays(*a)).map(|(a, seq)| seq.saturating_sub(c.stored.sealed.get(&a).copied().unwrap_or(0))).sum();
        let wake = std::mem::replace(&mut c.wake, f64::INFINITY);
        if wake.is_finite() && !c.closed && wake != self.timer {
            self.timer = wake;
            c.effects.push(SyncEffect::Timer { at: wake });
        }
        std::mem::take(&mut c.effects)
    }
}

impl SyncEngine for Sync {
    type Effect = SyncEffect;
    type Io = SyncIo;

    fn local_batch(&mut self, b: &EncodedBatch) { self.queued.push(Queued::Batch(b.clone())) }
    fn live(&mut self, p: LivePreview) { self.queued.push(Queued::Live(p)) }
    fn presence(&mut self, p: PresenceState) {
        self.core.own_presence = Some(p.clone());
        self.queued.push(Queued::Presence(p));
    }

    fn tick(&mut self, now: f64, r: &mut dyn Replica) -> Vec<SyncEffect> {
        self.at(now);
        // The host's timer is spent (or this is an extra call): arm it again if something is still due later.
        // A timer that fires a little early must not leave the session without one.
        self.timer = f64::INFINITY;
        self.pump(r)
    }

    fn on_io(&mut self, ev: SyncIo, r: &mut dyn Replica) -> Vec<SyncEffect> {
        if self.core.closed {
            return Vec::new();
        }
        match ev {
            SyncIo::Http { id, status, etag, body } => match self.core.owners.remove(&id) {
                Some(SELF) if self.core.poll == Some(id) => {
                    self.core.poll = None;
                    self.poll_answer(status, etag, &body, r);
                }
                Some(owner) => self.one(owner, |p, cx| p.http(cx, id, status, &etag, &body, r)),
                None => {}
            },
            SyncIo::Digest { id, out } => {
                if let Some(owner) = self.core.owners.remove(&id) {
                    self.one(owner, |p, cx| p.digest(cx, id, &out, r));
                }
            }
            SyncIo::Deflate { id, out } => {
                if let Some(owner) = self.core.owners.remove(&id) {
                    self.one(owner, |p, cx| p.deflate(cx, id, &out, r));
                }
            }
            SyncIo::Tab(bytes) => {
                if let Some(f) = Frame::decode(&bytes).filter(|f| !matches!(f, Frame::Part { .. })) {
                    self.each(|p, cx| p.frame(cx, PeerRef::Tab, &f, r));
                }
            }
            SyncIo::Rtc(RtcEvent::Message { peer, bytes, .. }) => {
                let now = self.core.now;
                let frame = match Frame::decode(&bytes) {
                    Some(Frame::Part { msg_id, index, count, data }) => match self.core.assemblies.entry(peer).or_default().push(now, msg_id, index, count, data) {
                        Assembled::Pending => return self.pump(r),
                        Assembled::Done(bytes) => Frame::decode(&bytes).filter(|f| !matches!(f, Frame::Part { .. })),
                        Assembled::Close => None,
                    },
                    other => other,
                };
                match frame {
                    Some(f) => self.each(|p, cx| p.frame(cx, PeerRef::Peer(peer), &f, r)),
                    // §10.5: a frame that fails to decode closes the channel.
                    None => {
                        self.core.rtc(RtcReq::Close { peer });
                        self.each(|p, cx| p.rtc(cx, &RtcEvent::Fault { peer }, r));
                    }
                }
            }
            SyncIo::Rtc(ev) => {
                if let RtcEvent::ChannelClosed { peer, .. } | RtcEvent::State { peer, state: 4 | 5 } = ev {
                    self.core.assemblies.remove(&peer);
                }
                self.each(|p, cx| p.rtc(cx, &ev, r));
            }
            SyncIo::Lifecycle(ev) => self.lifecycle(ev, r),
            SyncIo::Locks(LockEvent::Net { held }) => {
                if held != self.core.leader && (held == self.core.lock_wanted || !held) {
                    self.core.leader = held;
                    if held {
                        self.core.poll_now();
                    } else {
                        self.core.poll = None;
                    }
                    self.change(Change::Leader(held), r);
                } else if held && !self.core.lock_wanted {
                    // Granted after we stopped wanting it (suspended meanwhile): give it back.
                    self.core.effects.push(SyncEffect::Lock { hold: false });
                }
            }
        }
        self.pump(r)
    }

    fn status(&self) -> SyncStatus {
        let mut s = self.core.status.clone();
        for (&actor, &seq) in &self.core.stored.sealed {
            s.relay_durable_through.raise(actor, seq);
        }
        s
    }

    fn peers(&self) -> &[PeerView] { &self.views }
}
