# v5.16.10

Smoother folder opening and closing.

- Build visible cards first, then prepare offscreen cards in short batches after the zoom. Preserve every item, its scroll position, keyboard access and tap target.
- Clone only visible card artwork for the outgoing scene; defer Drive reconciliation and focus until the transition finishes.
- Keep the animated cover in 3D while flattening stationary icons inside it. Replace procedural paper noise with the same baked texture and avoid moving backdrop filters.
- Build action menus when opened, removing 20 hidden elements per resting card.
- A 140-item nested folder in Chromium with CPU throttled 6x reached its first frame in 216 ms instead of 981 ms; total opening fell from 3.07 s to 0.93 s. These are controlled browser measurements, not Android device FPS claims.
- Regression coverage checks real animation frames under CPU throttling, nested/back navigation, scroll, focus, reduced motion and resize.
- Applies to web and Android web assets; no new APK or updater catalog is required.
