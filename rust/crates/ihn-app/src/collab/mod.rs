//! Collab: binding of `ihn-sync` to the open document session (ARCHITECTURE §4.4, §5; SYNC §6–§11, §18), the
//! presence avatars and popup (`presence.rs`, SYNC §9.1) and the remote live-ink overlay. Owned by W4.5.
//!
//! One [`ihn_sync::Sync`] per open document. The binding lends the session to it as a `Replica`
//! (`session::with_replica`), feeds it every input (committed local batches, live packets of the pen and the
//! eraser, page lifecycle, P2P and tab messages, Drive answers from `drive::relay`, digests and deflates) and
//! executes its effects: Drive requests (`drive::relay::exec`), RTC and BroadcastChannel records of `host/peer.js`
//! (family 3, ops `0x60..0x67`, NOTES-W2.8), SHA-256 / deflate effects, persistence (`session::persist`) and its
//! timer. After every call it reports `SyncStatus` to the session, re-renders the avatars when they changed and
//! repaints the remote overlays when the live table moved.
//!
//! The `ihn6-net` Web Lock is the session's (NOTES-W3.1): `Lock { hold: true }` is answered from
//! `Session::net_holder` (polled while waiting), `Lock { hold: false }` at once (the physical lock stays with the
//! session until it closes).

pub mod presence;

use crate::abi::{chan, fam, life, tag, Event};
use crate::app::Mod;
use crate::doc::{save_state, session};
use crate::drive::relay;
use crate::editor::{eraser, pages, pen};
use crate::shell::{self, fx};
use crate::store::schema::{self as schema, kv, DocRecord, Record};
use crate::store::{self, sv::Value, Reply, Tx as StoreTx};
use crate::testapi::json_str;
use crate::{auth, App, Host};
use ihn_core::codec::{Reader, Writer};
use ihn_core::doc::{BlobHandle, Compacted, LivePreview, PeerRef, PresenceMode, PresenceState, ReceiveResult, Replica, SyncEngine};
use ihn_core::id::{fnv1a64, ActorId, DocId, Hash, Id};
use ihn_core::op::Color;
use ihn_core::render::{overlay, StrokeRef};
use ihn_core::snapshot::{EncodedBatch, EncodedSnapshot};
use ihn_core::stamp::{Stamp, VersionVector};
use ihn_sync::wire::Profile;
use ihn_sync::{ArchiveRef, AssetRef, Channel, Config, Dest, Lifecycle, LockEvent, Phase, Range, RtcEvent, RtcReq, SdpKind, Stored, Sync, SyncEffect, SyncIo, SyncRecord};

const T_TICK: u32 = tag(Mod::Collab, 1);
const T_LOCK: u32 = tag(Mod::Collab, 2);
pub(crate) const T_RING: u32 = tag(Mod::Collab, 3);
pub(crate) const L_AVATARS: u32 = tag(Mod::Collab, 4);
pub(crate) const L_POPUP_CLOSE: u32 = tag(Mod::Collab, 5);
pub(crate) const L_DOC_CLICK: u32 = tag(Mod::Collab, 6);
pub(crate) const L_DOC_KEY: u32 = tag(Mod::Collab, 7);

/// `host/peer.js` ops (family 3).
mod op {
    pub const RTC_CREATE: u8 = 0x60;
    pub const RTC_REMOTE: u8 = 0x61;
    pub const RTC_ANSWER: u8 = 0x62;
    pub const RTC_CLOSE: u8 = 0x63;
    pub const RTC_SEND: u8 = 0x64;
    pub const BC_OPEN: u8 = 0x65;
    pub const BC_SEND: u8 = 0x66;
    pub const BC_CLOSE: u8 = 0x67;
}

/// The session's document after it closed: what a leaving engine may still read (its own unsealed batches).
struct Detached {
    doc_id: DocId,
    vv: VersionVector,
    batches: Vec<EncodedBatch>,
}

impl Replica for Detached {
    fn doc_id(&self) -> &DocId { &self.doc_id }
    fn version_vector(&self) -> &VersionVector { &self.vv }
    fn batches_since(&self, have: &VersionVector, _max: usize) -> Result<Vec<EncodedBatch>, Compacted> {
        Ok(self.batches.iter().filter(|b| b.header().is_some_and(|(a, s, _)| !have.contains(a, s))).cloned().collect())
    }
    fn receive(&mut self, b: &[EncodedBatch], _from: PeerRef) -> ReceiveResult { ReceiveResult { duplicate: b.len() as u32, ..ReceiveResult::default() } }
    fn snapshot(&self) -> EncodedSnapshot { EncodedSnapshot(Vec::new()) }
    fn load_snapshot(&mut self, _s: &EncodedSnapshot, _from: PeerRef) -> ReceiveResult { ReceiveResult::default() }
    fn missing_assets(&self) -> Vec<Hash> { Vec::new() }
    fn put_asset(&mut self, _h: Hash, _b: BlobHandle) {}
    fn asset(&self, _h: &Hash) -> Option<BlobHandle> { None }
    fn can_edit(&self) -> bool { false }
    fn retain(&mut self, _covered: &VersionVector) {}
}

