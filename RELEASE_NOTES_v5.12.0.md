# Inhouse Notes 5.12.0

- Remembered sessions open the library immediately while authentication refreshes; PDF libraries load from the app origin.
- A per-account offline library keeps the Drive folder tree and recently opened documents for 14 days (configurable). Pin documents for indefinite offline access; unavailable files appear dimmed without connectivity.
- Create folders and documents offline. A durable outbox resolves parent folders, retries interrupted uploads, and retains concurrent revisions before merging.
- Android background synchronization uses WorkManager in the new APK. Android can defer work for battery restrictions; a force-stopped app must be reopened. Browser background sync depends on browser support and valid authentication; complex merges resume in an open app.
- Rust/WASM accelerates scanner pixel warping and PDF color-channel preparation, with the existing JavaScript fallback. Browser measurements including copies: median warp 77.9 → 71.0 ms; channel preparation 24.4 → 15.0 ms on the validation machine, not a guarantee for every device.
