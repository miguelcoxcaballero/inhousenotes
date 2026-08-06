# Inhouse Notes v5.11.7

## Lower square box as a geometric reference

- The scanner now detects the lower square box independently from the outer
  yellow frame instead of allowing both lines to compete as the page edge.
- The box top and outer baseline are paired as two observations of the same
  paper bend, improving lower-edge correction without following square teeth.
- The four colour markers continue to establish the box orientation and scale.

## Stronger frame isolation

- Yellow samples are constrained to a narrow, smoothed corridor around each
  expected side and isolated hits fall back to calibrated geometry.
- Border candidates that lean away from the calibrated box are rejected, which
  prevents nearby sheets, yellow handwriting and surrounding objects from
  becoming false page edges.
- Missing sides inherit the perspective of a verified opposite side rather
  than expanding into clutter outside the photographed sheet.

## Cleaner curved-page output

- Curved mesh rendering now maps the printed frame area to its exact canonical
  A4 position and leaves the unseen outer margin white.
- Frame endpoints stay anchored to the marker-guided quadrilateral, avoiding
  pinched corners and dark folder artefacts.
- The processing animation shows the separately detected lower box alongside
  the real frame and uses the same bounded mesh as the final scan.

## Visual and automated validation

- Four real photographs supplied during development were imported through the
  actual scanner UI, rendered and visually checked, including a cluttered
  folder photo, two portrait pages and one rotated page.
- A browser regression now combines a curved frame, a dense lower square box
  and strong nearby yellow distractors, and verifies that the final frame is
  still detected and straightened.
