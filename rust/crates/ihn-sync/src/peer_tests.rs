//! W2.8 acceptance (BUILD-PLAN): peers, signalling mail, anti-entropy and live ink, through the simulator.

use crate::mode::{Link, PRESENCE_MS, VOUCH_MS};
use crate::peer::{State, CONNECT_TIMEOUT_MS, DEAD_MS};
use crate::sim::*;
use crate::wire::{Frame, Mail};
use crate::*;
use ihn_core::codec::{Reader, Writer};
use ihn_core::doc::{LivePreview, PeerRef};
use ihn_core::id::Id;
use ihn_core::op::{Color, Tool};

const ANN: &str = "ann@example.org";
const BOB: &str = "bob@example.org";
const EVE: &str = "eve@example.org";

// -- a stand-in relay (the real one is W2.7's `relay.rs`) -------------------------------------------------------

/// Writes the own record and a relay file holding only the mailbox, with the 2 s cap; reads the files of changed
/// records (retrying while the file is older than its record), vouches for them and delivers the mail. It never
/// carries batches, so everything a test sees arrive came over P2P or the tabs.
#[derive(Default)]
struct Mailman {
    gen: u64,
    wrote_at: Option<f64>,
    writing: u32,
    left: bool,
    applied: BTreeMap<u64, String>,
    reads: BTreeMap<u32, (u64, String, u64)>,
    retry: Vec<(f64, u64, String, u64)>,
}

fn relay_file(inst: u64) -> String { format!("relay-{inst:x}") }
fn rec_gen(value: &str) -> u64 { value.split('|').nth(1).and_then(|g| g.parse().ok()).unwrap_or(0) }

impl Mailman {
    fn write(&mut self, cx: &mut Core, flags: &str, keepalive: bool) {
        self.gen += 1;
        (self.wrote_at, self.writing) = (Some(cx.now), 2);
        let mut w = Writer::default();
        w.u64(cx.inst);
        w.leb(self.gen);
        w.leb(cx.mail_out.len() as u64);
        cx.mail_out.iter().for_each(|m| m.encode(&mut w));
        let file = relay_file(cx.inst);
        let value = format!("1|{}|{}|{file}|AAAAAAAAAAAAAAAA|{flags}", self.gen, self.gen);
        cx.http(HttpReq::Update { file, bytes: w.buf, keepalive });
        cx.http(HttpReq::Props { file: cx.doc_id.clone(), set: vec![(record_key(cx.inst), Some(value))], app: Vec::new(), keepalive });
        (cx.record_due, cx.mail_changed) = (false, false);
    }
    fn read(&mut self, cx: &mut Core, inst: u64, value: String) {
        let gen = rec_gen(&value);
        let id = cx.http(HttpReq::Media { file: relay_file(inst) });
        self.reads.insert(id, (inst, value, gen));
    }
}

impl Part for Mailman {
    fn meta(&mut self, cx: &mut Core, _r: &mut dyn Replica) {
        let changed: Vec<(u64, String)> = cx.presence.remotes.iter().filter(|(i, r)| !r.value.is_empty() && self.applied.get(i) != Some(&r.value)).map(|(i, r)| (*i, r.value.clone())).collect();
        for (inst, value) in changed {
            self.applied.insert(inst, value.clone());
            self.read(cx, inst, value);
        }
    }
    fn http(&mut self, cx: &mut Core, id: u32, status: u16, _etag: &str, body: &[u8], _r: &mut dyn Replica) {
        let Some((inst, value, gen)) = self.reads.remove(&id) else {
            self.writing = self.writing.saturating_sub(1);
            return;
        };
        let r = &mut Reader::new(body);
        let file_gen = r.u64().and_then(|_| r.leb()).unwrap_or(0);
        if status != 200 || file_gen < gen {
            return self.retry.push((cx.now + 300.0, inst, value, gen));
        }
        let n = r.leb().unwrap_or(0);
        let mail: Vec<Mail> = (0..n).map_while(|_| Mail::decode(r)).collect();
        if cx.presence.remotes.get(&inst).is_some_and(|x| x.value == value) {
            cx.presence.verified(inst);
            cx.mail_in.extend(mail.into_iter().filter(|m| m.to == cx.inst).map(|m| (inst, m)));
        }
    }
    fn change(&mut self, cx: &mut Core, c: Change, _r: &mut dyn Replica) {
        if matches!(c, Change::Suspend | Change::Leave) && cx.networking() && cx.can_edit && !self.left && self.gen > 0 {
            self.left = true;
            self.write(cx, "l", true);
        }
        if c == Change::Visible {
            self.left = false;
        }
    }
    fn tick(&mut self, cx: &mut Core, _r: &mut dyn Replica) {
        for (at, inst, value, gen) in std::mem::take(&mut self.retry) {
            if cx.now >= at {
                self.read(cx, inst, value);
            } else {
                self.retry.push((at, inst, value, gen));
                cx.wake(at);
            }
        }
        if !cx.networking() || !cx.can_edit || !cx.visible || self.writing > 0 || self.left {
            return;
        }
        let last = self.wrote_at.unwrap_or(f64::NEG_INFINITY);
        let due = match cx.cadence().refresh {
            _ if cx.record_due || cx.mail_changed => cx.now,
            Some(every) if self.gen > 0 => last + every,
            _ => return,
        }
        .max(last + mode::WRITE_CAP_MS);
        if cx.now >= due {
            self.write(cx, "c", false);
        } else {
            cx.wake(due);
        }
    }
}

