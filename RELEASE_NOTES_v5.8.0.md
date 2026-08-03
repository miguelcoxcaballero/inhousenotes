# Inhouse Notes v5.8.0

This release makes cold starts feel immediate and brings home pull-to-refresh to touch tablets.

## Faster, smoother startup

- Paints the correct light or dark background in the first frame, before the full stylesheet and application runtime load.
- Shows the signed-in home and its loading placeholders as soon as its HTML exists.
- Starts Drive loading in parallel with local IndexedDB recovery.
- Avoids rendering hidden document canvases and waiting for a hidden viewport when opening on the home screen.
- Loads the large PDF libraries after the visible interface, and loads web fonts without blocking the first paint.

## Tablet refresh

- Enables the existing pull-to-refresh gesture on every touch-capable tablet instead of limiting it to phone-width screens.
- Retains horizontal-axis locking over document cards so sideways browsing does not trigger a refresh.
- Accepts tiny fractional scroll offsets reported by some tablet browsers when the home is visually at the top.

## Android 1.0.8

- Uses matching native launch, status-bar, navigation-bar, and WebView colours.
- Supplies light and dark launch resources so Android no longer exposes unrelated black and white frames while creating the WebView.
- Keeps the local loader visually consistent if the remote page needs an extra moment to open.