/// One running engine.
pub struct Run {
    pub sync: Sync,
    /// Session number: answers of an earlier engine are dropped.
    pub gen: u32,
    pub doc: String,
    account: String,
    /// The document's BroadcastChannel (`ihn6:` + fnv1a64hex(account) + `:` + docId).
    bc: String,
    detached: Option<Detached>,
    want_lock: bool,
    leaving: bool,
    phase_code: u8,
}

impl Run {
    /// The engine is leaving its document (Home / close): it still flushes, says goodbye and may export.
    pub fn leaving(&self) -> bool { self.leaving }
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    pub run: Option<Box<Run>>,
    gen: u32,
    /// The `kv` read that precedes an engine: (request, doc id).
    kv_req: (u32, String),
    /// Digest / deflate effects in flight: (request, gen, sync id, is digest).
    fx: Vec<(u32, u32, u32, bool)>,
    contact: bool,
    /// Live overlay: the version painted and the pages that show remote ink.
    overlay_ver: u64,
    overlay_pages: Vec<Id>,
    overlay_armed: bool,
    pub(crate) presence: presence::State,
}

pub fn active(app: &App) -> bool { app.st.collab.run.is_some() }

/// The open document's engine, for the export desk (W5.4: answer `core.export.build/merge`, set `force`), then
/// [`kick`] to let it continue.
pub fn with_sync<R>(app: &mut App, f: impl FnOnce(&mut Sync) -> R) -> Option<R> { app.st.collab.run.as_mut().map(|r| f(&mut r.sync)) }

/// Runs the engine once now (after [`with_sync`] changed its inputs).
pub fn kick(app: &mut App, host: &mut dyn Host) {
    let now = host.now();
    drive(app, host, |s, r| s.tick(now, r));
}

/// W5.4: the Drive file of a document created on this device now exists (SYNC §16.1 `NotCreated` → polling).
pub fn created(app: &mut App, host: &mut dyn Host) { io(app, host, SyncIo::Lifecycle(Lifecycle::Created)) }

// ---- conversions --------------------------------------------------------------------------------------------------

fn vv_bytes(vv: &VersionVector) -> Vec<u8> {
    let mut w = Writer::default();
    vv.encode(&mut w);
    w.buf
}
fn vv_of(bytes: &[u8]) -> VersionVector { VersionVector::decode(&mut Reader::new(bytes)).unwrap_or_default() }

fn ranges_bytes(ranges: &[Range]) -> Vec<u8> {
    let mut w = Writer::default();
    for r in ranges {
        w.u64(r.actor.0);
        w.u32(r.first);
        w.u32(r.last);
    }
    w.buf
}
fn ranges_of(bytes: &[u8]) -> Vec<Range> {
    let mut r = Reader::new(bytes);
    let mut out = Vec::new();
    while let (Some(a), Some(first), Some(last)) = (r.u64(), r.u32(), r.u32()) {
        out.push(Range { actor: ActorId(a), first, last });
    }
    out
}

fn d16_bytes(d16: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (o, b) in out.iter_mut().zip(d16.bytes()) {
        *o = b;
    }
    out
}

/// `SyncRecord` → the store's `Record` (ARCHITECTURE §7.1 table "SyncRecord"); `record` is the current mirror.
fn record_of(rec: SyncRecord, record: &DocRecord) -> Record {
    match rec {
        SyncRecord::Seq { actor, seq } => Record::Seq { actor: actor.0, seq },
        SyncRecord::RelayPlan { gen, file_id, epoch_day } => Record::RelayPlan { gen: gen as u32, file_id, epoch_day },
        SyncRecord::RelaySealed { sealed, archives, assets, perm } => {
            let mut map = record.relay.sealed.clone();
            for (a, s) in sealed {
                map.insert(a.0, s);
            }
            Record::RelaySealed {
                sealed: map,
                archives: archives.into_iter().map(|a| schema::Archive { file_id: a.file_id, digest: a.digest.to_vec(), ranges: ranges_bytes(&a.ranges) }).collect(),
                assets: assets.into_iter().map(|a| schema::RelayAsset { hash: a.hash, file_id: a.file_id, size: a.size as f64 }).collect(),
                perm: perm.to_le_bytes().to_vec(),
                relay_pending: false,
            }
        }
        SyncRecord::RelayApplied { inst, gen, d16 } => Record::RelayApplied { inst, gen: gen as u32, d16: d16_bytes(&d16) },
        SyncRecord::KnownExport { head, vv } => Record::KnownExport { head, vv: vv_bytes(&vv) },
        SyncRecord::LastExport { head, vv, etag, at } => Record::LastExport(schema::LastExport { head, vv: vv_bytes(&vv), etag, at: at as f64 }),
        SyncRecord::Flags { relay_pending, export_pending, created } => Record::Flags { relay_pending, export_pending, created },
        SyncRecord::DriveName { v, observed_at: s } => Record::DriveName(schema::DriveName { v, observed_at: schema::Stamp { clock: s.clock, actor: s.actor.0, idx: s.idx } }),
        SyncRecord::IdPool(ids) => Record::IdPool(ids),
        SyncRecord::RelayFolder(id) => Record::RelayFolder(id),
        SyncRecord::NoLink(v) => Record::NoLink(v),
        SyncRecord::RelayGc(ids) => Record::RelayGc(ids),
    }
}