fn full() -> Vec<Box<dyn Part>> {
    let mut parts: Vec<Box<dyn Part>> = vec![Box::new(Mailman::default())];
    parts.extend([peer::part(), entropy::part(), live::part()].into_iter().flatten());
    parts
}

fn world(n: usize, skews: &[f64]) -> (Sim, String, Vec<usize>) {
    let mut sim = Sim::new(11);
    let doc = sim.drive.create_doc(ANN, "Notes");
    let accounts = [ANN, BOB, EVE];
    let mut devs = Vec::new();
    for i in 0..n {
        sim.drive.share(&doc, accounts[i], true);
        let d = sim.add_dev(accounts[i], 0x1000 * (i as u64 + 1), skews.get(i).copied().unwrap_or(0.0), full);
        let inst = sim.devs[d].inst;
        sim.drive.files.insert(relay_file(inst), DFile { owner: accounts[i].into(), link: true, ..Default::default() });
        devs.push(d);
    }
    (sim, doc, devs)
}

fn link(sim: &Sim, d: usize, other: usize) -> Link { sim.core(d).presence.link(sim.devs[other].inst) }
fn mode_of(sim: &Sim, d: usize) -> Option<Mode> { if let Phase::Live(m) = sim.phase(d) { Some(m) } else { None } }

/// Opens all devices, makes them write (an edit's relay flush) and waits until every pair is linked.
fn linked(sim: &mut Sim, doc: &str, devs: &[usize]) {
    for &d in devs {
        sim.open(d, doc);
    }
    sim.run(1000.0);
    for &d in devs {
        sim.sync(d).core.record_due = true;
        sim.tick(d);
    }
    let pairs: Vec<(usize, usize)> = devs.iter().flat_map(|&a| devs.iter().filter(move |&&b| b != a).map(move |&b| (a, b))).collect();
    assert!(sim.run_until(60_000.0, |s| pairs.iter().all(|&(a, b)| link(s, a, b) == Link::Open)), "not linked");
}

fn sent(sim: &Sim, d: usize, from: usize, pred: impl Fn(&Frame) -> bool) -> usize {
    sim.devs[d].effects[from..].iter().filter(|e| matches!(e, SyncEffect::Send { bytes, .. } if Frame::decode(bytes).is_some_and(|f| pred(&f)))).count()
}

// -- scenarios ------------------------------------------------------------------------------------------------

fn alone_company_linked(skews: &[f64]) {
    let (mut sim, doc, d) = world(2, skews);
    let (a, b) = (d[0], d[1]);
    sim.open(a, &doc);
    sim.run(1000.0);
    assert_eq!(mode_of(&sim, a), Some(Mode::Alone));
    sim.edit(a);
    sim.open(b, &doc);
    sim.run(500.0);
    // B writes (its first edit's flush): A sees it at its next poll, writes its record at once, both meet.
    sim.edit(b);
    sim.sync(b).core.record_due = true;
    sim.tick(b);
    assert!(sim.run_until(25_000.0, |s| mode_of(s, a) == Some(Mode::Company)));
    let t = sim.now;
    assert!(sim.run_until(30_000.0, |s| mode_of(s, a) == Some(Mode::Linked) && mode_of(s, b) == Some(Mode::Linked)));
    assert!(sim.now - t < 25_000.0, "setup took {} ms", sim.now - t);
    // The Hello exchange delivered both histories: equal states.
    assert!(sim.run_until(1000.0, |s| s.converged(&[a, b])));
    assert_eq!(sim.sync(a).peers()[0].link, ihn_core::doc::PeerLink::Connected);
    assert_eq!(sim.sync(a).peers()[0].name, "bob");
    assert!(sim.core(a).mail_out.is_empty() && sim.core(b).mail_out.is_empty(), "mail leaves once open");
    // First ink point at the peer: one link delay (+ scheduling), no Drive involved.
    let live = 77;
    let start = sim.now;
    sim.sync(a).live(LivePreview::Begin { live, page: Id(1), tool: Tool::Pen, color: Color::Hex(0x112233), width: 2.0, has_p: false });
    sim.sync(a).live(LivePreview::Points { live, from: 0, xy: vec![1.0, 2.0, 3.0, 4.0], p: None });
    sim.tick(a);
    while sim.core(b).live.strokes.get(&live).is_none_or(|s| s.xy.len() < 4) {
        sim.run(1.0);
        assert!(sim.now - start < 100.0);
    }
    assert!(sim.now - start <= sim.link_ms + 16.7, "{} ms", sim.now - start);
    // Pen up: the batch replaces the preview; states equal.
    let item = Id(0xabc);
    sim.sync(a).live(LivePreview::End { live, item });
    sim.edit(a);
    assert!(sim.run_until(500.0, |s| s.converged(&[a, b])));
    assert!(sim.core(b).live.strokes.contains_key(&live), "the edit's stroke is another item: the preview waits for its TTL");
    assert!(sim.run_until(16_000.0, |s| s.core(b).live.strokes.is_empty()));
    // Linked: 10 s polls.
    let t = sim.now - T0;
    sim.run(60_000.0);
    assert!((5..=8).contains(&sim.drive.count(a, "meta", t, t + 60_000.0)));
    assert_eq!(mode_of(&sim, a), Some(Mode::Linked));
}

