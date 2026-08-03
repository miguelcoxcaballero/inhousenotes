# Inhouse Notes v5.9.5

## Live erasing

- Streams whole-stroke eraser removals to connected devices during the gesture instead of waiting for pointer-up.
- Streams exact area eraser replacements as removed IDs plus newly split stroke segments, so partial erasing is also visible live.
- Coalesces erase operations into responsive 18 ms frames and forces the last pending frame when the gesture ends.
- Deduplicates and relays live erase packets across the peer graph without echoing them back to their source.
- Keeps live erase state as a transient visual preview; the normal tombstones and durable snapshot remain the automatic recovery path after packet loss or reconnection.
- Clears the preview only after the matching durable snapshot arrives, avoiding a flash of the old strokes between live erasing and synchronization.

## Android

- The existing Android `1.0.10` shell loads this web release automatically; no native binary change is required.
