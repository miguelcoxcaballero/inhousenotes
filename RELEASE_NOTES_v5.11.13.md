# Inhouse Notes v5.11.13

## Reliable yellow-border scanning

- Adapts the yellow threshold to the paper and lighting in each individual photo.
- Recovers faded borders under low saturation, blur, shadows and warm camera processing.
- Requires local colour contrast so warm unprinted paper and wooden backgrounds are not mistaken for a frame.
- Uses the four printed colour circles and the lower square strip as independent scale and orientation references.
- Tries narrow and broad strip fits so compression or curvature can damage one reference without breaking the scan.
- Rejects marker combinations that do not share the same physical row.
- Keeps the correct sheet selected when several yellow-bordered pages overlap.

## Verification

- Correctly detected the yellow frame in all six supplied photos.
- Correctly detected 16 of 16 faded, dark, warm-shadow and blurred variants derived from those photos.
- Added regression coverage for faded yellow frames and warm pages without a printed frame.
- Passed the complete unit, smoke and browser test suites.
