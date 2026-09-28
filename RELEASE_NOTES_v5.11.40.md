# Inhouse Notes v5.11.40

- Fixed initial P2P document convergence: asynchronous page cloning is now awaited before hashing, sending or relaying a snapshot to another device or local tab. Previously unresolved Promise objects could become empty pages on the wire even while the connection appeared active.
- Cooperative copies reject and retry snapshots if an edit or pending remote stroke changes the document during copying. Malformed page packets are rejected without consuming their sequence or confirming a successful merge.
- Snapshot receipts distinguish delivery from convergence using the actual merged content hash. If a receiver retained additional edits, the sender requests that merged version automatically instead of treating its old snapshot as a confirmed delta base.
- Full-document merge hashes include pages held in IndexedDB, not just visible page bodies. ACK tracking begins before sending chunks, so a fast response on the interactive stream cannot be overwritten by completion of the bulk send.
- Regression coverage uses real WebRTC between isolated browser contexts with simulated Drive signalling: divergent three-page documents converge without another edit, deletions remain deleted, simultaneous edits survive, and offline edits converge again after reconnecting. Existing live preview and three-account connection coverage remains in place.

The Android shell is unchanged; the hosted app receives this update. Reopen the document on both devices to load the corrected client. Connection and transfer time still depend on network conditions and document size.