#[test]
fn two_editors_alone_company_linked() { alone_company_linked(&[0.0, 0.0]); }

#[test]
fn ten_minutes_of_clock_skew_change_nothing() {
    alone_company_linked(&[600_000.0, -600_000.0]);
    alone_company_linked(&[-600_000.0, 600_000.0]);
}

#[test]
fn live_preview_is_dropped_by_its_batch_cancel_or_ttl() {
    let (mut sim, doc, d) = world(2, &[]);
    let (a, b) = (d[0], d[1]);
    linked(&mut sim, &doc, &d);
    let page = Id(5);
    let begin = |live| LivePreview::Begin { live, page, tool: Tool::Highlighter, color: Color::Hex(1), width: 9.0, has_p: true };
    // The real stroke: its batch puts the item named by InkEnd.
    sim.sync(a).live(begin(1));
    sim.sync(a).live(LivePreview::Points { live: 1, from: 0, xy: vec![0.0, 0.0, 1.0, 1.0], p: Some(vec![0.5, 0.6]) });
    sim.tick(a);
    sim.run(30.0);
    sim.sync(a).live(LivePreview::Points { live: 1, from: 1, xy: vec![1.0, 1.0, 2.0, 2.0], p: Some(vec![0.6, 0.7]) }); // overlaps one point
    sim.tick(a);
    sim.run(30.0);
    let s = sim.core(b).live.strokes[&1].clone();
    assert_eq!((s.xy, s.p, s.from), (vec![0.0, 0.0, 1.0, 1.0, 2.0, 2.0], Some(vec![0.5, 0.6, 0.7]), PeerRef::Peer(sim.devs[a].inst)));
    // Pen up: the stroke's batch is committed, InkEnd names its item, then the batch is handed to sync.
    let wall = sim.local(a) as u64;
    let Sim { devs, rng, .. } = &mut sim;
    let batch = devs[a].replica.edit(wall, rng);
    let item = batch.decode().unwrap().ops.iter().find_map(|op| if let ihn_core::op::Op::ItemPut { item, .. } = op { Some(*item) } else { None }).unwrap();
    sim.sync(a).live(LivePreview::End { live: 1, item });
    sim.sync(a).local_batch(&batch);
    sim.tick(a);
    sim.run(5.0);
    // InkEnd went out before the batch (live() is dispatched first), so the preview is gone with the batch.
    assert!(sim.run_until(200.0, |s| !s.core(b).live.strokes.contains_key(&1) && s.converged(&[a, b])));
    // Cancel.
    sim.sync(a).live(begin(2));
    sim.tick(a);
    sim.run(30.0);
    assert!(sim.core(b).live.strokes.contains_key(&2));
    sim.sync(a).live(LivePreview::Cancel { live: 2 });
    sim.tick(a);
    sim.run(30.0);
    assert!(sim.core(b).live.strokes.is_empty());
    // Erase previews: throttled to one per 50 ms, the last one always sent before EraseEnd.
    let mark = sim.devs[a].effects.len();
    for i in 0..10u64 {
        sim.sync(a).live(LivePreview::Erase { gesture: 3, page, hidden: (0..=i).map(|h| Id(h as u128)).collect(), fragments: Vec::new() });
        sim.tick(a);
        sim.run(10.0);
    }
    assert!(sim.core(b).live.erases.contains_key(&3));
    sim.sync(a).live(LivePreview::EraseEnd { gesture: 3 });
    sim.tick(a);
    sim.run(30.0);
    let erases = sent(&sim, a, mark, |f| matches!(f, Frame::Erase { .. })) / 2; // tabs + peer
    assert!((2..=4).contains(&erases), "{erases} erase frames in 100 ms");
    let last_hidden = sim.devs[a].effects[mark..].iter().rev().find_map(|e| match e {
        SyncEffect::Send { bytes, .. } => match Frame::decode(bytes) {
            Some(Frame::Erase { hidden, .. }) => Some(hidden.len()),
            _ => None,
        },
        _ => None,
    });
    assert_eq!(last_hidden, Some(10));
    assert!(sim.core(b).live.erases.is_empty());
    // An abandoned stroke expires after 15 s; points beyond 24,000 are not sent.
    sim.sync(a).live(begin(4));
    sim.sync(a).live(LivePreview::Points { live: 4, from: 23_999, xy: vec![0.0; 6], p: Some(vec![0.0; 3]) });
    sim.tick(a);
    sim.run(30.0);
    let v = sim.core(b).live.version;
    assert_eq!(sim.core(b).live.strokes[&4].xy.len(), 0, "a gap is not drawn");
    sim.run(14_000.0);
    assert!(sim.core(b).live.strokes.contains_key(&4));
    sim.run(1_100.0);
    assert!(sim.core(b).live.strokes.is_empty() && sim.core(b).live.version > v);
    let pts = sim.devs[a].effects.iter().rev().find_map(|e| match e {
        SyncEffect::Send { bytes, .. } => match Frame::decode(bytes) {
            Some(Frame::InkPts { xy, p, .. }) => Some((xy.len(), p.map(|p| p.len()))),
            _ => None,
        },
        _ => None,
    });
    assert_eq!(pts, Some((2, Some(1))));
    // Viewers draw nothing.
    sim.sync(b).core.can_edit = false;
    let mark = sim.devs[b].effects.len();
    sim.sync(b).live(begin(9));
    sim.tick(b);
    assert_eq!(sent(&sim, b, mark, |f| matches!(f, Frame::InkBegin { .. })), 0);
}

