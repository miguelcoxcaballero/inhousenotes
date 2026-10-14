# v5.15.2

More reliable scan lighting and reference colours.

Web only; Android loads this update without a new APK.

- The camera uses the ordered red, black, blue and green reference circles and
  nearby paper to distinguish the sheet's illumination from a coloured desk.
- Still-image analysis recovers dim but measurable ink using verified colour
  references. A two-sided line contrast check keeps flat, separately lit desks
  out of the yellow-frame mask.
- Colour calibration samples each circle separately, relative to local paper,
  rather than mixing neighbouring circles and their yellow outlines. Usable
  colour references no longer depend on a clearly coloured yellow frame.
- Scan ink remains editable in the editor's native pen palette. The existing
  Rust pixel processing path, sharpness checks and physical dot validation are
  retained.

Validation covers dim, warm, cool, green, magenta and mixed illumination;
missing/corrupt reference pigments; JPEG halos; faint pencil; Rust/JavaScript
pixel parity; physical surface geometry; and the capture-to-editable-page flow.
Original photographs and their focus measurements remain unchanged. Clipped or
unreadable geometry still requires another capture.
