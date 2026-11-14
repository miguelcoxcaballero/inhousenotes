//! Editor: Pointer pipeline and tool dispatch (ARCHITECTURE §11.1).
//!
//! 5.18.0 binds `setupCanvasEvents` to every page canvas. v6 listens once on `#canvas-container` (the canvases'
//! common ancestor, below `#canvas-viewport`, so it runs before the viewport's pan listeners) and finds the canvas
//! in the event path. A contact is taken by the current tool's [`ToolHandler`]; while it lasts the pointer is
//! captured by its canvas, the geometry read at pointerdown is reused for every sample, and pointer events only
//! append samples (no allocation, no layout read, no out-buffer record except one frame request per frame).
//!
//! The tool state (`state.currentTool`, colour, width, the last non-eraser tool), the toolbar's tool buttons, the
//! pen's background double tap and "Draw with finger" (`touchDrawEnabled`) live here because the acceptance rule
//! needs them; `editor::toolbar` owns the toolbar's drag and the eraser menu (called where 5.18.0 opens/closes it).

use super::{eraser, pages, pen, toolbar, view, viewport};
use crate::abi::{ety, ev, life, ptr, ret, PointerBatch, Sample};
use crate::{tag, App, Event, Host, LsOp, Mod, Node, Prop, Rect, Ui};
use ihn_core::id::Id;
use ihn_core::jsmath;

// A listener id is attached to one event type: each type has its own.
const L_DOWN: u32 = tag(Mod::EditorInput, 1);
const L_MOVE: u32 = tag(Mod::EditorInput, 2);
const L_RAW: u32 = tag(Mod::EditorInput, 9);
const L_UP: u32 = tag(Mod::EditorInput, 3);
const L_CANCEL: u32 = tag(Mod::EditorInput, 10);
const L_LOST: u32 = tag(Mod::EditorInput, 11);
const L_DOC_UP: u32 = tag(Mod::EditorInput, 4);
const L_DOC_CANCEL: u32 = tag(Mod::EditorInput, 12);
const L_BACKGROUND: u32 = tag(Mod::EditorInput, 5);
/// One listener per toolbar button (`L_TOOL + i`; a listener id is attached once).
const L_TOOL: u32 = tag(Mod::EditorInput, 16);
const L_TOUCH_DRAW: u32 = tag(Mod::EditorInput, 7);
const T_PEN_IDLE: u32 = tag(Mod::EditorInput, 8);

/// v5: `DOCUMENT_SAVE_IDLE_MS` (the pen-writing idle tail) and the re-arm while a pointer is still down.
const PEN_IDLE_MS: f64 = 180.0;
const PEN_IDLE_RETRY_MS: f64 = 80.0;
/// v5: palm rejection window after the last pen event.
const PALM_MS: f64 = 2000.0;
/// v5: `PEN_DOUBLE_TAP_MS`, `PEN_DOUBLE_TAP_DIST`.
const DOUBLE_TAP_MS: f64 = 320.0;
const DOUBLE_TAP_DIST: f64 = 42.0;
const TOUCH_DRAW_KEY: &str = "touchDrawEnabled";

/// v5 `state.currentTool`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Pen,
    Highlighter,
    EraserStroke,
    EraserArea,
    Lasso,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Pen => "pen",
            Tool::Highlighter => "highlighter",
            Tool::EraserStroke => "eraser-stroke",
            Tool::EraserArea => "eraser-area",
            Tool::Lasso => "lasso",
        }
    }
    pub fn is_eraser(self) -> bool { matches!(self, Tool::EraserStroke | Tool::EraserArea) }
}

/// A tool that takes contacts on a page canvas. Implemented by `pen` (pen, highlighter) and `eraser` (both erasers);
/// the lasso (W4.2) adds its arm to [`handler`]. The pipeline calls, in order: `down` once (the pointer is already
/// captured and [`contact`] is set), `moved` for every pointermove / pointerrawupdate of the contact (and the
/// pointerup itself, for its release samples), then exactly one of `end` (pointerup, pointercancel, lost capture,
/// window blur, a new pen down: the gesture is committed) or `discard` (Read mode switched on mid-contact).
/// `moved` runs per pointer event: no allocation, no layout read; request a frame with `app.sched.frame`.
pub trait ToolHandler: Sync {
    fn down(&self, app: &mut App, host: &mut dyn Host, x: f64, y: f64, pressure: f64);
    fn moved(&self, app: &mut App, host: &mut dyn Host, p: &PointerBatch);
    fn end(&self, app: &mut App, host: &mut dyn Host);
    fn discard(&self, app: &mut App, host: &mut dyn Host);
}

/// The handler of `tool` (`None`: nothing is drawn; the contact is still taken so that the viewport does not pan).
pub fn handler(tool: Tool) -> Option<&'static dyn ToolHandler> {
    match tool {
        Tool::Pen | Tool::Highlighter => Some(&pen::Pen),
        Tool::EraserStroke | Tool::EraserArea => Some(&eraser::Eraser),
        Tool::Lasso => Some(&super::selection::Lasso),
    }
}

