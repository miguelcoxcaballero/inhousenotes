//! Presence table, modes and cadence, scribe election (SYNC §9, §10.1, §14.1). Owner: W2.6.
//!
//! Every time in this module is the receiver's own clock (`now`, ms). Nothing a remote device wrote is ever
//! compared with it, so presence, vouching and modes behave the same under any clock skew.

use crate::wire::Profile;
use ihn_core::doc::{PeerLink, PeerView, PresenceState};
use std::collections::BTreeMap;

pub const PRESENCE_MS: f64 = 120_000.0;
pub const VOUCH_MS: f64 = 600_000.0;
/// Minimum spacing of relay writes (file + record) per device; exit flushes are exempt.
pub const WRITE_CAP_MS: f64 = 2_000.0;
pub const RECORD_REFRESH_MS: f64 = 60_000.0;
pub const ALONE_IDLE_AFTER_MS: f64 = 300_000.0;
pub const HIDDEN_SUSPEND_MS: f64 = 60_000.0;

/// SYNC §9.2 (and §12.1 for viewers).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Alone,
    Company,
    Linked,
    RelayOnly,
    Viewer,
}

/// P2P state of a remote installation, as far as modes and avatars care (set by `peer.rs`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Link {
    #[default]
    Idle,
    /// Offering / Answering / Connecting / Backoff.
    Handshake,
    Open,
    /// 3 consecutive failures: relay only for 5 min.
    Unreachable,
}

/// The parts of a record value (SYNC §8.4) presence needs. The full codec is `relay.rs`'s.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RecordFlags {
    pub live: bool,
    pub left: bool,
    pub no_link: bool,
}

/// `"1|" gen36 "|" n36 "|" fileId "|" d16 "|" flags`; anything else is malformed and ignored.
pub fn record_flags(value: &str) -> Option<RecordFlags> {
    let mut parts = value.split('|');
    let ok = parts.next() == Some("1")
        && parts.by_ref().take(2).all(|n| !n.is_empty() && n.len() <= 13 && n.bytes().all(|b| b.is_ascii_digit() || b.is_ascii_lowercase()))
        && parts.next().is_some_and(|id| !id.is_empty())
        && parts.next().is_some_and(|d| d.len() == 16);
    let flags = parts.next()?;
    if !ok || parts.next().is_some() || !flags.bytes().all(|b| matches!(b, b'c' | b'l' | b'x')) {
        return None;
    }
    Some(RecordFlags { live: flags.contains('c'), left: flags.contains('l'), no_link: flags.contains('x') })
}

/// `"ihn6_" + inst13` → inst.
pub fn record_inst(key: &str) -> Option<u64> {
    let digits = key.strip_prefix("ihn6_").filter(|d| d.len() == 13 && !d.starts_with('+'))?;
    u64::from_str_radix(digits, 36).ok().filter(|_| digits.bytes().all(|b| !b.is_ascii_uppercase()))
}

/// What this device knows about one remote installation.
#[derive(Clone, Debug, Default)]
pub struct Remote {
    /// Current record value; empty when the installation is known only as a peer.
    pub value: String,
    /// Local time at which the current value was first seen.
    pub seen_at: f64,
    pub left: bool,
    /// The relay file of the current record generation passed the digest check (§10.1 b).
    pub verified: bool,
    pub link: Link,
    pub profile: Option<Profile>,
    pub presence: Option<PresenceState>,
}

#[derive(Default)]
pub struct Presence {
    pub remotes: BTreeMap<u64, Remote>,
}

