//! Stamps, the hybrid logical clock, LWW registers and version vectors (ARCHITECTURE §4.1, SYNC §3).

use crate::codec::{Reader, Writer};
use crate::id::ActorId;
use std::collections::{BTreeMap, BTreeSet};

/// A received clock further than this ahead of the wall clock is applied but never observed (SYNC §3.4).
pub const CLOCK_HORIZON_MS: u64 = 24 * 60 * 60 * 1000;

/// Identity of one write. Total order `(clock, actor, idx)`; native stamps are unique.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct Stamp {
    pub clock: u64,
    pub actor: ActorId,
    pub idx: u32,
}

impl Stamp {
    /// The stamp of a register that was never written; every real stamp is greater.
    pub const ZERO: Stamp = Stamp { clock: 0, actor: ActorId(0), idx: 0 };
    pub fn new(clock: u64, actor: ActorId, idx: u32) -> Stamp { Stamp { clock, actor, idx } }
}

/// Last-writer-wins register.
#[derive(Clone, PartialEq, Default, Debug)]
pub struct Reg<T> {
    pub v: T,
    pub at: Stamp,
}

impl<T> Reg<T> {
    pub fn new(v: T, at: Stamp) -> Reg<T> { Reg { v, at } }
    /// Writes only if `at` is greater than the stored stamp. Returns whether it wrote.
    pub fn set(&mut self, v: T, at: Stamp) -> bool {
        let wins = at > self.at;
        if wins {
            *self = Reg { v, at };
        }
        wins
    }
}

/// Hybrid logical clock of one writer in one document.
#[derive(Clone, Copy, Default, Debug)]
pub struct Clock {
    /// Clock of the writer's previous batch.
    pub last: u64,
    /// Greatest clock seen in applied batches (own and remote), within the horizon.
    pub observed: u64,
}

impl Clock {
    /// Clock of the next batch: `max(prev + 1, wall_ms, observed + 1)`.
    pub fn next(&mut self, wall_ms: u64) -> u64 {
        self.last = (self.last + 1).max(wall_ms).max(self.observed + 1);
        self.last
    }
    /// Observes the greatest clock of an applied batch unless it lies beyond `wall_ms + 24 h`, so that one device
    /// with a corrupt clock cannot drag every device forward.
    pub fn observe(&mut self, clock: u64, wall_ms: u64) {
        if clock <= wall_ms.saturating_add(CLOCK_HORIZON_MS) {
            self.observed = self.observed.max(clock);
        }
    }
}

/// Applied batches per actor: the highest contiguous seq, plus the seqs applied above a gap ("extras").
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct VersionVector {
    seq: BTreeMap<ActorId, u32>,
    extras: BTreeMap<ActorId, BTreeSet<u32>>,
}

