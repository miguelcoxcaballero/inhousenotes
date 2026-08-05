# Inhouse Notes v5.10.1

This patch makes concurrent P2P drawing converge without losing or temporarily hiding valid strokes.

## Lossless live strokes

- Final strokes are now transferred as a recoverable batch with a causal stamp and exact point count.
- Receivers rebuild chunks by offset, so relayed packets can arrive out of order without dropping part of a stroke.
- Duplicate delivery through multiple peer or same-device paths is harmless and cannot create duplicate strokes.

## Durable convergence

- A completed remote stroke is merged into the local page immediately instead of remaining only as a visual preview until the next document snapshot.
- Local and remote strokes are combined by stable identity and causal metadata, preserving simultaneous edits from different devices.
- Remote final strokes are checkpointed in IndexedDB without creating a Drive echo loop.

## Snapshot race protection

- Snapshot creation waits for active drawing and completed remote-stroke commits before serializing the document.
- A final stroke arriving while pages are being hydrated causes the stale build to retry instead of publishing an incomplete state.
- A snapshot clears a live preview only when it actually contains the stroke or a causally valid deletion for it.

The Android native wrapper remains version 1.0.10 because it loads this web release directly and no native code or signed APK content changed.
