# v5.15.22

Stable previews and native Android asset loading.

Web only publication; Android 1.0.19 is built and verified separately before its A catalog is published.

- Keep existing cards and decoded images when Drive rotates thumbnail URLs.
  Decode a changed image before swapping pixels; preserve the last good image
  on a failed refresh. Reuse cards while updating their opening metadata.
- Reduce idle home polling from five to thirty seconds and suspend it during
  folder animations, drawing, document opening and offline operation.
- Bound retained preview object URLs instead of retaining every image revision.
- Prepare 238 content-addressed public resources for the APK. Android serves
  matching bytes through a native streaming WebView client on its request thread,
  with exact SHA-256 matching, same-origin restrictions and normal Capacitor
  fallback for changed versions. Account data remains in its existing storage.
- The bundled PDF worker, UI fonts and core scripts avoid network/cache lookups
  on first load. New web versions reuse unchanged assets and fetch changed ones.
- Prioritize the foreground Android renderer without retaining it when hidden.

Validation: two native manifest/installer checks (including rejection of stale
bundled bytes), preview identity and decode-failure tests, cached offline opening
and library tests, folder navigation and PDF/ink worker rendering. Folder tests
now mock profile/root queries when reconnecting instead of sending fake tokens
to the real Google API. Physical Android performance still needs device testing.