impl Presence {
    /// One poll's records of the other installations. Returns the installations whose record changed.
    pub fn observe<'a>(&mut self, records: impl Iterator<Item = (u64, &'a str)>, now: f64) -> Vec<u64> {
        let mut changed = Vec::new();
        let mut listed = Vec::new();
        for (inst, value) in records {
            listed.push(inst);
            let Some(flags) = record_flags(value) else { continue };
            let r = self.remotes.entry(inst).or_default();
            if r.value != value {
                *r = Remote { value: value.into(), seen_at: now, left: flags.left, verified: false, ..std::mem::take(r) };
                changed.push(inst);
            }
        }
        // A record that is gone (GC, copy clean-up) leaves only what the peer machine knows.
        self.remotes.retain(|inst, r| {
            if !listed.contains(inst) && !r.value.is_empty() {
                *r = Remote { link: r.link, profile: r.profile.take(), presence: r.presence.take(), left: true, ..Default::default() };
            }
            !r.value.is_empty() || r.link != Link::Idle
        });
        changed
    }

    /// The relay file named by `inst`'s current record passed the digest check.
    pub fn verified(&mut self, inst: u64) {
        if let Some(r) = self.remotes.get_mut(&inst) {
            r.verified = true;
        }
    }

    pub fn set_link(&mut self, inst: u64, link: Link) { self.remotes.entry(inst).or_default().link = link }

    pub fn link(&self, inst: u64) -> Link { self.remotes.get(&inst).map_or(Link::Idle, |r| r.link) }

    fn is_present(r: &Remote, now: f64) -> bool {
        r.link == Link::Open || (!r.value.is_empty() && !r.left && now - r.seen_at <= PRESENCE_MS)
    }

    /// §9.1: record without `l` first seen ≤ 120 s ago, or a connected peer.
    pub fn present(&self, inst: u64, now: f64) -> bool { self.remotes.get(&inst).is_some_and(|r| Self::is_present(r, now)) }

    /// §10.1: record present-or-recent (≤ 10 min) and its relay file verified.
    pub fn vouched(&self, inst: u64, now: f64) -> bool {
        self.remotes.get(&inst).is_some_and(|r| r.verified && !r.value.is_empty() && now - r.seen_at <= VOUCH_MS)
    }

    pub fn present_insts(&self, now: f64) -> impl Iterator<Item = u64> + '_ {
        self.remotes.iter().filter(move |(_, r)| Self::is_present(r, now)).map(|(&i, _)| i)
    }

    /// §9.2. Only records make an editor: viewers never write one.
    pub fn mode(&self, now: f64, can_edit: bool) -> Mode {
        if !can_edit {
            return Mode::Viewer;
        }
        let mut mode = Mode::Alone;
        for r in self.remotes.values().filter(|r| Self::is_present(r, now)) {
            mode = match (r.link, mode) {
                (Link::Idle | Link::Handshake, _) => return Mode::Company,
                (Link::Unreachable, _) | (_, Mode::RelayOnly) => Mode::RelayOnly,
                (Link::Open, _) => Mode::Linked,
            };
        }
        mode
    }

    /// §14.1: the smallest present editor installation, self included.
    pub fn scribe(&self, own: u64, now: f64) -> u64 { self.present_insts(now).chain([own]).min().unwrap_or(own) }

    /// Local time of the newest record change (viewer cadence, §12.1).
    pub fn last_change(&self) -> Option<f64> {
        self.remotes.values().filter(|r| !r.value.is_empty()).map(|r| r.seen_at).reduce(f64::max)
    }

    /// The earliest future time at which presence or the mode can change by itself.
    pub fn next_expiry(&self, now: f64) -> Option<f64> {
        self.remotes.values().filter(|r| !r.value.is_empty()).flat_map(|r| [r.seen_at + PRESENCE_MS, r.seen_at + VOUCH_MS]).filter(|&t| t >= now).reduce(f64::min)
    }

    /// Present remote devices for the avatar row (§9.1), ascending by installation.
    pub fn views(&self, now: f64) -> Vec<PeerView> {
        let view = |(&inst, r): (&u64, &Remote)| {
            let p = r.profile.clone().unwrap_or_default();
            let link = match r.link {
                Link::Open => PeerLink::Connected,
                Link::Handshake => PeerLink::Connecting,
                Link::Idle | Link::Unreachable => PeerLink::Relay,
            };
            let save_phase = r.presence.as_ref().map_or(p.save_phase, |s| s.save_phase);
            PeerView { inst, email: p.email, name: p.name, photo_url: p.photo_url, device_type: p.device_type, label: p.label, link, presence: r.presence.clone(), save_phase }
        };
        self.remotes.iter().filter(|(_, r)| Self::is_present(r, now)).map(view).collect()
    }
}

