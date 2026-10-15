# v5.15.9

Visible folder opening and lighter thumbnail labels.

Web only; Android loads this update without a new APK.

- Open the folder lid, zoom its interior to cover the content area, then reveal
  the contents. Reverse the movement when closing. Keep navigation responsive
  and preserve reduced-motion and keyboard behaviour.
- Reveal contents from the actual zoom completion, preventing a timer from
  exposing the next view before the folder reaches full size.
- Let thumbnails show through their lightly blurred footer, with a matching
  tint and contrasting text.

Validation: five folder navigation browser checks, including frame-by-frame
coverage, cached/offline navigation, stale responses and reduced motion. Visual
checks of light/dark thumbnails and menus at desktop and mobile sizes.
