//! Web Locks (effect family 3.9, executed by `host/idb.js`) and the three locks of ARCHITECTURE §7.1:
//!
//! | Name | Holder | Helper |
//! |---|---|---|
//! | `ihn6-slot:<account>:<n>` | each tab, for its lifetime: actor slot `n` | [`acquire_slot`] |
//! | `ihn6-net:<account>:<docId>` | one tab (or the Android headless page) per document | [`net_name`] + [`request`] |
//! | `ihn6-migrate:<account>` | the migrating tab | [`migrate_name`] + [`request`] |
//!
//! A lock is identified by the request id that asked for it. [`request`] completes with an `Event::Io` at the
//! module that owns the id: `status` [`GRANTED`], [`BUSY`] (only with `if_available`), or
//! [`status::UNAVAILABLE`] / [`status::ERROR`]. Without `if_available` the request waits until the holder
//! releases it or its page goes away. [`release`] frees a held lock or withdraws a waiting request (which then
//! never completes). The browser frees every lock of a page that is closed or crashes.

use super::sv::Value;
use super::{fx, later, status};
use crate::abi::{fam, Io};
use crate::app::Mod;
use crate::{App, Host};

/// `Io::status` of a granted lock request.
pub const GRANTED: i32 = 1;
/// `Io::status` of an `if_available` request for a lock that is held elsewhere.
pub const BUSY: i32 = 0;
/// `Io::status` given to [`acquire_slot`]'s `notify` when all 256 slots are taken (SYNC §7.1: the tab then opens
/// documents read-only).
pub const EXHAUSTED: i32 = -17;

pub fn slot_prefix(account: &str) -> String { format!("ihn6-slot:{account}:") }
pub fn slot_name(account: &str, slot: u8) -> String { format!("ihn6-slot:{account}:{slot}") }
pub fn net_name(account: &str, doc: &str) -> String { format!("ihn6-net:{account}:{doc}") }
pub fn migrate_name(account: &str) -> String { format!("ihn6-migrate:{account}") }

/// Asks for the exclusive lock `name`.
pub fn request(host: &mut dyn Host, req: u32, name: &str, if_available: bool) {
    let out = host.out();
    let m = out.w.rec(fam::FX, fx::LOCK_REQUEST);
    out.w.u32(req);
    out.w.str(name);
    out.w.u8(if_available as u8);
    out.w.end(m);
}

/// Releases the lock requested with `req`, or withdraws the request if it still waits.
pub fn release(host: &mut dyn Host, req: u32) { host.out().fx_id(fx::LOCK_RELEASE, req); }

/// Asks which locks whose name starts with `prefix` are held or waited for, in any tab of this origin. The
/// completion is read with [`names`].
pub fn query(host: &mut dyn Host, req: u32, prefix: &str) {
    let out = host.out();
    let m = out.w.rec(fam::FX, fx::LOCK_QUERY);
    out.w.u32(req);
    out.w.str(prefix);
    out.w.end(m);
}

/// The `(held, pending)` lock names of a [`query`] completion, each sorted.
pub fn names(io: &Io) -> (Vec<String>, Vec<String>) {
    let v = if io.status >= 0 { Value::decode(io.data).unwrap_or_default() } else { Value::Undef };
    let list = |key: &str| v.get(key).items().iter().filter_map(|n| n.as_str().map(String::from)).collect();
    (list("held"), list("pending"))
}

/// The actor of a tab: the install id (low 8 bits zero) with the slot.
pub fn actor(inst: u64, slot: u8) -> u64 { inst & !0xff | slot as u64 }

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SlotState {
    #[default]
    None,
    /// Being acquired.
    Pending,
    Held(u8),
    /// All 256 slots are held by other tabs.
    Exhausted,
    /// No Web Locks in this context.
    Unavailable,
}

/// The actor slot of this tab.
#[derive(Default)]
pub struct Slot {
    account: String,
    state: SlotState,
    /// The pending query or lock request; once held, the lock's id.
    req: u32,
    querying: bool,
    candidate: u8,
    /// Slots known to be held elsewhere.
    taken: [u64; 4],
    notify: Vec<u32>,
}