/// One pointer contact on a page canvas (v5 `activePointerId`, `drawGeometry`, `lastSampleTime`).
#[derive(Clone, Copy, Debug)]
pub struct Contact {
    pub pointer_id: i32,
    pub ptype: u8,
    pub tool: Tool,
    pub page: Id,
    pub canvas: Node,
    /// `getBoundingClientRect()` of the canvas at pointerdown.
    pub rect: Rect,
    /// Page px per client px.
    pub sx: f64,
    pub sy: f64,
    /// `lastSampleTime`: samples older than this are duplicates (pointermove + pointerrawupdate).
    pub last_t: f64,
}

impl Contact {
    /// v5 `getCanvasPointerPos`: client → page coordinates with the geometry read at pointerdown.
    pub fn page_xy(&self, s: &Sample) -> (f64, f64) { ((s.x - self.rect.x) * self.sx, (s.y - self.rect.y) * self.sy) }
}

/// A toolbar `.tool-btn[data-tool]` button.
struct ToolButton {
    node: Node,
    tool: &'static str,
    color: &'static str,
    width: f64,
}

/// v5 `index.html` toolbar presets (`.tool-btn[data-tool]`, in document order).
const TOOL_BUTTONS: [(&str, &str, &str, f64); 6] = [
    ("[title=\"Black Pen\"]", "pen", "#4D4D4D", 2.0),
    ("[title=\"Blue Pen\"]", "pen", "#002FD9", 2.0),
    ("[title=\"Red Pen\"]", "pen", "#E81010", 2.0),
    ("[data-tool=\"highlighter\"]", "highlighter", "#ffde00", 16.0),
    ("#btn-eraser", "eraser", "", 0.0),
    ("[data-tool=\"lasso\"]", "lasso", "", 0.0),
];

