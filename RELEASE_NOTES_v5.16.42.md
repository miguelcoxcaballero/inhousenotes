# v5.16.42

Preserve offline ink when Home resumes uploads.

- Home no longer replaces a durable per-document checkpoint with the single-editor cache restored at startup. Pending uploads use the saved draft until the document is explicitly opened.
- Restore the notebook page/journal session at startup, including existing caches without session metadata, so persisted ink is not rejected as belonging to another document.
- Offline checkpoint session identities differ across app launches, preventing previous-session prepared PDFs and acknowledgements from matching a new editor session.
- Invalidate prepared PDF bytes whenever a new durable checkpoint is committed.
- Android uses regenerated offline/native assets; no installer changes.

Regression coverage reproduces the lost stroke on cold Home restart and checks that the hidden processor merges and uploads a real PDF containing the final durable stroke without opening the document.

Direct ICE-TCP was tested separately with scripts/probe-direct-ice-tcp.mjs: Chromium establishes the normal local data channel but supplies no TCP candidates and cannot connect with UDP candidates excluded. This is a local diagnostic, not an Android/network traversal guarantee. No relay or ineffective TCP-only production setting was added.
