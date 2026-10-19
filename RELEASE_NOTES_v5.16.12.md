# v5.16.12

Smoother folders and high-refresh display support.

- Run the folder lid and zoom on the browser compositor, with synchronized native animation timelines instead of a JavaScript update on every frame.
- Rasterize the interior at its final size before zooming to avoid a second large raster job during the enlargement. Keep the 3D lid, visible contents, exact handoff and nested-folder navigation.
- Start the lightweight lid immediately and keep preparation outside the main zoom duration. Cancel pending work cleanly on interrupted navigation, resize and backgrounding.
- Android 1.0.22 requests the highest supported refresh rate up to 120 Hz at the current display resolution, with explicit hardware acceleration. Respect power saving and Android display limits; release preferences and listeners in the background.
- Verify compositor interpolation with 8.33 ms synthetic frames, navigation regressions and the generated Android display policy. Physical device frame rates depend on hardware and system settings.
- Prepare the signed Android 1.0.22 build. The A catalog remains on the previous verified APK until the new binary has been published and verified.
