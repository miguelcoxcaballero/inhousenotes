# Inhouse Notes v5.2.0

This release makes fast navigation safe and substantially improves documents opened from public links.

- Home is shown only after the latest strokes and page metadata have reached a durable IndexedDB checkpoint.
- A pending-upload marker survives an immediate browser or app close and automatically resumes the Drive upload on the next launch.
- Browsers warn before closing while a Drive upload is still pending.
- Successful normal or background uploads clear the recovery marker only after Drive confirms the new revision.
- Anonymous public links now stay inside the regular Inhouse Notes editor chrome instead of switching to a separate full-screen Google preview.
- Public documents use the normal read-only canvas whenever direct public PDF access is available.
- Anonymous revision checks now run every 1.8–2.5 seconds instead of every 60 seconds, with content fingerprints preventing unnecessary re-imports.
- If direct public PDF access is blocked by the browser, a double-buffered Drive preview remains inside the editor and refreshes without replacing the app UI.
- Public tabs on the same device also receive immediate document snapshots through the existing local live channel.
