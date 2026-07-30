# Inhouse Notes v5.0.0

Inhouse Notes 5 introduces direct, resilient collaboration while keeping the Drive PDF as the only permanent document.

## Live collaboration

- Signed-in editors on multiple devices and different Google accounts connect directly with WebRTC.
- Short-lived encrypted rendezvous messages are attached to the existing Drive PDF; no Inhouse Notes processing server or secondary document is required.
- Same-device tabs coordinate through a local broadcast channel and a leader lease so they do not create duplicate remote sessions.
- Complete document state includes strokes, erasures, transformed objects, images, page order, page additions/deletions, page sizes, backgrounds, photo pages, calendar configuration and side panels.
- Large snapshots are chunked with backpressure and safety limits.
- Drive permissions are rechecked during a session. Losing edit access immediately changes the client to view-only.
- The existing Drive PDF polling and three-way merge remain active as the automatic fallback when peer-to-peer networking is unavailable.

## Public sharing without Google sign-in

- “Copy link” now confirms an `Anyone with the link — Viewer` Drive permission before copying.
- Public links retain the Drive resource key and are handled before OAuth.
- Public PDFs use a restricted browser Drive API key when configured.
- When browser CORS or download policy prevents importing PDF bytes, Inhouse Notes opens Google Drive’s anonymous embedded preview instead of displaying a sign-in requirement.
- Public views are read-only and refresh automatically. Workspace policies that prohibit public sharing are reported explicitly.

## Robust timeline

- Timeline histories from different devices are validated, deduplicated and merged instead of replacing one another.
- Histories are saved separately in IndexedDB using a document-specific key for local crash recovery.
- Every entry has a content hash, parent link, schema version and device identity.
- Restores create a durable recovery milestone first and are cancelled safely if that recovery point cannot be created.
- Timeline snapshots now preserve custom/photo backgrounds, images, legacy cover strokes, page sizes, page order, calendar configuration and page side panels.
- Identical saves are suppressed, nearby autosaves are coalesced and bounded pruning protects milestones without allowing unlimited PDF metadata growth.
- Remote timeline updates are merged into the local recovery history and persisted immediately.

## Compatibility and security

- OAuth and refresh tokens never cross a peer connection.
- Public anonymous visitors cannot edit or automatically join the private peer channel.
- Presence metadata now uses two compact Drive properties per device, supporting many more simultaneous devices and working across the web and Android OAuth clients.
- Full stroke fingerprints prevent transformed strokes from being missed during conflict detection.