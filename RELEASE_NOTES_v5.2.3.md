# Inhouse Notes v5.2.3

This patch simplifies the anonymous public viewer.

- The centered “Public view · updates automatically” status banner has been removed.
- The unused legacy external-open action has been removed from the public-view code.
- The embedded Drive toolbar is cropped out of the fallback viewport, removing its pop-out/open control while preserving document scrolling.
- Anonymous viewers now get a passive Inhouse Notes pill labelled “Read only” at the bottom of the screen.
- The normal yellow permission banner stays hidden for anonymous viewer links, leaving the document unobstructed.
- Editor links still retain their sign-in-to-edit banner and are unaffected by this visual change.
