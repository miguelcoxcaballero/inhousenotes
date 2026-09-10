# Inhouse Notes v5.11.19

## No more full reload for small Drive changes

- Changing a folder's colour, starring/unstarring, moving a file, or trashing/restoring one no longer wipes every card in Recents, Starred, Shared and the current folder back to a loading skeleton before redrawing them.
- These actions now reuse the existing diff-render path (already used by the 5-second background refresh) so only the item that actually changed is patched in place.

## Create folders from the file manager

- Added a "New folder" button next to My Drive, with a small name prompt, so folders can be created directly in the app instead of only from drive.google.com.
- The new folder appears immediately in the current view without a full reload.

## Slightly faster first peer-to-peer connection

- Tightened the cadence at which an initiating device checks for the other device's WebRTC answer during the 0.8-3 second window where it typically arrives, shaving roughly half a second to a second off a typical first connection.
- Added a steady (non-pulsing) ring around a peer's avatar once a direct connection is established, so "connected" reads as clearly as the pulsing "connecting" state does.

### Known limitation, not changed in this release

Peer-to-peer connects over WebRTC signalled through Google Drive, using STUN only (no TURN relay). Two devices behind a symmetric NAT or carrier mobile network (a common case for phone + tablet on different mobile connections) may not be able to establish a direct route at all; the app keeps retrying and silently falls back to Drive-based sync, which is reliable but slower. Making peer-to-peer always connect on every network would require a TURN relay, which is an infrastructure decision, not a code fix.

## Saving

Investigated the reported saving issues in depth. The exit-save path (Home button, tab close, backgrounding) already routes through the hardened, test-covered logic added in v5.4.0-v5.11.2 and did not reproduce any data-loss or stuck-save bug in this pass. If saving problems are still happening, please share the exact steps (device, browser, what you were doing) so a future fix can target the real cause instead of guessing.

## Tests

- `npm run test:unit` no longer fails on a default Windows checkout: `tests/release-v5-smoke.mjs` was comparing multi-line source snippets against CRLF-normalized files, which never matched with `core.autocrlf=true`. Source reads are now normalized to LF before comparison.

## Android

- The existing Android `1.0.10` shell loads this web release automatically; no native binary change is required.