#[test]
fn p2p_blocked_backs_off_then_unreachable_and_retries_after_five_minutes() {
    let (mut sim, doc, d) = world(2, &[]);
    let (a, b) = (d[0], d[1]);
    sim.block_p2p(a, b);
    for x in [a, b] {
        sim.open(x, &doc);
    }
    sim.run(1000.0);
    for x in [a, b] {
        sim.sync(x).core.record_due = true;
        sim.tick(x);
    }
    assert!(sim.run_until(25_000.0, |s| link(s, a, b) == Link::Handshake));
    // Three handshakes time out (12 s each after the answer) with 1 s and 2 s backoffs in between.
    let t = sim.now;
    assert!(sim.run_until(120_000.0, |s| link(s, a, b) == Link::Unreachable));
    assert!(sim.now - t > 2.0 * CONNECT_TIMEOUT_MS && sim.now - t < 80_000.0, "{}", sim.now - t);
    assert_eq!(mode_of(&sim, a), Some(Mode::RelayOnly));
    // The answering side reaches the same verdict on its own.
    assert!(sim.run_until(60_000.0, |s| link(s, b, a) == Link::Unreachable));
    // 5 minutes later it tries again (the records stay fresh: relay-only pairs refresh every 60 s).
    let t = sim.now;
    assert!(sim.run_until(400_000.0, |s| link(s, a, b) == Link::Handshake));
    assert!(sim.now - t >= 299_000.0, "{}", sim.now - t);
    let creates = sim.devs[a].effects.iter().filter(|e| matches!(e, SyncEffect::Rtc(RtcReq::Create { offer: true, .. }))).count();
    assert!((4..=5).contains(&creates), "{creates} offers");
}

/// A session of one device with only the peer machine (and entropy), driven by hand.
struct Solo {
    sync: Sync,
    rep: SimReplica,
    now: f64,
    fx: Vec<SyncEffect>,
}

impl Solo {
    fn new(inst: u64, parts: Vec<Box<dyn Part>>) -> Solo {
        let cfg = Config { doc_id: "doc".into(), inst, actor: ActorId::tab(inst, 0), profile: Default::default(), can_edit: true, visible: true, online: true, stored: Stored { created: true, ..Default::default() }, seed: inst };
        let mut s = Solo { sync: Sync::with_parts(cfg, parts), rep: SimReplica::new("doc", ActorId::tab(inst, 0)), now: 1000.0, fx: Vec::new() };
        s.tick(1000.0);
        s.io(SyncIo::Locks(LockEvent::Net { held: true }));
        s.take();
        s
    }
    fn io(&mut self, io: SyncIo) {
        let fx = self.sync.at(self.now).on_io(io, &mut self.rep);
        self.fx.extend(fx);
    }
    fn tick(&mut self, now: f64) {
        self.now = now;
        let fx = self.sync.tick(now, &mut self.rep);
        self.fx.extend(fx);
    }
    /// Another installation's record, verified.
    fn sees(&mut self, inst: u64, gen: u32) {
        let value = format!("1|{gen}|{gen}|f|AAAAAAAAAAAAAAAA|c");
        let c = &mut self.sync.core;
        c.presence.observe([(inst, value.as_str())].into_iter(), self.now);
        c.presence.verified(inst);
    }
    fn take(&mut self) -> Vec<SyncEffect> { std::mem::take(&mut self.fx) }
    fn state(&self, inst: u64) -> Link { self.sync.core.presence.link(inst) }
    fn frames_to(&mut self, inst: u64) -> Vec<Frame> {
        self.take().into_iter().filter_map(|e| if let SyncEffect::Send { to: Dest::Peer(p), bytes, .. } = e { (p == inst).then(|| Frame::decode(&bytes)).flatten() } else { None }).collect()
    }
}

fn peer_parts() -> Vec<Box<dyn Part>> { [peer::part(), entropy::part()].into_iter().flatten().collect() }

const SMALL: u64 = 0x1000;
const LARGE: u64 = 0x2000;

