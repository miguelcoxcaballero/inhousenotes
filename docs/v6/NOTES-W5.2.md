# W5.2 SHARE-PUBLIC — notes for the other units

Branch `v6-w5_2` (from `v6-int` at `185da5c`). Owns `rust/crates/ihn-app/src/{panels/share.rs, drive/perms.rs,
doc/public.rs}`, `tests/v6/e2e/07-share.spec.mjs`, `tests/v6/parity/states/share.mjs`.

## What exists now

- `drive::perms` — 5.18.0's permission calls: `list`, `add`, `update_role`, `revoke`, `owner_info` (+ `parse_owner`),
  `file_info`, and `set_link(file, reader|writer|restricted)` (list → DELETE / PATCH / POST `anyone` with
  `allowFileDiscovery:false`; one completion for the caller). `parse_list` reads `permissions(id,type,role,…)`.
- `panels::share` — the whole modal of spec/home.md §9.1: loading skeleton, owner info → permission list, owner rows
  (5.18.0's inline-styled DOM, avatar fallback, `Role for <name>` selects, `Remove`), the reader's owner-only row,
  invite (`Sending...`, `Invitation sent` / `Failed to share`), row role change (reverted on failure), remove,
  General access and link role, Copy link (`Preparing public link...`, the checks and messages of 5.18.0, the §8.1 URL
  built by `share::share_link`, clipboard). It also shows `#btn-drive-share` while a Drive document (or a public one)
  is open (`sig::OPENED` / `CLOSED` / `VIEW`) — **W3.2**: 5.18.0 does this in `updateDocTitle`; keep or drop yours,
  the two agree.
- `doc::public` — signed-out links (SYNC §12.4): on `sig::AUTH` while not connected with a pending `?drive=` link, the
  anonymous download (usercontent, then `uc`, `credentials:'omit'`, `no-store`, 45 s, three attempts, `%PDF-` in the
  first KiB, `Content-Disposition` name), the size + FNV-1a fingerprint of the first and last 64 KiB, the read-only
  open, the 2.5 s poll (skipped while hidden or in flight; a new fingerprint is merged into the open document and
  shows `Public document updated.`), the preview iframe fallback (`#public-inline-preview`, gestures, zoom controls,
  iframe reload at most every 7 s while the poll fails), the writer link's sign-in gate, the anonymous reader's
  `Read only` toggle with the banner hidden, and the banner's `Ask to Edit` button (`askToEdit`). No request carries
  `key=` (e2e-checked).

## Deviations

1. **`doc/open.rs` (W3.1's file) got a small entry point**, `open::open_public(file, blob, size, refresh, notify)`:
   the same pdf.js → Keywords → inflate → `load_snapshot` → attachments pipeline, from a blob downloaded anonymously,
   memory-only session (empty account), `can_edit:false`. `refresh` merges a newer download into the open document
   without the overlay; its failure leaves the document as it was. Changes: two `Job` flags, `start_pdf` factored out
   of `checked`, three guarded lines in `advance` and `fail`. Duplicating the pipeline in `public.rs` would have been
   ~120 lines.
2. **5.x and plain PDFs behind a public link** go through the same `sig::FOREIGN` seam as a signed-in open; until W3.4
   claims them, the open fails and the preview iframe is shown (as 5.18.0 does when the import fails).
3. **`doc::links` keeps its own pending link** after an anonymous reader open (5.18.0 clears it). `doc::public` clears
   `shell::pending_share`, so Home's status is right; but if that reader then signs in, `links` opens the document
   with the account (5.18.0 would stay on the anonymous copy). Harmless; `links.rs` is W3.1's.
4. **`Make a Copy`** on the viewer banner is not wired: it opens the folder picker (`openFolderPicker('copy')`), W5.5.
5. **Owner facts are not written back** to the session's `driveMeta` when the modal refreshes them (5.18.0
   `saveDriveSession`); they live in the modal's state for the next refresh. A capability change still reaches the
   session through the next open's metadata and, later, W4.5's revocation probe.
6. **Editor-owned parts of the public view** (toolbar hidden, `state.pages` cleared, zoom reset) are W3.2's; the
   preview clears `#canvas-container` as 5.18.0 does.
7. The wasm size gate is report-only (808,538 bytes raw / 329,945 gzip for the whole app on this branch).

## Verified

- `cargo test -p ihn-app`: 103 green (link format, endpoints, preview URL, `Content-Disposition`, `%PDF-` scan, FNV
  fingerprint against `Math.imul`, permission/owner parsing).
- e2e `07-share.spec.mjs` (4 tests): owner adds Ben (POST `?sendNotificationEmail=true`), changes and removes him,
  anyone-with-link Viewer → Editor, Copy link → exactly `…/?drive=v6-doc&public=1&role=writer&name=Fixture.pdf&mode=edit`
  and the viewer form, Restricted removes the link; a reader's owner-only modal and backdrop close; the signed-out
  reader link opens read-only from `drive.usercontent.google.com` (no `/files` API call), an owner export is picked up
  by the poll within 2.5 s + download, never `key=`; the writer link's gate; the preview fallback with zoom and the
  failing poll still downloading.
- Parity `--quick --states share` (10 states × 2 cells): **`share/public-preview` equal on both cells** (preview
  iframe, zoom controls, `Read only` toggle, title, hidden toolbar). The other states open a 5.x scene
  (TESTING §3.1): v6 cannot import 5.x before W3.4, so the open fails — the signed-in states never get a share button
  (script error on B) and `public-viewer` / `sign-in-gate` show the preview fallback instead of the document. They
  are the gate for W3.2 + W3.4 + W5.2 together; the modal itself is covered by the e2e above.
- Also run: 00-boot, 02-home, internal-doc green. `01-signin` "offline: NETWORK signal…" fails with `#drive-status` =
  `Unable to load this folder.` (Home's folder listing, W2.5, overriding W1.3's expected text) — not touched here.
- The parity owner check found and fixed: the public view sets the document title and hides the toolbar
  (5.18.0 `updateDocTitle` / `updateViewerModeUI`), since no editor session exists in the preview.
