# Inhouse Notes v5.10.0

This release focuses on maintainability, recovery history size, production testing and content security.

## Modular runtime

- Moved the application runtime out of the two-megabyte HTML document and into a cacheable `app-v5.js` entrypoint.
- Isolated pre-paint boot, bounded network helpers, shared-content security and Timeline encoding into independent modules.
- Kept direct collaboration in its existing dedicated core and transport modules, with explicit versioned loading for every local runtime file.

## Lighter, safer Timeline

- Introduced Timeline schema v3 with a full checkpoint every eight entries and at every milestone.
- Stores intervening history as page-level incremental deltas, including page additions, deletion, reordering, document name and calendar configuration changes.
- Verifies every delta chain before restoring it and rejects corrupt archives instead of applying partial history.
- Reads legacy full-snapshot Timeline arrays and migrates local IndexedDB history transparently on the next save.

## Production and chaos coverage

- Added real Chromium checks for startup, blocked IndexedDB, last-second local checkpoints, slow network timeouts and hostile shared content.
- Added a multi-client browser harness that exercises concurrent strokes, page moves, page deletion, cross-account field edits, reconnection after a network change and delayed stale Drive fallback data.
- Added a GitHub Actions production gate covering unit tests, release checks, browser chaos tests and the typed rebuild's test/build pipeline.

## Security hardening

- Added a restrictive Content Security Policy with inline script execution disabled.
- Added SHA-384 integrity checks to the pinned jsPDF, PDF.js and pdf-lib browser dependencies.
- Hardened PDF loading by disabling PDF JavaScript evaluation, enforcing source/size/page/image bounds and rejecting unsafe URL schemes.
- Moved persisted calendar HTML sanitization behind a strict tag, class and attribute allowlist with an input-size limit.

The Android native wrapper remains version 1.0.10 because it loads this web release directly and no native code or signed APK content changed.