/// The persisted sync state of the open document (`docs` record + the account's `kv` keys).
fn stored_of(rec: &DocRecord, kv: &[Value; 4]) -> Stored {
    let r = &rec.relay;
    let strings = |v: &Value| v.items().iter().filter_map(|x| x.as_str().map(String::from)).collect::<Vec<_>>();
    let mut perm = [0u8; 8];
    for (o, b) in perm.iter_mut().zip(&r.perm) {
        *o = *b;
    }
    Stored {
        seq: rec.seq.iter().map(|(a, &s)| (ActorId(a), s)).collect(),
        relay_file: r.file_id.clone(),
        relay_epoch_day: r.epoch_day,
        relay_gen: r.gen as u64,
        sealed: r.sealed.iter().map(|(a, &s)| (ActorId(a), s)).collect(),
        archives: r.archives.iter().map(|a| ArchiveRef { file_id: a.file_id.clone(), digest: a.digest.get(..12).and_then(|d| d.try_into().ok()).unwrap_or([0; 12]), ranges: ranges_of(&a.ranges) }).collect(),
        assets: r.assets.iter().map(|a| AssetRef { hash: a.hash, file_id: a.file_id.clone(), size: a.size as u64 }).collect(),
        perm: u64::from_le_bytes(perm),
        applied: r.applied.iter().map(|(i, (g, d))| (i, (*g as u64, String::from_utf8_lossy(d).trim_end_matches('\0').to_string()))).collect(),
        exports: rec.exports.iter().map(|(h, v)| (h.clone(), vv_of(v))).collect(),
        last_export: rec.last_export.as_ref().map(|l| (l.head.clone(), vv_of(&l.vv), l.etag.clone(), l.at as u64)),
        relay_pending: rec.flags.relay_pending,
        export_pending: rec.flags.export_pending,
        created: rec.flags.created,
        drive_name: rec.drive_name.as_ref().map(|n| (n.v.clone(), Stamp { clock: n.observed_at.clock, actor: ActorId(n.observed_at.actor), idx: n.observed_at.idx })),
        id_pool: strings(&kv[0]),
        relay_folder: kv[1].as_str().filter(|s| !s.is_empty()).map(String::from),
        no_link: kv[2].as_bool().unwrap_or(false),
        relay_gc: strings(&kv[3]),
    }
}

/// 5.18.0 `detectDeviceType()` as `b'm' | b't' | b'p'`.
pub fn device_type(app: &App) -> u8 {
    let ua = app.cfg.user_agent.to_ascii_lowercase();
    let has = |s: &str| ua.contains(s);
    let android = has("android");
    if (android && !has("mobile")) || has("ipad") || has("tablet") || has("playbook") || (app.cfg.max_touch_points > 1 && has("macintosh") && app.cfg.viewport.0 >= 768.0) {
        b't'
    } else if has("mobi") || has("iphone") || has("ipod") || has("blackberry") || has("iemobile") || has("opera mini") {
        b'm'
    } else {
        b'p'
    }
}

/// 5.18.0 `detectDeviceLabel()` (from the user agent: the platform string is not in the boot config).
pub fn device_label(app: &App) -> &'static str {
    let ua = &app.cfg.user_agent;
    let has = |s: &str| ua.to_ascii_lowercase().contains(s);
    match () {
        _ if has("iphone") => "iPhone",
        _ if has("ipad") => "iPad",
        _ if has("android") => "Android",
        _ if has("windows") => "Windows PC",
        _ if has("mac") => "Mac",
        _ if has("linux") => "Linux PC",
        _ => match device_type(app) {
            b'm' => "Mobile",
            b't' => "Tablet",
            _ => "PC",
        },
    }
}

fn channel_name(account: &str, doc: &str) -> String { format!("ihn6:{:016x}:{doc}", fnv1a64(account.as_bytes())) }

// ---- engine lifecycle ---------------------------------------------------------------------------------------------

/// A document finished opening: read the account's sync keys, then start its engine.
fn opened(app: &mut App, host: &mut dyn Host) {
    let Some(s) = session::get(app) else { return };
    if !s.persistent || s.account.is_empty() || auth::signed_out(app) {
        return;
    }
    if app.st.collab.run.as_ref().is_some_and(|r| r.doc == s.doc_id && !r.leaving) {
        return;
    }
    let (account, doc) = (s.account.clone(), s.doc_id.clone());
    stop(app, host);
    let mut t = StoreTx::new();
    for key in [kv::INST, kv::IDPOOL, kv::RELAY_FOLDER, kv::RELAY_NOLINK, kv::RELAY_GC] {
        t.kv_get(&account, key);
    }
    let req = app.req(Mod::Collab);
    app.st.collab.kv_req = (req, doc);
    store::tx(app, host, req, t);
}

