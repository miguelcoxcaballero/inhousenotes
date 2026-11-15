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

`share/people` and `share/copy` on phone-360 (the globe icon, 15–37 px, max Δ 5) have two renderings in both
apps: with `--runs 3` the stability check fails for 5.18.0 against itself and for v6 against itself, and a
parity comparison passes whenever both apps land on the same rendering. `home/folder-colours` on phone-412 dark
(a header icon, 26 px) failed once in the full run and passed in two reruns. No tolerance was added, so these
cells can report a failure that is not an app difference; `--runs 3` on the state shows which it is.

## Not done

- 5.18.0 waits for the structure mutation to settle before `addPage`, `movePage`, page delete and restore
  repaint. v6 does not port that wait; the states pass without it.