#[test]
fn glare_the_smaller_offer_wins() {
    let mut s = Solo::new(SMALL, peer_parts());
    let mut l = Solo::new(LARGE, peer_parts());
    s.sees(LARGE, 1);
    l.sees(SMALL, 1);
    s.tick(1000.0);
    l.tick(1000.0);
    // The smaller installation offers at once; the larger waits 8 s for it.
    assert!(s.take().iter().any(|e| matches!(e, SyncEffect::Rtc(RtcReq::Create { peer: LARGE, offer: true, .. }))));
    assert!(!l.take().iter().any(|e| matches!(e, SyncEffect::Rtc(RtcReq::Create { .. }))));
    l.tick(9001.0);
    assert!(l.take().iter().any(|e| matches!(e, SyncEffect::Rtc(RtcReq::Create { peer: SMALL, offer: true, .. }))));
    s.now = 9001.0;
    s.io(SyncIo::Rtc(RtcEvent::Local { peer: LARGE, kind: SdpKind::Offer, sdp: "sdp-s".into() }));
    l.io(SyncIo::Rtc(RtcEvent::Local { peer: SMALL, kind: SdpKind::Offer, sdp: "sdp-l".into() }));
    let (ms, ml) = (s.sync.core.mail_out[0].clone(), l.sync.core.mail_out[0].clone());
    assert!(s.sync.core.mail_changed && l.sync.core.mail_changed);
    assert_eq!((ms.to, ms.kind, ms.ttl_ms, ml.to, ml.kind), (LARGE, 1, 60_000, SMALL, 1));
    s.take();
    l.take();
    // Both offers cross: the smaller ignores the larger's; the larger drops its own and answers.
    s.sync.core.mail_in.push((LARGE, ml.clone()));
    s.tick(9500.0);
    assert!(!s.take().iter().any(|e| matches!(e, SyncEffect::Rtc(_))));
    l.sync.core.mail_in.push((SMALL, ms.clone()));
    l.tick(9500.0);
    let fx = l.take();
    assert!(matches!(&fx[..3], [SyncEffect::Rtc(RtcReq::Close { peer: SMALL }), SyncEffect::Rtc(RtcReq::Create { offer: false, .. }), SyncEffect::Rtc(RtcReq::SetRemote { kind: SdpKind::Offer, .. })]), "{fx:?}");
    assert!(l.sync.core.mail_out.is_empty());
    l.io(SyncIo::Rtc(RtcEvent::Local { peer: SMALL, kind: SdpKind::Answer, sdp: "ans".into() }));
    let answer = l.sync.core.mail_out[0].clone();
    assert_eq!((answer.kind, answer.sid), (2, ms.sid));
    // A replayed or expired mail is ignored; the answer is applied.
    s.sync.core.mail_in.push((LARGE, ml));
    s.sync.core.mail_in.push((LARGE, answer.clone()));
    s.tick(9600.0);
    let fx = s.take();
    assert_eq!(fx.iter().filter(|e| matches!(e, SyncEffect::Rtc(RtcReq::SetRemote { kind: SdpKind::Answer, .. }))).count(), 1, "{fx:?}");
    s.sync.core.mail_in.push((LARGE, answer));
    s.tick(9700.0);
    assert!(!s.take().iter().any(|e| matches!(e, SyncEffect::Rtc(_))));
    // Channels open on both sides: Hello, checked; Open removes the mail.
    for (x, peer) in [(&mut s, LARGE), (&mut l, SMALL)] {
        x.io(SyncIo::Rtc(RtcEvent::ChannelOpen { peer, ch: Channel::Ctl }));
        x.io(SyncIo::Rtc(RtcEvent::ChannelOpen { peer, ch: Channel::Bulk }));
    }
    let hs = s.frames_to(LARGE);
    let hl = l.frames_to(SMALL);
    assert!(matches!(&hs[..], [Frame::Hello { inst: SMALL, .. }]) && matches!(&hl[..], [Frame::Hello { inst: LARGE, .. }]));
    s.io(SyncIo::Rtc(RtcEvent::Message { peer: LARGE, ch: Channel::Ctl, bytes: hl[0].encode() }));
    l.io(SyncIo::Rtc(RtcEvent::Message { peer: SMALL, ch: Channel::Ctl, bytes: hs[0].encode() }));
    assert_eq!((s.state(LARGE), l.state(SMALL)), (Link::Open, Link::Open));
    assert!(l.sync.core.mail_out.is_empty() && l.sync.core.mail_changed);
    // A Hello for another document or from another installation closes the connection.
    let mut x = Solo::new(SMALL, peer_parts());
    x.sees(LARGE, 1);
    x.tick(1000.0);
    x.io(SyncIo::Rtc(RtcEvent::Local { peer: LARGE, kind: SdpKind::Offer, sdp: "o".into() }));
    let sid = x.sync.core.mail_out[0].sid;
    x.sync.core.mail_in.push((LARGE, Mail { to: SMALL, sid, kind: 2, ttl_ms: 60_000, sdp: "a".into() }));
    x.tick(1100.0);
    x.io(SyncIo::Rtc(RtcEvent::ChannelOpen { peer: LARGE, ch: Channel::Ctl }));
    x.io(SyncIo::Rtc(RtcEvent::ChannelOpen { peer: LARGE, ch: Channel::Bulk }));
    let Frame::Hello { vv, profile, presence, .. } = x.frames_to(LARGE).remove(0) else { panic!() };
    let wrong = Frame::Hello { doc_id: "other".into(), inst: LARGE, vv, profile, presence };
    x.io(SyncIo::Rtc(RtcEvent::Message { peer: LARGE, ch: Channel::Ctl, bytes: wrong.encode() }));
    assert!(x.take().iter().any(|e| matches!(e, SyncEffect::Rtc(RtcReq::Close { peer: LARGE }))));
    assert_eq!(x.state(LARGE), Link::Handshake, "backoff after a failed handshake");
}