fn start(app: &mut App, host: &mut dyn Host, io: &crate::abi::Io) {
    let doc = std::mem::take(&mut app.st.collab.kv_req.1);
    let Some(s) = session::get(app).filter(|s| s.doc_id == doc) else { return };
    let mut reply = Reply::new(io);
    let mut values: [Value; 5] = Default::default();
    if reply.ok() {
        for v in values.iter_mut() {
            *v = reply.value();
        }
    }
    let inst_kv = values[0].as_str().and_then(crate::store::sv::unhex16).unwrap_or(0);
    let actor = match s.author.as_ref() {
        Some(a) => a.actor,
        None => ActorId::tab(if inst_kv != 0 { inst_kv } else { 0x7f00 }, 0),
    };
    let [_, a, b, c, d] = values;
    let stored = stored_of(&s.record, &[a, b, c, d]);
    let profile = auth::profile(app);
    let profile = Profile {
        email: profile.map_or(String::new(), |p| p.email.clone()),
        name: profile.map_or(String::new(), |p| p.name.clone()),
        photo_url: profile.map_or(String::new(), |p| p.photo.clone()),
        device_type: device_type(app),
        label: device_label(app).into(),
        save_phase: save_state::phase(app).code(),
    };
    let mut seed = [0u8; 8];
    host.random(&mut seed);
    let cfg = Config {
        doc_id: s.doc_id.clone(),
        inst: actor.inst(),
        actor,
        profile,
        can_edit: s.writable(),
        visible: !app.cfg.has(crate::abi::mode::HIDDEN),
        online: !shell::offline(app),
        stored,
        seed: u64::from_le_bytes(seed),
    };
    let account = s.account.clone();
    let st = &mut app.st.collab;
    st.gen += 1;
    let bc = channel_name(&account, &doc);
    let m = host.out().w.rec(fam::FX, op::BC_OPEN);
    host.out().w.str(&bc);
    host.out().w.end(m);
    let phase_code = save_state::phase(app).code();
    let st = &mut app.st.collab;
    st.run = Some(Box::new(Run { sync: Sync::new(cfg), gen: st.gen, doc, account, bc, detached: None, want_lock: false, leaving: false, phase_code }));
    pen::set_live_sink(app, true);
    eraser::set_live_sink(app, true);
    // Batches committed before the engine existed (the open itself, an edit while the keys were read).
    let early = session::drain_local(app);
    let now = host.now();
    drive(app, host, |sync, r| {
        for b in &early {
            sync.local_batch(b);
        }
        sync.presence(own_presence(phase_code));
        sync.tick(now, r)
    });
}

fn own_presence(save_phase: u8) -> PresenceState { PresenceState { mode: PresenceMode::Editing, page: Id(0), tool_colour: Color::Hex(0), save_phase } }

/// Drops the engine at once (sign-out, another document): no request of it is answered any more.
fn stop(app: &mut App, host: &mut dyn Host) {
    let Some(run) = app.st.collab.run.take() else { return };
    relay::abort(app, host, run.gen);
    app.st.collab.fx.retain(|f| f.1 != run.gen);
    for &inst in run.sync.core.presence.remotes.keys() {
        let m = peer_rec(host, op::RTC_CLOSE, inst);
        host.out().w.end(m);
    }
    let m = host.out().w.rec(fam::FX, op::BC_CLOSE);
    host.out().w.str(&run.bc);
    host.out().w.end(m);
    app.sched.clear(host.out(), T_TICK);
    app.sched.clear(host.out(), T_LOCK);
    pen::set_live_sink(app, false);
    eraser::set_live_sink(app, false);
    session::set_sync_status(app, host, None);
    save_state::set_catching_up(app, host, false);
    clear_overlays(app, host);
    presence::clear(app, host);
}

/// Home / close: the engine says goodbye (`Bye`, record `l`, keepalive flush) and closes when its requests ended.
fn leave(app: &mut App, host: &mut dyn Host) {
    let Some(run) = app.st.collab.run.as_mut() else { return };
    if run.leaving {
        return;
    }
    run.leaving = true;
    // Keep what the engine may still upload: its own batches above the seal.
    let sealed = run.sync.core.stored.sealed.clone();
    let inst = run.sync.core.inst;
    let doc = run.doc.clone();
    let detached = session::with_replica(app, host, |r| {
        let rv = r.version_vector().clone();
        let mut have = VersionVector::default();
        for (a, s) in rv.iter() {
            have.raise(a, if a.inst() == inst { sealed.get(&a).copied().unwrap_or(0).min(s) } else { s });
        }
        Detached { doc_id: doc.clone(), batches: r.batches_since(&have, usize::MAX).unwrap_or_default(), vv: rv }
    });
    if let Some(run) = app.st.collab.run.as_mut() {
        run.detached = Some(detached.unwrap_or(Detached { doc_id: doc, vv: VersionVector::default(), batches: Vec::new() }));
    }
    pen::set_live_sink(app, false);
    eraser::set_live_sink(app, false);
    clear_overlays(app, host);
    presence::clear(app, host);
    io(app, host, SyncIo::Lifecycle(Lifecycle::Leave));
}

// ---- the call loop ------------------------------------------------------------------------------------------------

/// Calls the engine with the replica (the session, or the detached copy while leaving) and executes the effects.
fn drive(app: &mut App, host: &mut dyn Host, f: impl FnOnce(&mut Sync, &mut dyn Replica) -> Vec<SyncEffect>) {
    let Some(mut run) = app.st.collab.run.take() else { return };
    let effects = match run.detached.as_mut() {
        Some(d) => f(&mut run.sync, d),
        None => {
            let mut sync_ref = Some(&mut run.sync);
            let mut f = Some(f);
            match session::with_replica(app, host, |r| (f.take().unwrap())(sync_ref.take().unwrap(), r)) {
                Some(e) => e,
                None => {
                    // The session is gone without a Leave (another document opened): leave with nothing to read.
                    run.leaving = true;
                    let mut d = Detached { doc_id: run.doc.clone(), vv: VersionVector::default(), batches: Vec::new() };
                    let mut e = f.take().map_or(Vec::new(), |f| f(&mut run.sync, &mut d));
                    let now = host.now();
                    e.extend(run.sync.at(now).on_io(SyncIo::Lifecycle(Lifecycle::Leave), &mut d));
                    run.detached = Some(d);
                    e
                }
            }
        }
    };
    let gen = run.gen;
    app.st.collab.run = Some(run);
    let follow = apply(app, host, gen, effects);
    for ev in follow {
        io(app, host, ev);
    }
    after(app, host);
}

