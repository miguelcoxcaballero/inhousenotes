# Shared Google login

`google-oauth.js` is the canonical OAuth implementation for Notes, Read and Paellas.
It is extracted from Notes' PKCE, refresh-token and GIS flow, with random state,
transaction expiry, single-flight requests, request deadlines, scope validation and
sign-out generation guards. Consumers supply identity, redirect URI, storage keys,
UI callbacks and their browser-opening adapter. Do not fork authentication in apps.

Sync with `python tools/sync.py notes ../inhousenotes --auth-only` (also `read` and
`paellas`). This independently locks OAuth in `inhouse-oauth.lock.json`; it does not
replace a consumer's existing updater revision or modify the frozen installer.
The auth-only files are not used by YouTube, Instagram, Photos or Learn.

## Android account reuse

New APKs register `GoogleSessionProvider` and the main-frame, exact-origin
`GoogleSessionBridge`. Grants are encrypted with a non-exportable Android Keystore
AES-GCM key and written atomically in the app's no-backup directory. Providers
expose only read IPC; callers and providers must match BOTH a fixed package and its
permanent release certificate SHA-256. Debug and repackaged APKs cannot share.
There is no wildcard origin, general JavascriptInterface, public file or external
storage containing the shared grant. Read also checks its /inhouse-read/ path.

A newly installed app discovers the most recently used account from installed,
upgraded official apps. An app with its own existing grant keeps that account;
a background refresh never switches it to another app's newly selected account. It keeps the originating OAuth client ID when renewing a
shared refresh token. It checks scopes before using any token; Paellas requests
its additional Sheets/Forms/Apps Script permissions separately and uses the shared
account's email as a Google login hint. Google can still require consent or account
interaction. Existing Notes/Read refresh tokens migrate on their next successful
refresh. Existing APKs need an update before they can provide or consume sessions.

Local sign-out clears this app's encrypted grant and persists a local opt-out from
rediscovery; it does not revoke the grant in the other apps. Explicit sign-in clears
the opt-out. Uninstalling every app removes the stored grants. Old shells lacking
the new bridge retain their previous per-app authorization path.

The browser uses the same module with Google Identity Services. Browser sessions
remain origin-local: cross-domain token sharing is deliberately not implemented.
Google may reuse its browser account, subject to its consent and cookie policies.
No backend is required. Paellas uses its newly registered Android OAuth client and
`com.inhouse.paellas:/oauth2redirect` with PKCE on new APKs. Older APKs retain their
existing implicit callback, selected by the absence of the new native bridge.

## Provenance and verification

Notes release android-v1.0.28 (manifest SHA-256 ac3e141b753b8eb17ed56627bfdbb0c577d626158c9817a60b251f833b89cec7)
and Read android-v1.1.25 (manifest SHA-256 b7705e72b92dce041e09c3a241b33b636c349ef0fde23d36ff04c102a49b7abb)
supply the pinned signing certificates, read from their APK v2 signing blocks.
Paellas android-v1.0.1 was also verified with apksigner: its permanent Inhouse
certificate matches android/README.md (SHA-1 3dbeba8c6210b9003221dbdd0f459469d1896b76).
Changing a pin requires verifying a real release's signing identity; never trust
package names alone or register the updater's frozen provider as this provider.

Run `node --test tests/google-oauth.test.cjs`. With sibling Notes and Read checkouts,
run `python tests/oauth_registration.py` to verify the actual generated shells.
With Notes served at localhost:4173 and Paellas at localhost:4174 and Playwright
installed in Notes, run `node tests/google-oauth.integration.cjs` for the mocked
PKCE-to-cross-app-restore browser test (requires Chromium at /usr/bin/chromium). Build all consumer APKs and test on a
device: install Notes, connect, install Read, restore without a picker; then install
Paellas, grant missing permissions, restart all apps; test cancellation, local logout,
revocation, offline startup, process recreation and unsigned/repackaged callers.
Unit tests do not replace this device acceptance test.
