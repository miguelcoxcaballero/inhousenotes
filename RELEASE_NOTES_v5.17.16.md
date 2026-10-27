# v5.17.16

Keep unchanged page caches when remote ink arrives.

- Successful merges now report the pages whose caches they already invalidated. The journal adapter no longer discards every page's PDF overlays, raster export cache and scroll previews for one incoming stroke.
- The merge tracks actual mutations, including duplicate-item removal on another page. A normal stroke invalidates one page; a cross-page move invalidates both affected pages while preserving the rest.
- Reorders, legacy page identities, interrupted recovery, agenda changes and renames retain document-wide invalidation. Direct P2P receives the same completed invalidation guarantee; older or malformed adapter results keep the safe fallback.
- Durable page/metadata ordering, PDF version receipts, original photo bytes and stroke geometry are unchanged.
- Validation: 50 browser cases covering cache reuse, duplicate reconciliation, recovery, photo deltas, live synchronization and PDF save latency; native-asset and release smoke checks.

Web and Android web assets updated; APK/frozen installer unchanged.
