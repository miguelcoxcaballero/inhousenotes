# Shared updater validation · 2026-10-11

Android 1.0.11 (code 12) is published in channel A with the existing signing key. Web 5.11.43 includes the shared updater pinned by `inhouse-updater.lock.json`.

- Shared policy suite: 8 JavaScript cases and 4 publication cases pass in the canonical repository.
- Notes unit/release checks pass.
- The real Notes page passes 25 focused browser checks, including Android detection, native version display, both settings buttons, optional popup, persisted A/B selection, empty B, manual override and the 14-day reminder.
- Production 5.11.42 CI: 47 browser checks passed; the version assertion still expected 5.11.41 (fixed to read package.json) and the scanner `strong-right-curl-rose` geometry case failed its 0.015 maximum curve error threshold (measured 0.021147).
- Scanner code, geometry test and synthetic fixture are unchanged from pre-updater commit `da4e884e35483942ae7b9a30b4d8feb6dc756bab`; this release does not alter scanner geometry or weaken its test.
- The exact canonical JavaScript was fetched from the live site and compared byte for byte. Physical-device installation and signed-in account behavior were not exercised here.
