# v5.17.9

Sync ink without resending unchanged photos.

- Finished strokes on unchanged photo pages travel as small ink operations. Idle capture skips redundant page shells and full photo strings without inventing a save receipt.
- Page metadata baselines include causal field revisions, so a newer revision restoring the same photo still carries its bytes to a divergent device. Each page's fields travel with that page's payload.
- Initial recovery, new pages, photo changes, deletions and document metadata retain their existing durable path. Failed captures retry without installing a successful baseline; edits during asynchronous capture defer mixed-version payloads.
- Sixteen browser regressions pass, including two isolated replicas, simultaneous photo page creation, persisted photo bytes, recovery, erasures and live strokes. The unchanged 1 MiB photo fixture emits a 255-byte stroke operation; this is a payload measurement, not a network latency guarantee.

Web and Android web assets updated; APK/frozen installer unchanged.
