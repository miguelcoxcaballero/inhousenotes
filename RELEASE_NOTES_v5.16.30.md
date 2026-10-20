# v5.16.30

Smoother folder opening on Android and the web.

- The folder's name stays above its rule and the ⋮ menu stays in its corner
  when the lid starts to open. The opening ghost is laid out at its final size,
  so its pixel measures (name spacing, menu, corners, grain, shadows) now scale
  with it instead of shrinking, and the menu is no longer removed from it.
- Folders and documents inside keep their normal grain, shadows and labels
  during the zoom instead of jumping to them when the folder lands.
- The folder board's shadow is a box-shadow instead of filter: drop-shadow():
  it looks the same and is painted once rather than recomputed on every frame.

The Android app loads this web release; no new APK is needed.
