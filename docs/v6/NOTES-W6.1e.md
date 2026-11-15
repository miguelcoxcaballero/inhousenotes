# W6.1e — full-matrix parity runner and fixes

## Runner

- `tests/v6/parity/run.mjs` no longer keeps the screenshots, DOM dumps and canvas traces of passing captures in
  memory until the report is written; only failing, warned and expected captures are retained. A full state file
  now runs inside a 6 GB Node heap. Run one state file per invocation
  (`NODE_OPTIONS=--max-old-space-size=6000`) and merge the runs with
  `node tests/v6/parity/report.mjs --summary <run dir>…`.
- `determinism-init.js` counts a same-origin script load as work in flight, so a capture never lands between the
  request for a lazily loaded script (pdf.js, pdf-lib) and its execution.

## App fixes

- `ihn-core` `v5/import.rs`: `Import::open_yields` reports how many times 5.18.0 yields to the event loop while it
  opens a document of that shape. `doc::open::finish_foreign` waits the same number of frames (plus one or two,
  as 5.18.0 does for the first paint), so the editor appears on the same frame as in 5.18.0.
- `editor/pages.rs`: `arm_clean_swap` swaps a page canvas only once its replacement is fully painted, as 5.18.0
  does; the photo-page states of `canvas` pass.

## Full matrix (22 state files, 2387 comparisons)

See `tests/v6/parity/REPORT.md`. No capture differs outside the cases listed there.

## Known harness instability

`share/people` and `share/copy` on phone-360 (the globe icon, 15–131 px, max Δ up to 58) had two renderings in
both apps: with `--runs 3` the stability check failed for 5.18.0 against itself and for v6 against itself, and a
parity comparison passed whenever both apps landed on the same rendering.

Cause: the screenshots themselves. `page.screenshot({caret: 'hide'})` makes Playwright insert a
`caret-color: transparent` style just before each screenshot and remove it right after, so the frame that
produces the screenshot is also the one that recalculates style, repaints and rasterises. Pixels rasterised in
that frame are not reproducible: the globe (an SVG placed at a half device pixel) came out in one of two
renderings, its upper third differing most. Measured on the settled 5.18.0 dialog (dark, phone-360, three
browsers, ten screenshots per method each, hash of a crop around the icon): `caret: 'initial'` gave one rendering
30 of 30 times; `caret: 'hide'` gave the other one in 18 of 30, flipping between consecutive screenshots of the
same still page; the same caret style inserted by the harness with two real frames before the screenshot gave the
one rendering 30 of 30 times, inserted without those frames both renderings again (14 of 30 the other one). A full
repaint before the screenshot (every element invalidated) also gave the one rendering 30 of 30 times. Layer
tree, element rects, animations, hover and focus were identical in both renderings.

Fix (`run.mjs`, `Driver#caret`, `capture`): the harness hides the caret with its own style in every frame before
`#freshRaster` paints the page, the screenshots use `caret: 'initial'` and change nothing, and the style is removed
after the screenshots (before the DOM dump). No tolerance, mask or skip. After the fix the three repro commands
(`share/people`, `share/copy` on phone-360, `home/folder-colours` on phone-412, `--runs 3`) each passed three
times in a row with no stability or parity failure.

`home/folder-colours` on phone-412 dark (a header icon, 26 px) failed once in the full run before the fix and
was not reproduced since (three `--runs 3` passes after the fix; 36 screenshots with either caret mode of that
header, per app and theme, gave one rendering each). Whether it had the same cause is not established; if it
recurs, `--runs 3` on the state shows whether it is the app or the harness.

## Not done

- 5.18.0 waits for the structure mutation to settle before `addPage`, `movePage`, page delete and restore
  repaint. v6 does not port that wait; the states pass without it.
