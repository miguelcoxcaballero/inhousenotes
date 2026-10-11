# Inhouse Notes v5.11.49 · A

Clearer save status and safer retries.

Save details distinguish the local copy from Google Drive, show upload progress and explain offline, reconnect and failure states. The status button is separate from the read/edit toggle, with keyboard opening, Escape dismissal and focus restoration.

Local save failures remain visible until a successful local write. Full checkpoints retry up to three times, including pending edits in read mode; retries and late results stay tied to their document session. Confirmed local timestamps no longer borrow a Drive save time.

Closed save details and hidden page thumbnails avoid unnecessary rendering, and reopening Manage Pages no longer adds duplicate drag handlers.

The existing Android loader uses the hosted application; no APK or channel catalog changes are required for this web update.