pub struct State {
    pub tool: Tool,
    /// v5 `state.currentColor` (as written in the toolbar's `data-color`).
    pub color: &'static str,
    pub width: f64,
    /// v5 `state.eraserMode === 'stroke'` and `state.eraserWidth` (set by the eraser menu, W4.4).
    pub eraser_stroke: bool,
    pub eraser_width: f64,
    /// v5 `state.lastNonEraserTool`.
    last_tool: (Tool, &'static str, f64),
    /// v5 `touchDrawEnabled`.
    pub touch_draw: bool,
    /// v5 `lastPenEventTime` (event clock).
    last_pen_t: f64,
    contact: Option<Contact>,
    last_tap: Option<(f64, f64, f64)>,
    buttons: Vec<ToolButton>,
    /// Pointer events handled (test counter).
    pub events: u32,
}

impl Default for State {
    fn default() -> Self {
        State {
            tool: Tool::Pen,
            color: "#002FD9",
            width: 2.0,
            eraser_stroke: true,
            eraser_width: 12.0,
            last_tool: (Tool::Pen, "#002FD9", 2.0),
            touch_draw: false,
            last_pen_t: f64::NEG_INFINITY,
            contact: None,
            last_tap: None,
            buttons: Vec::new(),
            events: 0,
        }
    }
}

// ---- public API -------------------------------------------------------------------------------------------------

/// The contact being drawn, if any.
pub fn contact(app: &App) -> Option<&Contact> { app.st.editor_input.contact.as_ref() }
pub fn contact_mut(app: &mut App) -> Option<&mut Contact> { app.st.editor_input.contact.as_mut() }

/// v5: `shouldAcceptDrawInput(e)` (records the pen's time).
fn accepts(st: &mut State, ptype: u8, t: f64) -> bool {
    match ptype {
        ptr::PEN => {
            st.last_pen_t = t;
            true
        }
        ptr::MOUSE => st.touch_draw,
        ptr::TOUCH => st.touch_draw && t - st.last_pen_t > PALM_MS,
        _ => false,
    }
}

/// v5: `setActiveToolButton(toolName, color, width)`.
fn set_active_button(app: &mut App, host: &mut dyn Host, tool: Tool, color: Option<&str>, width: Option<f64>) {
    let st = &app.st.editor_input;
    let name = if tool.is_eraser() { "eraser" } else { tool.name() };
    let inked = matches!(tool, Tool::Pen | Tool::Highlighter);
    let color = color.or(inked.then_some(st.color));
    let width = width.or(inked.then_some(st.width));
    let out = host.out();
    for b in &st.buttons {
        let mut on = b.tool == name;
        if on && inked {
            on = !(color.is_some_and(|c| !b.color.is_empty() && !b.color.eq_ignore_ascii_case(c)) || width.is_some_and(|w| b.width != 0.0 && b.width != w));
        }
        out.class(b.node, "active", on);
    }
}

/// v5: `rememberNonEraserTool(tool)`.
fn remember(st: &mut State) {
    if !st.tool.is_eraser() && st.tool != Tool::Lasso {
        st.last_tool = (st.tool, st.color, st.width);
    }
}

fn eraser_tool(st: &State) -> Tool { if st.eraser_stroke { Tool::EraserStroke } else { Tool::EraserArea } }

/// v5: `setActiveTool(name, {color, width})` (`None` tool = the eraser in its current mode).
pub fn set_tool(app: &mut App, host: &mut dyn Host, tool: Option<Tool>, color: Option<&'static str>, width: Option<f64>) {
    if view::read_only(app) {
        return;
    }
    let st = &mut app.st.editor_input;
    match tool {
        None => {
            st.tool = eraser_tool(st);
            let t = st.tool;
            set_active_button(app, host, t, None, None);
        }
        Some(t) => {
            if let Some(c) = color {
                st.color = c;
            }
            if let Some(w) = width {
                st.width = w;
            }
            st.tool = t;
            set_active_button(app, host, t, color, width);
            toolbar::close_menu(app, host);
        }
    }
    remember(&mut app.st.editor_input);
}

/// v5: `toggleEraserFromGesture()`.
pub fn toggle_eraser(app: &mut App, host: &mut dyn Host) {
    if view::read_only(app) {
        return;
    }
    let st = &mut app.st.editor_input;
    if st.tool.is_eraser() {
        // v5: restoreLastNonEraserTool
        let (t, c, w) = st.last_tool;
        (st.tool, st.color, st.width) = (t, c, w);
        set_active_button(app, host, t, Some(c), Some(w));
        toolbar::close_menu(app, host);
        return;
    }
    remember(st);
    set_tool(app, host, None, None, None);
}

/// The pen writes or just wrote (v5 `notePenActivity`): touches do not pan, repaints wait.
fn note_pen(app: &mut App) { app.st.editor_viewport.pen_writing = true; }

/// v5: `armPenWritingIdleTimer(DOCUMENT_SAVE_IDLE_MS)`.
fn arm_idle(app: &mut App, host: &mut dyn Host, ms: f64) {
    app.sched.timer(host.out(), T_PEN_IDLE, ms);
}

// ---- contacts ---------------------------------------------------------------------------------------------------

/// The page canvas in an event path (nearest first).
fn canvas_in(app: &App, p: &PointerBatch) -> Option<(Node, Id)> {
    p.path.iter().find_map(|n| pages::page_of_node(app, n).filter(|&id| pages::active(app, id).is_some_and(|a| a.canvas == n)).map(|id| (n, id)))
}

/// v5: the canvas' `pointerdown` (`setupCanvasEvents`).
fn down(app: &mut App, host: &mut dyn Host, p: &PointerBatch) -> u32 {
    let Some((canvas, page)) = canvas_in(app, p) else { return 0 };
    app.st.editor_input.events += 1;
    let read_only = view::read_only(app);
    if let Some(c) = app.st.editor_input.contact {
        if p.ptype != ptr::PEN {
            // v5: the canvas ignores it, and so does the viewport (a drawing-tool touch on a canvas).
            if !read_only && accepts(&mut app.st.editor_input, p.ptype, p.raw.t) {
                viewport::claim(app, p.pointer_id);
            }
            return 0;
        }
        // Some pen drivers lose the up: a new pen down is always a new stroke.
        finish(app, host, c.pointer_id);
    }
    if read_only {
        return 0;
    }
    if let Some(i) = pages::order(app).iter().position(|&id| id == page) {
        pages::set_active_page(app, i);
    }
    let st = &mut app.st.editor_input;
    if !accepts(st, p.ptype, p.raw.t) {
        return 0;
    }
    let tool = st.tool;
    // v5: a drawing pointerdown clears the selection (the lasso clears it in its own down).
    if tool != Tool::Lasso {
        super::selection::clear(app, host);
    }
    let Some(h) = handler(tool) else {
        viewport::claim(app, p.pointer_id);
        return 0;
    };
    let Some((w, ht)) = app.st.editor_pages.rows.iter().find(|r| r.id == page).map(|r| (r.w, r.h)) else { return 0 };
    let rect = host.rect(canvas);
    let c = Contact { pointer_id: p.pointer_id, ptype: p.ptype, tool, page, canvas, rect, sx: w / rect.w.max(1.0), sy: ht / rect.h.max(1.0), last_t: p.raw.t };
    host.out().pointer_capture(canvas, p.pointer_id, true);
    app.st.editor_input.contact = Some(c);
    app.sched.set_busy(Mod::EditorInput, true);
    note_pen(app);
    let (x, y) = c.page_xy(&p.raw);
    let pressure = if p.raw.pressure != 0.0 && !p.raw.pressure.is_nan() { p.raw.pressure as f64 } else { 0.5 };
    h.down(app, host, x, y, pressure);
    ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION
}

/// v5: `moveDrawing(e)` (pointermove and pointerrawupdate).
fn moved(app: &mut App, host: &mut dyn Host, p: &PointerBatch) -> u32 {
    let st = &mut app.st.editor_input;
    let Some(c) = st.contact.filter(|c| c.pointer_id == p.pointer_id) else { return 0 };
    st.events += 1;
    if view::read_only(app) {
        return 0;
    }
    // A hovering pen ended its contact without an up: end it here, without the hover position.
    if p.ptype == ptr::PEN && p.raw.buttons == 0 && p.raw.pressure == 0.0 {
        finish(app, host, c.pointer_id);
        return 0;
    }
    if p.kind == ptr::RAW_UPDATE && (p.ptype != ptr::PEN || !matches!(c.tool, Tool::Pen | Tool::Highlighter)) {
        return 0;
    }
    if !accepts(&mut app.st.editor_input, p.ptype, p.raw.t) {
        return 0;
    }
    note_pen(app);
    if let Some(h) = handler(c.tool) {
        h.moved(app, host, p);
    }
    ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION
}

/// v5: `endDrawing(e)` (pointerup, pointercancel, lostpointercapture; the document-level release recovery).
fn up(app: &mut App, host: &mut dyn Host, p: &PointerBatch) -> u32 {
    let Some(c) = app.st.editor_input.contact.filter(|c| c.pointer_id == p.pointer_id) else { return 0 };
    app.st.editor_input.events += 1;
    if p.kind == ptr::UP && !view::read_only(app) {
        if let Some(h) = handler(c.tool) {
            h.moved(app, host, p);
        }
    }
    finish(app, host, c.pointer_id);
    0
}

/// Ends the contact `pointer_id`: commits it, or discards it in Read mode.
fn finish(app: &mut App, host: &mut dyn Host, pointer_id: i32) {
    let st = &mut app.st.editor_input;
    let Some(c) = st.contact.filter(|c| c.pointer_id == pointer_id) else { return };
    st.contact = None;
    app.sched.set_busy(Mod::EditorInput, false);
    arm_idle(app, host, PEN_IDLE_MS);
    host.out().pointer_capture(c.canvas, pointer_id, false);
    // The handler still sees the contact through its own state; `contact()` is already `None`.
    if let Some(h) = handler(c.tool) {
        if view::read_only(app) {
            h.discard(app, host);
        } else {
            h.end(app, host);
        }
    }
}

/// The contact in progress is committed (window blur, v5: `pointercancel` sent to every drawing canvas).
pub fn cancel_contact(app: &mut App, host: &mut dyn Host) {
    if let Some(c) = app.st.editor_input.contact {
        finish(app, host, c.pointer_id);
    }
}

/// v5: `handlePenBackgroundDoubleTap(e)` (the viewport's pen pointerdown outside every page).
fn background_tap(app: &mut App, host: &mut dyn Host, p: &PointerBatch) -> u32 {
    if view::read_only(app) || p.ptype != ptr::PEN {
        return 0;
    }
    if p.path.iter().any(|n| pages::page_of_node(app, n).is_some()) {
        return 0;
    }
    let tap = (p.raw.t, p.raw.x, p.raw.y);
    if let Some((t, x, y)) = app.st.editor_input.last_tap {
        if tap.0 - t <= DOUBLE_TAP_MS && jsmath::hypot(tap.1 - x, tap.2 - y) <= DOUBLE_TAP_DIST {
            app.st.editor_input.last_tap = None;
            toggle_eraser(app, host);
            return ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION;
        }
    }
    app.st.editor_input.last_tap = Some(tap);
    0
}

/// v5: the toolbar's `.tool-btn[data-tool]` click handler.
fn tool_click(app: &mut App, host: &mut dyn Host, node: Node) {
    if view::read_only(app) {
        return;
    }
    let Some(i) = app.st.editor_input.buttons.iter().position(|b| b.node == node) else { return };
    let (name, color, width) = {
        let b = &app.st.editor_input.buttons[i];
        (b.tool, b.color, b.width)
    };
    let out = host.out();
    for b in &app.st.editor_input.buttons {
        out.class(b.node, "active", b.node == node);
    }
    let st = &mut app.st.editor_input;
    if name == "eraser" && st.tool.is_eraser() {
        toolbar::toggle_menu(app, host);
        return;
    }
    if name != "lasso" {
        super::selection::clear(app, host);
    }
    let st = &mut app.st.editor_input;
    st.tool = match name {
        "eraser" => eraser_tool(st),
        "highlighter" => Tool::Highlighter,
        "lasso" => Tool::Lasso,
        _ => Tool::Pen,
    };
    if name != "eraser" {
        if !color.is_empty() {
            st.color = color;
        }
        if width != 0.0 {
            st.width = width;
        }
    }
    remember(st);
    if name != "eraser" {
        toolbar::close_menu(app, host);
    }
}

// ---- module contract --------------------------------------------------------------------------------------------

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(app: &mut App, host: &mut dyn Host) {
    if app.cfg.has(crate::abi::mode::HEADLESS) {
        return;
    }
    let out = host.out();
    out.listen(Ui::CANVAS_CONTAINER, "pointerdown", L_DOWN, 0);
    for (e, id) in [("pointermove", L_MOVE), ("pointerrawupdate", L_RAW), ("pointerup", L_UP), ("pointercancel", L_CANCEL), ("lostpointercapture", L_LOST)] {
        out.listen(Ui::CANVAS_CONTAINER, e, id, 0);
    }
    // v5: app-v5.js:1943 — a release that did not pass through the canvas.
    out.listen(Node::DOCUMENT, "pointerup", L_DOC_UP, 0);
    out.listen(Node::DOCUMENT, "pointercancel", L_DOC_CANCEL, 0);
    out.listen(Ui::CANVAS_VIEWPORT, "pointerdown", L_BACKGROUND, 0);
    let mut buttons = Vec::with_capacity(TOOL_BUTTONS.len());
    for (i, (sel, tool, color, width)) in TOOL_BUTTONS.into_iter().enumerate() {
        let node = if sel.starts_with('#') { Ui::BTN_ERASER } else { out.query(Ui::TOOLBAR, &format!("button{sel}")) };
        out.listen(node, "click", L_TOOL + i as u32, 0);
        buttons.push(ToolButton { node, tool, color, width });
    }
    out.listen(Ui::TOUCH_DRAW_TOGGLE, "change", L_TOUCH_DRAW, 0);
    app.st.editor_input.buttons = buttons;
    // v5: touchDrawEnabled = localStorage.getItem('touchDrawEnabled') === 'true'; the checkbox follows.
    let on = host.ls(LsOp::Get, TOUCH_DRAW_KEY, None).as_deref() == Some("true");
    app.st.editor_input.touch_draw = on;
    host.out().prop(Ui::TOUCH_DRAW_TOGGLE, "checked", Prop::Bool(on));
}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Pointer(p) => match p.listener {
            L_DOWN => down(app, host, &p),
            L_MOVE | L_RAW => moved(app, host, &p),
            L_UP | L_CANCEL | L_LOST | L_DOC_UP | L_DOC_CANCEL => up(app, host, &p),
            L_BACKGROUND => background_tap(app, host, &p),
            _ => 0,
        },
        Event::Dom(e) if (L_TOOL..L_TOOL + TOOL_BUTTONS.len() as u32).contains(&e.listener) && e.ty == ety::CLICK => {
            tool_click(app, host, e.node);
            0
        }
        Event::Dom(e) if e.listener == L_TOUCH_DRAW => {
            let on = e.has(ev::CHECKED);
            app.st.editor_input.touch_draw = on;
            host.ls(LsOp::Set, TOUCH_DRAW_KEY, Some(if on { "true" } else { "false" }));
            0
        }
        Event::Timer(T_PEN_IDLE) => {
            // v5: finishPenWritingIdle — still down: wait 80 ms more.
            if app.st.editor_input.contact.is_some() {
                arm_idle(app, host, PEN_IDLE_RETRY_MS);
            } else {
                app.st.editor_viewport.pen_writing = false;
            }
            0
        }
        Event::Life(l) if l.code == life::BLUR => {
            cancel_contact(app, host);
            0
        }
        _ => 0,
    }
}

