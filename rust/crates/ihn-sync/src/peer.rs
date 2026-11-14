//! Per-peer connection machine, glare, backoff, signalling mail, vouching and health (SYNC §10.1–§10.4, §10.8).
//! Owner: W2.8.
//!
//! One [`Conn`] per remote installation. Only the network-leader tab of an editor holds peers, and only with
//! installations that are present and vouched (§10.1). The SDP travels as [`Mail`] inside the sealed relay files:
//! this part writes `Core::mail_out` (the relay writer publishes it) and drains `Core::mail_in` (the relay reader
//! fills it from verified files). Every time here is this device's own clock.

use crate::mode::Link;
use crate::wire::{Frame, Mail, MAIL_TTL_MS};
use crate::{Change, Channel, Core, Dest, Part, Phase, RtcEvent, RtcReq, SdpKind};
use ihn_core::doc::{PeerRef, PresenceMode, PresenceState, Replica};
use ihn_core::id::Id;
use ihn_core::op::Color;
use std::collections::BTreeMap;

/// The four Google STUN servers of 5.x (`live-collaboration-v5.js`).
pub const ICE: [&str; 4] = ["stun:stun.l.google.com:19302", "stun:stun1.l.google.com:19302", "stun:stun2.l.google.com:19302", "stun:stun3.l.google.com:19302"];
pub const OFFER_TIMEOUT_MS: f64 = 15_000.0;
pub const CONNECT_TIMEOUT_MS: f64 = 12_000.0;
/// The larger installation offers only if no offer arrived this long after it first saw the other.
pub const RESPONDER_AFTER_MS: f64 = 8_000.0;
/// A new record generation is unverified until its file is read; meanwhile the previous vouching holds this long.
pub const VOUCH_GRACE_MS: f64 = 30_000.0;
pub const BACKOFF_BASE_MS: f64 = 1_000.0;
pub const BACKOFF_MAX_MS: f64 = 60_000.0;
pub const UNREACHABLE_AFTER: u32 = 3;
pub const UNREACHABLE_MS: f64 = 300_000.0;
pub const PING_IDLE_MS: f64 = 2_000.0;
pub const DEAD_MS: f64 = 8_000.0;

/// `Bye` reasons.
pub const BYE_LEAVE: u8 = 1;
pub const BYE_REVOKED: u8 = 2;
pub const BYE_OFFLINE: u8 = 3;

/// SYNC §10.4.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum State {
    #[default]
    Idle,
    Offering,
    Answering,
    Connecting,
    Open,
    Backoff,
    Unreachable,
}

#[derive(Default, Debug)]
pub struct Conn {
    pub state: State,
    /// Signalling session: our offer's, or the offer we answer.
    pub sid: u64,
    /// Deadline of Offering/Answering/Connecting, end of Backoff/Unreachable.
    pub until: f64,
    /// Consecutive failures.
    pub fails: u32,
    /// Local time at which the installation was first seen eligible (responder rule).
    seen_at: Option<f64>,
    /// Bit 0: control channel open, bit 1: bulk channel open.
    chans: u8,
    hello: bool,
    rx_at: f64,
    ping_at: f64,
    /// Network-change probe: ping value and deadline.
    probe: Option<(u32, f64)>,
    rtt: f64,
}

#[derive(Default)]
pub struct Peers {
    pub conns: BTreeMap<u64, Conn>,
    /// `(from, sid, kind)` → (first local sighting, processed).
    seen: BTreeMap<(u64, u64, u8), (f64, bool)>,
    /// Own mail entries `(to, sid, kind)` → local time posted.
    posted: BTreeMap<(u64, u64, u8), f64>,
    /// Last local time each installation was vouched (bridges the moments between a new record generation and
    /// the digest check of its file).
    vouched_at: BTreeMap<u64, f64>,
    sids: u64,
}

pub fn part() -> Option<Box<dyn Part>> { Some(Box::new(Peers::default())) }

/// The own presence for `Hello` (5.18.0 default: viewing, no page).
pub fn own_presence(cx: &Core) -> PresenceState {
    cx.own_presence.clone().unwrap_or(PresenceState { mode: PresenceMode::Viewing, page: Id(0), tool_colour: Color::Hex(0), save_phase: 0 })
}

/// Our `Hello` for a peer or the tabs.
pub fn hello(cx: &Core, r: &dyn Replica) -> Frame {
    Frame::Hello { doc_id: cx.doc_id.clone(), inst: cx.inst, vv: r.version_vector().clone(), profile: cx.profile.clone(), presence: own_presence(cx) }
}

