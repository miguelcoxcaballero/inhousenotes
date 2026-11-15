# W6.1f — presence popup: location, device label, last activity

Closes NOTES-W4.5 deviations 4 and 5 (the `Actividad del documento` popup against 5.18.0
`renderPresencePopup`, app-v5.js:24982).

## What changed

- **Boot config** (`host/main.js`, `abi::Config`): after the touch points, `ihn_init` carries three optional
  strings: `platform` = `(navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || ''`,
  `time_zone` = `Intl.DateTimeFormat().resolvedOptions().timeZone || ''`, `region` =
  `new Intl.Locale(navigator.language || '').region || ''` (`''` when it throws). The region is not derived in Rust:
  `Intl.Locale` canonicalises tags (case, aliases, scripts), which a tag split would not reproduce. Missing strings
  decode as empty (the export worker sends none).
- **Location** (`collab::location_label`): 5.18.0 `detectPresenceLocationLabel()`: `REGION · tz`, tz, region or
  `Unknown`. Shown on the own row. Other devices read `Ubicación desconocida`, exactly as in 5.18.0: its packed
  presence (`packPresenceIdentity`/`parsePackedPresence`) has no location slot (`l: ''`), so 5.18.0 never shows a
  remote location. The relay `Profile` and P2P frames are therefore unchanged (no wire change).
- **Device label** (`collab::device_label` / `label_of`): 5.18.0 `detectDeviceLabel()` in full: iPhone / iPad /
  Android from the user agent, then `/Windows/i`, `/Mac/i`, `/Linux/i` on the platform → Windows PC / Mac / Linux PC,
  else the raw platform, else Mobile / Tablet / PC. The label sent to other devices is clamped as 5.18.0's packed
  identity (`clampPresenceValue(…, 16)` without `|`, `wire_label`); the own row shows it unclamped.
- **Last activity** (`presence::render_popup`): the own device = now (5.18.0 `t: Date.now()`). Another device = when
  this device last heard from it, on this device's clock (B-Y5, SYNC §9.1): now while its channel is open, else the
  local time its record value was first seen (`Remote::seen_at`), converted to `Date.now()` ms; each row has its own
  absolute time (`toLocaleString`). Devices in an account and accounts are ordered newest activity first, stable,
  as 5.18.0 `groupPresenceByAccount` (`presence::group`).
- `BEHAVIOUR-CHANGES.md` B-Y5 now names the popup's last-activity time.

## What still differs from 5.18.0

- **Other device's last activity** (B-Y5): 5.18.0 shows the other device's last heartbeat stamped on *its* clock
  (heartbeats every 2–20 s, so normally `hace menos de 1 min`). v6 never reads another device's clock: a relay-only
  device shows when its record last changed here (records refresh every 60 s in Company/Linked, so a present
  relay-only device can read `hace 1 min`); a device with an open channel shows now.
- **Main device** (B-Y6): the scribe, unchanged.
- `presence/popup` on phone-412 fails both before and after this change (same cause, fewer pixels: 260 973 →
  246 235 light): in 5.18.0 the other device's `… · Main device` line wraps to two lines, so 5.18.0's popup is
  taller than v6's and its extra row falls outside the expected box measured on v6's popup. That is B-Y6, not
  location or label.

Popup DOM after the fix (desktop-1280 light; phone-412 is the same), `<` 5.18.0, `>` v6:

```
<   <div class="presence-device-connection">
<     "This device · coordination leader · idle"
>   <div class="presence-device-connection main">
>     "This device · coordination leader · idle · Main device"
<     "Ubicación desconocida · Última actividad hace menos de 1 min (10/30/2026, 10:00:04 AM)"
<   <div class="presence-device-connection main">
<     "Reconnecting peer-to-peer · Drive active · idle · Main device"
>     "Ubicación desconocida · Última actividad hace menos de 1 min (10/30/2026, 10:00:00 AM)"
>   <div class="presence-device-connection">
>     "Reconnecting peer-to-peer · Drive active · idle"
```

The own row (`Linux PC · This device` / `Android · This device`, `US · UTC · Última actividad hace menos de 1 min
(10/30/2026, 10:00:05 AM)`) and the other device's label and location lines are identical.

## Not changed

- `device_type` still compares the viewport width where 5.18.0 `detectDeviceType` reads `screen.width` (only for a
  touch Macintosh); outside this unit.
