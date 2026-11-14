# W2.8 SYNC-PEER — notes for W2.7, W4.5 and integration

Files: `rust/crates/ihn-sync/src/{peer.rs,entropy.rs,live.rs,peer_tests.rs}`, `host/peer.js`,
`tests/v6/e2e/internal-peer.spec.mjs`. Branch `v6-w2_8`. `cargo test -p ihn-sync`: 34 tests (11 of them W2.8's),
< 1 s debug; `internal-peer.spec.mjs`: 2 tests, ~3 s.

## What is there

- `peer.rs` (`Peers`, a `crate::Part`): one `Conn` per remote installation, SYNC §10.4 states
  `Idle/Offering/Answering/Connecting/Open/Backoff/Unreachable` mapped onto `mode::Link` (Handshake for the four
  middle ones). Initiator = smaller `inst`; the larger offers only 8 s after it first saw the other; glare as §10.4;
  offer 15 s / connect 12 s timeouts; backoff `1 s × 2^(f−1)` ≤ 60 s ±20 %; 3 failures ⇒ Unreachable for 5 min
  (mode RelayOnly). Mail: own offers/answers in `Core::mail_out` (+`mail_changed`), removed when the channel opens
  or after the 60 s TTL; incoming `Core::mail_in` accepted once per `(from, sid, kind)` within the TTL since first
  local sighting, only from vouched installations. Hello is checked (`doc_id`, `inst`); Ping after 2 s of silence,
  dead after 8 s, network-change probe with `clamp(3×RTT+200, 300, 2000)`; transient losses retry at once. `Bye` on
  Suspend/Leave/leader lost/CanEdit(false)/offline. Hello/Presence frames of a peer fill `presence.remotes[inst]`
  (`profile`, `presence`) for the avatar row.
- `entropy.rs` (`Entropy`): per open peer `known` (acked) / `sent` vectors; every call sends what the replica holds
  beyond `sent` (≤ 1 MiB per `Batches` frame, ≤ 32 frames per destination per call), so local batches, forwarding
  between peers, peer → tabs and relay → tabs/peers are one rule. `Err(Compacted)` ⇒ `Snapshot`. `Have` 200 ms after
  receiving; resend from `known` 5 s after an unacknowledged send. Tabs: `Hello` once, a tab `Hello` is answered with
  `Have` + what it lacks, no acks. `Core::peer_vv` holds open peers, closed peers while still vouched (so a short
  reconnect gets batches, §7.3), and the tabs' last heard VV for 60 s.
- `live.rs` (`Live`): `LivePreview` → `InkBegin/InkPts/InkEnd/InkCancel/Erase/EraseEnd` to the tabs and direct
  peers (`≤ 24,000` points, erase ≤ every 50 ms with the last state always sent before `EraseEnd`); own
  `PresenceState` → `Presence` frame. Receiver table `Core::live: live::Previews` (`strokes` by live id, `erases` by
  gesture, `version` bumped on every change): a preview goes when a `Batches` frame puts its `InkEnd` item, on cancel /
  erase end, when its peer is no longer open, or 15 s after its last frame. A peer's live frames are forwarded to the
  tabs by the leader tab only.

## How W2.7 plugs in (relay)

- Publish `cx.mail_out` in every relay write and clear `cx.mail_changed`; a changed mailbox is a write trigger
  (§8.5 "Due", still under the 2 s cap).
- After the digest check of a record's file: `cx.presence.verified(inst)`, then push the entries addressed to us as
  `cx.mail_in.push((inst, mail))` (re-pushing on later generations is harmless: dedupe is here).
- Batches the relay reader applies reach peers/tabs by themselves (entropy compares VVs every call).

## How W4.5 plugs in (binding + overlay)

- Remote ink overlay: read `sync.core.live` after each `tick`/`on_io`; repaint when `version` changed.
- `SyncEffect::Rtc` / `Send` → effect ops of family 3 below; `Send { to: Tabs }` → `bc_send` on the document's
  channel `ihn6:` + fnv1a64hex(account) + `:` + docId (opened with `bc_open` when the session opens, closed with it).
- Events arrive as `MESSAGE` with `chan::PEER` (6); map them to `SyncIo::Rtc(..)` / `SyncIo::Tab(..)`.

### `host/peer.js` ABI (family 3, ops `0x60..0x67`)

Peer = `u32 lo, u32 hi` of the installation id. Strings and bytes as in the rest of the out-buffer.

| Op | Payload | Meaning |
|---|---|---|
| `0x60` rtc_create | peer, `u8 offer`, `u32 n`, n × `str ice` | new `RTCPeerConnection` (`max-bundle`, `rtcpMux require`) with negotiated channels 0 `ihn6`, 1 `ihn6-bulk`; offer=1 ⇒ create the offer and report it once gathered |
| `0x61` rtc_remote | peer, `u8 kind` (1 offer, 2 answer), `str sdp` | `setRemoteDescription` |
| `0x62` rtc_answer | peer | create the answer, report it once gathered |
| `0x63` rtc_close | peer | close; no event follows for that connection |
| `0x64` rtc_send | peer, `u8 ch`, `bytes` | send; the bulk channel queues while `bufferedAmount ≥ 256 KiB` (low threshold 128 KiB) |
| `0x65` bc_open / `0x66` bc_send / `0x67` bc_close | `str name` [, `bytes`] | the document's BroadcastChannel |

Events (`host.message(6, bytes)`), `bytes = [u8 ev][u32 lo][u32 hi][rest]`: 1 local description (`u8 kind`, SDP
UTF-8) → `RtcEvent::Local`; 2 channel open (`u8 ch`); 3 channel closed (`u8 ch`); 4 message (`u8 ch`, data);
5 bulk drained; 6 connection state (`u8`: 0 new, 1 connecting, 2 connected, 3 disconnected, 4 failed, 5 closed; a
failed step also reports 4). Tab message: `[u8 16][u32 name length][name][data]` → `SyncIo::Tab(data)`.
Gathering rule: complete, or 70 ms after the first `srflx` candidate, at most 650 ms. ICE servers are given by Rust
(`peer::ICE`, the four Google STUN servers of 5.x).

## Deviations

1. `lib.rs` (W2.6's): one additive field, `Core::live: live::Previews`. The parts are private to `Sync`, and the
   overlay needs the receiver table; no other way to reach it without editing `lib.rs`.
2. `scripts/build-wasm.mjs` (W1.1's): the first-party JS budget (100 KiB) now goes through the same
   `IHN_SIZE_REPORT_ONLY` switch W3.1 added for the wasm budgets. The tree was at ~100.4 KB before this unit; the
   complete executor is 5.3 KB (compacted; its documentation lives here). JS is 105,361 bytes now. The default
   build still fails on it: the budget needs a decision at integration (raise to ~112 KiB, or minify `host/*`).
3. P2P asset pulls (`AssetWant`/`AssetPart`) are not implemented: `Replica` exposes assets only as host blob
   handles (`BlobHandle = u32`), so the pure crate cannot read or create asset bytes. Assets come from the relay
   asset files and the export (§10.7's fallback); a received `AssetWant` is ignored (the asker times out and falls
   back). Doing it needs a `SyncEffect` to read/make blobs — an ABI change for W4.5/integration.
4. A tab `Hello` is answered with `Have` (+ the batches the new tab lacks), not with `Hello`: tab messages carry no
   sender identity (all tabs share `inst`), so a Hello answered with a Hello would loop. Same information.
5. Vouching grace: a new record generation is unverified until its file is read, which would drop every connection
   at each 60 s record refresh. The peer machine keeps the previous vouching for 30 s (`VOUCH_GRACE_MS`) while the
   record itself is ≤ 10 min old; a demoted editor (no more records) still lapses at 10 min.
6. Live ink drawn in a non-leader tab reaches the other tabs only (SYNC §10.7: live ink is not forwarded, except
   to tabs); its batches reach peers through the leader.
7. `peer.selftest` is a browser test of the shipped executor driven with out-buffer records from the spec (loopback
   RTC: offer/answer, both channels, ordered control and 2.4 MB bulk with backpressure and `drained`, close with no
   stray events; BroadcastChannel echo between two pages). No Rust driver yet: the Rust encoder of these ops and the
   event decoder are part of the session binding (W4.5).
8. The scenario tests use a 90-line stand-in relay (`Mailman` in `peer_tests.rs`: record + mailbox file, digest
   check replaced by a generation check, no batches), so every batch a test sees arrive came over P2P or the tabs.
