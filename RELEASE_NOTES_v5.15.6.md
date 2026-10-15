# v5.15.6

Thumbnail colours with contrasting file labels.

Web only; Android loads this update without a new APK.

- Home file footers use a colour sampled from the visible thumbnail, with
  black or white names and actions chosen for readable contrast.
- Colour sampling is small, cached and limited to two concurrent probes.
  Private thumbnails remain visible if pixel access is unavailable.
- Missing images retain their labels, menu and light blurred fallback.

Checked on desktop and mobile in light and dark themes, including bright,
dark, landscape and unavailable previews and restricted thumbnail access.
All six offline library checks pass.
