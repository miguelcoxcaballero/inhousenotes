# v5.17.4

Sharper photos; faster image import.

- Photo pages now render at up to 3× page resolution (3840-pixel long-side cap) with JPEG quality 0.94, instead of the old 2200-pixel cap. Small sources are not enlarged unnecessarily.
- Bounded PNG photo pages without padding retain the original bytes. This preserves fine text/line detail without an extra decode/encode generation.
- Inserted/pasted images use one high-quality encode, capped at 3200 pixels / 8 megapixels, replacing up to 15 lossy attempts targeting 130 KB. Bounded PNGs retain their bytes and transparency.
- Existing documents and their source images are unchanged. Photos already reduced by an earlier version cannot regain missing detail; the improvement applies when importing the original again.
- Browser checks cover image bounds, lossless PNG reuse, one-pass encoding, transparency, local reload, PDF reopen and the raster fallback.

Web and Android web assets are updated; the APK and frozen installer are unchanged.