impl VersionVector {
    /// Highest contiguous applied seq of `actor` (0 = none).
    pub fn get(&self, actor: ActorId) -> u32 { self.seq.get(&actor).copied().unwrap_or(0) }
    /// Whether batch `(actor, seq)` was applied (contiguous range or extras).
    pub fn contains(&self, actor: ActorId, seq: u32) -> bool {
        seq != 0 && (seq <= self.get(actor) || self.extras.get(&actor).is_some_and(|e| e.contains(&seq)))
    }
    /// Records batch `(actor, seq)`; `false` if it was already there (or `seq == 0`, which is not a batch).
    pub fn add(&mut self, actor: ActorId, seq: u32) -> bool {
        if seq == 0 || self.contains(actor, seq) {
            return false;
        }
        self.extras.entry(actor).or_default().insert(seq);
        self.settle(actor);
        true
    }
    /// Moves extras that became contiguous into the contiguous seq.
    fn settle(&mut self, actor: ActorId) {
        let mut top = self.get(actor);
        if let Some(extras) = self.extras.get_mut(&actor) {
            while extras.first().is_some_and(|&s| s <= top + 1) {
                top = top.max(extras.pop_first().unwrap_or(0));
            }
            if extras.is_empty() {
                self.extras.remove(&actor);
            }
        }
        if top > 0 {
            self.seq.insert(actor, top);
        }
    }
    /// Raises the contiguous seq of `actor` to at least `seq` (a snapshot or a seal covers everything below).
    pub fn raise(&mut self, actor: ActorId, seq: u32) {
        if seq > self.get(actor) {
            self.seq.insert(actor, seq);
            self.settle(actor);
        }
    }
    /// Pointwise union (snapshot merge).
    pub fn merge(&mut self, other: &VersionVector) {
        for (&actor, &seq) in &other.seq {
            self.raise(actor, seq);
        }
        for (actor, seq) in other.extras() {
            self.add(actor, seq);
        }
    }
    /// Whether every batch of `other` is in `self`.
    pub fn covers(&self, other: &VersionVector) -> bool {
        other.seq.iter().all(|(&a, &s)| self.get(a) >= s) && other.extras().all(|(a, s)| self.contains(a, s))
    }
    /// `(actor, contiguous seq)` in ascending actor order.
    pub fn iter(&self) -> impl Iterator<Item = (ActorId, u32)> + '_ { self.seq.iter().map(|(&a, &s)| (a, s)) }
    /// `(actor, seq)` of the batches applied above a gap, ascending.
    pub fn extras(&self) -> impl Iterator<Item = (ActorId, u32)> + '_ {
        self.extras.iter().flat_map(|(&a, set)| set.iter().map(move |&s| (a, s)))
    }
    /// Every actor with a contiguous seq or extras, ascending.
    pub fn actors(&self) -> BTreeSet<ActorId> { self.seq.keys().chain(self.extras.keys()).copied().collect() }
    pub fn is_empty(&self) -> bool { self.seq.is_empty() && self.extras.is_empty() }

    /// SYNC §4.1 `VV`: `uv n` + n × (`u64 actor`, `uv seq`), actors strictly ascending, seq ≥ 1. Extras are not
    /// part of it (a peer asks again for what lies above its gap); snapshots add them with [`Self::encode_extras`].
    pub fn encode(&self, w: &mut Writer) {
        w.leb(self.seq.len() as u64);
        for (actor, seq) in self.iter() {
            w.u64(actor.0);
            w.leb(seq as u64);
        }
    }
    pub fn decode(r: &mut Reader) -> Option<VersionVector> {
        let mut vv = VersionVector::default();
        let mut prev = None;
        for _ in 0..r.leb()? {
            let actor = ActorId(r.u64()?);
            let seq = u32::try_from(r.leb()?).ok().filter(|&s| s >= 1)?;
            if prev.is_some_and(|p| p >= actor) {
                return None;
            }
            prev = Some(actor);
            vv.seq.insert(actor, seq);
        }
        Some(vv)
    }
    /// `uv n` + n × (`u64 actor`, `uv seq`), strictly ascending pairs, each above the actor's contiguous seq.
    pub fn encode_extras(&self, w: &mut Writer) {
        w.leb(self.extras().count() as u64);
        for (actor, seq) in self.extras() {
            w.u64(actor.0);
            w.leb(seq as u64);
        }
    }
    pub fn decode_extras(&mut self, r: &mut Reader) -> Option<()> {
        let mut prev = None;
        for _ in 0..r.leb()? {
            let pair = (ActorId(r.u64()?), u32::try_from(r.leb()?).ok()?);
            if prev.is_some_and(|p| p >= pair) || pair.1 <= self.get(pair.0) + 1 {
                return None; // not canonical: unordered, or not above a gap
            }
            prev = Some(pair);
            self.extras.entry(pair.0).or_default().insert(pair.1);
        }
        Some(())
    }
}

