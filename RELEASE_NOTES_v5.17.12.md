# v5.17.12

Avoid serializing photos that exceed the ink limit.

- The lightweight journal checks a conservative lower bound of photo and embedded-image sizes before JSON serialization. Repeated attempts with obviously oversized assets no longer allocate multi-megabyte JSON strings just to reject them.
- Payloads that may fit still use the exact existing size/escaping check. Custom JSON serialization and omitted fields retain their original behavior.
- Oversize errors have a specific code for future PDF fallback handling. This release changes neither save receipts nor fallback semantics: original photos, durable storage failures and pending state remain intact.
- Five regressions cover repeated oversized attempts without serialization or queue writes, aggregate image sizes, exact fitting bytes, escaped overflow and genuine storage failure.
- Validation: 61 journal, live-preview and native-asset checks; 22 browser cases covering photo edits, recovery and live synchronization; release smoke passed.

Web and Android web assets updated; APK/frozen installer unchanged.
