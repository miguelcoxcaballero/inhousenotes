# v5.15.7

Faster folders with smooth content fades.

Web only; Android loads this update without a new APK.

- Folder names sit at the bottom of their covers, above a larger menu target.
- Opening and closing now use short compositor animations (320/240 ms),
  without enlarging textured layers to full screen or repainting colours per frame.
- Cached folder contents appear immediately while Drive refreshes. Nested
  navigation and late arrivals fade in; closing keeps the contents visible
  through their fade out. Unchanged cards keep their loaded thumbnails.
- Rapid navigation cancels old effects, restores keyboard focus, and rejects
  stale responses even when returning to the same folder. Reduced motion is respected.

Validation: ten navigation/offline browser checks pass, including delayed Drive
responses and repeated navigation. Visual checks cover mobile/desktop, both themes,
long names and 30 cached files. Release smoke checks pass.
