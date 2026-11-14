# W4.5 COLLAB — notes for W5.x and integration

Branch `v6-w4_5`. Owns `rust/crates/ihn-app/src/collab/{mod.rs,presence.rs}`, `rust/crates/ihn-app/src/drive/relay.rs`,
`tests/v6/e2e/11-two-devices.spec.mjs`, `tests/v6/parity/states/presence.mjs`.

## What exists now

- `collab` (binding, `mod.rs`): one `ihn_sync::Sync` per open document, started on `session::sig::OPENED` (the
  session must be persistent and signed in) after one `kv` read (`inst`, `idpool`, `relay-folder`, `relay-nolink`,
  `relay-gc`); `Stored` is built from the `docs` record + those keys. Inputs: committed local batches
  (`sig::COMMITTED` → `drain_local` → `local_batch` + `Lifecycle::Input`), live packets of the pen and the eraser
  (`collab::wake`, called by `pen.rs`/`eraser.rs` right where they push a packet: same frame, no extra rAF), pen contact
  (`Lifecycle::Contact` from `sched.is_busy()`), visibility / pagehide / online / offline / `shell::sig::NETWORK`,
  `chan::PEER` messages (RTC events and the document's BroadcastChannel), Drive answers from `drive::relay`,
  digest / deflate answers. Effects: Drive → `drive::relay::exec`; RTC / tab sends → `host/peer.js` records
  `0x60..0x67`; digest / deflate → `fx`; `Persist` → `session::persist` (SyncRecord → store `Record` conversion; a
  leaving engine whose session already closed writes the store directly); `Timer` → `T_TICK`.
  After every call: `session::set_sync_status`, `save_state::set_catching_up` (phase `CatchingUp`), the desk rename
  (§14.7: `base_title(drive name)` → `DocName` op when the register differs, `record.name` = the Drive name), the
  avatars, and the remote overlay (repainted in the next frame when `core.live.version` moved: v5
  `redrawRemoteLiveStrokePreviews` on the page's `remote-live-overlay`).
- Lifecycle: `shell::sig::VIEW` ≠ `editor` or `session::sig::CLOSED` → `Lifecycle::Leave` with a *detached* replica
  (the document's VV and its own batches above the seal, captured while the session still exists), so the exit flush
  and the `l` record still go out; the engine is dropped when it reports `Closed` and no relay request is pending.
  Sign-out / another document → dropped at once (requests aborted).
- `ihn6-net`: the session owns the lock (NOTES-W3.1). `Lock { hold: true }` is answered from `Session::net_holder`
  (polled every 500 ms while waiting: tab 2 takes over when tab 1 closes), `Lock { hold: false }` at once.
- `presence.rs`: avatars per SYNC §9.1 with 5.18.0's markup (`renderPresenceAvatars`: overflow `+N`, last-first order,
  `--presence-z`, title suffixes, `live` device badge, peer badge, save ring with `--presence-ring-delay`), the popup
  (`renderPresencePopup`: own device first, accounts grouped by e-mail, transport `This device · coordination
  leader|local tab` / `Peer-to-peer` / `Drive fallback`, `Main device` = the scribe, phase labels), open/close by the
  avatars, the × button, a click elsewhere and Escape, and `positionPresencePopup`. Ring timing: `uploading` reads
  `processing` for 900 ms, `saved` lasts 2.2 s after a change (receiver-local times).
- `drive/relay.rs`: the executor contract of NOTES-W2.7 for every `HttpReq` (Meta → `DriveMeta`, Media, Create
  multipart into the sync folder, Update media PATCH, Props PATCH with `null` deletes, Put = multipart PATCH with
  `If-Match` answering `headRevisionId`, Delete, GenerateIds one per line, SyncFolder find → create → share
  answering `<id>` or `<id>\n1`). Creates wait while a folder lookup is in flight (or start one): a relay file must
  land in the link-shared folder or other accounts cannot read it. Status 0 = no answer; Drive's error body is kept.
- Test commands: `collab.state` (`{active, doc, inst, phase, leader, mode, leaving, peers[], unsealed,
  peersConnected, live, overlayPages, avatars}`), `collab.live`, `drive.relay.busy`.

## For W5.3 / W5.4 / W5.6

- The export desk (`sync.core.export.build/merge/force/done`) is **not answered** here: `collab::with_sync(app, |s| …)`
  gives the engine, `collab::kick(app, host)` runs it after you set `built`/`merged`/`force`. Until W5.4 answers,
  exports never run (no PDF is uploaded; the relay alone carries edits) and a leaving engine closes after its relay
  requests (desk jobs pending ⇒ the 30 s `LEAVE_LIMIT`).
- `collab::created(app, host)` feeds `Lifecycle::Created` once the first Drive create of a document made on this
  device succeeded (W5.4); until then its engine stays `Loading { not_created }` (no poll, no relay).

## Verified

- `cargo test -p ihn-app`: green (new: relay JSON/meta/multipart, record ↔ `Stored` round trip, avatar markup,
  ring timing).
- e2e `11-two-devices.spec.mjs` (5 tests, ~2.5 min): (a) P2P over Chromium loopback: `p2p-connecting` →
  `p2p-connected` + peer badge, live ink in B's overlay table before A lifts (first point typically 10–40 ms in the
  test, measured with polling), 12 concurrent strokes + a stroke erase converge to identical snapshots, both relays
  sealed, two `ihn6_*` records, popup rows; (b) P2P blocked (relay-only ICE), Ana + Ben editing + Cho viewer:
  converge through the relay with 100 ms mock latency, relay latency samples < 4 s, the viewer shows both editors
  and sends no write request; (c) B offline, both edit, back → converge and seal; (d) two tabs: tab 2 gets tab 1's
  stroke over BroadcastChannel at once, closing tab 1 makes tab 2 the leader, its stroke is sealed and a fresh device
  opens both; (e) a Drive-side rename by Ben reaches A's open document title and stays.

## Deviations

1. **Files outside the owned list:** `editor/pen.rs` and `editor/eraser.rs` call `crate::collab::wake(app, host)` where
   they push live packets (6 one-line calls). Polling `drain_live` once per frame would add one frame of latency (the
   collab frame runs in the next rAF) or need a permanent rAF loop.
2. **Packets before batches:** a `STATE` signal (raised inside `session::edit`) only queues the presence and arms the
   tick; live packets are queued before any pump. Otherwise the stroke's batch (already in the session's VV before its
   IDB commit) went out ahead of its `InkEnd`, and the receiver kept the preview for 15 s.
   **Found, not mine (W3.1 / W2.8):** `Replica::batches_since` serves local batches before their IDB commit (they are
   in `doc.vv` and the journal at `edit` time), so P2P/tab peers may receive a batch before it is durable locally
   (SYNC §6 invariant 1). The relay seals only what it uploads, so durability accounting is unaffected.
