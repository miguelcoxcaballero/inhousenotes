//! Editor: Toolbar drag/snap and the eraser menu (5.18.0 `setupToolbarDrag`, `clampToolbarPosition`,
//! `{open,close,toggle,position}EraserMenu`, `syncEraserMenuState`, the eraser size buttons and stroke toggle).
//!
//! The tool state, the tool buttons (5.18.0 has fixed presets, no colour picker) and Draw with finger live in
//! `editor::input`; it calls [`toggle_menu`] / [`close_menu`] where 5.18.0 calls `toggleEraserMenu` /
//! `closeEraserMenu`. Layout is read only where 5.18.0 reads it, and a drag reads it once at pointerdown: each
//! pointermove writes `left`/`top` only.

use super::input::Tool;
use super::view;
use crate::abi::{ety, ev, life, ptr, PointerBatch};
use crate::host::Metric;
use crate::shell;
use crate::{tag, App, Event, Host, Mod, Node, Prop, Ui};
use ihn_core::jsfmt;

const L_DOWN: u32 = tag(Mod::EditorToolbar, 1);
const L_MOVE: u32 = tag(Mod::EditorToolbar, 2);
const L_UP: u32 = tag(Mod::EditorToolbar, 3);
const L_OUTSIDE: u32 = tag(Mod::EditorToolbar, 4);
const L_STROKE: u32 = tag(Mod::EditorToolbar, 5);
/// One listener per eraser size button (`L_SIZE + i`).
const L_SIZE: u32 = tag(Mod::EditorToolbar, 8);
const T_SNAP: u32 = tag(Mod::EditorToolbar, 1);
const T_TILT: u32 = tag(Mod::EditorToolbar, 2);

/// `#eraser-menu .size-btn[data-eraser-size]`, in document order.
const SIZES: [f64; 4] = [4.0, 12.0, 22.0, 32.0];
const SNAP_THRESHOLD: f64 = 80.0;

/// Work waiting for the next animation frame (5.18.0's `requestAnimationFrame` callbacks).
const F_NORMALIZE: u8 = 1;
const F_CLAMP: u8 = 2;
const F_MENU: u8 = 4;

#[derive(Clone, Copy, Default)]
struct Limits {
    min_top: f64,
    max_top: f64,
    min_left: f64,
    max_left: f64,
}

impl Limits {
    fn left(&self, x: f64) -> f64 { self.min_left.max(x.min(self.max_left)) }
    fn top(&self, y: f64) -> f64 { self.min_top.max(y.min(self.max_top)) }
}

#[derive(Default)]
pub struct State {
    header: Node,
    mode_group: Node,
    sizes: Vec<Node>,
    /// The drag in progress: offsets from the toolbar's corner and the limits read at pointerdown.
    drag: Option<(f64, f64, Limits)>,
    /// `toolbar.classList.contains('horizontal')`.
    horizontal: bool,
    tilt: Option<&'static str>,
    menu_open: bool,
    frame: u8,
}

// ---- geometry ---------------------------------------------------------------------------------------------------

fn px(x: f64) -> String {
    let mut s = jsfmt::num(x);
    s.push_str("px");
    s
}

/// v5: `getEditorTopOffset()`.
fn top_offset(app: &App, host: &mut dyn Host) -> f64 {
    let header = app.st.editor_toolbar.header;
    let h = if header == Node::NONE { 60.0 } else { host.rect(header).h };
    let banner = host.style(Ui::READONLY_BANNER, "display");
    if banner.is_empty() || banner == "none" || host.style(Ui::READONLY_BANNER, "visibility") == "hidden" {
        return h;
    }
    h + host.rect(Ui::READONLY_BANNER).h
}

/// v5: `getMobileFloatingControlsInset()`.
fn bottom_inset(app: &App, host: &mut dyn Host) -> f64 {
    let group = app.st.editor_toolbar.mode_group;
    if group == Node::NONE || !host.media("(max-width: 720px)") || host.style(Ui::MODE_TOGGLE_BTN, "display") == "none" {
        return 0.0;
    }
    if host.style(group, "display") == "none" || host.style(group, "visibility") == "hidden" || host.style(group, "position") != "fixed" {
        return 0.0;
    }
    let inset = (host.metric(Node::WINDOW, Metric::InnerHeight) - host.rect(group).y).max(0.0);
    if !inset.is_finite() || inset <= 0.0 { 0.0 } else { inset + 8.0 }
}

