# v5.16.5

Quicker, calmer folder navigation.

- Use a short local lid turn and gentle content fade instead of enlarging the entire cover across the screen.
- Open in 260 ms and close in 210 ms, using compositor animations without per-frame JavaScript layout work.
- Preserve nested folder artwork and geometry before replacing the parent view so Back and breadcrumbs can animate closing too.
- Restore the parent scroll position and retain a short fade when resized geometry is no longer valid.
- Keep cancellation safe during rapid Back/reopen/tab changes and respect reduced motion.
- Avoid a home-screen flash when the departing folder reveals the home underneath it.

Validated with seven browser regressions: three nesting levels in mobile and
desktop sizes, visible lid motion in both directions, bounded geometry, scroll
restoration, resize fallback, reduced motion, interrupted navigation and delayed
Drive responses. Release smoke and native resource manifest checks also pass.

Web only: existing Android APKs load the new hashed app resource. No APK,
package identity, shared updater or A/B catalog change is required.
