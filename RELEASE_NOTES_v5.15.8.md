# v5.15.8

Consistent scan colours and smoother handwriting.

Web only; Android loads this update without a new APK.

- Resolve pen colours along coherent paths and opposite branches at crossings,
  preventing shaded sections of blue writing from alternating with black.
  Genuine crossing black pens and sustained red/blue changes stay separate.
- Rust and the JavaScript fallback retain saturated dark navy ink without
  accepting faint colour fringes as reliable pigment.
- Apply slight resolution-aware smoothing before simplifying editable paths,
  preserving endpoints, sharp turns, loop seams and standard pen widths.

Validation: eleven scanner browser checks, including colour/shadow crossings,
1x/2x smoothing, Rust/fallback parity, photographed import, reload/PDF persistence,
lasso, recolouring, partial erasing and undo. Eight Rust adapter checks pass.