fn link_of(s: State) -> Link {
    match s {
        State::Idle => Link::Idle,
        State::Open => Link::Open,
        State::Unreachable => Link::Unreachable,
        _ => Link::Handshake,
    }
}

impl Peers {
    /// May hold a channel with `inst` at all (§10.1, §10.4).
    fn allowed(&self, cx: &Core, inst: u64) -> bool {
        let vouched = cx.presence.vouched(inst, cx.now)
            || (self.vouched_at.get(&inst).is_some_and(|t| cx.now - t <= VOUCH_GRACE_MS) && cx.presence.remotes.get(&inst).is_some_and(|r| !r.value.is_empty() && cx.now - r.seen_at <= crate::mode::VOUCH_MS));
        vouched && cx.networking() && cx.can_edit && !matches!(cx.phase(), Phase::Suspended | Phase::Leaving | Phase::Closed)
    }

    fn set(cx: &mut Core, inst: u64, c: &mut Conn, state: State, until: f64) {
        c.state = state;
        c.until = until;
        cx.presence.set_link(inst, link_of(state));
    }

    fn unpost(&mut self, cx: &mut Core, to: u64) {
        let before = cx.mail_out.len();
        cx.mail_out.retain(|m| m.to != to);
        self.posted.retain(|k, _| k.0 != to);
        cx.mail_changed |= cx.mail_out.len() != before;
    }

    fn post(&mut self, cx: &mut Core, to: u64, sid: u64, kind: SdpKind, sdp: String) {
        self.unpost(cx, to);
        cx.mail_out.push(Mail { to, sid, kind: kind as u8, ttl_ms: MAIL_TTL_MS, sdp });
        self.posted.insert((to, sid, kind as u8), cx.now);
        cx.mail_changed = true;
    }

    fn offer(&mut self, cx: &mut Core, inst: u64) {
        self.sids += 1;
        let sid = cx.jitter().to_bits() ^ self.sids << 52 ^ cx.inst;
        let c = self.conns.entry(inst).or_default();
        (c.sid, c.chans, c.hello, c.probe) = (sid, 0, false, None);
        Self::set(cx, inst, c, State::Offering, cx.now + OFFER_TIMEOUT_MS);
        cx.rtc(RtcReq::Create { peer: inst, ice: ICE.map(String::from).to_vec(), offer: true });
    }

    /// Ends a connection. `failed`: a handshake failure (backoff, 3 ⇒ unreachable); otherwise a transient loss or
    /// an orderly end: back to Idle, so the first retry is immediate (§10.4 fast reconnect).
    fn close(&mut self, cx: &mut Core, inst: u64, failed: bool, bye: Option<u8>) {
        let Some(c) = self.conns.get_mut(&inst) else { return };
        if c.state == State::Idle || c.state == State::Backoff || c.state == State::Unreachable {
            return;
        }
        if let (Some(reason), State::Open) = (bye, c.state) {
            cx.send(Dest::Peer(inst), &Frame::Bye(reason));
        }
        cx.rtc(RtcReq::Close { peer: inst });
        (c.chans, c.hello, c.probe) = (0, false, None);
        if failed {
            c.fails += 1;
            if c.fails >= UNREACHABLE_AFTER {
                Self::set(cx, inst, c, State::Unreachable, cx.now + UNREACHABLE_MS);
            } else {
                let wait = cx.backoff(BACKOFF_BASE_MS, BACKOFF_MAX_MS, c.fails);
                Self::set(cx, inst, c, State::Backoff, cx.now + wait);
            }
        } else {
            (c.fails, c.seen_at) = (0, None);
            Self::set(cx, inst, c, State::Idle, 0.0);
        }
        self.unpost(cx, inst);
    }

    fn close_all(&mut self, cx: &mut Core, bye: u8) {
        let insts: Vec<u64> = self.conns.keys().copied().collect();
        for inst in insts {
            self.close(cx, inst, false, Some(bye));
        }
    }