/// `s` and `l` connected by hand; returns them Open.
fn open_pair() -> (Solo, Solo) {
    let mut s = Solo::new(SMALL, peer_parts());
    let mut l = Solo::new(LARGE, peer_parts());
    s.sees(LARGE, 1);
    l.sees(SMALL, 1);
    s.tick(1000.0);
    s.io(SyncIo::Rtc(RtcEvent::Local { peer: LARGE, kind: SdpKind::Offer, sdp: "o".into() }));
    l.sync.core.mail_in.push((SMALL, s.sync.core.mail_out[0].clone()));
    l.tick(1000.0);
    l.io(SyncIo::Rtc(RtcEvent::Local { peer: SMALL, kind: SdpKind::Answer, sdp: "a".into() }));
    s.sync.core.mail_in.push((LARGE, l.sync.core.mail_out[0].clone()));
    s.tick(1000.0);
    for (x, peer) in [(&mut s, LARGE), (&mut l, SMALL)] {
        x.io(SyncIo::Rtc(RtcEvent::ChannelOpen { peer, ch: Channel::Ctl }));
        x.io(SyncIo::Rtc(RtcEvent::ChannelOpen { peer, ch: Channel::Bulk }));
    }
    let (hs, hl) = (s.frames_to(LARGE), l.frames_to(SMALL));
    s.io(SyncIo::Rtc(RtcEvent::Message { peer: LARGE, ch: Channel::Ctl, bytes: hl[0].encode() }));
    l.io(SyncIo::Rtc(RtcEvent::Message { peer: SMALL, ch: Channel::Ctl, bytes: hs[0].encode() }));
    assert_eq!((s.state(LARGE), l.state(SMALL)), (Link::Open, Link::Open));
    s.take();
    l.take();
    (s, l)
}

#[test]
fn unacknowledged_batches_are_resent_after_five_seconds_and_acks_stop_it() {
    let (mut s, mut l) = open_pair();
    let mut rng = ihn_core::testutil::Rng::new(1);
    let b1 = s.rep.edit(1000, &mut rng);
    s.sync.local_batch(&b1);
    s.tick(1100.0);
    let sent1: Vec<Frame> = s.frames_to(LARGE);
    assert!(matches!(&sent1[..], [Frame::Batches(b)] if b == &vec![b1.clone()]));
    // Lost: no Have. Nothing is resent before 5 s, then the same batch goes again.
    s.tick(5000.0);
    assert!(s.frames_to(LARGE).iter().all(|f| !matches!(f, Frame::Batches(_))));
    s.tick(6100.0);
    let again = s.frames_to(LARGE);
    assert!(matches!(&again[..], [Frame::Batches(b)] if b == &vec![b1.clone()]), "{again:?}");
    // Delivered now: the receiver applies it and acknowledges 200 ms later; then nothing more is sent.
    l.now = 6120.0;
    l.io(SyncIo::Rtc(RtcEvent::Message { peer: SMALL, ch: Channel::Ctl, bytes: again[0].encode() }));
    assert!(l.rep.doc.vv.contains(b1.header().unwrap().0, 1));
    assert!(l.frames_to(SMALL).iter().all(|f| !matches!(f, Frame::Have(_))));
    l.tick(6330.0);
    let have = l.frames_to(SMALL);
    assert!(matches!(&have[..], [Frame::Have(vv)] if vv.contains(b1.header().unwrap().0, 1)), "{have:?}");
    s.now = 6350.0;
    s.io(SyncIo::Rtc(RtcEvent::Message { peer: LARGE, ch: Channel::Ctl, bytes: have[0].encode() }));
    s.tick(12_000.0);
    assert!(s.frames_to(LARGE).iter().all(|f| !matches!(f, Frame::Batches(_))));
    assert_eq!(s.sync.core.peer_vv.get(&PeerRef::Peer(LARGE)).map(|v| v.get(b1.header().unwrap().0)), Some(1));
    // Health: a Ping after 2 s of silence; 8 s without anything received ⇒ closed, and the smaller side offers again.
    let quiet = s.now;
    s.tick(quiet + 2100.0);
    assert!(s.frames_to(LARGE).iter().any(|f| matches!(f, Frame::Ping(_))));
    s.tick(quiet + DEAD_MS + 100.0);
    let fx = s.take();
    assert!(fx.iter().any(|e| matches!(e, SyncEffect::Rtc(RtcReq::Close { peer: LARGE }))) && fx.iter().any(|e| matches!(e, SyncEffect::Rtc(RtcReq::Create { peer: LARGE, offer: true, .. }))), "{fx:?}");
    // Not vouched any more (the record is older than 10 min) ⇒ dropped, with Bye.
    let (mut s, _l) = open_pair();
    s.tick(1000.0 + VOUCH_MS + 1.0);
    let fx = s.take();
    assert!(fx.iter().any(|e| matches!(e, SyncEffect::Send { to: Dest::Peer(LARGE), bytes, .. } if bytes[0] == 0x22)));
    assert!(fx.iter().any(|e| matches!(e, SyncEffect::Rtc(RtcReq::Close { peer: LARGE }))));
    assert_eq!(s.state(LARGE), Link::Idle);
    assert!(s.sync.core.peer_vv.is_empty());
}

