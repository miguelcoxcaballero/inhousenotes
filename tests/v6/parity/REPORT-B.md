# W6.1 parity sweep — agent B (editor)

Branch `v6-w6_1b`. Scope: parity states `editor-view`, `canvas`, `editor-tools`, `toolbar`, `erasers`, `selection`,
`images`, `pages`, `pdf-backgrounds`; e2e 04, 04b, 04c, 05, 06, 11, 12, internal-canvas/doc/editor-view/peer.
Code and harness changes: `docs/v6/NOTES-W6.1b.md`.

## Quick matrix (desktop-1280 + phone-360, light, web)

| Run | Comparisons | Fail | Expected |
|---|---|---|---|
| start (`v6-int` 776700d) | 141 | 35 | 0 |
| after harness/rule fixes (all 9 files) | 141 | 17 | 19 |
| `erasers,selection` desktop after the repaint fix | 13 | 0 | 4 |
| `selection,pages,images` after all fixes | 40 | 3 | 13 |

Fixed (each a real difference):

| Difference | Cause | Fix |
|---|---|---|
| `toolbar/snap-*` whole toolbar Δ4 | harness gave an animation started by a timer the whole preceding clock slice | `determinism-init.js` |
| `erasers/stroke`, `erasers/area` @lift, ink Δ1–2 | 5.18.0 repaints the whole page after damage repaints (`needsRedraw`); v6 kept the clipped result | `editor::eraser` full repaint |
| `selection/move`/`scale`/`rotate`/`cross-page` ink Δ1–5 | same, for the selection transform | `editor::selection` full repaint |
| `selection/move` undo button | the state pressed twice; 5.18.0 then drops the transform's history entry | `Driver.pen({press:false})` |
| `toolbar/draw-with-finger*` version label | intended (`v5.18.0` → `v6.0.0`) | `VERSION` rule |
| save dot (desktop), 19 states | N-1: 5.18.0 uploads within 0.4 s, v6 seals the relay after 5 s idle | `SAVE_DOT` rule (tag N-1, dot only) |
| photo page edge rows (part) | v6 cleared the wrapper preview of active pages; 5.18.0 re-applies it in every visible pass | `editor::pages` |

Also fixed (not visible in parity): own batches were served to peers/tabs before their IndexedDB commit
(`Replica::batches_since`); remote live erase previews were never drawn (now drawn; e2e 11 (a) checks it).

## Full matrix (5 viewports × light/dark, web)

Run per state file. Completed: `editor-tools`, `toolbar` (others did not finish in the time available: ≈35 s per
cell, ≈470 cells ≈ 4.5 h with one worker). Several cells ended with `Target crashed` on either side (A and B) while the
machine was loaded; environment errors, not differences.

Remaining differences (all phone), unexplained → open:

1. **phone-412 (DPR 2.625): one device row at the top edge of page 1, Δ1, full width** — `editor-tools/*`,
   `toolbar/pen-presets`, `toolbar/undo-redo-states`, `toolbar/draw-with-finger*`. The canvas edge falls inside a
   device pixel so the wrapper background shows through: 5.18.0 shows the page's scroll preview (`cover`), v6 the
   template URL. 5.18.0 re-applies previews in an `updateVisiblePages` pass triggered by the click on the floating
   toggle area; v6 runs no visible pass for that click (its preview exists: `editor.pages.state` `preview: true`).
   Owner: `editor::pages`/`editor::viewport` (which interactions call `schedule_visible`).
2. **phone-360 share button edge, 3 px Δ ≤ 7 at CSS (222, 25–27)** — intermittent, more frequent under load. A 3-run
   `--runs 3` stability check did not reproduce it on either side; no animation runs there; hiding SVGs during the
   raster refresh did not help (reverted).
3. **`pages/photo-modal@added`** — the photo page's top/bottom antialiased rows: after the insert 5.18.0's wrapper
   shows the template (its scroll preview of the new page was invalidated by the insert), v6 a preview. Same mechanism
   as 1.
4. **`toolbar/eraser-menu-horizontal` phone-360 dark** — once: different scroll position on a run where the Edit
   toggle click did not take in either app; not reproduced elsewhere.
5. **`canvas/dense-page@repaint` desktop, 3 px Δ2** (known since W3.4) — not investigated further.

Expected (tagged) differences seen: N-1 (save dot), B-E3, B-E4, B-E6, VERSION-LABEL.

## E2E

`04-ink`, `04b-erase`, `04c-select`, `05-pages`, `06-images`, `11-two-devices` (a–e), `12-open-v5`,
`internal-canvas`, `internal-editor-view`, `internal-peer`: pass, except `06-images` (asset count after the drop;
fails identically on `v6-int` 776700d, timing-dependent) and `internal-doc` "a new document takes a reserved Drive
id…" (fails identically on `v6-int`). `cargo test -p ihn-app`: 165 passed.