/// Timing of one mode (ms).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Cadence {
    pub poll: f64,
    /// Relay flush: this long after the last batch …
    pub flush: f64,
    /// … but no later than this after the first unsealed batch.
    pub flush_max: f64,
    /// Record refresh period (Company/Linked/RelayOnly).
    pub refresh: Option<f64>,
}

impl Cadence {
    /// `idle_ms`: time since the last local input (Alone) or since any editor record changed (Viewer).
    pub fn of(mode: Mode, idle_ms: f64) -> Cadence {
        let c = |poll, flush, flush_max, refresh| Cadence { poll, flush, flush_max, refresh };
        match mode {
            Mode::Alone => c(if idle_ms >= ALONE_IDLE_AFTER_MS { 60_000.0 } else { 20_000.0 }, 5_000.0, 30_000.0, None),
            Mode::Company => c(500.0, 150.0, 150.0, Some(RECORD_REFRESH_MS)),
            Mode::Linked => c(10_000.0, 2_000.0, 10_000.0, Some(RECORD_REFRESH_MS)),
            Mode::RelayOnly => c(1_000.0, 150.0, 150.0, Some(RECORD_REFRESH_MS)),
            Mode::Viewer => c(
                if idle_ms <= PRESENCE_MS {
                    1_000.0
                } else if idle_ms <= 1_800_000.0 {
                    5_000.0
                } else {
                    20_000.0
                },
                f64::INFINITY,
                f64::INFINITY,
                None,
            ),
        }
    }

    /// When the relay write for pending batches is due (§8.5 "Due"): the mode's flush delay, never within 2 s of
    /// the previous write. `first_pending_at`/`last_batch_at`: local times of the oldest and newest unsealed batch.
    pub fn flush_at(&self, first_pending_at: f64, last_batch_at: f64, last_write_at: f64) -> f64 {
        (last_batch_at + self.flush).min(first_pending_at + self.flush_max).max(last_write_at + WRITE_CAP_MS)
    }
}

/// Test command hook: `__IHN_TEST_API__.cmd('sync.mode.<name>', args)`.
pub fn test_cmd(_name: &str, _args: &str) -> Option<String> { None }

#[cfg(test)]
mod tests {
    use super::*;

    const A: u64 = 0x100;
    const B: u64 = 0x200;
    const C: u64 = 0x300;

    fn rec(n: u32, flags: &str) -> String { format!("1|{n}|kz0|file-{n}|AAAAAAAAAAAAAAAA|{flags}") }
    fn see(p: &mut Presence, now: f64, records: &[(u64, &str)]) -> Vec<u64> { p.observe(records.iter().map(|(i, v)| (*i, *v)), now) }

    #[test]
    fn record_values_and_keys() {
        assert_eq!(record_flags(&rec(1, "")), Some(RecordFlags { live: false, left: false, no_link: false }));
        assert_eq!(record_flags(&rec(1, "clx")), Some(RecordFlags { live: true, left: true, no_link: true }));
        for bad in ["", "1", "2|1|a|f|AAAAAAAAAAAAAAAA|", "1|1|a|f|AAAAAAAAAAAAAAAA", "1|1|a|f|short|", "1||a|f|AAAAAAAAAAAAAAAA|", "1|1|a|f|AAAAAAAAAAAAAAAA|q", "1|1|a|f|AAAAAAAAAAAAAAAA||", "1|1|A|f|AAAAAAAAAAAAAAAA|"] {
            assert_eq!(record_flags(bad), None, "{bad}");
        }
        let inst = 0x1234_5678_9abc_de00;
        let key = format!("ihn6_{}", ihn_core::id::inst13(inst));
        assert_eq!(key.len(), 18);
        assert_eq!(record_inst(&key), Some(inst));
        assert_eq!(record_inst("ihn6_0000000000zz"), None);
        assert_eq!(record_inst("ihn6_+000000000001"), None);
        assert_eq!(record_inst("ihn6_000000000000Z"), None);
        assert_eq!(record_inst("ihn_live_key_v1"), None);
    }