/// One input from the host or a Drive answer.
fn io(app: &mut App, host: &mut dyn Host, ev: SyncIo) {
    let now = host.now();
    drive(app, host, |s, r| s.at(now).on_io(ev, r));
}

/// A Drive answer for session `gen` (from `drive::relay`).
pub fn http_answer(app: &mut App, host: &mut dyn Host, gen: u32, ev: SyncIo) {
    if app.st.collab.run.as_ref().is_some_and(|r| r.gen == gen) {
        io(app, host, ev);
    }
}

fn peer_rec(host: &mut dyn Host, op: u8, peer: u64) -> usize {
    let w = &mut host.out().w;
    let m = w.rec(fam::FX, op);
    w.u32(peer as u32);
    w.u32((peer >> 32) as u32);
    m
}

fn apply(app: &mut App, host: &mut dyn Host, gen: u32, effects: Vec<SyncEffect>) -> Vec<SyncIo> {
    let mut follow = Vec::new();
    for e in effects {
        match e {
            SyncEffect::Http { id, req } => {
                let folder = app.st.collab.run.as_ref().and_then(|r| r.sync.core.stored.relay_folder.clone()).unwrap_or_default();
                relay::exec(app, host, gen, id, req, &folder);
            }
            SyncEffect::Rtc(req) => {
                let m = match req {
                    RtcReq::Create { peer, ice, offer } => {
                        let m = peer_rec(host, op::RTC_CREATE, peer);
                        let w = &mut host.out().w;
                        w.u8(offer as u8);
                        w.u32(ice.len() as u32);
                        ice.iter().for_each(|s| w.str(s));
                        m
                    }
                    RtcReq::SetRemote { peer, kind, sdp } => {
                        let m = peer_rec(host, op::RTC_REMOTE, peer);
                        host.out().w.u8(kind as u8);
                        host.out().w.str(&sdp);
                        m
                    }
                    RtcReq::Answer { peer } => peer_rec(host, op::RTC_ANSWER, peer),
                    RtcReq::Close { peer } => peer_rec(host, op::RTC_CLOSE, peer),
                };
                host.out().w.end(m);
            }
            SyncEffect::Send { to: Dest::Peer(peer), ch, bytes } => {
                let m = peer_rec(host, op::RTC_SEND, peer);
                host.out().w.u8((ch == Channel::Bulk) as u8);
                host.out().w.bytes(&bytes);
                host.out().w.end(m);
            }
            SyncEffect::Send { to: Dest::Tabs, bytes, .. } => {
                let Some(run) = app.st.collab.run.as_ref() else { continue };
                let w = &mut host.out().w;
                let m = w.rec(fam::FX, op::BC_SEND);
                w.str(&run.bc);
                w.bytes(&bytes);
                w.end(m);
            }
            SyncEffect::Digest { id, bytes } => {
                let req = app.req(Mod::Collab);
                app.st.collab.fx.push((req, gen, id, true));
                fx::digest(host.out(), req, fx::Src::Bytes(&bytes));
            }
            SyncEffect::Deflate { id, inflate, bytes } => {
                let req = app.req(Mod::Collab);
                app.st.collab.fx.push((req, gen, id, false));
                fx::deflate(host.out(), req, if inflate { fx::deflate_flag::INFLATE } else { 0 }, fx::Src::Bytes(&bytes));
            }
            SyncEffect::Persist(rec) => {
                let Some(r) = session::get(app).filter(|s| app.st.collab.run.as_ref().is_some_and(|run| run.doc == s.doc_id)).map(|s| record_of(rec.clone(), &s.record)) else {
                    persist_detached(app, host, rec);
                    continue;
                };
                session::persist(app, host, r);
            }
            SyncEffect::Timer { at } => {
                let delay = (at - host.now()).max(0.0);
                app.sched.timer(host.out(), T_TICK, delay);
            }
            SyncEffect::Lock { hold } => {
                let Some(run) = app.st.collab.run.as_mut() else { continue };
                run.want_lock = hold;
                let doc = run.doc.clone();
                if !hold {
                    follow.push(SyncIo::Locks(LockEvent::Net { held: false }));
                } else if session::get(app).is_some_and(|s| s.net_holder && s.doc_id == doc) {
                    follow.push(SyncIo::Locks(LockEvent::Net { held: true }));
                } else {
                    app.sched.timer(host.out(), T_LOCK, 500.0);
                }
            }
        }
    }
    follow
}

/// A record of a leaving engine whose session already closed: written to the store directly.
fn persist_detached(app: &mut App, host: &mut dyn Host, rec: SyncRecord) {
    let Some(run) = app.st.collab.run.as_ref() else { return };
    let (account, doc) = (run.account.clone(), run.doc.clone());
    let mut mirror = DocRecord { account: account.clone(), ..DocRecord::default() };
    mirror.relay.sealed = run.sync.core.stored.sealed.iter().map(|(a, s)| (a.0, *s)).fold(schema::IdMap::default(), |mut m, (a, s)| {
        m.insert(a, s);
        m
    });
    let r = record_of(rec, &mirror);
    let mut t = StoreTx::new();
    t.persist(&account, &doc, &mut mirror, r);
    let req = app.req(Mod::Collab);
    store::tx(app, host, req, t);
}