fn window(host: &mut dyn Host) -> (f64, f64) { (host.metric(Node::WINDOW, Metric::InnerWidth), host.metric(Node::WINDOW, Metric::InnerHeight)) }

/// v5: `getToolbarLimits(getToolbarSize(), margin)`.
fn limits(app: &App, host: &mut dyn Host, margin: f64) -> Limits {
    let r = host.rect(Ui::TOOLBAR);
    let min_top = top_offset(app, host) + margin;
    let inset = bottom_inset(app, host);
    let (vw, vh) = window(host);
    Limits { min_top, max_top: min_top.max(vh - r.h - margin - inset), min_left: margin, max_left: margin.max(vw - r.w - margin) }
}

/// v5: `updateToolbarScale(margin)`.
fn update_scale(app: &App, host: &mut dyn Host, margin: f64) {
    let available_h = host.metric(Node::WINDOW, Metric::InnerHeight) - top_offset(app, host) - margin - bottom_inset(app, host);
    let available_w = host.metric(Node::WINDOW, Metric::InnerWidth) - margin * 2.0;
    let (bw, bh) = (host.metric(Ui::TOOLBAR, Metric::OffsetWidth), host.metric(Ui::TOOLBAR, Metric::OffsetHeight));
    if bw == 0.0 || bh == 0.0 {
        return;
    }
    let scale = 1f64.min(available_h / bh).min(available_w / bw);
    let safe = if scale.is_finite() { scale.max(0.1) } else { 1.0 };
    host.out().style(Ui::TOOLBAR, "--toolbar-scale", Some(&jsfmt::num(safe)));
}

/// Writes `left`/`top` (or `right` when `right` is set) with the other two sides `auto`.
fn place(host: &mut dyn Host, left: Option<f64>, top: f64, right: Option<f64>) {
    let out = host.out();
    out.style(Ui::TOOLBAR, "left", Some(&left.map_or("auto".into(), px)));
    out.style(Ui::TOOLBAR, "top", Some(&px(top)));
    out.style(Ui::TOOLBAR, "right", Some(&right.map_or("auto".into(), px)));
    out.style(Ui::TOOLBAR, "bottom", Some("auto"));
}

/// v5: `clampToolbarPosition()` (margin 12) and the drag's window `resize` handler (margin 10, rescaled first).
fn clamp(app: &mut App, host: &mut dyn Host, margin: f64, rescale: bool) {
    if rescale {
        update_scale(app, host, margin);
    }
    let r = host.rect(Ui::TOOLBAR);
    let l = limits(app, host, margin);
    place(host, Some(l.left(r.x)), l.top(r.y), None);
    host.out().style(Ui::TOOLBAR, "--toolbar-translate", Some("0px"));
}

/// v5: `snapToolbar()` at the end of a drag.
fn snap(app: &mut App, host: &mut dyn Host) {
    const MARGIN: f64 = 15.0;
    let (vw, vh) = window(host);
    let r = host.rect(Ui::TOOLBAR);
    let was = app.st.editor_toolbar.horizontal;
    let (dl, dr, db) = (r.x, vw - r.right(), vh - r.bottom());
    let min = dl.min(dr).min(db);
    let out = host.out();
    out.class(Ui::TOOLBAR, "snapping", true);
    out.style(Ui::TOOLBAR, "--toolbar-translate", Some("0px"));
    let horizontal = min == db && db < SNAP_THRESHOLD;
    app.st.editor_toolbar.horizontal = horizontal;
    out.class(Ui::TOOLBAR, "horizontal", horizontal);
    update_scale(app, host, MARGIN);
    let l = limits(app, host, MARGIN);
    if horizontal {
        let s = host.rect(Ui::TOOLBAR);
        place(host, Some(l.left((vw - s.w) / 2.0)), l.top(vh - s.h - MARGIN), None);
    } else if min == dl && dl < SNAP_THRESHOLD {
        place(host, Some(l.min_left), l.top(r.y), None);
    } else if min == dr && dr < SNAP_THRESHOLD {
        place(host, None, l.top(r.y), Some(l.min_left));
    } else {
        place(host, Some(l.left(r.x)), l.top(r.y), None);
    }
    if was != horizontal {
        let tilt = if horizontal { "toolbar-tilt-back" } else { "toolbar-tilt-forward" };
        let out = host.out();
        out.class(Ui::TOOLBAR, "toolbar-tilt-forward", false);
        out.class(Ui::TOOLBAR, "toolbar-tilt-back", false);
        host.metric(Ui::TOOLBAR, Metric::OffsetWidth); // v5: `void toolbar.offsetWidth` restarts the animation
        host.out().class(Ui::TOOLBAR, tilt, true);
        app.st.editor_toolbar.tilt = Some(tilt);
        app.sched.timer(host.out(), T_TILT, 360.0);
    }
    app.sched.timer(host.out(), T_SNAP, 200.0);
}

