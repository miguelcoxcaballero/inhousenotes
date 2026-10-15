# v5.15.12

Softer, more transparent file labels.

Web only; Android loads this update without a new APK.

- Reduce the thumbnail footer tint so more of the underlying preview remains
  visible. Keep the sampled image colour and contrasting black or white text.
- Reduce the footer blur from 6px to 1.5px while preserving its feathered edge.

Validation: release smoke checks, offline asset build and browser checks of
desktop/mobile layouts, light/dark themes, image tones, missing or cross-origin
thumbnails and file menus.