/// After every engine call: status, presence, overlay, desk rename, end of a leaving engine.
fn after(app: &mut App, host: &mut dyn Host) {
    let Some(run) = app.st.collab.run.as_ref() else { return };
    let closed = run.sync.phase() == Phase::Closed;
    if run.leaving {
        // W5.4: a device leaving alone still exports (SYNC §14.1); the desk is answered while the engine leaves.
        if !closed {
            crate::export::upload::desk(app, host);
        }
        let Some(run) = app.st.collab.run.as_ref() else { return };
        if closed && relay::busy(app, run.gen) == 0 {
            let run = app.st.collab.run.take().unwrap();
            let m = host.out().w.rec(fam::FX, op::BC_CLOSE);
            host.out().w.str(&run.bc);
            host.out().w.end(m);
            app.sched.clear(host.out(), T_TICK);
        }
        return;
    }
    let Some(run) = app.st.collab.run.as_mut() else { return };
    let rename = run.sync.core.export.rename.take();
    let catching = matches!(run.sync.phase(), Phase::CatchingUp);
    let status = run.sync.status();
    let live_ver = run.sync.core.live.version;
    if closed {
        stop(app, host);
        return;
    }
    let mut status = status;
    crate::export::upload::overlay(app, &mut status);
    session::set_sync_status(app, host, Some(status));
    save_state::set_catching_up(app, host, catching);
    if let Some(drive_name) = rename {
        // §14.7: the register holds the title (5.18.0 `getBaseDocTitle`); the record keeps the Drive name.
        let name = crate::editor::view::base_title(&drive_name);
        if let Some(s) = session::get_mut(app) {
            s.record.name = drive_name;
        }
        if session::get(app).is_some_and(|s| s.doc.name.v != name) {
            session::edit(app, host, |tx| tx.set_name(&name));
        }
    }
    crate::export::upload::desk(app, host);
    presence::update(app, host);
    if live_ver != app.st.collab.overlay_ver && !std::mem::replace(&mut app.st.collab.overlay_armed, true) {
        app.sched.frame(host.out(), Mod::Collab);
    }
}

// ---- live ink -----------------------------------------------------------------------------------------------------

/// The pen or the eraser produced live packets: hand them to the engine now (same frame, SYNC §17).
pub fn wake(app: &mut App, host: &mut dyn Host) {
    if app.st.collab.run.as_ref().is_none_or(|r| r.leaving) {
        return;
    }
    let mut packets = pen::drain_live(app);
    packets.extend(eraser::drain_live(app));
    let busy = app.sched.is_busy();
    let contact = std::mem::replace(&mut app.st.collab.contact, busy) != busy;
    if packets.is_empty() && !contact {
        return;
    }
    let now = host.now();
    drive(app, host, |s, r| {
        // The packets are queued first: any pump below sends them before the batch they precede (`InkEnd`).
        let begun = packets.iter().any(|p| matches!(p, LivePreview::Begin { .. }));
        for p in packets {
            s.live(p);
        }
        let ev = if contact { Lifecycle::Contact(busy) } else if begun { Lifecycle::Input } else { return s.tick(now, r) };
        s.at(now).on_io(SyncIo::Lifecycle(ev), r)
    });
}

/// v5 `redrawRemoteLiveStrokePreviews` for every page whose remote ink changed.
fn paint_overlays(app: &mut App, host: &mut dyn Host) {
    app.st.collab.overlay_armed = false;
    let Some(run) = app.st.collab.run.as_ref() else { return };
    app.st.collab.overlay_ver = run.sync.core.live.version;
    let mut pages_now: Vec<Id> = run.sync.core.live.strokes.values().filter(|s| !s.xy.is_empty()).map(|s| s.page).collect();
    ihn_core::sort::ord(&mut pages_now);
    pages_now.dedup();
    let mut todo = std::mem::replace(&mut app.st.collab.overlay_pages, pages_now.clone());
    todo.extend(pages_now);
    ihn_core::sort::ord(&mut todo);
    todo.dedup();
    for page in todo {
        let ink = pages::with_gfx(app, |app, g| {
            let a = pages::active(app, page)?;
            let run = app.st.collab.run.as_ref()?;
            let refs: Vec<StrokeRef> = run.sync.core.live.strokes.values().filter(|s| s.page == page).map(|s| StrokeRef::new(s.tool, &s.color, s.width, &s.xy)).collect();
            Some(overlay::remote_redraw(&mut g.cmds, &mut g.paths, a.remote_surf, a.cursor_px, a.cursor_scale, &refs))
        });
        if let Some(ink) = ink {
            pages::overlay_ink(app, host, page, 2, ink);
        }
    }
    pages::flush(app, host);
}

fn clear_overlays(app: &mut App, host: &mut dyn Host) {
    for page in std::mem::take(&mut app.st.collab.overlay_pages) {
        let done = pages::with_gfx(app, |app, g| {
            let a = pages::active(app, page)?;
            Some(overlay::remote_redraw(&mut g.cmds, &mut g.paths, a.remote_surf, a.cursor_px, a.cursor_scale, &[]))
        });
        if done.is_some() {
            pages::overlay_ink(app, host, page, 2, false);
        }
    }
    pages::flush(app, host);
}

