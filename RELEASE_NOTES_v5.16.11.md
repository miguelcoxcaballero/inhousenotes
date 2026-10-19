# v5.16.11

Folder motion responds sooner and stays visible.

- Show the lightweight folder lid before preparing its interior, taking catalogue and card work out of the opening input task.
- Keep offscreen cards lightweight until scrolling or keyboard focus needs them, avoiding background card and thumbnail work between folder taps.
- Give the opening 720 ms and the closing 560 ms of visible motion, with a distinct hinge phase before the main zoom.
- Synchronize the lid and zoom to a shared frame clock. Limit time advances after a blocked frame so initial rendering or a mid-animation stall cannot consume the unseen animation.
- Read the outgoing viewport before adding the closing scene and avoid redundant scroll restoration to prevent synchronous layout on the closing input task.
- Finish pending navigation exactly once and cancel frame callbacks on interruption, resize or backgrounding. Preserve scroll, focus, reduced motion and live contents.
- Add regression checks for initial and mid-animation stalls, input latency and cancellation before preparation.
- Applies to web and Android web assets; no new APK or updater catalog is required.