/// Test command hook: `__IHN_TEST_API__.cmd('core.stamp.<name>', args)`.
pub fn test_cmd(_name: &str, _args: &str) -> Option<String> { None }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Rng;

    #[test]
    fn stamps_order_by_clock_then_actor_then_idx() {
        let s = |c, a, i| Stamp::new(c, ActorId(a), i);
        assert!(s(2, 0, 0) > s(1, 9, 9));
        assert!(s(1, 2, 0) > s(1, 1, 9));
        assert!(s(1, 1, 2) > s(1, 1, 1));
        assert!(s(0, 0, 1) > Stamp::ZERO);
        let mut r = Reg::new(1, Stamp::ZERO);
        assert!(r.set(2, s(5, 1, 0)));
        assert!(!r.set(3, s(5, 1, 0)));
        assert!(!r.set(3, s(4, 9, 9)));
        assert_eq!(r.v, 2);
    }

    #[test]
    fn clock_is_monotonic_and_ignores_the_far_future() {
        let mut c = Clock::default();
        assert_eq!(c.next(1000), 1000);
        assert_eq!(c.next(1000), 1001); // same millisecond
        assert_eq!(c.next(900), 1002); // wall clock went back
        c.observe(5000, 1000);
        assert_eq!(c.next(1000), 5001);
        let far = 1000 + 10 * CLOCK_HORIZON_MS;
        c.observe(far, 1000);
        assert_eq!(c.observed, 5000);
        assert_eq!(c.next(1000), 5002);
        c.observe(1000 + CLOCK_HORIZON_MS, 1000); // exactly at the horizon: observed
        assert_eq!(c.next(1000), 1000 + CLOCK_HORIZON_MS + 1);
    }

    #[test]
    fn version_vector_contiguity_and_extras() {
        let (a, b) = (ActorId(1), ActorId(2));
        let mut vv = VersionVector::default();
        assert!(!vv.add(a, 0));
        assert!(vv.add(a, 3) && vv.add(a, 5));
        assert_eq!(vv.get(a), 0);
        assert!(vv.contains(a, 3) && !vv.contains(a, 4) && !vv.add(a, 3));
        assert!(vv.add(a, 1) && vv.add(a, 2));
        assert_eq!(vv.get(a), 3);
        assert_eq!(vv.extras().collect::<Vec<_>>(), vec![(a, 5)]);
        assert!(vv.add(a, 4));
        assert_eq!((vv.get(a), vv.extras().count()), (5, 0));
        let mut other = VersionVector::default();
        other.add(b, 1);
        other.add(a, 7);
        assert!(!vv.covers(&other));
        vv.merge(&other);
        assert!(vv.covers(&other) && vv.contains(a, 7) && vv.get(b) == 1 && vv.get(a) == 5);
        vv.raise(a, 6);
        assert_eq!((vv.get(a), vv.extras().count()), (7, 0));
    }

    #[test]
    fn any_arrival_order_gives_the_same_vector_and_codec_round_trips() {
        let mut rng = Rng::new(3);
        for round in 0..200 {
            let mut all: Vec<(ActorId, u32)> = (1..=4).flat_map(|a| (1..=12).map(move |s| (ActorId(a * 256), s))).collect();
            rng.shuffle(&mut all);
            let keep = rng.below(all.len() as u64 + 1) as usize;
            let (mut x, mut y) = (VersionVector::default(), VersionVector::default());
            for &(a, s) in &all[..keep] {
                x.add(a, s);
            }
            let mut again = all[..keep].to_vec();
            rng.shuffle(&mut again);
            for &(a, s) in &again {
                y.add(a, s);
            }
            assert_eq!(x, y, "round {round}");
            let mut w = Writer::default();
            x.encode(&mut w);
            x.encode_extras(&mut w);
            let mut r = Reader::new(&w.buf);
            let mut back = VersionVector::decode(&mut r).unwrap();
            back.decode_extras(&mut r).unwrap();
            assert!(r.is_empty());
            assert_eq!(back, x);
            for cut in 0..w.buf.len() {
                let mut r = Reader::new(&w.buf[..cut]);
                assert!(VersionVector::decode(&mut r).and_then(|mut v| v.decode_extras(&mut r)).is_none());
            }
        }
        // Not ascending / seq 0 are rejected.
        let bad = |bytes: &[u8]| VersionVector::decode(&mut Reader::new(bytes)).is_none();
        assert!(bad(&[2, 2, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 1]));
        assert!(bad(&[1, 1, 0, 0, 0, 0, 0, 0, 0, 0]));
    }
}