    fn mail(&mut self, cx: &mut Core, from: u64, m: Mail) {
        if m.to != cx.inst {
            return;
        }
        let now = cx.now;
        let allowed = self.allowed(cx, from);
        let entry = self.seen.entry((from, m.sid, m.kind)).or_insert((now, false));
        if entry.1 || now - entry.0 >= m.ttl_ms as f64 {
            entry.1 = true;
            return;
        }
        if !allowed {
            return; // not (yet) vouched: a later sighting within the TTL may still be accepted
        }
        entry.1 = true;
        let c = self.conns.entry(from).or_default();
        if m.kind == SdpKind::Offer as u8 {
            if c.state == State::Offering && cx.inst < from {
                return; // glare: the smaller side keeps its own offer
            }
            if c.state != State::Idle {
                cx.rtc(RtcReq::Close { peer: from }); // the larger side drops its own offer; a restarted peer
                self.unpost(cx, from);
            }
            let c = self.conns.entry(from).or_default();
            (c.sid, c.chans, c.hello, c.probe) = (m.sid, 0, false, None);
            Self::set(cx, from, c, State::Answering, now + CONNECT_TIMEOUT_MS);
            cx.rtc(RtcReq::Create { peer: from, ice: ICE.map(String::from).to_vec(), offer: false });
            cx.rtc(RtcReq::SetRemote { peer: from, kind: SdpKind::Offer, sdp: m.sdp });
            cx.rtc(RtcReq::Answer { peer: from });
        } else if c.state == State::Offering && c.sid == m.sid {
            Self::set(cx, from, c, State::Connecting, now + CONNECT_TIMEOUT_MS);
            cx.rtc(RtcReq::SetRemote { peer: from, kind: SdpKind::Answer, sdp: m.sdp });
        }
    }
}

impl Part for Peers {
    fn rtc(&mut self, cx: &mut Core, ev: &RtcEvent, r: &mut dyn Replica) {
        match ev {
            RtcEvent::Local { peer, kind, sdp } => {
                let Some(c) = self.conns.get_mut(peer) else { return };
                match (c.state, kind) {
                    (State::Offering, SdpKind::Offer) => {
                        let sid = c.sid;
                        self.post(cx, *peer, sid, SdpKind::Offer, sdp.clone());
                    }
                    (State::Answering, SdpKind::Answer) => {
                        let sid = c.sid;
                        Self::set(cx, *peer, c, State::Connecting, cx.now + CONNECT_TIMEOUT_MS);
                        self.post(cx, *peer, sid, SdpKind::Answer, sdp.clone());
                    }
                    _ => {}
                }
            }
            RtcEvent::ChannelOpen { peer, ch } => {
                let Some(c) = self.conns.get_mut(peer).filter(|c| matches!(c.state, State::Answering | State::Connecting)) else { return };
                c.chans |= if *ch == Channel::Ctl { 1 } else { 2 };
                if c.chans == 3 && !c.hello {
                    (c.hello, c.rx_at, c.ping_at) = (true, cx.now, cx.now);
                    let f = hello(cx, r);
                    cx.send(Dest::Peer(*peer), &f);
                }
            }
            RtcEvent::ChannelClosed { peer, .. } | RtcEvent::State { peer, state: 4 | 5 } | RtcEvent::Fault { peer } => {
                let failed = self.conns.get(peer).is_some_and(|c| c.state != State::Open);
                self.close(cx, *peer, failed, None);
            }
            _ => {}
        }
    }

    fn frame(&mut self, cx: &mut Core, from: PeerRef, f: &Frame, r: &mut dyn Replica) {
        let PeerRef::Peer(inst) = from else { return };
        let now = cx.now;
        let Some(c) = self.conns.get_mut(&inst).filter(|c| c.state != State::Idle) else { return };
        c.rx_at = now;
        match f {
            Frame::Hello { doc_id, inst: claimed, profile, presence, .. } => {
                if *doc_id != cx.doc_id || *claimed != inst || !matches!(c.state, State::Answering | State::Connecting | State::Open) {
                    return self.close(cx, inst, true, None);
                }
                if !c.hello {
                    c.hello = true;
                    let h = hello(cx, r);
                    cx.send(Dest::Peer(inst), &h);
                }
                (c.fails, c.ping_at) = (0, now);
                Self::set(cx, inst, c, State::Open, 0.0);
                self.unpost(cx, inst);
                let remote = cx.presence.remotes.entry(inst).or_default();
                (remote.profile, remote.presence) = (Some(profile.clone()), Some(presence.clone()));
            }
            _ if c.state != State::Open => {}
            Frame::Ping(t) => cx.send(Dest::Peer(inst), &Frame::Pong(*t)),
            Frame::Pong(t) => {
                if let Some((sent, _)) = c.probe.filter(|p| p.0 == *t) {
                    c.rtt = (now as u64 as u32).wrapping_sub(sent) as f64;
                    c.probe = None;
                }
            }
            Frame::Bye(_) => self.close(cx, inst, false, None),
            Frame::Presence(p) => cx.presence.remotes.entry(inst).or_default().presence = Some(p.clone()),
            _ => {}
        }
    }

