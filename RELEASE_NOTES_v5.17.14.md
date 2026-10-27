# v5.17.14

Reuse unchanged live ink frames without decoding again.

- Repeated identical live-preview responses reuse a bounded, source-scoped cache instead of allocating all path points again. A 6,000-point frame read 50 times is parsed once instead of 50 times.
- Original expiry and activity deadlines still apply. Changed bytes always validate, while HTTP failures, failed preview callbacks, source replacement and stopped sessions retain their existing safety behavior.
- Cached strings have a total four-million-character limit and least-recently-used eviction; no stroke coordinates, colors or widths are simplified.
- Browser regressions now check definitive pen ink on the page canvas and the real cache-first startup/release activation behavior. The full CI suite has 45 minutes to finish instead of timing out after 20 minutes.
- Validation: 65 journal/live-preview checks, native asset checks, release smoke, and 33 browser cases covering synchronization, resilience and release activation.

Web and Android web assets updated; APK/frozen installer unchanged.
