# v5.15.17

Preserve handwriting; distinguish black ink from coloured reflections.

Web only; Android loads this update without a new APK.

- Require pigment contrast relative to the locally measured paper-to-ink
  contrast before colour calibration. A small red/blue reflection on dark
  black ink previously had high RGB saturation, which the calibration then
  amplified into a coloured pen. Black stays neutral while genuine navy and
  dark red retain their palette colours. Rust/Wasm and JavaScript agree.
- Refine measured scan axes with ordered, topology-preserving thinning,
  guided by the original ink coverage and distance from the ink boundary.
  Preserve the topology guide's endpoints so weak hooks and branches cannot
  retreat to a darker crossing. Counters, normal widths and native editable
  pens remain intact. Generic input without measured coverage keeps the
  original geometry path.
- Add regressions for local additive coloured flare, thin JPEG-compressed
  black/navy/red pens, and the complete silhouette of photographed cursive
  letters, rather than only testing their centre lines.

Validation: the new standard-width cursive fixture improves silhouette F1
from 0.861 to 0.892 and reduces ink missed beyond a half-editor-pixel tolerance
from 3.93% to 2.57%. All 54 counters in the existing photographed-letter fixture
remain open. The oblique faint-ink fixture retains 99.4% of sampled positions.
Rust and JavaScript produce identical native strokes in these regressions.

Private attachment checks isolate ink extraction from the already warped
photograph; they do not certify that compressed attachment's camera geometry
or replace validation on the user's phone. No user photographs are committed.
Previously converted pages retain their existing strokes. Rescan the original
page to recover details discarded by an earlier conversion.
