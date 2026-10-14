# v5.14.2

- Check focus on the frozen photograph before automatic capture, independently of yellow-frame confidence.
- Show the captured photo immediately and make the first verified corrected scan available while backup photos finish.
- Failed page correction never enables adding the original photo; automatic scans return to the camera for another attempt.
- Rust accelerates full-resolution colour correction and transparent ink extraction, with a cancellable JavaScript fallback.
- Calibrate from red, black, blue, green, yellow and measured paper using the exact printed/editor colours. Restore thin pen chroma lost by camera JPEG processing.
- Recover faint photographed dots while retaining independent surface, perimeter and raster alignment checks.

This is a web only update for the existing Android APK.
