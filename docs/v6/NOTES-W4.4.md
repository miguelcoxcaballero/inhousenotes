# W4.4 TOOLBAR — notes for the other units

Branch `v6-w4_4` (from `v6-int` at `58039fc`). Owns `rust/crates/ihn-app/src/editor/toolbar.rs`,
`tests/v6/parity/states/toolbar.mjs`.

## What exists now

- `editor::toolbar` — 5.18.0 `setupToolbarDrag` (drag by `#toolbar-handle`, limits under the header / read-only
  banner and above the phone's floating mode toggle, `--toolbar-scale`, snap left / right / bottom-horizontal / free
  with `snapping` 200 ms and the 360 ms tilt classes, window resize clamp, first-frame normalisation),
  `clampToolbarPosition` on `shell::sig::VIEW = editor` (next frame), and the eraser menu: `open_menu`,
  `close_menu`, `toggle_menu`, `menu_open`, `positionEraserMenu` (next frame, beside or above the button),
  `syncEraserMenuState` at boot and on open, size buttons → `input.eraser_width`, `#eraser-stroke-toggle` →
  `input.eraser_stroke` (and the active eraser's mode), a pointerdown outside the menu and the button closes it,
  Read mode (`view::sig::MODE = read`) closes it.
- Tool state, tool buttons and Draw with finger stay in `editor::input` (W3.5); 5.18.0 has fixed pen/highlighter
  presets, no colour picker. `input` now calls `toolbar::toggle_menu` (click on the active eraser) and
  `toolbar::close_menu` (other tool clicks, `set_tool`, the double-tap restore), as 5.18.0 does.
- Undo/redo button states are `editor::history::update_buttons` (W3.5); the parity state checks them.
- Test API: `editor.toolbar.state` → `{menuOpen, horizontal, dragging, eraserWidth, eraserStroke}`.
- Performance: a drag reads layout once at pointerdown and listens to document `pointermove`/`pointerup` only while
  it lasts (5.18.0 listens for the whole session); the outside-click listener exists only while the menu is open.

## Verified

- `cargo test -p ihn-app` green (123; 5 new toolbar tests: menu toggle/close paths and placement, outside click and
  Read mode, sizes and stroke toggle, drag clamp + bottom/right snaps + timers, editor-view and resize clamps).
- `npm run build` (report-only): 977,093 bytes raw / 393,821 gzip.
- Parity `--self --quick --states toolbar` (A on both sides): all states run in 5.18.0.

## Deviations

1. **Parity B blocked on W3.4** (as W3.2/W3.5): every `toolbar` state opens the 5.x fixture `r0`; v6 ends on Home,
   so B fails at the mode toggle ("Element is not visible"). Rerun `--quick --states toolbar` after W3.4 merges.
2. `toolbar/eraser-menu` runs on desktop/tablet only: in 5.18.0 the eraser button of the toolbar's initial position
   is below a 360×800 window; `toolbar/eraser-menu-horizontal` covers phones.
3. `toolbar/undo-redo-states` on phone-360 differs between two runs of 5.18.0 itself (`--self`): A's stroke pixels
   are not deterministic there; not a v6 issue.
4. Edited `editor/input.rs` (W3.5's) at the four `closeEraserMenu`/`toggleEraserMenu` call sites, as its notes asked.