    #[test]
    fn presence_uses_first_sighting_of_each_value() {
        let mut p = Presence::default();
        let (r1, r2) = (rec(1, "c"), rec(2, "c"));
        assert_eq!(see(&mut p, 1000.0, &[(B, &r1)]), [B]);
        // The same value seen again does not refresh the window.
        assert!(see(&mut p, 100_000.0, &[(B, &r1)]).is_empty());
        assert!(p.present(B, 121_000.0));
        assert!(!p.present(B, 121_001.0));
        // A new value (the 60 s refresh) does.
        assert_eq!(see(&mut p, 121_500.0, &[(B, &r2)]), [B]);
        assert!(p.present(B, 241_500.0));
        // `l` = left at once; a malformed value is ignored (the last good one stays).
        let left = rec(3, "l");
        see(&mut p, 130_000.0, &[(B, &left)]);
        assert!(!p.present(B, 130_000.0));
        see(&mut p, 131_000.0, &[(B, &r2), (C, "garbage")]);
        see(&mut p, 131_500.0, &[(B, "garbage")]);
        assert!(p.present(B, 131_500.0) && !p.present(C, 131_000.0));
        // A record that disappeared is forgotten unless a channel is open.
        p.set_link(B, Link::Open);
        see(&mut p, 132_000.0, &[]);
        assert!(p.present(B, 900_000.0) && !p.vouched(B, 132_000.0));
        p.set_link(B, Link::Idle);
        see(&mut p, 133_000.0, &[]);
        assert!(p.remotes.is_empty());
    }

    #[test]
    fn vouching_needs_a_verified_record_within_ten_minutes() {
        let mut p = Presence::default();
        let r1 = rec(1, "");
        see(&mut p, 0.0, &[(B, &r1)]);
        assert!(!p.vouched(B, 1.0));
        p.verified(B);
        assert!(p.vouched(B, VOUCH_MS) && !p.vouched(B, VOUCH_MS + 1.0));
        // A new generation must be verified again; a demoted editor (no more record writes) lapses after 10 min.
        let r2 = rec(2, "");
        see(&mut p, 5_000.0, &[(B, &r2)]);
        assert!(!p.vouched(B, 5_001.0));
        p.verified(B);
        assert!(p.vouched(B, 5_000.0 + VOUCH_MS));
        assert_eq!(p.next_expiry(5_000.0), Some(5_000.0 + PRESENCE_MS));
        assert_eq!(p.next_expiry(200_000.0), Some(5_000.0 + VOUCH_MS));
        assert_eq!(p.next_expiry(700_000.0), None);
    }

    #[test]
    fn modes_follow_presence_and_links() {
        let mut p = Presence::default();
        assert_eq!(p.mode(0.0, true), Mode::Alone);
        assert_eq!(p.mode(0.0, false), Mode::Viewer);
        let (rb, rc) = (rec(1, "c"), rec(1, "c"));
        see(&mut p, 0.0, &[(B, &rb), (C, &rc)]);
        assert_eq!(p.mode(1.0, true), Mode::Company);
        p.set_link(B, Link::Handshake);
        p.set_link(C, Link::Open);
        assert_eq!(p.mode(1.0, true), Mode::Company);
        p.set_link(B, Link::Open);
        assert_eq!(p.mode(1.0, true), Mode::Linked);
        p.set_link(C, Link::Unreachable);
        assert_eq!(p.mode(1.0, true), Mode::RelayOnly);
        p.set_link(B, Link::Idle);
        assert_eq!(p.mode(1.0, true), Mode::Company);
        // Records lapse: the unreachable one is no longer present, the open peer still is.
        p.set_link(B, Link::Open);
        assert_eq!(p.mode(PRESENCE_MS + 1.0, true), Mode::Linked);
        p.set_link(B, Link::Idle);
        assert_eq!(p.mode(PRESENCE_MS + 1.0, true), Mode::Alone);
        assert_eq!(p.mode(1.0, false), Mode::Viewer);
    }

