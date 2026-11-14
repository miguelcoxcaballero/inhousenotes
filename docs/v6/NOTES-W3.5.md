# W3.5 INK — notes for the other units

Branch `v6-w3_5` (from `v6-w3_2` at `d02c188`). Owns `rust/crates/ihn-app/src/editor/{input,pen,history}.rs`,
`tests/v6/e2e/04-ink.spec.mjs`, `tests/v6/parity/states/editor-tools.mjs`.

## What exists now

- `editor::input` — the pointer pipeline (ARCHITECTURE §11.1, 5.18.0 `setupCanvasEvents`). One set of listeners on
  `#canvas-container` (one listener id per event type: `dom.js` attaches an id once) finds the page canvas in the
  event path; the document's `pointerup`/`pointercancel` recover releases outside the canvas; window blur commits
  the contact. Acceptance is 5.18.0's `shouldAcceptDrawInput` (pen always; mouse and touch only with Draw with
  finger; touch 2 s after the last pen event). A taken contact: one `getBoundingClientRect` (cached in
  `input::Contact`), pointer capture, `preventDefault + stopPropagation` (so `editor::viewport` never sees it),
  `pen_writing` and `sched.set_busy(EditorInput)` until the lift; `pen_writing` clears 180 ms after the lift
  (80 ms re-arm while a pointer is down). A second non-pen pointer on a canvas during a contact is ignored and
  `viewport::claim`ed (5.18.0: neither the canvas nor the viewport takes it).
  - **Tool dispatch for W4.1/W4.2/W4.3:** implement `input::ToolHandler` (`down`, `moved`, `end`, `discard`) and add
    your arm to `input::handler(tool)`. `input::contact(app)` gives the geometry (`Contact::page_xy(sample)`) and
    `last_t` (duplicate-sample filter, update it through `contact_mut`). Tools without a handler (eraser, lasso
    today) still take the contact so that the viewport does not pan.
  - **Tool state lives here:** `app.st.editor_input.{tool, color, width, eraser_stroke, eraser_width,
    touch_draw}`, `input::set_tool` (v5 `setActiveTool`), `input::toggle_eraser` (v5 `toggleEraserFromGesture`),
    the toolbar's `.tool-btn[data-tool]` click handler and the pen's background double tap (320 ms / 42 px).
    `#touch-draw-toggle` and `localStorage.touchDrawEnabled` are read and written here as well (five lines of
    5.18.0 that the acceptance rule needs). **W4.4** owns the eraser menu (`toggleEraserMenu`, `closeEraserMenu`,
    eraser size buttons → set `eraser_width` / `eraser_stroke`), the toolbar drag and the undo/redo button styling;
    the click on an already active eraser button returns without opening the menu until W4.4 adds that call.
  - **W4.2:** 5.18.0 calls `clearSelection()` at every drawing pointerdown and on tool clicks other than the lasso;
    add those calls to `input::down` / `input::tool_click` when `selection::clear` exists.
- `editor::pen` — pen and highlighter (`pen::Pen`). Samples go to buffers reserved at boot (1024 samples; longer
  strokes grow once); 5.18.0's `appendPenSamples` filters (older timestamps, non-finite, min distance 0.001 px /
  `max(0.6, 0.12·width)`), one frame per animation frame (`render::live::update`), commit at lift
  (`render::live::commit` with the unsimplified points, then `simplifyStrokePoints` for the highlighter with
  `jsmath::hypot/pow`), one `session::edit` batch, one history step; the bitmap already shows the stroke, so the page
  is `touch`ed without a repaint. Pressure is stored (`Body::Stroke.p`), never rendered.
  - **W4.5 (collab):** `pen::set_live_sink(app, true)` then `pen::drain_live(app)` → `LivePreview::{Begin, Points,
    End{item}, Cancel}` for `SyncEngine::live` (one `Points` per frame; rate limiting is SYNC's). No packets are
    built without a sink.
- `editor::history` — undo/redo by inverse ops (`Doc::inverse`): a step holds the ops of its next move; undo and
  redo apply them as a new batch and keep that batch's inverse. 10 steps, 25,000 points (12,500 when hidden), the
  newest step kept; steps on a deleted page are skipped as 5.18.0 does. Buttons (`updateHistoryButtons`), Ctrl+z /
  Ctrl+y (lowercase, Ctrl only), cleared on `OPENED`/`CLOSED` and when a remote change alters the page order
  (5.18.0's structural-merge rule; local page edits refresh the reference on `session::sig::STATE`).
  - **Other units:** `history::record(app, host, page, edit.inverse, points_added)` after your `session::edit`;
    `history::clear(app, host)` for 5.18.0's other clearing rules (import, restore, photo page, page delete).

## Verified

- `cargo test --workspace` green (ihn-app 114): pen stroke → one batch, undo/redo, redo branch dropped; acceptance
  and palm rejection; Read mode mid-contact discards; a new pen down commits the old stroke; hover ends a lost
  contact without the hover point; highlighter min distance and simplification; background double tap; 10-step
  limit and remote structural clear; live packets; **0 allocations per pointer event** (counting global allocator,
  200 coalesced move/rawupdate events) and only the frame request written.
- e2e `04-ink.spec.mjs`: CDP stylus stroke (Read mode does not draw; Edit draws on the bitmap), highlighter with the
  mouse (Draw with finger), undo/redo by buttons and Ctrl+Z/Ctrl+Y with the pixels following, reopen through the link
  gives the same snapshot hash and paints both strokes; `desynchronized === false`. `00-boot`, `internal-doc`,
  `internal-canvas`, `internal-editor-view` still pass.
- `npm run build` (report-only): 883,120 bytes raw / 354,024 gzip (+32 KB raw over W3.2).

## Deviations

1. **Parity not met (blocked on W3.4), as W3.2.** `editor-tools` opens the 5.x fixture `r0`; on v6 the open ends on
   Home, so all 14 quick cells fail on B at the mode toggle ("Element is not visible"); A (5.18.0) runs every
   script. Rerun `--quick --states editor-tools` after W3.4 merges. The single-tap dot state is tagged `B-E3`.
2. **Files outside the owned list:** none edited. Tool state, tool buttons, Draw with finger and the background double
   tap are in `input.rs` (see above) instead of `toolbar.rs`, because the input pipeline needs them before W4.4.
3. **Found in W3.2's `viewport.rs` (not fixed, not mine):** `init` attaches `pointerup`, `pointercancel` and
   `pointerleave` under the same listener id `L_UP`; `host/dom.js` keeps one listener per id, so only
   `pointerleave` is attached (a mouse pan ends when the pointer leaves the viewport, not on release). Give each
   type its own id.
4. Pen-writing idle: the 180 ms timer is armed at the lift only (not re-armed per move as `notePenActivity` does), so
   pointer events write nothing; the observable `isPenWriting` interval is the same.
