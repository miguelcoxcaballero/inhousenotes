# W1.2 HARNESS — notes for the lead and for units that use the harness

## Commands

| What | Command |
|---|---|
| Parity, 5.18.0 against itself (the harness' own proof) | `node tests/v6/parity/run.mjs --self --quick --runs 2` |
| Parity, 5.18.0 vs v6, a unit's states | `node tests/v6/parity/run.mjs --quick --states welcome,home-shell` |
| Regenerate the 5.x fixtures / check they regenerate byte for byte | `node tests/v6/fixtures/make-v5-fixtures.mjs` / `… --check` |
| Record the 5.18.0 canvas traces / check | `node tests/v6/fixtures/record-traces.mjs` / `… --check` |
| Performance profile | `node tests/v6/perf/profile.mjs --app v5\|v6 [--short] [--throttle 4]` |

Servers listen on a free port unless `--port-a/--port-b` are given. The 5.18.0 tree is extracted once from commit
`1d6e8a0` into `build/v5-ref`.

## Results on this branch

- Parity `--self --quick --runs 2`: 96 comparisons, 0 failures, 0 warnings, in two consecutive invocations (about
  2 min 15 s each).
- Traces: `--check` identical in every run after the last fix (dense page included).
- Fixtures: all Drive PDFs, their import records and the storage dumps regenerate byte for byte — also with three
  busy-loop processes competing for the four cores — with one exception listed under deviations
  (`recovery-conflict`).
- Profile: `--app v5 --short` completes all four phases on the `dense-20` document.

## How determinism is obtained (read before writing states)

1. **Time moves only between timer instants.** `Driver.advance` lets the page run its Playwright clock from one due
   timer to the next and stops as soon as a callback started work the clock does not own (request, IndexedDB, worker
   message, lock, `scheduler.yield/postTask`, decode, DOM change). The harness then waits until that work is finished
   before time moves again. Playwright's own `runFor(20)` yields to real tasks between two timers, which let a worker
   reply decide which `Date.now()` a later callback saw (ids and timeline stamps one millisecond or one frame apart).
2. **The clock's frame phase is fixed.** `clock.install` and `clock.pauseAt` are logged with the wall time of the call
   and a new document replays the gap between them into its tick counter; both calls now see one wall time.
3. **Idle means nothing but gated requests is in flight** (`busy === net`): a request waiting at the mock's gate no
   longer hides an open IndexedDB transaction.
4. **Workers report idleness on their own port**, after the replies they sent, so a report cannot overtake a reply;
   inside a worker (and the page) stream reads (`reader.read()`), `scheduler.yield/postTask`, Web Lock requests and
   MessageChannel hops count as work in flight. Before that, 5.18.0's inflating worker looked idle between two chunks
   and the result depended on the machine's load.
5. **Captures are rastered from scratch** (`#freshRaster`): images decoded, Chromium's image cache emptied, one
   paint with `image-rendering: pixelated`, then the normal paint; and the screenshot is repeated until two in a row
   are equal. Without it a Home card preview differed by one level in one capture of eight.
6. `IHN_DET_TRAIL=1` makes the page keep a log `"<fake ms> <what>"` of everything counted; comparing two runs' logs
   per instant shows where a difference starts. `make-v5-fixtures.mjs` writes it next to each storage dump.

Rules that follow for state scripts: never wait in real time for the app; use `t.advance`, `t.until`, the action
helpers; a second page of the same context is advanced with `t.advance(ms, { page })`.

## Deviations from the plan

- **`recovery-conflict` storage dump is a one-run sample, not reproducible byte for byte.** The row 5.18.0's outbox
  runner writes when Drive changed underneath an offline draft (`localBlob` + `remoteBlob`) only appears after the app
  is started again, and then the library refresh and the outbox runner — each a chain of IndexedDB, Web Lock and
  worker round trips inside one instant of fake time — race in real time (lease owner ids, and in one run of five
  the runner does not get the lease at all). It is generated only with `--only recovery-conflict` and skipped by
  `--check`. The reproducible `recoveries` dump contains the other kind of row (`last-upload-draft:<id>`, the editable
  draft kept after an upload) next to a dirty document. Removing the race would need a scheduler that orders every
  asynchronous completion; not worth it for one fixture.
- **Tracked files are larger than usual**: `tests/v6/fixtures/traces` 21 MB (the two dense-page traces are 9 MB each,
  about 1.8 MB each compressed in the pack) and `tests/v6/fixtures/v5` 14 MB.
- **The Playwright clock is driven through its page-side controller** (`globalThis.__pwClock.controller`: `_timers`,
  `_now`, `runFor`, `log`), which is not public API. It is pinned by `@playwright/test` 1.62.1; when the page has no
  such controller the harness falls back to `page.clock.runFor`.
- **Scenes opened through `/?drive=<id>` start in Read mode** (5.18.0 behaviour): scripts that need tools click
  `#mode-toggle-btn` first, as the fixture generator, the trace recorder and the profile do.
- `tests/v6/fixtures/ink/*.json` (TESTING §3.4) is not in W1.2's file list: each ink trace carries the pen samples it
  was recorded with in its `"input"` field (CSS px relative to the page's top-left corner, ms, pressure).
- The parity matrix beyond `--quick` (five viewports, dark, app mode) is implemented in `matrix.mjs` but was only
  spot-checked here; the acceptance runs are the quick matrix.

## For other units

- `expect.json` of a Drive fixture is what 5.18.0 holds after importing the bytes in a fresh session (pages, stroke
  rows `[id, tool, color, width, points]`, a SHA-256 of all points, images, timeline entries). Background images are
  hashed by content, not by their `blob:` URL.
- `unsyncedStrokeIds` of a storage dump: strokes the device holds that the Drive fixture it opened does not contain.
- Trace surfaces carry the size they were last set to, also when the canvas was garbage-collected before the trace
  was taken (it used to be `null`).
- Blob images passed to `drawImage` are recorded as `{"image":"blob"}` with their natural size: their content is not
  part of the trace.
- The shared `.git/info/exclude` of this clone (written during wave 1, not by W1.2) ignores `tests/v6/mock/`,
  `tests/v6/parity/*.{mjs,js}`, `tests/v6/fixtures/`, `tests/v6/perf/` and `tests/v6/server.mjs` in every worktree:
  new files under those paths are only committed with `git add -f`. The fixtures and traces were added that way.
