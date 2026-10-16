# v5.15.21

Faster local opening; consistent offline library.

Web only; the installed Android shell loads this update without a new APK.

- Show the saved library immediately, including the same recent, starred and
  shared lists, folder hierarchy and colours. Refresh Drive without replacing
  saved cards with skeletons or waiting for Google's login script.
- Keep thumbnails, the account avatar and UI fonts on the device. Documents
  without a local copy stay visible but disabled in grey. Probe actual connectivity
  because a WebView can report an online network that has no internet access.
- Open cached documents before contacting Drive, online or offline. Restore native
  snapshots directly and hydrate only the first page initially. Revalidate remote
  revisions using the existing merge path, retaining unsynced local changes.
- Keep a compact per-account availability index; opening the library no longer
  deserializes every cached PDF and handwriting snapshot. Upgrade existing drafts
  and pending operations without modifying their content.
- Serve the installed app shell immediately from local storage and check for
  releases in the background. Fix the clean-original PDF cache lookup and retain
  metadata and previews while recording local edits.

Validation: cold offline PDF opening, persistent previews/avatar/fonts and folder
navigation, cached opening while Drive requests are stalled, v1 storage migration,
retention, offline creation, conditional uploads and headless concurrent merging.

Checks passed: 37 distinct browser regressions across the targeted suites,
172 unit tests, offline asset generation and release smoke checks. Local opening
of the three-page fixture took approximately 230–250 ms on the test machine;
this is not a physical Android-device benchmark.