// ---- drag -------------------------------------------------------------------------------------------------------

fn drag_down(app: &mut App, host: &mut dyn Host, p: &PointerBatch) -> u32 {
    app.sched.clear(host.out(), T_SNAP);
    let out = host.out();
    out.class(Ui::TOOLBAR, "snapping", false);
    out.style(Ui::TOOLBAR, "transition", Some("none"));
    update_scale(app, host, 10.0);
    let r = host.rect(Ui::TOOLBAR);
    let l = limits(app, host, 10.0);
    app.st.editor_toolbar.drag = Some((p.raw.x - r.x, p.raw.y - r.y, l));
    let out = host.out();
    out.style(Ui::TOOLBAR, "--toolbar-translate", Some("0px"));
    place(host, Some(r.x), r.y, None);
    let out = host.out();
    out.style(Ui::TOOLBAR_HANDLE, "cursor", Some("grabbing"));
    out.pointer_capture(Ui::TOOLBAR_HANDLE, p.pointer_id, true);
    // v5 listens on the document for the whole session; v6 only while a drag lasts.
    out.listen(Node::DOCUMENT, "pointermove", L_MOVE, 0);
    out.listen(Node::DOCUMENT, "pointerup", L_UP, 0);
    crate::abi::ret::PREVENT_DEFAULT
}

fn drag_move(app: &mut App, host: &mut dyn Host, p: &PointerBatch) {
    let Some((ox, oy, l)) = app.st.editor_toolbar.drag else { return };
    let out = host.out();
    out.style(Ui::TOOLBAR, "left", Some(&px(l.left(p.raw.x - ox))));
    out.style(Ui::TOOLBAR, "top", Some(&px(l.top(p.raw.y - oy))));
}

fn drag_up(app: &mut App, host: &mut dyn Host, p: &PointerBatch) {
    if app.st.editor_toolbar.drag.take().is_none() {
        return;
    }
    let out = host.out();
    out.unlisten(L_MOVE);
    out.unlisten(L_UP);
    out.style(Ui::TOOLBAR_HANDLE, "cursor", Some("grab"));
    out.pointer_capture(Ui::TOOLBAR_HANDLE, p.pointer_id, false);
    out.style(Ui::TOOLBAR, "transition", Some(""));
    snap(app, host);
}

// ---- eraser menu ------------------------------------------------------------------------------------------------

/// v5: `syncEraserMenuState()`.
fn sync_menu(app: &mut App, host: &mut dyn Host) {
    let (stroke, width) = (app.st.editor_input.eraser_stroke, app.st.editor_input.eraser_width);
    let out = host.out();
    out.prop(Ui::ERASER_STROKE_TOGGLE, "checked", Prop::Bool(stroke));
    for (&n, size) in app.st.editor_toolbar.sizes.iter().zip(SIZES) {
        out.class(n, "active", size == width);
    }
}

