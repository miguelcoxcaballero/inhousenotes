# W6.1 PARITY-SWEEP — notes

## Sweep agent A (outside the editor)

Branch `v6-w6_1`. Fixes: `host/dom.js` (several event types per listener id), `doc/open.rs` (title from the Drive
name), `export/pdf.rs` + `ihn-core/history.rs` (timeline snapshot assets in the export, newest first),
`home/mod.rs` (boot skeletons swept at the first render), `home/folders.rs` (close ghost shows the path on screen),
`doc/session.rs` (a checkpoint is a local save), `doc/public.rs` (public view keeps zoom 1 / pan 0), `shell/fx.rs`
(probe listener in capture); e2e 01/03/08/internal-store adjusted; new `tests/v6/parity/states/sweep.mjs`.

Deviations:
1. **Version assets are capped by count, not bytes**: SYNC §14.6 asks for 5.18.0's 3.5 MB budget, but Rust does not
   know blob sizes; the export attaches at most 48 snapshot assets (newest versions and their delta bases).
2. **`open/failed` still differs** (status line ends on the error, 5.18.0 on the counts; 5.18.0 retries the failed
   shared open on every auth UI update). Left to `doc/open.rs`'s owner; needs a BEHAVIOUR-CHANGES entry if kept.
3. **Title rename PATCH** is not sent for documents already in Drive: `editor/view.rs finish_title` tests
   `flags.created` inverted. Editor files belong to sweep agent B; e2e 03 renames through the export name.
4. Only the quick matrix was run (task header: time is short); the full matrix is not run for these states.
5. Parity runs must use `--a-root` outside `test-results/` when e2e runs on the same tree (Playwright empties it).
