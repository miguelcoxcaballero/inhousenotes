//! Live ink / live erase sender and the receiver's preview table with its TTL (SYNC §10.6); own presence frames.
//! Owner: W2.8.
//!
//! Sender: one frame per `SyncEngine::live` call (the editor calls it once per animation frame), to the tabs and
//! the direct peers; erase previews at most every 50 ms. Receiver: [`Previews`] in `Core::live`, read by the
//! remote-ink overlay (`ihn-app::collab`, W4.5), which repaints when [`Previews::version`] moves. A preview goes
//! when its item's batch arrives, on cancel / erase end, when its peer is gone, or 15 s after its last frame.
//! Live ink is not durable and not forwarded, except from a peer to the tabs of the network-leader tab.

use crate::mode::Link;
use crate::wire::{Frame, MAX_LIVE_POINTS};
use crate::{Change, Core, Dest, Part};
use ihn_core::doc::{LivePreview, PeerRef, PresenceState, Replica};
use ihn_core::id::Id;
use ihn_core::op::{Body, Color, Op, Tool};
use std::collections::{BTreeMap, BTreeSet};

pub const PREVIEW_TTL_MS: f64 = 15_000.0;
pub const ERASE_EVERY_MS: f64 = 50.0;

/// A remote in-progress stroke.
#[derive(Clone, PartialEq, Debug)]
pub struct Stroke {
    pub from: PeerRef,
    pub page: Id,
    pub tool: Tool,
    pub color: Color,
    pub width: f64,
    pub xy: Vec<f64>,
    pub p: Option<Vec<f32>>,
    /// Pen up: the batch putting this item follows.
    pub item: Option<Id>,
    at: f64,
}

/// A remote eraser gesture in progress.
#[derive(Clone, PartialEq, Debug)]
pub struct Erasing {
    pub from: PeerRef,
    pub page: Id,
    pub hidden: Vec<Id>,
    pub fragments: Vec<(u64, Body)>,
    at: f64,
}

#[derive(Default, Debug)]
pub struct Previews {
    /// By `live` id.
    pub strokes: BTreeMap<u64, Stroke>,
    /// By gesture id.
    pub erases: BTreeMap<u64, Erasing>,
    /// Bumped on every change.
    pub version: u64,
}

impl Previews {
    fn expire(&mut self, now: f64, gone: impl Fn(PeerRef) -> bool) {
        let n = self.strokes.len() + self.erases.len();
        self.strokes.retain(|_, s| now - s.at < PREVIEW_TTL_MS && !gone(s.from));
        self.erases.retain(|_, e| now - e.at < PREVIEW_TTL_MS && !gone(e.from));
        self.version += (n != self.strokes.len() + self.erases.len()) as u64;
    }
    fn next_expiry(&self) -> Option<f64> { self.strokes.values().map(|s| s.at).chain(self.erases.values().map(|e| e.at)).reduce(f64::min).map(|t| t + PREVIEW_TTL_MS) }

    /// Applies one received live frame; `false` if it is not a live frame.
    pub fn receive(&mut self, from: PeerRef, f: &Frame, now: f64) -> bool {
        match f {
            Frame::InkBegin { live, page, tool, color, width, has_p } => {
                let p = has_p.then(Vec::new);
                self.strokes.insert(*live, Stroke { from, page: *page, tool: *tool, color: color.clone(), width: *width, xy: Vec::new(), p, item: None, at: now });
            }
            Frame::InkPts { live, from: first, xy, p } => {
                let Some(s) = self.strokes.get_mut(live) else { return true };
                // In order on one channel; a repeated range is skipped, a gap waits for the batch.
                let (have, first) = (s.xy.len() / 2, *first as usize);
                if first > have {
                    return true;
                }
                let skip = (have - first).min(xy.len() / 2);
                s.xy.extend_from_slice(&xy[skip * 2..]);
                if let (Some(dst), Some(src)) = (s.p.as_mut(), p) {
                    dst.extend_from_slice(src.get(skip..).unwrap_or(&[]));
                }
                s.at = now;
            }
            Frame::InkEnd { live, item } => {
                let Some(s) = self.strokes.get_mut(live) else { return true };
                (s.item, s.at) = (Some(*item), now);
            }
            Frame::InkCancel { live } => drop(self.strokes.remove(live)),
            Frame::Erase { gesture, page, hidden, fragments } => {
                self.erases.insert(*gesture, Erasing { from, page: *page, hidden: hidden.clone(), fragments: fragments.clone(), at: now });
            }
            Frame::EraseEnd { gesture } => drop(self.erases.remove(gesture)),
            _ => return false,
        }
        self.version += 1;
        true
    }