/// v5: `positionEraserMenu()`.
fn position_menu(app: &mut App, host: &mut dyn Host) {
    const MARGIN: f64 = 12.0;
    let b = host.rect(Ui::BTN_ERASER);
    let m = host.rect(Ui::ERASER_MENU);
    let top_offset = top_offset(app, host);
    let inset = bottom_inset(app, host);
    let (vw, vh) = window(host);
    let (mut left, mut top) = (b.right() + 12.0, b.y + (b.h - m.h) / 2.0);
    if app.st.editor_toolbar.horizontal {
        left = b.x + (b.w - m.w) / 2.0;
        top = b.y - m.h - 12.0;
    }
    left = MARGIN.max(left.min(vw - m.w - MARGIN));
    top = (top_offset + MARGIN).max(top.min(vh - m.h - MARGIN - inset));
    let out = host.out();
    out.style(Ui::ERASER_MENU, "left", Some(&px(left)));
    out.style(Ui::ERASER_MENU, "top", Some(&px(top)));
}

fn request(app: &mut App, host: &mut dyn Host, what: u8) {
    app.st.editor_toolbar.frame |= what;
    app.sched.frame(host.out(), Mod::EditorToolbar);
}

/// v5: `openEraserMenu()`.
pub fn open_menu(app: &mut App, host: &mut dyn Host) {
    sync_menu(app, host);
    let out = host.out();
    out.class(Ui::ERASER_MENU, "visible", true);
    out.attr(Ui::ERASER_MENU, "aria-hidden", Some("false"));
    out.listen(Node::DOCUMENT, "pointerdown", L_OUTSIDE, 0);
    app.st.editor_toolbar.menu_open = true;
    request(app, host, F_MENU);
}

/// v5: `closeEraserMenu()`.
pub fn close_menu(app: &mut App, host: &mut dyn Host) {
    if !std::mem::take(&mut app.st.editor_toolbar.menu_open) {
        return;
    }
    let out = host.out();
    out.class(Ui::ERASER_MENU, "visible", false);
    out.attr(Ui::ERASER_MENU, "aria-hidden", Some("true"));
    out.unlisten(L_OUTSIDE);
}

/// v5: `toggleEraserMenu()` (a click on the eraser button while an eraser is the tool).
pub fn toggle_menu(app: &mut App, host: &mut dyn Host) {
    if app.st.editor_toolbar.menu_open {
        close_menu(app, host);
    } else {
        open_menu(app, host);
    }
}

pub fn menu_open(app: &App) -> bool { app.st.editor_toolbar.menu_open }

/// v5: the eraser size buttons' click.
fn size_click(app: &mut App, host: &mut dyn Host, i: usize) {
    if view::read_only(app) {
        return;
    }
    let out = host.out();
    for (j, &n) in app.st.editor_toolbar.sizes.iter().enumerate() {
        out.class(n, "active", j == i);
    }
    app.st.editor_input.eraser_width = SIZES[i];
}

/// v5: `#eraser-stroke-toggle` change.
fn stroke_change(app: &mut App, on: bool) {
    if view::read_only(app) {
        return;
    }
    let st = &mut app.st.editor_input;
    st.eraser_stroke = on;
    if st.tool.is_eraser() {
        st.tool = if on { Tool::EraserStroke } else { Tool::EraserArea };
    }
}

