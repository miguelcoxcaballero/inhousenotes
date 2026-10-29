# v5.17.22

A much lighter app: less heat, less data, smoother writing.

- Nothing that belongs to a document keeps running after you leave it: on Home, Drive traffic drops from about 400 requests per minute to none. After a minute in the background the document's live work pauses and resumes when you return.
- Alone in a document, the editor checks Drive for changes every 20 s instead of every 0.6 s and writes its presence every 30 s instead of every 2 s; live-collaboration signalling no longer polls Drive comments several times a second. When another device is present the fast cadence returns.
- Writing: whole-document saves, PDF builds and page reconciliation wait for a pause in writing instead of interrupting it; empty overlay canvases are no longer composited; page thumbnails are built without GPU readbacks; a hidden loading spinner no longer animates all session. In a throttled phone profile, long tasks while writing fell from 102 to 11 and the slowest frame from 550 ms to 150 ms.
- Idle animations (Home refresh spinner, loading shimmer, the "connecting" ring) stop drawing frames when nothing is happening.
- Background uploads are driven by events with backoff and reuse one processor instead of reloading the app every 10 s. A document left before its Drive upload finished now uploads right after the app is reopened (about 2 s instead of a minute or never).
- Web and Android web assets updated; APK/frozen installer unchanged.