// ---- host messages ------------------------------------------------------------------------------------------------

/// `host/peer.js` events (`chan::PEER`): `[u8 ev][u32 lo][u32 hi][rest]`, tab messages `[16][u32 n][name][data]`.
fn peer_message(app: &mut App, host: &mut dyn Host, data: &[u8]) {
    let Some(run) = app.st.collab.run.as_ref() else { return };
    let mut r = Reader::new(data);
    let Some(ev) = r.u8() else { return };
    if ev == 16 {
        let Some(name) = r.bytes() else { return };
        if name == run.bc.as_bytes() {
            let bytes = r.buf.to_vec();
            io(app, host, SyncIo::Tab(bytes));
        }
        return;
    }
    let (Some(lo), Some(hi)) = (r.u32(), r.u32()) else { return };
    let peer = (hi as u64) << 32 | lo as u64;
    let ch = |c: u8| if c == 1 { Channel::Bulk } else { Channel::Ctl };
    let e = match ev {
        1 => {
            let Some(kind) = r.u8() else { return };
            RtcEvent::Local { peer, kind: if kind == 1 { SdpKind::Offer } else { SdpKind::Answer }, sdp: String::from_utf8_lossy(r.buf).into_owned() }
        }
        2 => RtcEvent::ChannelOpen { peer, ch: ch(r.u8().unwrap_or(0)) },
        3 => RtcEvent::ChannelClosed { peer, ch: ch(r.u8().unwrap_or(0)) },
        4 => {
            let c = ch(r.u8().unwrap_or(0));
            RtcEvent::Message { peer, ch: c, bytes: r.buf.to_vec() }
        }
        5 => RtcEvent::Drained { peer },
        6 => RtcEvent::State { peer, state: r.u8().unwrap_or(4) },
        _ => return,
    };
    io(app, host, SyncIo::Rtc(e));
}

fn fx_done(app: &mut App, host: &mut dyn Host, io_ev: &crate::abi::Io) -> bool {
    let st = &mut app.st.collab;
    let Some(at) = st.fx.iter().position(|f| f.0 == io_ev.req) else { return false };
    let (_, gen, id, digest) = st.fx.swap_remove(at);
    if st.run.as_ref().is_none_or(|r| r.gen != gen) {
        return true;
    }
    let out = if io_ev.status >= 0 { io_ev.data.to_vec() } else { Vec::new() };
    io(app, host, if digest { SyncIo::Digest { id, out } } else { SyncIo::Deflate { id, out } });
    true
}

// ---- module contract ----------------------------------------------------------------------------------------------

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(app: &mut App, host: &mut dyn Host) { presence::init(app, host) }

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Msg(m) => match m.chan {
            session::sig::OPENED if session::get(app).is_some_and(|s| s.doc_id.as_bytes() == m.data) => opened(app, host),
            session::sig::CLOSED => {
                if app.st.collab.run.as_ref().is_some_and(|r| r.doc.as_bytes() == m.data) {
                    leave(app, host);
                }
            }
            session::sig::COMMITTED if app.st.collab.run.as_ref().is_some_and(|r| !r.leaving) => {
                let batches = session::drain_local(app);
                if !batches.is_empty() {
                    let now = host.now();
                    drive(app, host, |s, r| {
                        for b in &batches {
                            s.local_batch(b);
                        }
                        let mut e = s.at(now).on_io(SyncIo::Lifecycle(Lifecycle::Input), r);
                        e.extend(s.tick(now, r));
                        e
                    });
                }
            }
            session::sig::STATE => {
                let code = save_state::phase(app).code();
                let Some(run) = app.st.collab.run.as_mut().filter(|r| !r.leaving && r.phase_code != code) else { return 0 };
                run.phase_code = code;
                run.sync.core.profile.save_phase = code;
                // Queued, not pumped here: this signal can come from inside `session::edit`, before the pen's
                // `InkEnd` is out, and a pump would send the new batch ahead of it.
                run.sync.presence(own_presence(code));
                app.sched.timer(host.out(), T_TICK, 0.0);
            }
            shell::sig::VIEW if m.data != b"editor" => leave(app, host),
            shell::sig::SIGNED_OUT => stop(app, host),
            shell::sig::NETWORK => io(app, host, SyncIo::Lifecycle(if m.data == b"1" { Lifecycle::Online } else { Lifecycle::Offline })),
            chan::PEER => peer_message(app, host, m.data),
            _ => {}
        },
        Event::Life(l) => match l.code {
            life::VISIBILITY => io(app, host, SyncIo::Lifecycle(if l.a != 0.0 { Lifecycle::Hidden } else { Lifecycle::Visible })),
            life::PAGEHIDE => io(app, host, SyncIo::Lifecycle(Lifecycle::PageHide)),
            life::PAGESHOW => io(app, host, SyncIo::Lifecycle(Lifecycle::Visible)),
            life::ONLINE => io(app, host, SyncIo::Lifecycle(Lifecycle::Online)),
            life::OFFLINE => io(app, host, SyncIo::Lifecycle(Lifecycle::Offline)),
            _ => {}
        },
        Event::Timer(T_TICK) => kick(app, host),
        Event::Timer(T_LOCK) => {
            let Some(run) = app.st.collab.run.as_ref() else { return 0 };
            if !run.want_lock || run.sync.core.leader {
                return 0;
            }
            if session::get(app).is_some_and(|s| s.net_holder && s.doc_id == run.doc) {
                io(app, host, SyncIo::Locks(LockEvent::Net { held: true }));
            } else {
                app.sched.timer(host.out(), T_LOCK, 500.0);
            }
        }
        Event::Timer(T_RING) => presence::update(app, host),
        Event::Frame(_) => paint_overlays(app, host),
        Event::Io(io_ev) if io_ev.req == app.st.collab.kv_req.0 && io_ev.req != 0 => {
            app.st.collab.kv_req.0 = 0;
            start(app, host, &io_ev);
        }
        Event::Io(io_ev) => {
            fx_done(app, host, &io_ev);
        }
        Event::Dom(e) => return presence::dom(app, host, &e),
        _ => {}
    }
    0
}