// ---- module contract --------------------------------------------------------------------------------------------

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(app: &mut App, host: &mut dyn Host) {
    if app.cfg.has(crate::abi::mode::HEADLESS) {
        return;
    }
    let out = host.out();
    let header = out.query(Ui::EDITOR_VIEW, "header");
    let mode_group = out.query(Node::DOCUMENT, ".mode-toggle-group");
    out.listen(Ui::TOOLBAR_HANDLE, "pointerdown", L_DOWN, 0);
    out.listen(Ui::ERASER_STROKE_TOGGLE, "change", L_STROKE, 0);
    let mut sizes = Vec::with_capacity(SIZES.len());
    for (i, s) in SIZES.into_iter().enumerate() {
        let n = out.query(Ui::ERASER_MENU, &format!(".size-btn[data-eraser-size=\"{}\"]", jsfmt::num(s)));
        out.listen(n, "click", L_SIZE + i as u32, 0);
        sizes.push(n);
    }
    let st = &mut app.st.editor_toolbar;
    (st.header, st.mode_group, st.sizes) = (header, mode_group, sizes);
    // v5: syncEraserMenuState() after setupEventListeners (the markup's checkbox starts unchecked).
    sync_menu(app, host);
    // v5: setupToolbarDrag's first-frame normalisation.
    request(app, host, F_NORMALIZE);
}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Pointer(p) => match p.listener {
            L_DOWN if p.kind == ptr::DOWN => drag_down(app, host, &p),
            L_MOVE => {
                drag_move(app, host, &p);
                0
            }
            L_UP => {
                drag_up(app, host, &p);
                0
            }
            L_OUTSIDE => {
                // v5: a pointerdown outside the menu and the eraser button closes it.
                if !p.path.contains(Ui::ERASER_MENU) && !p.path.contains(Ui::BTN_ERASER) {
                    close_menu(app, host);
                }
                0
            }
            _ => 0,
        },
        Event::Dom(e) if e.listener == L_STROKE => {
            stroke_change(app, e.has(ev::CHECKED));
            0
        }
        Event::Dom(e) if e.ty == ety::CLICK && (L_SIZE..L_SIZE + SIZES.len() as u32).contains(&e.listener) => {
            size_click(app, host, (e.listener - L_SIZE) as usize);
            0
        }
        Event::Frame(_) => {
            let what = std::mem::take(&mut app.st.editor_toolbar.frame);
            if what & F_NORMALIZE != 0 {
                update_scale(app, host, 10.0);
                let r = host.rect(Ui::TOOLBAR);
                place(host, Some(r.x), r.y, None);
                host.out().style(Ui::TOOLBAR, "--toolbar-translate", Some("0px"));
            }
            if what & F_CLAMP != 0 {
                clamp(app, host, 12.0, false);
            }
            if what & F_MENU != 0 && app.st.editor_toolbar.menu_open {
                position_menu(app, host);
            }
            0
        }
        Event::Timer(T_SNAP) => {
            host.out().class(Ui::TOOLBAR, "snapping", false);
            0
        }
        Event::Timer(T_TILT) => {
            if let Some(t) = app.st.editor_toolbar.tilt.take() {
                host.out().class(Ui::TOOLBAR, t, false);
            }
            0
        }
        Event::Life(l) if l.code == life::RESIZE => {
            if app.st.editor_toolbar.menu_open {
                position_menu(app, host);
            }
            clamp(app, host, 10.0, true);
            0
        }
        Event::Msg(m) => {
            match (m.chan, m.data) {
                // v5: showEditorView → requestAnimationFrame(clampToolbarPosition).
                (shell::sig::VIEW, b"editor") => request(app, host, F_CLAMP),
                // v5: setReadOnlyMode(true) closes the eraser menu.
                (view::sig::MODE, b"read") => close_menu(app, host),
                _ => {}
            }
            0
        }
        _ => 0,
    }
}

