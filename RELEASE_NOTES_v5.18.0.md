# v5.18.0

A much smaller app: about 59,000 lines removed, with the same look and behaviour.

- The scanner, the Diary, the Agenda and Google Calendar are taken out for now. Their buttons stay where they were with a "Soon" badge and show "Soon" when tapped. Existing agenda/diary documents and pages with scanned images still open, render and save as before (their side-panel data is kept).
- Removed compatibility code for app versions older than 5.0, the implicit OAuth redirect, the PDF export bridge v1, pre-5.0 presence formats, dead functions, unused CSS and markup, an abandoned TypeScript rebuild and 220 old release notes. The Rust core shrank from 127 KB to 54 KB.
- Live collaboration: closing a document to Home stops its signalling at once, a second device connects in about 4 s instead of 20 s, and edits no longer build a whole-notebook snapshot when nobody can receive it. Read-only viewers see live ink sooner.
- Fixed Drive PDF export failing with an undefined progress callback.
- Web and Android web assets updated; APK/frozen installer unchanged.