    #[test]
    fn scribe_is_the_smallest_present_installation() {
        let mut p = Presence::default();
        assert_eq!(p.scribe(B, 0.0), B);
        let (ra, rc, left) = (rec(1, "c"), rec(1, "c"), rec(2, "l"));
        see(&mut p, 0.0, &[(A, &ra), (C, &rc)]);
        assert_eq!(p.scribe(B, 1.0), A);
        assert_eq!(p.scribe(B, PRESENCE_MS + 1.0), B);
        see(&mut p, 10.0, &[(A, &left), (C, &rc)]);
        assert_eq!(p.scribe(B, 11.0), B);
        assert_eq!(p.scribe(C + 0x100, 11.0), C);
        // Every device elects the same scribe from the same records.
        let mut q = Presence::default();
        let rb = rec(1, "c");
        see(&mut q, 500.0, &[(B, &rb), (C, &rc)]);
        assert_eq!(q.scribe(A, 501.0), A);
        assert_eq!(p.scribe(B, 11.0), B);
    }

    #[test]
    fn cadence_table() {
        let c = Cadence::of(Mode::Alone, 0.0);
        assert_eq!((c.poll, c.flush, c.flush_max, c.refresh), (20_000.0, 5_000.0, 30_000.0, None));
        assert_eq!(Cadence::of(Mode::Alone, ALONE_IDLE_AFTER_MS).poll, 60_000.0);
        let c = Cadence::of(Mode::Company, 0.0);
        assert_eq!((c.poll, c.flush, c.refresh), (500.0, 150.0, Some(60_000.0)));
        let c = Cadence::of(Mode::Linked, 0.0);
        assert_eq!((c.poll, c.flush, c.flush_max), (10_000.0, 2_000.0, 10_000.0));
        assert_eq!(Cadence::of(Mode::RelayOnly, 0.0).poll, 1_000.0);
        assert_eq!([0.0, 120_000.0, 120_001.0, 1_800_000.0, 1_800_001.0].map(|idle| Cadence::of(Mode::Viewer, idle).poll), [1000.0, 1000.0, 5000.0, 5000.0, 20_000.0]);
        // Alone: 5 s after the last batch, at most 30 s after the first; always ≥ 2 s after the previous write.
        let alone = Cadence::of(Mode::Alone, 0.0);
        assert_eq!(alone.flush_at(1000.0, 1000.0, -1e9), 6000.0);
        assert_eq!(alone.flush_at(1000.0, 29_000.0, -1e9), 31_000.0);
        let company = Cadence::of(Mode::Company, 0.0);
        assert_eq!(company.flush_at(1000.0, 1000.0, -1e9), 1150.0);
        assert_eq!(company.flush_at(1000.0, 1000.0, 900.0), 2900.0);
        // Views carry the link and the newest save phase.
        let mut p = Presence::default();
        let r = rec(1, "c");
        see(&mut p, 0.0, &[(B, &r)]);
        p.remotes.get_mut(&B).unwrap().profile = Some(Profile { name: "Bea".into(), device_type: b'm', save_phase: 3, ..Default::default() });
        p.set_link(B, Link::Handshake);
        let v = p.views(1.0);
        assert_eq!((v.len(), v[0].name.as_str(), v[0].link, v[0].save_phase), (1, "Bea", PeerLink::Connecting, 3));
        assert!(p.views(PRESENCE_MS + 1.0).is_empty());
    }
}