    /// Drops the finished strokes whose item one of `batches` puts.
    pub fn applied(&mut self, batches: &[ihn_core::snapshot::EncodedBatch]) {
        if !self.strokes.values().any(|s| s.item.is_some()) {
            return;
        }
        let items: BTreeSet<Id> = batches
            .iter()
            .filter_map(|b| b.decode().ok())
            .flat_map(|b| b.ops)
            .filter_map(|op| if let Op::ItemPut { item, .. } = op { Some(item) } else { None })
            .collect();
        let n = self.strokes.len();
        self.strokes.retain(|_, s| !s.item.is_some_and(|i| items.contains(&i)));
        self.version += (n != self.strokes.len()) as u64;
    }
}

#[derive(Default)]
pub struct Live {
    pending_erase: Option<Frame>,
    erased_at: f64,
}

pub fn part() -> Option<Box<dyn Part>> { Some(Box::new(Live { pending_erase: None, erased_at: f64::NEG_INFINITY })) }

impl Live {
    fn erase(&mut self, cx: &mut Core, f: Frame) {
        self.erased_at = cx.now;
        cx.send_all(&f);
    }
}

impl Part for Live {
    fn live(&mut self, cx: &mut Core, p: &LivePreview) {
        if !cx.can_edit {
            return; // viewers never draw
        }
        let f = match p.clone() {
            LivePreview::Begin { live, page, tool, color, width, has_p } => Frame::InkBegin { live, page, tool, color, width, has_p },
            LivePreview::Points { live, from, mut xy, mut p } => {
                // ≤ 24,000 points per stroke (5.x).
                let room = MAX_LIVE_POINTS.saturating_sub(from as u64) as usize;
                if room == 0 || xy.len() < 2 {
                    return;
                }
                xy.truncate(room.min(xy.len() / 2) * 2);
                if let Some(p) = p.as_mut() {
                    p.truncate(xy.len() / 2);
                }
                Frame::InkPts { live, from, xy, p }
            }
            LivePreview::End { live, item } => Frame::InkEnd { live, item },
            LivePreview::Cancel { live } => Frame::InkCancel { live },
            LivePreview::Erase { gesture, page, hidden, fragments } => {
                let f = Frame::Erase { gesture, page, hidden, fragments };
                if cx.now - self.erased_at >= ERASE_EVERY_MS {
                    self.pending_erase = None;
                    self.erase(cx, f);
                } else {
                    self.pending_erase = Some(f);
                }
                return;
            }
            LivePreview::EraseEnd { gesture } => {
                if let Some(f) = self.pending_erase.take() {
                    self.erase(cx, f);
                }
                Frame::EraseEnd { gesture }
            }
        };
        cx.send_all(&f);
    }

    fn presence(&mut self, cx: &mut Core, p: &PresenceState) { cx.send_all(&Frame::Presence(p.clone())) }

    fn frame(&mut self, cx: &mut Core, from: PeerRef, f: &Frame, _r: &mut dyn Replica) {
        let open = match from {
            PeerRef::Tab => true,
            PeerRef::Peer(inst) => cx.presence.link(inst) == Link::Open,
            _ => false,
        };
        if !open {
            return;
        }
        if let Frame::Batches(b) = f {
            return cx.live.applied(b);
        }
        if cx.live.receive(from, f, cx.now) && matches!(from, PeerRef::Peer(_)) && cx.leader {
            cx.send(Dest::Tabs, f); // the other tabs of this browser draw it too
        }
    }

    fn change(&mut self, cx: &mut Core, c: Change, _r: &mut dyn Replica) {
        if matches!(c, Change::Leave | Change::Suspend) {
            cx.live = Previews { version: cx.live.version + 1, ..Default::default() };
        }
    }

    fn tick(&mut self, cx: &mut Core, _r: &mut dyn Replica) {
        let now = cx.now;
        if self.pending_erase.is_some() {
            if now - self.erased_at >= ERASE_EVERY_MS {
                let f = self.pending_erase.take().unwrap();
                self.erase(cx, f);
            } else {
                cx.wake(self.erased_at + ERASE_EVERY_MS);
            }
        }
        let presence = &cx.presence;
        cx.live.expire(now, |from| matches!(from, PeerRef::Peer(i) if presence.link(i) != Link::Open));
        if let Some(at) = cx.live.next_expiry() {
            cx.wake(at);
        }
    }
}

/// Test command hook: `__IHN_TEST_API__.cmd('sync.live.<name>', args)`.
pub fn test_cmd(_name: &str, _args: &str) -> Option<String> { None }
