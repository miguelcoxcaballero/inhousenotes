# v5.14.3

- Run the developable 3D numerical solver in Rust, retaining independent photographed-dot, perimeter and orientation checks.
- Avoid a second full border search when the first analysis already verifies the full-source 3D sheet; retry finer analysis if final raster alignment disagrees.
- Re-measure an independently recovered faint frame on the same live contour instead of rejecting it solely on the stricter live-seed colour threshold.
- Reject camera grain before indexing faint dots and prefer a one-pitch grid neighbour over a shorter spurious link.
- Disable rejected backup thumbnails and return to a corrected photograph if a selected backup fails.
- Keep JPEG colour bleed from receiving full ink contrast/chroma enhancement; remove colour-cast remnants of source-verified printed dots without cutting continuous writing.

This is a web only update for the existing Android APK.
