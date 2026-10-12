# Inhouse Notes v5.11.61

Parallel scan processing and automatic frame selection.

All three captured photos are corrected concurrently in separate workers. Detection, grid-guided 3D reconstruction, colour cleanup and JPEG encoding stay off the camera interface. The review selects the best verified correction after comparing the results. A failed frame does not discard successful alternatives. Selecting a thumbnail reuses its completed scan, and an explicit selection remains selected as other frames finish.

Closing or retaking a scan cancels the entire batch and releases its workers and image resources. The original photo remains available if none of the corrections can be verified. Older browsers retain a compatible processing fallback.

Border analysis avoids repeated point searches and unnecessary continuity calculations without changing its geometric acceptance criteria. Workers avoid UI scheduling delays, and the 3D fitting budget accounts for three captures sharing a CPU core so concurrent work does not prematurely discard a valid surface.

A JPEG regression also revealed that an interpolated calibration rail could exclude genuine dots from the last grid row. Exclusion now requires visible yellow ink at that particular segment, preserving the grid observations needed for a verified 3D correction.

Android 1.0.16 (versionCode 17) loads these scanner changes from the website. No APK reinstall or update catalog change is required. This release has not been tested on a physical Android device.