/// The slot this tab holds for `account`, if any.
pub fn slot(app: &App, account: &str) -> Option<u8> {
    match app.st.store.slot.state {
        SlotState::Held(n) if app.st.store.slot.account == account => Some(n),
        _ => None,
    }
}

pub fn slot_state(app: &App) -> SlotState { app.st.store.slot.state }

/// Takes the first free actor slot of `account` for this tab and keeps it for the tab's lifetime (SYNC §7.1).
/// `notify` gets an `Event::Io` whose `status` is the slot (0..=255), [`EXHAUSTED`] or [`status::UNAVAILABLE`] —
/// also when the slot is already held. A slot held for another account (sign-out, sign-in) is released first.
pub fn acquire_slot(app: &mut App, host: &mut dyn Host, account: &str, notify: u32) {
    let slot = &mut app.st.store.slot;
    if slot.account != account {
        release_slot(app, host);
    }
    let slot = &mut app.st.store.slot;
    match slot.state {
        SlotState::Held(n) => return later(app, host, notify, n as i32),
        SlotState::Pending => return slot.notify.push(notify),
        _ => {}
    }
    *slot = Slot { account: account.into(), state: SlotState::Pending, querying: true, notify: vec![notify], ..Slot::default() };
    let req = app.req(Mod::Store);
    app.st.store.slot.req = req;
    query(host, req, &slot_prefix(account));
}

/// Gives the slot back (sign-out). Pending notifications are dropped.
pub fn release_slot(app: &mut App, host: &mut dyn Host) {
    let slot = std::mem::take(&mut app.st.store.slot);
    if matches!(slot.state, SlotState::Held(_)) || (slot.state == SlotState::Pending && !slot.querying) {
        release(host, slot.req);
    }
}

fn settle(app: &mut App, host: &mut dyn Host, state: SlotState, status: i32) {
    let slot = &mut app.st.store.slot;
    slot.state = state;
    for notify in std::mem::take(&mut slot.notify) {
        later(app, host, notify, status);
    }
}

/// Requests the lowest slot not known to be taken.
fn try_next(app: &mut App, host: &mut dyn Host) {
    let slot = &mut app.st.store.slot;
    let Some(n) = (0..=255u8).find(|n| slot.taken[*n as usize / 64] & 1 << (n % 64) == 0) else {
        return settle(app, host, SlotState::Exhausted, EXHAUSTED);
    };
    slot.candidate = n;
    slot.querying = false;
    let name = slot_name(&slot.account, n);
    let req = app.req(Mod::Store);
    app.st.store.slot.req = req;
    request(host, req, &name, true);
}

