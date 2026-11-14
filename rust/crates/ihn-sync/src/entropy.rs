//! Hello/Have/Batches/Snapshot exchange, acknowledgement, resend and forwarding (SYNC §10.7, §11). Owner: W2.8.
//!
//! Every destination (an open peer, or the tabs of this browser) has a `sent` vector: what it has or was sent.
//! Each call sends whatever the replica holds beyond it, so local batches, batches received from another peer,
//! from a tab or from the relay, are all fanned out by the same rule (forwarding = "its known VV lacks it").
//! Peers acknowledge with `Have` (200 ms debounce); 5 s after a send that is still not covered, `sent` falls back
//! to the acknowledged vector and the rest is sent again. Tabs are reliable: no acks, no resend.

use crate::mode::Link;
use crate::peer::hello;
use crate::wire::Frame;
use crate::{Core, Dest, Part};
use ihn_core::doc::{PeerRef, Replica};
use ihn_core::stamp::VersionVector;
use std::collections::BTreeMap;

pub const HAVE_DEBOUNCE_MS: f64 = 200.0;
pub const RESEND_MS: f64 = 5_000.0;
/// One `Batches` frame carries at most this much (several frames per call when more is missing).
pub const FRAME_BYTES: usize = 1 << 20;
/// At most this many frames per destination per call; the rest follows on the next call (woken at once).
const FRAMES_PER_CALL: usize = 32;
/// A tab VV not refreshed for this long no longer holds back journal retention.
pub const TAB_VV_MS: f64 = 60_000.0;

#[derive(Default, Debug)]
struct Dst {
    /// Acknowledged (Hello / Have / what it sent us).
    known: VersionVector,
    /// `known` plus what was sent since.
    sent: VersionVector,
    check_at: Option<f64>,
    have_at: Option<f64>,
}

#[derive(Default)]
pub struct Entropy {
    peers: BTreeMap<u64, Dst>,
    /// What the tabs were sent (or sent us).
    tabs: VersionVector,
    hello_sent: bool,
    tab_heard_at: f64,
}

pub fn part() -> Option<Box<dyn Part>> { Some(Box::new(Entropy::default())) }

/// Sends what `r` holds beyond `sent` to `to`, raising `sent`. `false` when the frame budget ran out.
fn send_missing(cx: &mut Core, to: Dest, sent: &mut VersionVector, r: &dyn Replica) -> bool {
    for _ in 0..FRAMES_PER_CALL {
        if sent.covers(r.version_vector()) {
            return true;
        }
        match r.batches_since(sent, FRAME_BYTES) {
            Ok(batches) if !batches.is_empty() => {
                for (actor, seq, _) in batches.iter().filter_map(|b| b.header()) {
                    sent.add(actor, seq);
                }
                cx.send(to, &Frame::Batches(batches));
            }
            // Nothing to send although the vectors differ (nothing journaled beyond): stop asking.
            Ok(_) => sent.merge(r.version_vector()),
            // The range left the journal (§7.3): the snapshot carries everything, then batches continue from it.
            Err(_) => {
                cx.send(to, &Frame::Snapshot(r.snapshot()));
                sent.merge(r.version_vector());
            }
        }
    }
    sent.covers(r.version_vector())
}

fn mark(vv: &mut VersionVector, batches: &[ihn_core::snapshot::EncodedBatch]) {
    for (actor, seq, _) in batches.iter().filter_map(|b| b.header()) {
        vv.add(actor, seq);
    }
}

