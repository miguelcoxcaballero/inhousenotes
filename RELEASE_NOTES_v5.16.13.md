# v5.16.13

Smoother folder opening and closing at high refresh rates.

- Freeze thumbnail and skeleton shimmer placeholders inside the moving folder scene. Their main-thread background animation repainted the whole composited interior on every tick, which is what kept the zoom from holding a steady 120 Hz.
- Hold thumbnail swaps until the zoom has landed, so decoded previews no longer repaint the interior mid-motion.
- Softer easing: the zoom now settles gently onto the final layout instead of stopping dead, while the lid still turns before the large enlargement begins.
