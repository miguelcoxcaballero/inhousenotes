# v5.16.33

Faster folder navigation with Rust scene planning.

- Rust/WASM plans the folder zoom, lid rotation and fades, and indexes grid rows
  for visible-card selection. WAAPI still runs movement on the browser compositor;
  navigation never waits for WASM and has a matching offline fallback.
- Opening and closing keep the latest 3D lid, immediate start, timings and interior
  reveal. Transition snapshots copy visible cards and compact offscreen rows;
  scrolling no longer measures every card in a large folder.
- Drive catalogue indexes replace repeated full-library searches while preserving
  folder order, aliases, offline availability and saved Home views. A local CPU
  benchmark of decorating 1,000 cards in a 5,000-record catalogue fell from about
  71 ms to under 1 ms; this measures that operation, not device frame rate.

The Android app loads the updated Rust and web assets; no new APK is needed.
The existing native 120 Hz support is retained. Actual frame rate depends on the
phone, its refresh-rate settings and load; it has not been measured on a physical
120 Hz device for this release.

Validation: 204 unit tests and release smoke checks pass; 35 browser tests pass,
including 1,000-card folders, nested back navigation, exact snapshot geometry,
variable PDF row heights, offline fallback and compositor interpolation checks.
Rust was rebuilt with the pinned 1.90.0 toolchain and offline/native resource
hashes regenerated. The frozen Android installer and A/B APK catalogs are unchanged.