3. **Erase previews are not drawn** (the `erases` table is kept by sync; drawing hidden items needs a page render with
   an exclusion set from `editor::pages`, not available). Strokes in progress are drawn; erases show when the batch
   lands (P2P: one RTT later).
4. **Popup location** is always `Ubicación desconocida` (5.18.0 shows `region · timezone` for the own device: the
   boot config has no time zone and `IntlOp` cannot read `resolvedOptions().timeZone`). Remote last activity =
   now (a present device). The own device's name falls back as 5.18.0 (`name || email || You`).
5. **Device label** comes from the user agent (5.18.0 reads `navigator.platform` for Windows/Mac/Linux).
6. **The physical `ihn6-net` lock is not released on suspension** (hidden 60 s): the session holds it until it closes.
   The engine stops all network work logically; another tab of the same browser takes over only when this one closes.
7. **One engine at a time:** opening another document drops a leaving engine (its persisted `relay_pending` is
   resumed by the next session of that document).
8. **Join time:** a device alone writes nothing until it edits (SYNC §9.2), so two devices that only *open* the
   document do not see each other; presence and P2P start with the first edit (or a present editor's record).
9. **Parity:** `presence/connecting`, `presence/relay-only` (B-Y5), `presence/save-ring`, `presence/popup` seed the
   remote device per side (5.18.0 `p_*` packed presence; v6 `ihn6_*` record + a relay file in Ben's link-shared
   folder). `p2p-connected` needs two browsers on both sides and is covered by e2e 11 (a), not by parity.
   Quick run (`--quick --states presence`, 8 comparisons): 0 unexplained, 8 expected. The avatar DOM is identical to
   5.18.0's in `connecting` / `save-ring`; the remaining pixels are the phase of the `p2p-connecting` glow and ring
   animations, which start when the avatar is first drawn (B-Y5: presence derived on the receiver's clock). The
   popup differs in the own location line (deviation 4), the `Main device` (B-Y6: the scribe) and the other device's
   last-activity time (B-Y5). `relay-only` is B-Y5 (no P2P class on a present relay-only device).
10. **e2e outside this unit:** with this branch, `01-signin` (offline status line on Home) and `internal-store`
    (a `401` console line from the signed-out connectivity probe) fail; neither opens a document, so no collab code
    runs in them (not investigated further). `04-ink`, `04b-erase`, `04c-select`, `06-images`,
    `internal-editor-view`, `07-share`, `internal-peer` pass.
11. The wasm grows to 1,650,192 bytes raw (646,520 gzip): `ihn-sync` is linked into the app for the first time.