/// `__IHN_TEST_API__.cmd('editor.input.<name>')`: `state` → tool, colour, width, Draw with finger, contact,
/// pen writing, pointer events handled.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    let st = &app.st.editor_input;
    match name {
        "state" => Some(format!(
            "{{\"tool\":\"{}\",\"color\":\"{}\",\"width\":{},\"touchDraw\":{},\"contact\":{},\"penWriting\":{},\"events\":{}}}",
            st.tool.name(),
            st.color,
            ihn_core::jsfmt::num(st.width),
            st.touch_draw,
            st.contact.is_some(),
            app.st.editor_viewport.pen_writing,
            st.events
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{kind, mode, Config};
    use crate::doc::session;
    use crate::editor::history;
    use crate::host::FakeHost;
    use crate::shell;
    use ihn_core::codec::Writer;
    use ihn_core::op::{Background, Body, Template};
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    // Allocations made by this thread (the pointer-event budget test).
    thread_local! { static ALLOCS: Cell<usize> = const { Cell::new(0) }; }
    struct Counting;
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            ALLOCS.with(|c| c.set(c.get() + 1));
            unsafe { System.alloc(l) }
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) { unsafe { System.dealloc(p, l) } }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
            ALLOCS.with(|c| c.set(c.get() + 1));
            unsafe { System.realloc(p, l, n) }
        }
    }
    #[global_allocator]
    static COUNTING: Counting = Counting;

    const CANVAS_RECT: Rect = Rect { x: 100.0, y: 50.0, w: 794.0, h: 1123.0 };

    struct T {
        app: App,
        host: FakeHost,
        canvas: Node,
        page: Id,
    }

    fn boot() -> T {
        let cfg = Config { version: "6.0.0".into(), url: "http://127.0.0.1/?e2e=1".into(), user_agent: "test".into(), language: "en".into(), flags: mode::E2E | mode::ONLINE, viewport: (1280.0, 800.0), dpr: 1.0, max_touch_points: 0 };
        let (mut app, mut host) = (App::new(cfg), FakeHost::default());
        app.init(&mut host);
        host.rects.insert(Ui::CANVAS_VIEWPORT, Rect { x: 0.0, y: 69.0, w: 1280.0, h: 731.0 });
        session::load(&mut app, &mut host, "", "d", 0);
        let mut w = Writer::default();
        w.u32(tag(Mod::DocSession, 1));
        app.event(&mut host, kind::TIMER, &w.buf);
        session::edit(&mut app, &mut host, |tx| {
            let mut after = None;
            for _ in 0..3 {
                after = Some(tx.page_new(after, Template::Default, (794, 1123), Background::Template));
            }
        })
        .expect("a memory session is writable");
        shell::show_view(&mut app, &mut host, shell::View::Editor);
        shell::signal(&mut app, &mut host, session::sig::OPENED, "d");
        host.advance(500.0);
        let t = host.now;
        app.frame(&mut host, t);
        view::set_read_only(&mut app, &mut host, false, false);
        let page = session::get(&app).unwrap().doc.order()[0];
        let canvas = pages::active(&app, page).expect("page 1 is active").canvas;
        host.rects.insert(canvas, CANVAS_RECT);
        host.take();
        T { app, host, canvas, page }
    }

    /// A pointer event: `samples` are the coalesced list (client x, y, t); the raw event is the last one.
    fn encode(listener: u32, k: u8, ptype: u8, id: i32, samples: &[(f64, f64, f64)], buttons: u32, path: &[Node]) -> Vec<u8> {
        let mut w = Writer::default();
        let sample = |w: &mut Writer, s: &(f64, f64, f64)| {
            w.f64(s.0);
            w.f64(s.1);
            w.f64(s.2);
            w.f32(if buttons != 0 { 0.5 } else { 0.0 });
            w.f32(0.0);
            w.f32(0.0);
            w.u32(buttons);
        };
        w.u32(listener);
        w.u32(Ui::CANVAS_CONTAINER.0);
        w.u8(k);
        w.u8(ptype);
        w.i32(id);
        w.i32(0);
        w.u32(0);
        sample(&mut w, samples.last().unwrap());
        let coalesced = if k == ptr::MOVE { samples } else { &[] };
        w.u32(coalesced.len() as u32);
        for s in coalesced {
            sample(&mut w, s);
        }
        w.u32(path.len() as u32);
        for n in path {
            w.u32(n.0);
        }
        w.buf
    }

    impl T {
        fn send(&mut self, listener: u32, k: u8, ptype: u8, samples: &[(f64, f64, f64)], buttons: u32) -> u32 {
            let path = if k == ptr::DOWN { vec![self.canvas, Ui::CANVAS_CONTAINER] } else { vec![] };
            let bytes = encode(listener, k, ptype, 1, samples, buttons, &path);
            self.app.event(&mut self.host, kind::POINTER, &bytes)
        }
        fn frame(&mut self) {
            self.host.advance(16.0);
            let t = self.host.now;
            self.app.frame(&mut self.host, t);
        }
        /// A stroke through client points (offset by the canvas rect: page coordinates = client − (100, 50)).
        fn stroke(&mut self, ptype: u8, pts: &[(f64, f64)]) {
            let t0 = self.host.now;
            let s = |i: usize| (pts[i].0 + 100.0, pts[i].1 + 50.0, t0 + i as f64);
            self.send(L_DOWN, ptr::DOWN, ptype, &[s(0)], 1);
            for i in 1..pts.len() {
                self.send(L_MOVE, ptr::MOVE, ptype, &[s(i)], 1);
                self.frame();
            }
            self.send(L_UP, ptr::UP, ptype, &[s(pts.len() - 1)], 0);
        }
        fn live_strokes(&self) -> Vec<Vec<f64>> {
            let d = &session::get(&self.app).unwrap().doc;
            d.draw_list(self.page).strokes.iter().filter_map(|r| match &d.live(r.id)?.body.v {
                Body::Stroke { xy, .. } => Some(xy.clone()),
                _ => None,
            }).collect()
        }
    }

    #[test]
    fn pen_stroke_commits_one_batch_and_undo_redo_restore_it() {
        let mut t = boot();
        let r = t.send(L_DOWN, ptr::DOWN, ptr::PEN, &[(110.0, 60.0, 1.0)], 1);
        assert_eq!(r, ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION);
        assert!(t.app.st.editor_viewport.pen_writing);
        let out = t.host.take();
        assert!(out.contains("pointer_capture") || out.contains("23"), "{out}");
        // Coalesced samples of one event; a duplicate (older) rawupdate adds nothing.
        t.send(L_MOVE, ptr::MOVE, ptr::PEN, &[(120.0, 70.0, 2.0), (130.0, 75.0, 3.0)], 1);
        t.send(L_MOVE, ptr::RAW_UPDATE, ptr::PEN, &[(125.0, 72.0, 2.5)], 1);
        t.frame();
        t.send(L_UP, ptr::UP, ptr::PEN, &[(130.0, 75.0, 4.0)], 0);
        assert_eq!(t.live_strokes(), vec![vec![10.0, 10.0, 20.0, 20.0, 30.0, 25.0]]);
        assert_eq!(history::depth(&t.app), (1, 1));
        assert!(contact(&t.app).is_none());
        // The pen writes until 180 ms after the lift.
        assert!(t.app.st.editor_viewport.pen_writing);
        t.host.advance(200.0);
        let mut w = Writer::default();
        w.u32(T_PEN_IDLE);
        t.app.event(&mut t.host, kind::TIMER, &w.buf);
        assert!(!t.app.st.editor_viewport.pen_writing);

        history::undo(&mut t.app, &mut t.host);
        assert!(t.live_strokes().is_empty());
        assert_eq!(history::depth(&t.app), (1, 0));
        history::redo(&mut t.app, &mut t.host);
        assert_eq!(t.live_strokes().len(), 1);
        history::undo(&mut t.app, &mut t.host);
        history::undo(&mut t.app, &mut t.host); // nothing left: no-op
        assert!(t.live_strokes().is_empty());
        // A new action drops the redo branch.
        t.stroke(ptr::PEN, &[(1.0, 1.0), (5.0, 5.0)]);
        assert_eq!(history::depth(&t.app), (1, 1));
    }

    #[test]
    fn acceptance_follows_draw_with_finger_and_palm_rejection() {
        let mut t = boot();
        // Mouse and touch pan unless "Draw with finger" is on.
        assert_eq!(t.send(L_DOWN, ptr::DOWN, ptr::MOUSE, &[(150.0, 150.0, 1.0)], 1), 0);
        assert!(contact(&t.app).is_none());
        t.app.st.editor_input.touch_draw = true;
        t.stroke(ptr::MOUSE, &[(10.0, 10.0), (40.0, 40.0)]);
        assert_eq!(t.live_strokes().len(), 1);
        // A pen event, then a touch within 2 s: rejected (palm); after 2 s: draws.
        t.stroke(ptr::PEN, &[(10.0, 100.0), (40.0, 100.0)]);
        t.host.advance(1000.0);
        assert_eq!(t.send(L_DOWN, ptr::DOWN, ptr::TOUCH, &[(150.0, 300.0, t.host.now)], 1), 0);
        t.host.advance(1500.0);
        t.stroke(ptr::TOUCH, &[(10.0, 200.0), (40.0, 200.0)]);
        assert_eq!(t.live_strokes().len(), 3);
        // Read mode: nothing is taken.
        view::set_read_only(&mut t.app, &mut t.host, true, false);
        assert_eq!(t.send(L_DOWN, ptr::DOWN, ptr::PEN, &[(150.0, 150.0, t.host.now)], 1), 0);
        assert_eq!(history::depth(&t.app), (3, 3));
    }

    #[test]
    fn read_mode_mid_contact_discards_and_a_new_pen_down_commits_the_old_stroke() {
        let mut t = boot();
        t.send(L_DOWN, ptr::DOWN, ptr::PEN, &[(110.0, 60.0, 1.0)], 1);
        t.send(L_MOVE, ptr::MOVE, ptr::PEN, &[(150.0, 90.0, 2.0)], 1);
        // A second pen down without an up: the first stroke is committed, a new one starts.
        t.send(L_DOWN, ptr::DOWN, ptr::PEN, &[(300.0, 300.0, 3.0)], 1);
        assert_eq!(t.live_strokes().len(), 1);
        t.send(L_MOVE, ptr::MOVE, ptr::PEN, &[(320.0, 320.0, 4.0)], 1);
        view::set_read_only(&mut t.app, &mut t.host, true, false);
        t.send(L_UP, ptr::UP, ptr::PEN, &[(320.0, 320.0, 5.0)], 0);
        assert_eq!(t.live_strokes().len(), 1);
        // A hovering pen (no buttons, no pressure) ends a contact whose up was lost, without the hover point.
        view::set_read_only(&mut t.app, &mut t.host, false, false);
        t.send(L_DOWN, ptr::DOWN, ptr::PEN, &[(110.0, 400.0, 6.0)], 1);
        t.send(L_MOVE, ptr::MOVE, ptr::PEN, &[(130.0, 400.0, 7.0)], 1);
        t.send(L_MOVE, ptr::MOVE, ptr::PEN, &[(500.0, 700.0, 8.0)], 0);
        assert!(contact(&t.app).is_none());
        assert_eq!(t.live_strokes()[1], vec![10.0, 350.0, 30.0, 350.0]);
    }

    #[test]
    fn highlighter_is_simplified_after_its_commit() {
        let mut t = boot();
        let button = t.app.st.editor_input.buttons[3].node;
        tool_click(&mut t.app, &mut t.host, button);
        assert_eq!((t.app.st.editor_input.tool, t.app.st.editor_input.color, t.app.st.editor_input.width), (Tool::Highlighter, "#ffde00", 16.0));
        // 41 collinear samples 5 px apart: Douglas–Peucker keeps 2 < max(3, ceil(41/2)), so the filtered list stays.
        let pts: Vec<(f64, f64)> = (0..41).map(|i| (10.0 + 5.0 * i as f64, 300.0)).collect();
        t.stroke(ptr::PEN, &pts);
        assert_eq!(t.live_strokes()[0].len(), 82);
        // Samples closer than max(0.6, 0.12·16) = 1.92 px are not taken.
        t.stroke(ptr::PEN, &[(10.0, 500.0), (11.0, 500.0), (12.0, 500.0), (30.0, 500.0)]);
        assert_eq!(t.live_strokes()[1], vec![10.0, 500.0, 12.0, 500.0, 30.0, 500.0]);
        // A wavy line: Douglas–Peucker keeps only its corners, fewer than half of the samples, so all stay.
        let wave: Vec<(f64, f64)> = (0..60).map(|i| (10.0 + i as f64 * 3.0, 600.0 + if (i / 15) % 2 == 0 { (i % 15) as f64 } else { (15 - i % 15) as f64 } * 2.0)).collect();
        let xy: Vec<f64> = wave.iter().flat_map(|p| [p.0, p.1]).collect();
        let p = vec![0.5f32; wave.len()];
        let (kept, kp) = crate::editor::pen::simplify(&xy, &p).unwrap();
        assert_eq!(kept, xy);
        assert_eq!(kept.len() / 2, kp.len());
        assert_eq!(crate::editor::pen::simplify(&xy[..10], &p[..5]), None);
    }

    #[test]
    fn pen_double_tap_on_the_background_toggles_the_eraser() {
        let mut t = boot();
        let tap = |t: &mut T, x: f64| {
            let now = t.host.now;
            let bytes = encode(L_BACKGROUND, ptr::DOWN, ptr::PEN, 1, &[(x, 20.0, now)], 1, &[Ui::CANVAS_VIEWPORT]);
            t.app.event(&mut t.host, kind::POINTER, &bytes)
        };
        assert_eq!(tap(&mut t, 20.0), 0);
        t.host.advance(200.0);
        assert_eq!(tap(&mut t, 30.0), ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION);
        assert_eq!(t.app.st.editor_input.tool, Tool::EraserStroke);
        t.host.advance(1000.0);
        tap(&mut t, 20.0);
        t.host.advance(400.0); // too slow
        assert_eq!(tap(&mut t, 20.0), 0);
        t.host.advance(100.0);
        tap(&mut t, 20.0);
        assert_eq!((t.app.st.editor_input.tool, t.app.st.editor_input.color), (Tool::Pen, "#002FD9"));
    }

    #[test]
    fn history_keeps_ten_steps_and_a_remote_structure_change_clears_it() {
        let mut t = boot();
        for i in 0..12 {
            t.stroke(ptr::PEN, &[(10.0, 10.0 * i as f64), (50.0, 10.0 * i as f64)]);
        }
        assert_eq!(history::depth(&t.app), (10, 10));
        // A page added by this tab (pull-to-add) keeps the stack.
        crate::editor::pages::add_page(&mut t.app, &mut t.host, false);
        shell::signal(&mut t.app, &mut t.host, session::sig::CHANGED, "");
        assert_eq!(history::depth(&t.app), (10, 10));
        // Another device deletes a page.
        let other = {
            let s = session::get(&t.app).unwrap();
            let mut ids = ihn_core::id::IdGen::new(ihn_core::id::ActorId(7 << 8), 1);
            let mut author = ihn_core::doc::Author::new(ids.clone(), 0);
            let _ = &mut ids;
            let mut tx = ihn_core::doc::Tx::new(&s.doc, &mut author, 1);
            tx.page_delete(s.doc.order()[2]);
            tx.commit()
        };
        session::get_mut(&mut t.app).unwrap().doc.apply(&other);
        shell::signal(&mut t.app, &mut t.host, session::sig::CHANGED, "");
        assert_eq!(history::depth(&t.app), (0, 0));
    }

    #[test]
    fn live_packets_describe_the_contact_when_a_sink_is_attached() {
        let mut t = boot();
        pen::set_live_sink(&mut t.app, true);
        t.stroke(ptr::PEN, &[(10.0, 10.0), (20.0, 10.0), (30.0, 10.0)]);
        let kinds: Vec<&str> = pen::drain_live(&mut t.app).iter().map(|p| match p {
            ihn_core::doc::LivePreview::Begin { .. } => "begin",
            ihn_core::doc::LivePreview::Points { .. } => "points",
            ihn_core::doc::LivePreview::End { .. } => "end",
            _ => "other",
        }).collect();
        assert_eq!(kinds, ["begin", "points", "points", "points", "end"]);
    }

    /// ARCHITECTURE §11.1: in steady state a pointer event allocates nothing and writes nothing but the frame
    /// request of its frame.
    #[test]
    fn pointer_events_do_not_allocate() {
        let mut t = boot();
        t.send(L_DOWN, ptr::DOWN, ptr::PEN, &[(110.0, 60.0, 1.0)], 1);
        let events: Vec<Vec<u8>> = (0..200)
            .map(|i| {
                let x = 110.0 + i as f64;
                let k = if i % 2 == 0 { ptr::MOVE } else { ptr::RAW_UPDATE };
                encode(L_MOVE, k, ptr::PEN, 1, &[(x, 60.0 + (i % 7) as f64, 2.0 + i as f64), (x + 0.5, 61.0, 2.5 + i as f64)], 1, &[])
            })
            .collect();
        // Warm up the out-buffer and the scheduler with one frame.
        t.app.event(&mut t.host, kind::POINTER, &events[0]);
        t.frame();
        t.host.take();
        let mut written = 0;
        let before = ALLOCS.with(|c| c.get());
        for (i, e) in events[1..].iter().enumerate() {
            t.app.event(&mut t.host, kind::POINTER, e);
            if i % 4 == 3 {
                written += t.host.out.w.buf.len();
                t.host.out.w.buf.clear();
                let after = ALLOCS.with(|c| c.get());
                assert_eq!(after, before, "allocation in pointer event {i}");
                t.frame();
                t.host.out.w.buf.clear();
                ALLOCS.with(|c| c.set(before));
            }
        }
        assert!(written > 0 && written <= 50 * 16, "{written}");
        t.send(L_UP, ptr::UP, ptr::PEN, &[(400.0, 61.0, 500.0)], 0);
        assert_eq!(t.live_strokes().len(), 1);
    }
}
