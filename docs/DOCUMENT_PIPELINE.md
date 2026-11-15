# Document persistence and transport

Since 6.0.0 this is described in full by the v6 design documents; this page only points to them.

## Ownership

- Document model and edit operations: `rust/crates/ihn-core` (`doc.rs`, `op.rs`), `docs/v6/ARCHITECTURE.md` §4.
  Every edit is an order-independent operation; the session journals it to IndexedDB (`inhouse-notes-v6`, §7.1)
  before anything leaves the device.
- Sync: `rust/crates/ihn-sync` and `ihn-app/src/collab`, `docs/v6/SYNC.md`. Operations travel over P2P
  (`host/peer.js`) and through editor-sealed relay files in Drive; the save indicator's `saved` means "every own edit
  is sealed in the relay" (`docs/v6/BEHAVIOUR-CHANGES.md` N-1, N-2).
- PDF in Drive: rebuilt by one device (the scribe) after 10 s idle, when a device leaves alone and on Export
  (`ihn-app/src/export`, `host/export-worker.js` with pdf-lib, `docs/v6/ARCHITECTURE.md` §8). Ink is a vector layer
  over the unchanged source page.
- 5.x documents and local data: read by `ihn-core::v5` (§9) and migrated once by `ihn-app/src/migrate.rs` (§10).
- PDF parsing and rendering: pdf.js through `host/pdf.js`, with 5.18.0's size, type and page-count limits.

The 5.x pipeline (`runtime-core-v5.js`, `offline-library-v5.js`, `offline-sync-v5.js`, `document-processing*-v5.js`,
`peer-transport-v5.js`) is no longer used; its files stay deployed only for still-open 5.18.0 tabs until 6.0.1.