impl Part for Entropy {
    fn frame(&mut self, cx: &mut Core, from: PeerRef, f: &Frame, r: &mut dyn Replica) {
        let now = cx.now;
        match from {
            PeerRef::Tab => {
                match f {
                    Frame::Hello { doc_id, vv, .. } if *doc_id == cx.doc_id => {
                        // A new tab: tell it what we have and send what it lacks (§11).
                        cx.send(Dest::Tabs, &Frame::Have(r.version_vector().clone()));
                        send_missing(cx, Dest::Tabs, &mut vv.clone(), r);
                        cx.peer_vv.insert(PeerRef::Tab, vv.clone());
                    }
                    Frame::Have(vv) => {
                        send_missing(cx, Dest::Tabs, &mut vv.clone(), r);
                        cx.peer_vv.insert(PeerRef::Tab, vv.clone());
                    }
                    Frame::Batches(b) => {
                        r.receive(b, from);
                        mark(&mut self.tabs, b);
                    }
                    Frame::Snapshot(s) => {
                        r.load_snapshot(s, from);
                        self.tabs.merge(&s.version_vector().unwrap_or_default());
                    }
                    _ => return,
                }
                self.tab_heard_at = now;
            }
            PeerRef::Peer(inst) if cx.presence.link(inst) == Link::Open => match f {
                // `peer.rs` ran first and verified it: start (again) from what the peer says it has.
                Frame::Hello { vv, .. } => {
                    self.peers.insert(inst, Dst { known: vv.clone(), sent: vv.clone(), ..Default::default() });
                    cx.peer_vv.insert(from, vv.clone());
                }
                Frame::Have(vv) => {
                    let d = self.peers.entry(inst).or_default();
                    d.known.merge(vv);
                    d.sent.merge(vv);
                    cx.peer_vv.insert(from, d.known.clone());
                }
                Frame::Batches(b) => {
                    r.receive(b, from);
                    let d = self.peers.entry(inst).or_default();
                    mark(&mut d.known, b);
                    mark(&mut d.sent, b);
                    d.have_at.get_or_insert(now + HAVE_DEBOUNCE_MS);
                    cx.peer_vv.insert(from, d.known.clone());
                }
                Frame::Snapshot(s) => {
                    r.load_snapshot(s, from);
                    let vv = s.version_vector().unwrap_or_default();
                    let d = self.peers.entry(inst).or_default();
                    d.known.merge(&vv);
                    d.sent.merge(&vv);
                    d.have_at.get_or_insert(now + HAVE_DEBOUNCE_MS);
                }
                Frame::SnapshotWant => cx.send(Dest::Peer(inst), &Frame::Snapshot(r.snapshot())),
                _ => {}
            },
            _ => {}
        }
    }

    fn tick(&mut self, cx: &mut Core, r: &mut dyn Replica) {
        let now = cx.now;
        // Tabs: announce once, then broadcast whatever they were not sent.
        if !self.hello_sent {
            self.hello_sent = true;
            self.tabs = r.version_vector().clone();
            let h = hello(cx, r);
            cx.send(Dest::Tabs, &h);
        }
        let mut more = !send_missing(cx, Dest::Tabs, &mut self.tabs, r);
        if now - self.tab_heard_at > TAB_VV_MS {
            cx.peer_vv.remove(&PeerRef::Tab);
        }
        // Peers: only open channels; a closed peer's VV keeps holding the journal while it stays vouched, so a
        // short reconnect gets batches, not a snapshot (§7.3).
        self.peers.retain(|inst, _| cx.presence.link(*inst) == Link::Open);
        let (presence, at) = (&cx.presence, now);
        cx.peer_vv.retain(|k, _| !matches!(k, PeerRef::Peer(i) if !presence.vouched(*i, at) && presence.link(*i) != Link::Open));
        for (&inst, d) in self.peers.iter_mut() {
            if d.have_at.is_some_and(|t| now >= t) {
                d.have_at = None;
                cx.send(Dest::Peer(inst), &Frame::Have(r.version_vector().clone()));
            }
            if d.check_at.is_some_and(|t| now >= t) {
                d.check_at = None;
                if !d.known.covers(&d.sent) {
                    d.sent = d.known.clone(); // not acknowledged in 5 s: send it again
                }
            }
            if !d.sent.covers(r.version_vector()) {
                more |= !send_missing(cx, Dest::Peer(inst), &mut d.sent, r);
                d.check_at.get_or_insert(now + RESEND_MS);
            }
            for t in [d.have_at, d.check_at].into_iter().flatten() {
                cx.wake(t);
            }
        }
        if more {
            cx.wake(now);
        }
    }
}

/// Test command hook: `__IHN_TEST_API__.cmd('sync.entropy.<name>', args)`.
pub fn test_cmd(_name: &str, _args: &str) -> Option<String> { None }
