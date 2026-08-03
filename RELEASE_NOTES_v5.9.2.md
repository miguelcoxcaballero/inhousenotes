# Inhouse Notes v5.9.2

## Drawing tray icon repair

- Restores the visible outlines of the eraser, lasso, undo and redo controls in dark mode.
- Gives the intentionally light drawing tray its own surface-specific dark ink instead of inheriting the editor theme through `currentColor`.
- Applies the same stable contrast to the drag handle, stroke-size dots and active colour outline.
- Keeps the tray's existing dimensions, spacing, colours and interaction design unchanged.

## Android

- The existing Android `1.0.10` shell loads this web release automatically; no native binary change is required.
