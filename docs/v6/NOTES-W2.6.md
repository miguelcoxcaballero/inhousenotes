# W2.6 SYNC-BASE — notes for W2.7, W2.8 and the session (W3.1)

Files: `rust/crates/ihn-sync/src/{lib.rs,mode.rs,wire.rs,sim.rs}`. Branch `v6-w2_6`. `cargo test -p ihn-sync`:
23 tests, < 1 s debug.

## What is there

- `lib.rs`: `Sync` (implements `ihn_core::doc::SyncEngine`), `SyncEffect`, `SyncIo`, `HttpReq`, `DriveMeta`
  (the executor re-encodes Drive's metadata JSON into it; sync never parses JSON), `RtcReq`/`RtcEvent`, `Lifecycle`,
  `LockEvent`, `SyncRecord` + `Stored` (the folded persisted state the session is opened with), `Phase` (SYNC §16.1),
  `Config`, and the plug-in surface for the protocol parts: `trait Part`, `Core`, `Change`, `Flush`.
- The session itself owns: the metadata poll (§8.6 step 1) and its backoff, presence/modes/cadence, the phase,
  hidden → suspended after 60 s, the `ihn6-net` lock, routing of request answers to the part that asked, frame
  decoding, `Part` reassembly (a bad frame closes the channel and reaches parts as `RtcEvent::Fault`), journal
  retention every 10 s (`Replica::retain` with seal ∧ known peer/tab VVs ∧ newest known export), `SyncStatus` and
  `peers()`.
- `mode.rs`: `Presence` (receiver-local `seen_at`, `present`, `vouched`, `set_link`, `verified`, `scribe`, `views`),
  `Mode`, `Link`, `Cadence::{of, flush_at}`, `record_flags`, `record_inst`, the §20 timing constants.
- `wire.rs`: `Frame` (every §10.5 type), `Profile`, `Mail`, `fragment`, `Reassembly` with the §10.5 limits.
- `sim.rs` (`cfg(test)`): `Sim` (virtual clock, per-device skew, event queue), `Drive` (properties with the 124-byte
  and 30-key limits, etag on any write / head on content writes only, `If-Match` 412, permissions and
  `permissionIds`, link-readable sync-folder files, reserved ids, copies, `inject`, lost responses, `rate_limit`,
  request log with `count`), fake data channels (ordered, link delay, `block_p2p`), BroadcastChannel and Web Locks
  per browser (`add_sibling`), `crash` / `crash_on(effect, before|after)` and restart from `Dev::stored`,
  `SimReplica` (a real `Doc` + journal), `sha256`, stored-block `deflate`/`inflate`. Usage example in the module
  header.

## How W2.7 / W2.8 plug in

- Implement `crate::Part` in your module and return it from that module's `pub fn part()` (already declared in the
  five stubs, returning `None`); `lib.rs` collects them in the order relay, export, peer, entropy, live. You do not
  edit `lib.rs`.
- Requests: `cx.http / cx.digest / cx.deflate` return an id; the answer comes back only to the part that asked
  (`Part::http` …). `cx.persist(rec)` persists and folds into `cx.stored`. `cx.wake(at)` asks for a tick;
  `Part::tick` runs on every call into the session, so "do what is due, wake for the rest".
- Blackboard fields of `Core` (who writes / who reads): `flush` (session → relay writer clears it), `record_due`
  (session on Alone → company → relay writer clears it), `mail_out`/`mail_changed` (peer → relay), `mail_in`
  (relay → peer), `peer_vv` (entropy → retention), `need_export` (relay → export), `presence.set_link` (peer),
  `presence.verified(inst)` (relay reader, after the digest check — vouching §10.1 needs it), `remotes[..].profile` /
  `.presence` (relay reader / entropy, for the avatar row), `status.relay_writing` / `status.export` (their parts).
- `Change::Leave`: the session closes when no request of any part is still unanswered, or after 30 s.
- Tests: `Sim::add_dev(account, inst, skew, parts_fn)` takes `fn() -> Vec<Box<dyn Part>>`, so a scenario runs with
  exactly the parts it needs; `sim.rs`'s own tests show a minimal record writer (`Beacon`).

## For the host / session (W3.1, `host/peer.js` in W2.8)

- `on_io` carries no time: call `sync.at(now)` before every `on_io` (`tick(now, ..)` sets it itself).
- `SyncEffect::Timer { at }` replaces the previous timer; an early firing is harmless (it is re-armed).
- `SyncEffect::Lock { hold }` requests/releases `ihn6-net:<account>:<docId>`; answer with
  `SyncIo::Locks(LockEvent::Net { held })`. A tab without the lock stays in `Loading` and does no network work.
- `HttpReq::Meta` must be answered with `DriveMeta::encode()` bytes and the v2 etag; `status` 0 = no answer.

## Deviations from the plan

1. `SyncEffect::Lock`, `Sync::at`, `HttpReq`/`DriveMeta`, `RtcEvent::Fault`, `Lifecycle::{Created, Input, Contact,
   PageHide, NetChange}` are additions to the SYNC §18.2 sketch (which names only the families): the lock and the
   clock have to cross the boundary somewhere, and the metadata codec keeps JSON out of the pure crate.
2. One line (`pub fn part()`) was added to each of the stubs `relay.rs`, `export.rs`, `peer.rs`, `entropy.rs`,
   `live.rs` (owned by W2.7/W2.8, not yet started), so that those two units never need to edit `lib.rs` in the same
   wave.
3. A lone editor that has never written is invisible to a newcomer until either writes (§9.2 says the same: no
   writes unless the user edits); the record itself is written by the relay writer (W2.7). The mode tests therefore
   use a 40-line stand-in record writer.
4. No browser states belong to this unit (nothing in `ihn-app` calls the engine yet), so there is no parity state
   file; `cargo check --target wasm32-unknown-unknown -p ihn-wasm` passes.
