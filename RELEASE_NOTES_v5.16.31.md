# v5.16.31

Folders open and close the moment you tap them, on Android and the web.

- The zoom starts on the first frame after the tap. Before, it waited until
  the next screen had been built and painted, plus two more frames. The next
  screen is now built right after that first frame, underneath the turning
  lid, and its crossfade joins the running movement (still fully in by 70%).
- The motion gets its start time from the compositor when it actually
  presents the first frame, so a slow first frame can no longer skip part of
  the animation.
- Document labels move without their 1.5px backdrop blur, which made every
  frame of the zoom far more expensive; it eases back in after the folder
  lands instead of switching on in one frame.
- The opened folder is laid out once inside the zoom instead of twice.
- The paper inside folders is plain white instead of ruled.

The Android app loads this web release; no new APK is needed.