    fn change(&mut self, cx: &mut Core, c: Change, _r: &mut dyn Replica) {
        match c {
            Change::Suspend | Change::Leave => self.close_all(cx, BYE_LEAVE),
            Change::Leader(false) => self.close_all(cx, BYE_LEAVE),
            Change::CanEdit(false) => self.close_all(cx, BYE_REVOKED),
            Change::Offline => self.close_all(cx, BYE_OFFLINE),
            Change::NetChange => {
                let t = cx.now as u64 as u32;
                for (&inst, c) in self.conns.iter_mut().filter(|(_, c)| c.state == State::Open) {
                    c.probe = Some((t, cx.now + (3.0 * c.rtt + 200.0).clamp(300.0, 2_000.0)));
                    cx.send(Dest::Peer(inst), &Frame::Ping(t));
                }
            }
            _ => {}
        }
    }

    fn tick(&mut self, cx: &mut Core, _r: &mut dyn Replica) {
        let now = cx.now;
        for (from, m) in std::mem::take(&mut cx.mail_in) {
            self.mail(cx, from, m);
        }
        self.seen.retain(|_, (at, _)| now - *at < 2.0 * MAIL_TTL_MS as f64);
        // Own mail leaves the relay file after its TTL.
        let expired: Vec<_> = self.posted.iter().filter(|(_, at)| now - **at >= MAIL_TTL_MS as f64).map(|(k, _)| *k).collect();
        for (to, sid, kind) in expired {
            self.posted.remove(&(to, sid, kind));
            cx.mail_out.retain(|m| (m.to, m.sid, m.kind) != (to, sid, kind));
            cx.mail_changed = true;
        }
        if let Some(at) = self.posted.values().copied().reduce(f64::min) {
            cx.wake(at + MAIL_TTL_MS as f64);
        }
        let vouched: Vec<u64> = cx.presence.remotes.keys().copied().filter(|&i| cx.presence.vouched(i, now)).collect();
        for inst in vouched {
            self.vouched_at.insert(inst, now);
        }
        self.vouched_at.retain(|i, t| now - *t <= VOUCH_GRACE_MS || cx.presence.remotes.contains_key(i));
        // Discover present, vouched editors (we offer if we are the smaller installation, §10.4).
        let candidates: Vec<u64> = cx.presence.present_insts(now).filter(|&i| self.allowed(cx, i)).collect();
        for inst in candidates {
            let c = self.conns.entry(inst).or_default();
            let seen = *c.seen_at.get_or_insert(now);
            if c.state == State::Idle {
                let at = if cx.inst < inst { now } else { seen + RESPONDER_AFTER_MS };
                if now >= at {
                    self.offer(cx, inst);
                } else {
                    cx.wake(at);
                }
            }
        }
        let insts: Vec<u64> = self.conns.keys().copied().collect();
        for inst in insts {
            let allowed = self.allowed(cx, inst);
            let Some(c) = self.conns.get_mut(&inst) else { continue };
            match c.state {
                State::Idle if !allowed => {
                    self.conns.remove(&inst);
                }
                State::Idle => {}
                State::Backoff | State::Unreachable if now >= c.until => {
                    if c.state == State::Unreachable {
                        c.fails = 0;
                    }
                    Self::set(cx, inst, c, State::Idle, 0.0);
                    if allowed && cx.inst < inst {
                        self.offer(cx, inst);
                    } else if allowed {
                        c.seen_at = Some(now - RESPONDER_AFTER_MS); // retry after a failure: no extra wait
                    }
                }
                State::Backoff | State::Unreachable => cx.wake(c.until),
                _ if !allowed => self.close(cx, inst, false, Some(BYE_REVOKED)),
                State::Open => {
                    if now - c.rx_at >= DEAD_MS || c.probe.is_some_and(|p| now >= p.1) {
                        self.close(cx, inst, false, None);
                        if cx.inst < inst {
                            self.offer(cx, inst);
                        }
                        continue;
                    }
                    if now - c.ping_at.max(c.rx_at) >= PING_IDLE_MS {
                        c.ping_at = now;
                        cx.send(Dest::Peer(inst), &Frame::Ping(now as u64 as u32));
                    }
                    let next = (c.ping_at.max(c.rx_at) + PING_IDLE_MS).min(c.rx_at + DEAD_MS);
                    cx.wake(c.probe.map_or(next, |p| next.min(p.1)));
                }
                _ if now >= c.until => self.close(cx, inst, true, None),
                _ => cx.wake(c.until),
            }
        }
    }
}

/// Test command hook: `__IHN_TEST_API__.cmd('sync.peer.<name>', args)`.
pub fn test_cmd(_name: &str, _args: &str) -> Option<String> { None }

#[cfg(test)]
#[path = "peer_tests.rs"]
mod tests;