/// `__IHN_TEST_API__.cmd('collab.<name>', args)`:
/// * `state` → `{active, doc, phase, leader, mode, peers:[{inst, name, link}], unsealed, peersConnected, live,
///   overlayPages, avatars}`;
/// * `live` → the remote strokes in progress: `[{page, points}]`.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    let st = &app.st.collab;
    match name {
        "state" => {
            let Some(run) = st.run.as_ref() else { return Some("{\"active\":false}".into()) };
            let c = &run.sync.core;
            let peers: Vec<String> = run
                .sync
                .peers()
                .iter()
                .map(|p| format!("{{\"inst\":\"{:016x}\",\"name\":{},\"link\":\"{:?}\"}}", p.inst, json_str(&p.name), p.link))
                .collect();
            let status = run.sync.status();
            Some(format!(
                "{{\"active\":true,\"doc\":{},\"inst\":\"{:016x}\",\"phase\":{},\"leader\":{},\"mode\":\"{:?}\",\"leaving\":{},\"peers\":[{}],\"unsealed\":{},\"peersConnected\":{},\"live\":{},\"overlayPages\":{},\"avatars\":{}}}",
                json_str(&run.doc),
                c.inst,
                json_str(&format!("{:?}", run.sync.phase())),
                c.leader,
                c.mode,
                run.leaving,
                peers.join(","),
                status.unsealed_local,
                status.peers_connected,
                c.live.strokes.len(),
                st.overlay_pages.len(),
                st.presence.shown_count(),
            ))
        }
        "live" => {
            let run = st.run.as_ref()?;
            let items: Vec<String> = run.sync.core.live.strokes.values().map(|s| format!("{{\"page\":\"{:032x}\",\"points\":{},\"ended\":{}}}", s.page.0, s.xy.len() / 2, s.item.is_some())).collect();
            Some(format!("[{}]", items.join(",")))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_roundtrip_through_stored() {
        let mut rec = DocRecord::default();
        let t = |r: SyncRecord, rec: &mut DocRecord| {
            let r = record_of(r, rec);
            let mut tx = StoreTx::new();
            tx.persist("a@b", "doc", rec, r);
        };
        let actor = ActorId::tab(0x1234_5600, 3);
        t(SyncRecord::RelayPlan { gen: 7, file_id: "F1".into(), epoch_day: 20400 }, &mut rec);
        let arch = ArchiveRef { file_id: "A1".into(), digest: [5; 12], ranges: vec![Range { actor, first: 1, last: 9 }] };
        t(SyncRecord::RelaySealed { sealed: vec![(actor, 9)], archives: vec![arch.clone()], assets: vec![], perm: 0xabcdef }, &mut rec);
        t(SyncRecord::RelayApplied { inst: 0x7700, gen: 3, d16: "abcdefghijklmnop".into() }, &mut rec);
        let mut vv = VersionVector::default();
        vv.raise(actor, 4);
        t(SyncRecord::KnownExport { head: "h1".into(), vv: vv.clone() }, &mut rec);
        t(SyncRecord::Flags { relay_pending: Some(true), export_pending: None, created: Some(true) }, &mut rec);
        let s = stored_of(&rec, &[Value::from(vec![Value::from("id1")]), Value::from("fold"), Value::from(true), Value::Undef]);
        assert_eq!((s.relay_gen, s.relay_file.as_str(), s.relay_epoch_day), (7, "F1", 20400));
        assert_eq!(s.sealed.get(&actor), Some(&9));
        assert_eq!(s.archives, vec![arch]);
        assert_eq!(s.perm, 0xabcdef);
        assert_eq!(s.applied.get(&0x7700), Some(&(3, "abcdefghijklmnop".to_string())));
        assert_eq!(s.exports, vec![("h1".to_string(), vv)]);
        assert!(s.created && s.relay_pending && s.no_link);
        assert_eq!((s.id_pool, s.relay_folder), (vec!["id1".to_string()], Some("fold".to_string())));
    }

    #[test]
    fn device_and_channel() {
        assert_eq!(channel_name("a@b", "doc1"), format!("ihn6:{:016x}:doc1", fnv1a64(b"a@b")));
        let d = Detached { doc_id: "d".into(), vv: VersionVector::default(), batches: Vec::new() };
        assert!(d.batches_since(&VersionVector::default(), 10).unwrap().is_empty());
    }
}