#[test]
fn network_change_probes_open_channels() {
    let (mut s, mut l) = open_pair();
    s.now = 1500.0;
    s.io(SyncIo::Lifecycle(Lifecycle::NetChange));
    let ping = s.frames_to(LARGE).into_iter().find(|f| matches!(f, Frame::Ping(_))).expect("ping");
    l.now = 1520.0;
    l.io(SyncIo::Rtc(RtcEvent::Message { peer: SMALL, ch: Channel::Ctl, bytes: ping.encode() }));
    let pong = l.frames_to(SMALL).into_iter().find(|f| matches!(f, Frame::Pong(_))).expect("pong");
    s.now = 1540.0;
    s.io(SyncIo::Rtc(RtcEvent::Message { peer: LARGE, ch: Channel::Ctl, bytes: pong.encode() }));
    s.tick(4000.0);
    assert_eq!(s.state(LARGE), Link::Open);
    // No Pong within clamp(3 × RTT + 200, 300, 2000) = 320 ms ⇒ closed and retried at once.
    s.io(SyncIo::Lifecycle(Lifecycle::NetChange));
    s.take();
    s.tick(4330.0);
    let fx = s.take();
    assert!(fx.iter().any(|e| matches!(e, SyncEffect::Rtc(RtcReq::Close { peer: LARGE }))) && fx.iter().any(|e| matches!(e, SyncEffect::Rtc(RtcReq::Create { offer: true, .. }))));
    // Suspend / demotion close with Bye.
    let (mut s, _l) = open_pair();
    s.sync.core.can_edit = false;
    s.sync.change(Change::CanEdit(false), &mut s.rep);
    s.tick(4500.0);
    assert_eq!(s.state(LARGE), Link::Idle);
    assert!(s.frames_to(LARGE).iter().any(|f| matches!(f, Frame::Bye(peer::BYE_REVOKED))));
}

#[test]
fn compacted_only_beyond_the_journal() {
    let (mut sim, doc, d) = world(2, &[]);
    let (a, b) = (d[0], d[1]);
    linked(&mut sim, &doc, &d);
    let (ia, ib) = (sim.devs[a].inst, sim.devs[b].inst);
    for _ in 0..3 {
        sim.edit(a);
    }
    assert!(sim.run_until(1000.0, |s| s.converged(&[a, b])));
    // Retention needs an export and the seal; give A both (W2.7's job) for everything so far.
    let actor = sim.devs[a].replica.author.actor;
    let all = sim.devs[a].replica.doc.vv.clone();
    let core = &mut sim.sync(a).core;
    core.persist(SyncRecord::KnownExport { head: "rev-1".into(), vv: all.clone() });
    core.persist(SyncRecord::RelaySealed { sealed: vec![(actor, 3)], archives: Vec::new(), assets: Vec::new(), perm: 0 });
    // A short loss of the channel: B's VV still holds A's journal; the reconnect sends batches, not a snapshot.
    sim.run(11_000.0);
    sim.io(b, SyncIo::Rtc(RtcEvent::Fault { peer: ia }));
    sim.sync(b).core.rtc(RtcReq::Close { peer: ia });
    sim.tick(b);
    sim.edit(a);
    sim.edit(a);
    let mark = sim.devs[a].effects.len();
    assert!(sim.run_until(40_000.0, |s| link(s, a, b) == Link::Open && s.converged(&[a, b])));
    assert_eq!(sent(&sim, a, mark, |f| matches!(f, Frame::Snapshot(_))), 0);
    assert!(sent(&sim, a, mark, |f| matches!(f, Frame::Batches(_))) > 0);
    // B gone for longer than the vouching window: its VV no longer holds the journal; A seals and exports what it
    // wrote meanwhile, retention drops it, and B's reconnect gets a snapshot.
    sim.crash(b);
    for _ in 0..2 {
        sim.edit(a);
    }
    let all = sim.devs[a].replica.doc.vv.clone();
    let core = &mut sim.sync(a).core;
    core.persist(SyncRecord::KnownExport { head: "rev-2".into(), vv: all.clone() });
    core.persist(SyncRecord::RelaySealed { sealed: vec![(actor, all.get(actor))], archives: Vec::new(), assets: Vec::new(), perm: 0 });
    sim.run(VOUCH_MS + 60_000.0);
    assert!(!sim.core(a).peer_vv.contains_key(&PeerRef::Peer(ib)));
    assert!(sim.devs[a].replica.journal.keys().all(|k| k.0 != actor), "journal retained");
    let mark = sim.devs[a].effects.len();
    sim.open(b, &doc);
    sim.run(1000.0);
    sim.sync(b).core.record_due = true;
    sim.tick(b);
    assert!(sim.run_until(60_000.0, |s| link(s, a, b) == Link::Open && s.converged(&[a, b])));
    assert_eq!(sent(&sim, a, mark, |f| matches!(f, Frame::Snapshot(_))), 1);
}

