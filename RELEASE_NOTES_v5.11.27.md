# Inhouse Notes v5.11.27

## Immediate pen response

- Pen-down no longer resizes and repaints the full document canvas before showing the first point. Resolution updates wait until interaction settles.
- Starting a stroke no longer clears the lasso overlays of every page; only an existing selection is cleared.
- Routine saves do not recompress unchanged timeline history. The secondary localStorage backup also waits for an idle moment instead of interrupting handwriting.

## Android

- The existing Android 1.0.10 shell loads this hosted web release automatically; no native APK change is required.
