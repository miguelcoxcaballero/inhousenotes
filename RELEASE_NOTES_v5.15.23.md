# v5.15.23

Publish Android 1.0.19 (versionCode 20) to A: local resources and smoother previews.

- The APK includes the 238 verified, content-addressed resources introduced in
  v5.15.22. Matching requests load directly from Android assets; changed web
  resources continue through the normal update path.
- Preserve the existing package, signing key and user data. B remains unchanged.
- Keep the preview stability, bounded image memory and quieter background polling
  from v5.15.22. Install the APK update to receive native resource loading.

Validation: Android build and signature continuity checks succeeded; the exact
public APK was downloaded and compared with the built file in CI. Its SHA-256 is
02297827f49af1dd9ada88359cba8e318a38cc23bd2a6abe63e27391d70ebbdb
and its size is 6,793,608 bytes. All 238 bundled files match the source manifest.
The A catalog was generated from that verified public binary using the canonical
publisher. 174 unit checks and 16 targeted browser regressions passed for v5.15.22;
this follow-up changes only version/catalog metadata. No physical-device speed
benchmark has been performed.
