# v5.14.1

- Scan the first clear frame immediately instead of requiring an 800 ms hold and five matching detections.
- Freeze up to three fresh photos without waiting for JPEG encoding between them; later movement or focus changes do not discard the first good photo.
- Keep the existing focus, exposure, yellow-marker and page-boundary checks. A stalled or blurry feed never triggers automatic capture.

This is a web only update compatible with the current Android APK. The home changes from v5.14.0 remain included.
