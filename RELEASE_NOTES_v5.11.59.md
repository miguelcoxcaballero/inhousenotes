# Inhouse Notes v5.11.59

Stable outlines and consistent curved-page scanning.

The camera draws its observed outline continuously between analysis frames. A common motion filter keeps the frame and calibration boxes together, while spatial regularization removes small false bends without suppressing the paper's broad curvature. Capture decisions use the original measurements.

Still-image processing revalidates the page observed in the camera instead of silently choosing an overlapping book or other background. Border fitting reuses numeric buffers to reduce processing time while retaining the same hypotheses and scoring. Regression scenes cover pale books, coloured logos, yellow distractors, faint printing, camera JPEG capture and the contents of the final scan.

The processor now fits a physical 3D paper surface using the observed borders and printed dot grid, including the camera perspective, bend direction and depth profile. Independent grid observations validate the correction before it is used. If the grid cannot support that fit, the processor retains the boundary-based correction; unverified page geometry leaves the original photo clearly identified for review. Shadow and template cleanup run after geometric correction.

Android 1.0.16 (versionCode 17) loads these scanner changes from the website and already includes the camera permission. There is no APK reinstall or catalog change. The new screen recording was used to reproduce the incorrect crop. This release has not been tested on a physical Android device.