/// Handles the completions of the slot machine; false when `io` is not one of them.
pub(super) fn on_io(app: &mut App, host: &mut dyn Host, io: &Io) -> bool {
    let slot = &mut app.st.store.slot;
    if slot.state != SlotState::Pending || slot.req != io.req {
        return false;
    }
    if io.status < 0 {
        settle(app, host, SlotState::Unavailable, status::UNAVAILABLE);
    } else if slot.querying {
        let prefix = slot_prefix(&slot.account);
        for name in names(io).0 {
            if let Some(n) = name.strip_prefix(&prefix).and_then(|n| n.parse::<u8>().ok()) {
                slot.taken[n as usize / 64] |= 1 << (n % 64);
            }
        }
        try_next(app, host);
    } else if io.status == GRANTED {
        let n = slot.candidate;
        settle(app, host, SlotState::Held(n), n as i32);
    } else {
        // Another tab took it between the query and the request.
        slot.taken[slot.candidate as usize / 64] |= 1 << (slot.candidate % 64);
        try_next(app, host);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::fake::boot;
    use super::*;

    const ACCOUNT: &str = "ana@example.test";

    #[test]
    fn names_follow_the_architecture_table() {
        assert_eq!(slot_name(ACCOUNT, 7), "ihn6-slot:ana@example.test:7");
        assert_eq!(net_name(ACCOUNT, "doc-1"), "ihn6-net:ana@example.test:doc-1");
        assert_eq!(migrate_name(ACCOUNT), "ihn6-migrate:ana@example.test");
        assert_eq!(actor(0xaabb_cc00, 0x2a), 0xaabb_cc2a);
    }

    #[test]
    fn exclusive_locks_queue_and_release() {
        let (mut app, mut host, mut db) = boot();
        let (a, b, c, q) = (app.req(Mod::Store), app.req(Mod::Store), app.req(Mod::Store), app.req(Mod::Store));
        let name = net_name(ACCOUNT, "doc-1");
        request(&mut host, a, &name, true);
        request(&mut host, b, &name, true);
        request(&mut host, c, &name, false);
        query(&mut host, q, "ihn6-net:");
        db.pump(&mut app, &mut host);
        assert_eq!(db.take(a).unwrap().0, GRANTED);
        assert_eq!(db.take(b).unwrap().0, BUSY);
        assert!(db.take(c).is_none(), "waits for the holder");
        let (status, data) = db.take(q).unwrap();
        assert_eq!(names(&Io { req: q, status, obj: 0, data: &data }), (vec![name.clone()], vec![name.clone()]));
        release(&mut host, a);
        db.pump(&mut app, &mut host);
        assert_eq!(db.take(c).unwrap().0, GRANTED, "the next tab takes over");
        release(&mut host, c);
        db.pump(&mut app, &mut host);
        assert!(db.held.is_empty());
    }

    #[test]
    fn a_tab_takes_the_first_free_slot() {
        let (mut app, mut host, mut db) = boot();
        for n in [0, 1, 3] {
            db.held.insert(slot_name(ACCOUNT, n), 0);
        }
        db.held.insert(slot_name("zoe@example.test", 2), 0);
        let (first, second) = (app.req(Mod::Store), app.req(Mod::Store));
        acquire_slot(&mut app, &mut host, ACCOUNT, first);
        acquire_slot(&mut app, &mut host, ACCOUNT, second);
        assert_eq!(slot_state(&app), SlotState::Pending);
        db.pump(&mut app, &mut host);
        assert_eq!((db.take(first).unwrap().0, db.take(second).unwrap().0), (2, 2));
        assert_eq!(slot(&app, ACCOUNT), Some(2));
        assert_eq!(slot(&app, "zoe@example.test"), None);
        assert!(db.held.contains_key(&slot_name(ACCOUNT, 2)));
        // Asking again answers from a timer; another account releases the old slot first.
        let again = app.req(Mod::Store);
        acquire_slot(&mut app, &mut host, ACCOUNT, again);
        assert!(db.take(again).is_none());
        db.pump(&mut app, &mut host);
        assert_eq!(db.take(again).unwrap().0, 2);
        let other = app.req(Mod::Store);
        acquire_slot(&mut app, &mut host, "zoe@example.test", other);
        db.pump(&mut app, &mut host);
        assert_eq!(db.take(other).unwrap().0, 0);
        assert!(!db.held.contains_key(&slot_name(ACCOUNT, 2)));
    }

    #[test]
    fn a_slot_lost_to_a_racing_tab_moves_on_and_256_tabs_exhaust_them() {
        let (mut app, mut host, mut db) = boot();
        // The query sees nothing, then another tab wins slot 0 before the request.
        let notify = app.req(Mod::Store);
        acquire_slot(&mut app, &mut host, ACCOUNT, notify);
        db.step(&mut app, &mut host);
        db.held.insert(slot_name(ACCOUNT, 0), 0);
        db.pump(&mut app, &mut host);
        assert_eq!(db.take(notify).unwrap().0, 1);

        let (mut app, mut host, mut db) = boot();
        for n in 0..=255 {
            db.held.insert(slot_name(ACCOUNT, n), 0);
        }
        let notify = app.req(Mod::Store);
        acquire_slot(&mut app, &mut host, ACCOUNT, notify);
        db.pump(&mut app, &mut host);
        assert_eq!(db.take(notify).unwrap().0, EXHAUSTED);
        assert_eq!(slot_state(&app), SlotState::Exhausted);

        let (mut app, mut host, mut db) = boot();
        db.no_locks = true;
        let notify = app.req(Mod::Store);
        acquire_slot(&mut app, &mut host, ACCOUNT, notify);
        db.pump(&mut app, &mut host);
        assert_eq!(db.take(notify).unwrap().0, status::UNAVAILABLE);
        assert_eq!(slot_state(&app), SlotState::Unavailable);
    }
}
