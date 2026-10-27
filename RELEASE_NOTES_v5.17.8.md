# v5.17.8

Quieter retries; no redundant photo hashing.

- Backing-off journal queues stop repeatedly acquiring storage leases and updating status on every network tick. Recheck at the earlier retry deadline or two seconds; fresh edits wake the scan immediately without bypassing a failed file's network backoff.
- A wake-generation guard prevents an in-flight empty scan from putting a newly persisted edit back to sleep.
- Repeated presence hints and predecessor discovery no longer shorten failed-read backoff. Explicit retry resets it; successful reads do not clear unrelated write errors.
- Seeding collaborative page fields now computes values only for missing registers. Repeated snapshots of seeded photos no longer hash the same data URLs; first seed, new pages and explicit background changes still calculate their values. Original image bytes and conflict metadata remain unchanged.
- Regressions cover idle/retry scheduling, scan races, manual retry, photo metadata equality and authoritative remote revisions.

Web and Android web assets updated; APK/frozen installer unchanged.