/// `__IHN_TEST_API__.cmd('editor.toolbar.state')` → `{menuOpen, horizontal, dragging, eraserWidth, eraserStroke}`.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    let (st, i) = (&app.st.editor_toolbar, &app.st.editor_input);
    match name {
        "state" => Some(format!(
            "{{\"menuOpen\":{},\"horizontal\":{},\"dragging\":{},\"eraserWidth\":{},\"eraserStroke\":{}}}",
            st.menu_open,
            st.horizontal,
            st.drag.is_some(),
            jsfmt::num(i.eraser_width),
            i.eraser_stroke
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::kind;
    use crate::host::{FakeHost, Rect};
    use crate::shell::testkit::{self, click, life, msg};
    use ihn_core::codec::Writer;

    /// `input`'s listener of the eraser button (`L_TOOL + 4`).
    const ERASER_CLICK: u32 = tag(Mod::EditorInput, 20);

    fn boot() -> (App, FakeHost) {
        let (mut app, mut host) = testkit::boot(0, |_| {});
        view::set_read_only(&mut app, &mut host, false, false);
        for (m, v) in [(Metric::InnerWidth, 1280.0), (Metric::InnerHeight, 800.0)] {
            host.metrics.insert((Node::WINDOW, m), v);
        }
        host.metrics.insert((Ui::TOOLBAR, Metric::OffsetWidth), 56.0);
        host.metrics.insert((Ui::TOOLBAR, Metric::OffsetHeight), 420.0);
        let header = app.st.editor_toolbar.header;
        host.rects.insert(header, Rect { x: 0.0, y: 0.0, w: 1280.0, h: 60.0 });
        host.rects.insert(Ui::TOOLBAR, Rect { x: 20.0, y: 190.0, w: 56.0, h: 420.0 });
        host.take();
        (app, host)
    }

    fn pointer(app: &mut App, host: &mut FakeHost, listener: u32, k: u8, x: f64, y: f64, path: &[Node]) -> u32 {
        let mut w = Writer::default();
        w.u32(listener);
        w.u32(Node::DOCUMENT.0);
        w.u8(k);
        w.u8(ptr::MOUSE);
        w.i32(1);
        w.i32(0);
        w.u32(0);
        for v in [x, y, host.now] {
            w.f64(v);
        }
        for _ in 0..3 {
            w.f32(0.0);
        }
        w.u32(1);
        w.u32(0);
        w.u32(path.len() as u32);
        path.iter().for_each(|n| w.u32(n.0));
        app.event(host, kind::POINTER, &w.buf)
    }

    fn eraser_active(app: &mut App, host: &mut FakeHost) {
        click(app, host, ERASER_CLICK, Ui::BTN_ERASER);
        assert!(app.st.editor_input.tool.is_eraser());
        assert!(!menu_open(app));
    }

    #[test]
    fn eraser_button_toggles_the_menu_and_other_tools_close_it() {
        let (mut app, mut host) = boot();
        eraser_active(&mut app, &mut host);
        click(&mut app, &mut host, ERASER_CLICK, Ui::BTN_ERASER);
        assert!(menu_open(&app));
        let text = host.take();
        assert!(text.contains("visible") && text.contains("aria-hidden"), "{text}");
        // Positioned in the next frame, beside the button (vertical toolbar).
        host.rects.insert(Ui::BTN_ERASER, Rect { x: 26.0, y: 400.0, w: 44.0, h: 44.0 });
        host.rects.insert(Ui::ERASER_MENU, Rect { x: 0.0, y: 0.0, w: 180.0, h: 120.0 });
        app.frame(&mut host, 16.0);
        let text = host.take();
        assert!(text.contains("82px") && text.contains("362px"), "{text}");
        click(&mut app, &mut host, ERASER_CLICK, Ui::BTN_ERASER);
        assert!(!menu_open(&app));
        click(&mut app, &mut host, ERASER_CLICK, Ui::BTN_ERASER);
        assert!(menu_open(&app));
        crate::editor::input::set_tool(&mut app, &mut host, Some(Tool::Pen), Some("#002FD9"), Some(2.0));
        assert!(!menu_open(&app));
        // The pen's background double tap back to the pen also closes it.
        eraser_active(&mut app, &mut host);
        click(&mut app, &mut host, ERASER_CLICK, Ui::BTN_ERASER);
        crate::editor::input::toggle_eraser(&mut app, &mut host);
        assert!(!menu_open(&app));
    }

    #[test]
    fn outside_pointerdown_and_read_mode_close_the_menu() {
        let (mut app, mut host) = boot();
        eraser_active(&mut app, &mut host);
        click(&mut app, &mut host, ERASER_CLICK, Ui::BTN_ERASER);
        pointer(&mut app, &mut host, L_OUTSIDE, ptr::DOWN, 5.0, 5.0, &[Ui::ERASER_MENU, Node::BODY]);
        pointer(&mut app, &mut host, L_OUTSIDE, ptr::DOWN, 5.0, 5.0, &[Ui::BTN_ERASER, Ui::TOOLBAR]);
        assert!(menu_open(&app));
        pointer(&mut app, &mut host, L_OUTSIDE, ptr::DOWN, 5.0, 5.0, &[Ui::CANVAS_VIEWPORT, Node::BODY]);
        assert!(!menu_open(&app));
        assert!(host.take().contains("unlisten"));
        click(&mut app, &mut host, ERASER_CLICK, Ui::BTN_ERASER);
        assert!(menu_open(&app));
        view::set_read_only(&mut app, &mut host, true, false);
        assert!(!menu_open(&app));
    }

    #[test]
    fn sizes_and_stroke_toggle_set_the_eraser() {
        let (mut app, mut host) = boot();
        assert!(app.st.editor_input.eraser_stroke);
        let large = app.st.editor_toolbar.sizes[3];
        click(&mut app, &mut host, L_SIZE + 3, large);
        assert_eq!(app.st.editor_input.eraser_width, 32.0);
        eraser_active(&mut app, &mut host);
        assert_eq!(app.st.editor_input.tool, Tool::EraserStroke);
        testkit::dom(&mut app, &mut host, L_STROKE, Ui::ERASER_STROKE_TOGGLE, ety::CHANGE, 0, "", &[]);
        assert_eq!(app.st.editor_input.tool, Tool::EraserArea);
        assert!(!app.st.editor_input.eraser_stroke);
        // The menu shows the state when it opens.
        host.take();
        click(&mut app, &mut host, ERASER_CLICK, Ui::BTN_ERASER);
        let text = host.take();
        assert!(text.contains("checked"), "{text}");
        // Read mode: ignored.
        view::set_read_only(&mut app, &mut host, true, false);
        let tip = app.st.editor_toolbar.sizes[0];
        click(&mut app, &mut host, L_SIZE, tip);
        assert_eq!(app.st.editor_input.eraser_width, 32.0);
    }

    #[test]
    fn drag_follows_the_pointer_within_limits_and_snaps() {
        let (mut app, mut host) = boot();
        let r = pointer(&mut app, &mut host, L_DOWN, ptr::DOWN, 40.0, 200.0, &[Ui::TOOLBAR_HANDLE, Ui::TOOLBAR]);
        assert_eq!(r, crate::abi::ret::PREVENT_DEFAULT);
        let text = host.take();
        assert!(text.contains("grabbing") && text.contains("pointermove"), "{text}");
        // Above the header: clamped to 60 + 10.
        pointer(&mut app, &mut host, L_MOVE, ptr::MOVE, 300.0, 20.0, &[]);
        let text = host.take();
        assert!(text.contains("280px") && text.contains("70px"), "{text}");
        // Released near the bottom: horizontal, centred, tilted back.
        host.rects.insert(Ui::TOOLBAR, Rect { x: 600.0, y: 720.0, w: 56.0, h: 60.0 });
        pointer(&mut app, &mut host, L_UP, ptr::UP, 620.0, 730.0, &[]);
        assert!(app.st.editor_toolbar.horizontal && app.st.editor_toolbar.drag.is_none());
        let text = host.take();
        for want in ["horizontal", "snapping", "toolbar-tilt-back", "612px", "725px"] {
            assert!(text.contains(want), "{want} in {text}");
        }
        testkit::timer(&mut app, &mut host, T_SNAP);
        testkit::timer(&mut app, &mut host, T_TILT);
        assert!(app.st.editor_toolbar.tilt.is_none());
        // Moves after the release do nothing.
        host.take();
        pointer(&mut app, &mut host, L_MOVE, ptr::MOVE, 300.0, 300.0, &[]);
        assert!(!host.take().contains("px"));
        // Dragged to the right edge: vertical again, `right` set.
        pointer(&mut app, &mut host, L_DOWN, ptr::DOWN, 620.0, 730.0, &[Ui::TOOLBAR_HANDLE]);
        host.rects.insert(Ui::TOOLBAR, Rect { x: 1200.0, y: 300.0, w: 56.0, h: 420.0 });
        host.take();
        pointer(&mut app, &mut host, L_UP, ptr::UP, 1220.0, 310.0, &[]);
        assert!(!app.st.editor_toolbar.horizontal);
        let text = host.take();
        assert!(text.contains("toolbar-tilt-forward") && text.contains("right") && text.contains("15px"), "{text}");
    }

    #[test]
    fn editor_view_and_resize_clamp_the_toolbar() {
        let (mut app, mut host) = boot();
        host.rects.insert(Ui::TOOLBAR, Rect { x: 0.0, y: 0.0, w: 56.0, h: 420.0 });
        msg(&mut app, &mut host, shell::sig::VIEW, "editor");
        app.frame(&mut host, 16.0);
        let text = host.take();
        assert!(text.contains("12px") && text.contains("72px"), "{text}");
        host.rects.insert(Ui::TOOLBAR, Rect { x: 1250.0, y: 500.0, w: 56.0, h: 420.0 });
        life(&mut app, &mut host, life::RESIZE, 1280.0);
        let text = host.take();
        assert!(text.contains("1214px") && text.contains("370px") && text.contains("--toolbar-scale"), "{text}");
    }
}