#[test]
fn forwarding_over_a_partial_mesh_and_tabs() {
    let (mut sim, doc, d) = world(3, &[]);
    let (a, b, c) = (d[0], d[1], d[2]);
    sim.block_p2p(a, c);
    let a2 = sim.add_sibling(a);
    for x in [a, b, c, a2] {
        sim.open(x, &doc);
    }
    sim.run(1000.0);
    for x in [a, b, c] {
        sim.sync(x).core.record_due = true;
        sim.tick(x);
    }
    assert!(sim.run_until(60_000.0, |s| link(s, a, b) == Link::Open && link(s, b, c) == Link::Open));
    assert_ne!(link(&sim, a, c), Link::Open);
    // A's edit reaches C through B (the relay stand-in carries no batches); C's reaches A's other tab.
    sim.edit(a);
    assert!(sim.run_until(300.0, |s| s.converged(&[a, b, c, a2])));
    sim.edit(c);
    assert!(sim.run_until(300.0, |s| s.converged(&[a, b, c, a2])));
    // The non-leader tab's edit reaches the peers through the leader tab.
    assert!(!sim.core(a2).leader);
    sim.edit(a2);
    assert!(sim.run_until(300.0, |s| s.converged(&[a, b, c, a2])));
    assert!(sim.devs[c].replica.received.iter().all(|r| *r == PeerRef::Peer(sim.devs[b].inst)));
    // Live ink from C is drawn by B, and by A's tabs only if they are direct peers (not forwarded further).
    sim.sync(c).live(LivePreview::Begin { live: 5, page: Id(1), tool: Tool::Pen, color: Color::Hex(0), width: 1.0, has_p: false });
    sim.tick(c);
    sim.run(100.0);
    assert!(sim.core(b).live.strokes.contains_key(&5) && !sim.core(a).live.strokes.contains_key(&5));
    sim.sync(a).live(LivePreview::Begin { live: 6, page: Id(1), tool: Tool::Pen, color: Color::Hex(0), width: 1.0, has_p: false });
    sim.tick(a);
    sim.run(100.0);
    assert!(sim.core(a2).live.strokes.contains_key(&6) && sim.core(b).live.strokes.contains_key(&6));
    // A peer's ink also reaches the leader's other tabs.
    sim.sync(b).live(LivePreview::Begin { live: 7, page: Id(1), tool: Tool::Pen, color: Color::Hex(0), width: 1.0, has_p: false });
    sim.tick(b);
    sim.run(100.0);
    assert!(sim.core(a2).live.strokes.contains_key(&7));
    // A new tab gets the history from the open tabs (Hello → Have + batches).
    let a3 = sim.add_sibling(a);
    sim.open(a3, &doc);
    assert!(sim.run_until(300.0, |s| s.converged(&[a, a3])));
    // A–C never connect: after three failures that pair is relay-only.
    assert!(sim.run_until(120_000.0, |s| link(s, a, c) == Link::Unreachable));
    assert_eq!(mode_of(&sim, a), Some(Mode::RelayOnly));
}

#[test]
fn crash_points_of_the_peer_machine() {
    let (mut sim, doc, d) = world(2, &[]);
    let (a, b) = (d[0], d[1]);
    let ia = sim.devs[a].inst;
    for x in [a, b] {
        sim.open(x, &doc);
    }
    sim.run(1000.0);
    for x in [a, b] {
        sim.sync(x).core.record_due = true;
        sim.tick(x);
    }
    // B crashes right after asking for its answer; restarted, it answers A's next offer and they connect.
    sim.crash_on(b, |e| matches!(e, SyncEffect::Rtc(RtcReq::Answer { .. })), true);
    assert!(sim.run_until(30_000.0, |s| s.devs[b].sync.is_none()));
    sim.run(500.0);
    sim.open(b, &doc);
    sim.run(500.0);
    sim.sync(b).core.record_due = true;
    sim.tick(b);
    assert!(sim.run_until(60_000.0, |s| link(s, a, b) == Link::Open && link(s, b, a) == Link::Open));
    // A crashes while open: B notices within 8 s; edits made meanwhile arrive after A's restart.
    sim.crash(a);
    let t = sim.now;
    assert!(sim.run_until(DEAD_MS + 3000.0, |s| link(s, b, a) != Link::Open));
    assert!(sim.now - t <= DEAD_MS + 2100.0);
    sim.edit(b);
    sim.open(a, &doc);
    sim.run(500.0);
    sim.sync(a).core.record_due = true;
    sim.tick(a);
    assert!(sim.run_until(60_000.0, |s| link(s, a, b) == Link::Open && s.converged(&[a, b])));
    // Crash after the offer was posted (the mail is in a relay file that is never superseded): the peer's
    // answer to that dead offer is harmless, and the restarted device connects anew.
    let c = sim.devs[b].effects.len();
    sim.crash(b);
    sim.run(DEAD_MS + 2000.0);
    sim.crash_on(a, |e| matches!(e, SyncEffect::Http { req: HttpReq::Update { .. }, .. }), true);
    sim.open(b, &doc);
    sim.run(500.0);
    sim.sync(b).core.record_due = true;
    sim.tick(b);
    assert!(sim.run_until(60_000.0, |s| s.devs[a].sync.is_none()));
    sim.open(a, &doc);
    sim.run(500.0);
    sim.sync(a).core.record_due = true;
    sim.tick(a);
    assert!(sim.run_until(90_000.0, |s| link(s, a, b) == Link::Open && link(s, b, a) == Link::Open && s.converged(&[a, b])));
    assert!(sim.devs[b].effects.len() > c);
    let _ = ia;
}

#[test]
fn present_window_and_peers_survive_presence_expiry_while_open() {
    // Linked peers keep refreshing their records every 60 s, so they stay vouched for hours.
    let (mut sim, doc, d) = world(2, &[]);
    linked(&mut sim, &doc, &d);
    sim.run(3.0 * PRESENCE_MS + VOUCH_MS);
    assert_eq!((link(&sim, d[0], d[1]), mode_of(&sim, d[0])), (Link::Open, Some(Mode::Linked)));
    assert!(sim.devs[d[0]].effects.iter().all(|e| !matches!(e, SyncEffect::Rtc(RtcReq::Close { .. }))));
    let _ = State::Open;
}
