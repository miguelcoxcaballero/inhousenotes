//! Editor: Pan, zoom, wheel, inertia, overscroll, pull-to-add (ARCHITECTURE §11.3).
//!
//! `#canvas-container` carries `translate(panX px, panY px) scale(zoom)`; every gesture changes the three numbers
//! and the transform is applied in the next frame (5.18.0 `scheduleViewportTransformRender`) or at once
//! (`updateTransform`). Layout is read once per gesture (`Geometry`); canvas resizes and page activation wait until
//! the gesture settles (90 ms resync, `editor::pages`).

use super::{layout, pages, view};
use crate::abi::{life, listen, ptr, ret, DomEvent, PointerBatch};
use crate::host::{LsOp, Rect};
use crate::{shell, tag, App, Event, Host, Mod, Node, Ui};
use ihn_core::jsfmt::num;
use ihn_core::jsmath;

// v5: app-v5.js constants
pub const MIN_ZOOM: f64 = 0.5;
pub const MAX_ZOOM: f64 = 3.0;
const MIN_PAGE_VISIBLE: f64 = 5.0;
pub const OVERSCROLL_THRESHOLD: f64 = 360.0;
const OVERSCROLL_FRICTION: f64 = 0.24;
const OVERSCROLL_MARGIN: f64 = 100.0;
const CREATE_PAGE_MIN_BOTTOM_RATIO: f64 = 0.45;
const MAX_PAN_STEP_PX: f64 = 100.0;
const TOUCH_INERTIA_DECAY: f64 = 0.935;
const TOUCH_INERTIA_STOP: f64 = 0.015;
const CANVAS_RESYNC_IDLE_MS: f64 = 90.0;
const VIEW_SAVE_DELAY: f64 = 250.0;
pub const VIEW_STORAGE_KEY: &str = "notebook-view-v1";
/// v5 `hasRecentUserActivity(240)` inside `isViewportInteracting`.
const RECENT_ACTIVITY_MS: f64 = 240.0;

const L_DOWN: u32 = tag(Mod::EditorViewport, 1);
const L_MOVE: u32 = tag(Mod::EditorViewport, 2);
const L_UP: u32 = tag(Mod::EditorViewport, 3);
const L_WHEEL: u32 = tag(Mod::EditorViewport, 4);
/// `pointercancel` / `pointerleave`: `host/dom.js` keeps one listener per id, so each registration needs its own.
const L_CANCEL: u32 = tag(Mod::EditorViewport, 5);
const L_LEAVE: u32 = tag(Mod::EditorViewport, 6);
const T_WHEEL: u32 = tag(Mod::EditorViewport, 10);
const T_OVERSCROLL: u32 = tag(Mod::EditorViewport, 11);
const T_SAVE: u32 = tag(Mod::EditorViewport, 12);
const T_RESYNC: u32 = tag(Mod::EditorViewport, 13);
const T_TRANSITION: u32 = tag(Mod::EditorViewport, 14);
const T_GESTURE: u32 = tag(Mod::EditorViewport, 15);
/// v5: `INTERACTION_IDLE_MS`.
const INTERACTION_IDLE_MS: f64 = 100.0;

/// Axis lock of a one-finger pan (v5 `scrollLocked`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Lock {
    #[default]
    None,
    Vertical,
    Horizontal,
    Free,
}

/// v5 `viewportGestureGeometry`: the viewport rect and the content bounds, read once per gesture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub rect: Rect,
    pub bounds: [f64; 4],
}

pub struct State {
    pub zoom: f64,
    pub pan_x: f64,
    pub pan_y: f64,
    pub overscroll_y: f64,
    pub pending_create: bool,
    is_panning: bool,
    lock: Lock,
    last_pan: (f64, f64),
    pan_start: (f64, f64),
    pinch_dist: f64,
    initial_scale: f64,
    initial_pan: (f64, f64),
    pinch_center: (f64, f64),
    /// v5 `state.activePointers`: id → client position, in insertion order.
    pointers: Vec<(i32, f64, f64)>,
    vel: (f64, f64),
    last_pan_time: f64,
    inertia: bool,
    inertia_last: f64,
    inertia_lock: Lock,
    geometry: Option<Geometry>,
    raf_transform: bool,
    raf_resize: Option<(f64, f64)>,
    resync_force: bool,
    resync_armed: bool,
    /// Pointer ids a page canvas took for drawing (`claim`): the viewport ignores them.
    claimed: Vec<i32>,
    /// Set by the input pipeline while the pen writes (v5 `isPenWriting`).
    pub pen_writing: bool,
    indicator: Node,
    /// v5 `isGestureInteracting` and its idle deadline.
    gesture: bool,
    gesture_deadline: f64,
    gesture_armed: bool,
    /// Layout reads made by this module (gesture budget test).
    pub reads: u32,
}

impl Default for State {
    fn default() -> Self {
        State {
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            overscroll_y: 0.0,
            pending_create: false,
            is_panning: false,
            lock: Lock::None,
            last_pan: (0.0, 0.0),
            pan_start: (0.0, 0.0),
            pinch_dist: 0.0,
            initial_scale: 1.0,
            initial_pan: (0.0, 0.0),
            pinch_center: (0.0, 0.0),
            pointers: Vec::new(),
            vel: (0.0, 0.0),
            last_pan_time: 0.0,
            inertia: false,
            inertia_last: 0.0,
            inertia_lock: Lock::Free,
            geometry: None,
            raf_transform: false,
            raf_resize: None,
            resync_force: false,
            resync_armed: false,
            claimed: Vec::new(),
            pen_writing: false,
            indicator: Node::NONE,
            gesture: false,
            gesture_deadline: 0.0,
            gesture_armed: false,
            reads: 0,
        }
    }
}

// ---- public API -------------------------------------------------------------------------------------------------

/// The page canvas took pointer `id` for a drawing tool: the viewport neither pans nor pinches with it (v5: the
/// `canvas.page-canvas` + `shouldAcceptDrawInput` test of `handleViewportPointerDown`). Call from the canvas'
/// `pointerdown`, which runs before the viewport's.
pub fn claim(app: &mut App, pointer_id: i32) {
    let c = &mut app.st.editor_viewport.claimed;
    if !c.contains(&pointer_id) {
        c.push(pointer_id);
    }
}

/// v5: `isViewportInteracting()`.
pub fn interacting(app: &App, now: f64) -> bool {
    let st = &app.st.editor_viewport;
    st.is_panning || !st.pointers.is_empty() || st.inertia || st.pen_writing || st.gesture || app.sched.quiet_for(now) < RECENT_ACTIVITY_MS
}

/// A gesture holds cached geometry (v5 `viewportGestureGeometry` is set).
pub fn in_gesture(app: &App) -> bool { app.st.editor_viewport.geometry.is_some() }

/// The viewport rect: the gesture's cached one, else a fresh read.
pub fn rect(app: &mut App, host: &mut dyn Host) -> Rect {
    if let Some(g) = app.st.editor_viewport.geometry {
        return g.rect;
    }
    app.st.editor_viewport.reads += 1;
    host.rect(Ui::CANVAS_VIEWPORT)
}

fn geometry(app: &mut App, host: &mut dyn Host) -> Geometry {
    if let Some(g) = app.st.editor_viewport.geometry {
        return g;
    }
    app.st.editor_viewport.reads += 1;
    let g = Geometry { rect: host.rect(Ui::CANVAS_VIEWPORT), bounds: layout::metrics(app).bounds };
    app.st.editor_viewport.geometry = Some(g);
    g
}

/// v5: `invalidatePageBoundsCache()` also drops the gesture geometry.
pub fn forget_geometry(app: &mut App) { app.st.editor_viewport.geometry = None; }

/// The transform string of `#canvas-container`.
pub fn transform_css(zoom: f64, x: f64, y: f64) -> String { format!("translate({}px, {}px) scale({})", num(x), num(y), num(zoom)) }

/// v5: `clampPanToVisible(geometry)`.
pub fn clamp(app: &mut App, host: &mut dyn Host) {
    let g = app.st.editor_viewport.geometry;
    let r = match g {
        Some(g) => g.rect,
        None => {
            app.st.editor_viewport.reads += 1;
            host.rect(Ui::CANVAS_VIEWPORT)
        }
    };
    if r.w == 0.0 || r.h == 0.0 {
        return;
    }
    let b = g.map_or(layout::metrics(app).bounds, |g| g.bounds);
    let st = &mut app.st.editor_viewport;
    let (min_x, max_x) = (MIN_PAGE_VISIBLE - b[2] * st.zoom, r.w - MIN_PAGE_VISIBLE - b[0] * st.zoom);
    let (min_y, max_y) = (MIN_PAGE_VISIBLE - b[3] * st.zoom, r.h - MIN_PAGE_VISIBLE - b[1] * st.zoom);
    st.pan_x = if min_x > max_x { (min_x + max_x) / 2.0 } else { st.pan_x.min(max_x).max(min_x) };
    st.pan_y = if min_y > max_y { (min_y + max_y) / 2.0 } else { st.pan_y.min(max_y).max(min_y) };
}

/// v5: `applyViewportTransformStyle()`.
fn apply_style(app: &mut App, host: &mut dyn Host) {
    let st = &app.st.editor_viewport;
    let css = transform_css(st.zoom, st.pan_x, st.pan_y);
    host.out().style(Ui::CANVAS_CONTAINER, "transform", Some(&css));
    crate::home::scroll::schedule(app, host);
    pages::queue_scroll_previews(app, host, false);
}

/// v5: `scheduleViewportTransformRender()`.
pub fn schedule_render(app: &mut App, host: &mut dyn Host) {
    if !std::mem::replace(&mut app.st.editor_viewport.raf_transform, true) {
        app.sched.frame(host.out(), Mod::EditorViewport);
    }
}

/// v5: `updateTransform()`.
pub fn update_transform(app: &mut App, host: &mut dyn Host) {
    app.st.editor_viewport.raf_transform = false;
    clamp(app, host);
    apply_style(app, host);
    let now = host.now();
    if interacting(app, now) || in_gesture(app) {
        schedule_resync(app, host, false, false);
    } else {
        schedule_resync(app, host, true, false);
    }
    schedule_save(app, host);
    pages::schedule_visible(app, host, false);
}

/// v5: `scheduleCanvasResolutionSync({immediate, force})`.
pub fn schedule_resync(app: &mut App, host: &mut dyn Host, immediate: bool, force: bool) {
    if immediate {
        if std::mem::take(&mut app.st.editor_viewport.resync_armed) {
            app.sched.clear(host.out(), T_RESYNC);
        }
        resync_now(app, host, force);
        return;
    }
    app.st.editor_viewport.resync_force |= force;
    app.st.editor_viewport.resync_armed = true;
    app.sched.timer(host.out(), T_RESYNC, CANVAS_RESYNC_IDLE_MS);
}

fn resync_now(app: &mut App, host: &mut dyn Host, force: bool) {
    pages::warm_template_urls(app, host);
    pages::schedule_visible(app, host, true);
    pages::sync_resolution(app, host, force);
}

/// v5: `scheduleViewSave()`: `notebook-view-v1` 250 ms after the last change.
fn schedule_save(app: &mut App, host: &mut dyn Host) { app.sched.timer(host.out(), T_SAVE, VIEW_SAVE_DELAY); }

fn save_view(app: &mut App, host: &mut dyn Host) {
    let st = &app.st.editor_viewport;
    let data = format!("{{\"panX\":{},\"panY\":{},\"zoom\":{}}}", num(st.pan_x), num(st.pan_y), num(st.zoom));
    host.ls(LsOp::Set, VIEW_STORAGE_KEY, Some(&data));
}

/// Resets the view of a new session (v5 `openDriveFile`: zoom 1, pan 0, no overscroll).
pub fn reset(app: &mut App, host: &mut dyn Host) {
    release_gesture(app, host);
    let st = &mut app.st.editor_viewport;
    (st.zoom, st.pan_x, st.pan_y, st.overscroll_y, st.pending_create) = (1.0, 0.0, 0.0, 0.0, false);
    app.sched.clear(host.out(), T_OVERSCROLL);
}

/// v5: `alignViewportTopToWidth(width)`: page column horizontally centred, top at 20 px.
pub fn align_top(app: &mut App, host: &mut dyn Host, page_width: f64) {
    let r = rect(app, host);
    let st = &mut app.st.editor_viewport;
    st.pan_x = (r.w - page_width * st.zoom) / 2.0 - layout::PAGE_PADDING * st.zoom;
    st.pan_y = 20.0;
    st.overscroll_y = 0.0;
    app.sched.clear(host.out(), T_OVERSCROLL);
    update_transform(app, host);
}

/// v5: `centerViewOnTop()`.
pub fn center_on_top(app: &mut App, host: &mut dyn Host) {
    let w = layout::max_width(layout::pages(app));
    align_top(app, host, w);
    update_indicator(app, host);
}

/// v5: `goToPage(i)`: page `i` centred at the current zoom.
pub fn go_to_page(app: &mut App, host: &mut dyn Host, i: usize) {
    if i >= layout::len(app) {
        return;
    }
    let r = rect(app, host);
    let (top, (w, h)) = (layout::page_top(app, i), layout::dims(app, i));
    let st = &mut app.st.editor_viewport;
    st.pan_y = -(top * st.zoom) + r.h / 2.0 - (h * st.zoom) / 2.0;
    st.pan_x = r.w / 2.0 - (w * st.zoom) / 2.0 - layout::PAGE_PADDING * st.zoom;
    update_transform(app, host);
    pages::set_active_page(app, i);
    pages::schedule_visible(app, host, true);
}

/// The fast-scroll thumb's editor target (v5 `installDocumentFastScroller` with `canvasViewport`):
/// `(max, offset, rect)`.
pub fn scroll_metrics(app: &mut App, host: &mut dyn Host) -> (f64, f64, Rect) {
    let r = rect(app, host);
    let st = &app.st.editor_viewport;
    let max = (layout::content_height(app) * st.zoom - r.h + 40.0).max(0.0);
    (max, 20.0 - st.pan_y, r)
}

/// The thumb was dragged to `offset` (v5 `move` of the fast scroller).
pub fn scroll_to(app: &mut App, host: &mut dyn Host, offset: f64) {
    let st = &mut app.st.editor_viewport;
    st.pan_y = 20.0 - offset;
    st.overscroll_y = 0.0;
    if st.inertia {
        st.inertia = false;
    }
    schedule_render(app, host);
    pages::schedule_visible(app, host, false);
    schedule_save(app, host);
}

// ---- add-page indicator and vertical pan ------------------------------------------------------------------------

fn viewport_height(app: &mut App, host: &mut dyn Host) -> f64 { rect(app, host).h }

/// v5: `getBottomLimit().minPanY`.
fn min_pan_y(app: &mut App, host: &mut dyn Host) -> f64 {
    let vh = viewport_height(app, host);
    vh - OVERSCROLL_MARGIN - layout::content_height(app) * app.st.editor_viewport.zoom
}

fn can_create(app: &App, vh: f64) -> bool {
    let st = &app.st.editor_viewport;
    st.pan_y + layout::content_height(app) * st.zoom < vh * CREATE_PAGE_MIN_BOTTOM_RATIO
}

/// v5: `updateAddPageIndicator()`.
pub fn update_indicator(app: &mut App, host: &mut dyn Host) {
    let ind = app.st.editor_viewport.indicator;
    if ind.is_none() {
        return;
    }
    if view::read_only(app) || layout::len(app) == 0 {
        host.out().style(ind, "opacity", Some("0"));
        return;
    }
    let bottom = layout::content_height(app) - layout::PAGE_PADDING;
    let out = host.out();
    out.style(ind, "left", Some("50%"));
    out.style(ind, "top", Some(&format!("{}px", num(bottom + 20.0))));
    out.style(ind, "transform", Some("translateX(-50%)"));
    let vh = viewport_height(app, host);
    let ok = can_create(app, vh);
    let over = app.st.editor_viewport.overscroll_y;
    let mut opacity = 0.0;
    if over > 0.0 {
        opacity = (0.2 + (over / OVERSCROLL_THRESHOLD).min(1.0) * 0.8).min(1.0);
        if !ok {
            opacity *= 0.6;
        }
    }
    host.out().style(ind, "opacity", Some(&num(opacity)));
}

/// v5: `applyVerticalPan(dy)`.
pub fn vertical_pan(app: &mut App, host: &mut dyn Host, dy: f64) {
    if dy == 0.0 {
        return;
    }
    let min_y = min_pan_y(app, host);
    if view::read_only(app) {
        let st = &mut app.st.editor_viewport;
        st.pan_y = min_y.max(st.pan_y + dy);
        st.overscroll_y = 0.0;
        st.pending_create = false;
        update_indicator(app, host);
        return;
    }
    let next = app.st.editor_viewport.pan_y + dy;
    if dy < 0.0 && next < min_y {
        let vh = viewport_height(app, host);
        let required = ((vh * (1.0 - CREATE_PAGE_MIN_BOTTOM_RATIO) - OVERSCROLL_MARGIN) / OVERSCROLL_FRICTION).max(0.0);
        let max_over = (OVERSCROLL_THRESHOLD * 3.0).max(required * 1.1);
        let st = &mut app.st.editor_viewport;
        st.overscroll_y = max_over.min(st.overscroll_y + (min_y - next));
        st.pan_y = min_y - st.overscroll_y * OVERSCROLL_FRICTION;
        let ok = can_create(app, vh);
        let st = &mut app.st.editor_viewport;
        st.pending_create = st.overscroll_y >= OVERSCROLL_THRESHOLD && ok;
        update_indicator(app, host);
        app.sched.timer(host.out(), T_OVERSCROLL, 600.0);
        return;
    }
    if dy > 0.0 && app.st.editor_viewport.overscroll_y > 0.0 {
        let st = &mut app.st.editor_viewport;
        let remaining = (dy - st.overscroll_y).max(0.0);
        st.overscroll_y = (st.overscroll_y - dy).max(0.0);
        st.pan_y = min_y - st.overscroll_y * OVERSCROLL_FRICTION;
        if remaining > 0.0 {
            st.pan_y += remaining;
        }
        let vh = viewport_height(app, host);
        let ok = can_create(app, vh);
        let st = &mut app.st.editor_viewport;
        st.pending_create = st.overscroll_y >= OVERSCROLL_THRESHOLD && ok;
        update_indicator(app, host);
        return;
    }
    let st = &mut app.st.editor_viewport;
    st.pan_y = next;
    st.pending_create = false;
    update_indicator(app, host);
}

/// v5: `finalizePendingPageCreation()`.
pub fn finalize_create(app: &mut App, host: &mut dyn Host) -> bool {
    if view::read_only(app) {
        let st = &mut app.st.editor_viewport;
        st.pending_create = false;
        st.overscroll_y = 0.0;
        update_indicator(app, host);
        return false;
    }
    app.st.editor_viewport.reads += 1;
    let vh = host.rect(Ui::CANVAS_VIEWPORT).h;
    let ok = can_create(app, vh);
    let st = &app.st.editor_viewport;
    if st.pending_create && st.overscroll_y >= OVERSCROLL_THRESHOLD && ok {
        let target = min_pan_y(app, host);
        pages::add_page(app, host, true);
        let st = &mut app.st.editor_viewport;
        st.overscroll_y = 0.0;
        st.pending_create = false;
        update_indicator(app, host);
        // v5: animatePanTo(prevLimit.minPanY)
        host.out().style(Ui::CANVAS_CONTAINER, "transition", Some("transform 0.35s ease"));
        app.st.editor_viewport.pan_y = target;
        update_transform(app, host);
        app.sched.timer(host.out(), T_TRANSITION, 360.0);
        return true;
    }
    app.st.editor_viewport.pending_create = false;
    update_indicator(app, host);
    false
}

fn snap_back(app: &mut App, host: &mut dyn Host) {
    if app.st.editor_viewport.overscroll_y > 0.0 {
        let min_y = min_pan_y(app, host);
        let st = &mut app.st.editor_viewport;
        st.overscroll_y = 0.0;
        st.pan_y = st.pan_y.max(min_y);
        update_transform(app, host);
    }
}

// ---- gestures ---------------------------------------------------------------------------------------------------

fn cap(v: f64) -> f64 { v.clamp(-MAX_PAN_STEP_PX, MAX_PAN_STEP_PX) }

/// v5: `clampPanDelta` — a jump of more than 100 px is dropped (`None`).
fn delta(cur: (f64, f64), last: (f64, f64)) -> Option<(f64, f64)> {
    let (dx, dy) = (cur.0 - last.0, cur.1 - last.1);
    (dx.abs() <= MAX_PAN_STEP_PX && dy.abs() <= MAX_PAN_STEP_PX).then_some((dx, dy))
}

/// v5: `releaseViewportGesture()` (window blur).
pub fn release_gesture(app: &mut App, host: &mut dyn Host) {
    let st = &mut app.st.editor_viewport;
    st.pointers.clear();
    st.claimed.clear();
    st.is_panning = false;
    st.inertia = false;
    st.vel = (0.0, 0.0);
    st.raf_transform = false;
    st.geometry = None;
    app.sched.set_busy(Mod::EditorViewport, false);
    host.out().style(Ui::CANVAS_VIEWPORT, "cursor", Some(""));
}

/// v5: `stopInertia(triggerUpdate)`.
fn stop_inertia(app: &mut App, host: &mut dyn Host, update: bool) {
    let st = &mut app.st.editor_viewport;
    st.inertia = false;
    st.vel = (0.0, 0.0);
    st.last_pan_time = 0.0;
    if st.pointers.is_empty() {
        st.geometry = None;
    }
    busy(app);
    if update {
        update_transform(app, host);
    }
}

fn busy(app: &mut App) {
    let st = &app.st.editor_viewport;
    let on = st.is_panning || !st.pointers.is_empty() || st.inertia;
    app.sched.set_busy(Mod::EditorViewport, on);
}

fn pinch_distance(st: &State) -> f64 {
    if st.pointers.len() < 2 {
        return 0.0;
    }
    let (a, b) = (st.pointers[0], st.pointers[1]);
    let (dx, dy) = (a.1 - b.1, a.2 - b.2);
    (dx * dx + dy * dy).sqrt()
}

/// v5: `noteInteractionActivity()` / `refreshInteractionIdleTimer()`: one idle deadline 100 ms after the last
/// gesture event; when it passes with nothing held, the gesture has settled (resync, visible pages, previews, save).
pub fn note_interaction(app: &mut App, host: &mut dyn Host, t: f64) {
    app.sched.activity(t);
    let now = host.now();
    let st = &mut app.st.editor_viewport;
    st.gesture = true;
    st.gesture_deadline = now + INTERACTION_IDLE_MS;
    if !std::mem::replace(&mut st.gesture_armed, true) {
        app.sched.timer(host.out(), T_GESTURE, INTERACTION_IDLE_MS);
    }
}

/// v5: `isGestureInteracting = false` (Read mode ends gestures).
pub fn end_interaction(app: &mut App, host: &mut dyn Host) {
    let st = &mut app.st.editor_viewport;
    st.gesture = false;
    if std::mem::take(&mut st.gesture_armed) {
        app.sched.clear(host.out(), T_GESTURE);
    }
}

fn gesture_settle(app: &mut App, host: &mut dyn Host) {
    let now = host.now();
    let st = &mut app.st.editor_viewport;
    st.gesture_armed = false;
    let remaining = st.gesture_deadline - now;
    if remaining > 0.0 || !st.pointers.is_empty() || st.is_panning || st.inertia {
        st.gesture_armed = true;
        app.sched.timer(host.out(), T_GESTURE, INTERACTION_IDLE_MS.max(remaining));
        return;
    }
    st.gesture = false;
    st.gesture_deadline = 0.0;
    st.geometry = None;
    schedule_resync(app, host, false, false);
    pages::schedule_visible(app, host, false);
    pages::queue_scroll_previews(app, host, false);
    schedule_save(app, host);
}

/// v5: `handleWheel(e)`.
fn wheel(app: &mut App, host: &mut dyn Host, e: &DomEvent) -> u32 {
    note_interaction(app, host, e.t);
    let g = geometry(app, host);
    let scale = match e.delta_mode {
        1 => 16.0,
        2 => (g.rect.h * 0.85).max(240.0),
        _ => 1.0,
    };
    let (ndx, ndy) = (e.dx * scale, e.dy * scale);
    if e.has(crate::abi::ev::CTRL) || e.has(crate::abi::ev::META) {
        let st = &mut app.st.editor_viewport;
        let factor = if ndy > 0.0 { 0.94 } else { 1.06 };
        let zoom = (st.zoom * factor).max(MIN_ZOOM).min(MAX_ZOOM);
        let (mx, my) = (e.x - g.rect.x, e.y - g.rect.y);
        let (cx, cy) = ((mx - st.pan_x) / st.zoom, (my - st.pan_y) / st.zoom);
        st.zoom = zoom;
        st.pan_x = mx - cx * zoom;
        st.pan_y = my - cy * zoom;
    } else {
        app.st.editor_viewport.pan_x -= cap(ndx * 0.92);
        vertical_pan(app, host, -cap(ndy * 0.92));
    }
    schedule_render(app, host);
    app.sched.timer(host.out(), T_WHEEL, 300.0);
    ret::PREVENT_DEFAULT
}

/// v5: `handleViewportPointerDown(e)`.
fn down(app: &mut App, host: &mut dyn Host, p: &PointerBatch) -> u32 {
    if p.ptype == ptr::PEN {
        return 0; // the pen never pans (its background double tap is the input pipeline's)
    }
    if let Some(at) = app.st.editor_viewport.claimed.iter().position(|&id| id == p.pointer_id) {
        app.st.editor_viewport.claimed.swap_remove(at);
        return 0;
    }
    if p.ptype == ptr::TOUCH && app.st.editor_viewport.pen_writing {
        return 0;
    }
    note_interaction(app, host, p.raw.t);
    stop_inertia(app, host, false);
    geometry(app, host);
    let (x, y) = (p.raw.x, p.raw.y);
    let now = host.now();
    let st = &mut app.st.editor_viewport;
    if let Some(e) = st.pointers.iter_mut().find(|e| e.0 == p.pointer_id) {
        (e.1, e.2) = (x, y);
    } else {
        st.pointers.push((p.pointer_id, x, y));
    }
    if p.ptype == ptr::MOUSE {
        st.is_panning = true;
        st.last_pan = (x, y);
        busy(app);
        host.out().style(Ui::CANVAS_VIEWPORT, "cursor", Some("grabbing"));
        return 0;
    }
    if st.pointers.len() == 2 {
        let (a, b) = (st.pointers[0], st.pointers[1]);
        st.pinch_center = ((a.1 + b.1) / 2.0, (a.2 + b.2) / 2.0);
        st.pinch_dist = pinch_distance(st);
        st.initial_scale = st.zoom;
        st.initial_pan = (st.pan_x, st.pan_y);
        st.lock = Lock::None;
        st.vel = (0.0, 0.0);
        st.last_pan_time = 0.0;
    } else if st.pointers.len() == 1 {
        st.is_panning = true;
        st.lock = Lock::None;
        st.last_pan = (x, y);
        st.pan_start = (x, y);
        st.vel = (0.0, 0.0);
        st.last_pan_time = now;
    }
    busy(app);
    0
}

/// v5: `handleViewportPointerMove(e)`.
fn move_(app: &mut App, host: &mut dyn Host, p: &PointerBatch) -> u32 {
    if p.ptype == ptr::PEN {
        return 0;
    }
    let (x, y) = (p.raw.x, p.raw.y);
    if !app.st.editor_viewport.pointers.iter().any(|e| e.0 == p.pointer_id) {
        return 0;
    }
    note_interaction(app, host, p.raw.t);
    let st = &mut app.st.editor_viewport;
    let Some(e) = st.pointers.iter_mut().find(|e| e.0 == p.pointer_id) else { return 0 };
    (e.1, e.2) = (x, y);
    if p.ptype == ptr::MOUSE && st.is_panning {
        let d = delta((x, y), st.last_pan);
        st.last_pan = (x, y);
        if let Some((dx, dy)) = d {
            st.pan_x += dx;
            vertical_pan(app, host, dy);
            schedule_render(app, host);
        }
        return 0;
    }
    if st.pointers.len() == 2 {
        if st.pinch_dist > 0.0 {
            let dist = pinch_distance(st);
            let zoom = (st.initial_scale * (dist / st.pinch_dist)).max(MIN_ZOOM).min(MAX_ZOOM);
            let (a, b) = (st.pointers[0], st.pointers[1]);
            let (cx, cy) = ((a.1 + b.1) / 2.0, (a.2 + b.2) / 2.0);
            let r = rect(app, host);
            let st = &mut app.st.editor_viewport;
            let content_x = (st.pinch_center.0 - r.x - st.initial_pan.0) / st.initial_scale;
            let content_y = (st.pinch_center.1 - r.y - st.initial_pan.1) / st.initial_scale;
            st.zoom = zoom;
            st.pan_x = cx - r.x - content_x * zoom;
            st.pan_y = cy - r.y - content_y * zoom;
            schedule_render(app, host);
        }
    } else if st.pointers.len() == 1 && st.is_panning {
        let d = delta((x, y), st.last_pan);
        let (dx, dy) = d.unwrap_or((0.0, 0.0));
        if st.lock == Lock::None {
            let (tdx, tdy) = ((x - st.pan_start.0).abs(), (y - st.pan_start.1).abs());
            if tdx > 10.0 || tdy > 10.0 {
                st.lock = if tdy > tdx * 2.0 {
                    Lock::Vertical
                } else if tdx > tdy * 2.0 {
                    Lock::Horizontal
                } else {
                    Lock::Free
                };
            }
        }
        match st.lock {
            Lock::Vertical => vertical_pan(app, host, dy),
            Lock::Horizontal => st.pan_x += dx,
            _ => {
                st.pan_x += dx;
                vertical_pan(app, host, dy);
            }
        }
        let now = host.now();
        let st = &mut app.st.editor_viewport;
        st.last_pan = (x, y);
        if d.is_some() {
            let mut dt = now - if st.last_pan_time != 0.0 { st.last_pan_time } else { now };
            if !dt.is_finite() || dt < 8.0 {
                dt = 16.0;
            } else if dt > 64.0 {
                dt = 64.0;
            }
            st.vel = (st.vel.0 * 0.72 + (dx / dt) * 0.28, st.vel.1 * 0.72 + (dy / dt) * 0.28);
            st.last_pan_time = now;
            schedule_render(app, host);
        }
    }
    0
}

/// v5: `handleViewportPointerUp(e)` (also `pointercancel` and `pointerleave`).
fn up(app: &mut App, host: &mut dyn Host, p: &PointerBatch) -> u32 {
    if p.ptype == ptr::PEN {
        return 0;
    }
    let st = &mut app.st.editor_viewport;
    st.claimed.retain(|&id| id != p.pointer_id);
    st.pointers.retain(|e| e.0 != p.pointer_id);
    if p.ptype == ptr::MOUSE {
        st.is_panning = false;
        host.out().style(Ui::CANVAS_VIEWPORT, "cursor", Some(""));
        finalize_create(app, host);
        stop_inertia(app, host, false);
        snap_back(app, host);
        update_transform(app, host);
        busy(app);
        return 0;
    }
    let st = &mut app.st.editor_viewport;
    if st.pointers.len() < 2 {
        st.pinch_dist = 0.0;
    }
    if st.pointers.len() == 1 {
        let (_, x, y) = st.pointers[0];
        st.is_panning = true;
        st.lock = Lock::None;
        st.last_pan = (x, y);
        st.pan_start = (x, y);
        st.last_pan_time = host.now();
        st.vel = (0.0, 0.0);
    }
    if app.st.editor_viewport.pointers.is_empty() {
        let st = &mut app.st.editor_viewport;
        st.is_panning = false;
        let lock = if st.lock == Lock::None { Lock::Free } else { st.lock };
        st.lock = Lock::None;
        finalize_create(app, host);
        let st = &mut app.st.editor_viewport;
        if st.overscroll_y == 0.0 {
            if jsmath::hypot(st.vel.0, st.vel.1) > TOUCH_INERTIA_STOP {
                st.inertia_lock = lock;
                start_inertia(app, host);
            } else {
                update_transform(app, host);
            }
        } else {
            update_transform(app, host);
        }
    }
    snap_back(app, host);
    let st = &mut app.st.editor_viewport;
    if st.pointers.is_empty() && !st.inertia {
        st.geometry = None;
    }
    busy(app);
    0
}

/// v5: `startInertia()`.
fn start_inertia(app: &mut App, host: &mut dyn Host) {
    let now = host.now();
    let st = &mut app.st.editor_viewport;
    if st.inertia {
        return;
    }
    st.raf_transform = false;
    st.inertia = true;
    st.inertia_last = now;
    app.sched.frame(host.out(), Mod::EditorViewport);
}

fn inertia_step(app: &mut App, host: &mut dyn Host, t: f64) {
    let st = &mut app.st.editor_viewport;
    let dt = (t - st.inertia_last).max(1.0);
    st.inertia_last = t;
    let decay = jsmath::pow(TOUCH_INERTIA_DECAY, dt / 16.67);
    st.vel = (st.vel.0 * decay, st.vel.1 * decay);
    if st.vel.0.abs() < TOUCH_INERTIA_STOP && st.vel.1.abs() < TOUCH_INERTIA_STOP {
        stop_inertia(app, host, true);
        return;
    }
    let (dx, dy) = (cap(st.vel.0 * dt), cap(st.vel.1 * dt));
    match st.inertia_lock {
        Lock::Vertical => vertical_pan(app, host, dy),
        Lock::Horizontal => st.pan_x += dx,
        _ => {
            st.pan_x += dx;
            vertical_pan(app, host, dy);
        }
    }
    clamp(app, host);
    apply_style(app, host);
    app.sched.frame(host.out(), Mod::EditorViewport);
}

fn frame(app: &mut App, host: &mut dyn Host, t: f64) {
    if app.st.editor_viewport.inertia {
        inertia_step(app, host, t);
    } else if std::mem::take(&mut app.st.editor_viewport.raf_transform) {
        clamp(app, host);
        apply_style(app, host);
        if !interacting(app, host.now()) {
            schedule_resync(app, host, false, false);
            pages::schedule_visible(app, host, false);
        }
    }
    if let Some((ax, ay)) = app.st.editor_viewport.raf_resize.take() {
        app.st.editor_viewport.reads += 1;
        let r = host.rect(Ui::CANVAS_VIEWPORT);
        let st = &mut app.st.editor_viewport;
        st.pan_x = r.w / 2.0 - ax * st.zoom;
        st.pan_y = r.h / 2.0 - ay * st.zoom;
        st.overscroll_y = 0.0;
        st.pending_create = false;
        update_indicator(app, host);
        update_transform(app, host);
    }
}

/// v5: `preserveViewOnResize()`: the content point at the viewport centre stays there.
fn resized(app: &mut App, host: &mut dyn Host) {
    app.st.editor_viewport.geometry = None;
    if shell::view(app) != shell::View::Editor {
        return;
    }
    app.st.editor_viewport.reads += 1;
    let r = host.rect(Ui::CANVAS_VIEWPORT);
    if r.w == 0.0 || r.h == 0.0 {
        return;
    }
    let st = &mut app.st.editor_viewport;
    st.raf_resize = Some(((r.w / 2.0 - st.pan_x) / st.zoom, (r.h / 2.0 - st.pan_y) / st.zoom));
    app.sched.frame(host.out(), Mod::EditorViewport);
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(app: &mut App, host: &mut dyn Host) {
    if app.cfg.has(crate::abi::mode::HEADLESS) {
        return;
    }
    let out = host.out();
    for (event, id) in [("pointerdown", L_DOWN), ("pointermove", L_MOVE), ("pointerup", L_UP), ("pointercancel", L_CANCEL), ("pointerleave", L_LEAVE)] {
        out.listen(Ui::CANVAS_VIEWPORT, event, id, 0);
    }
    out.listen(Ui::CANVAS_VIEWPORT, "wheel", L_WHEEL, listen::ACTIVE);
    // v5: app-v5.js:1842 — the indicator is the container's last child.
    let ind = out.create("div");
    out.prop(ind, "id", crate::Prop::Str("add-page-indicator"));
    out.text(ind, "+");
    out.insert(Ui::CANVAS_CONTAINER, Node::NONE, ind);
    app.st.editor_viewport.indicator = ind;
}

/// The indicator node (the container's last child: page rows are inserted before it).
pub fn indicator(app: &App) -> Node { app.st.editor_viewport.indicator }

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Pointer(p) => match p.listener {
            L_DOWN => down(app, host, &p),
            L_MOVE => move_(app, host, &p),
            L_UP | L_CANCEL | L_LEAVE => up(app, host, &p),
            _ => 0,
        },
        Event::Dom(e) if e.listener == L_WHEEL => wheel(app, host, &e),
        Event::Frame(t) => {
            frame(app, host, t);
            0
        }
        Event::Timer(T_WHEEL) => {
            finalize_create(app, host);
            0
        }
        Event::Timer(T_OVERSCROLL) => {
            if app.st.editor_viewport.overscroll_y > 0.0 {
                let min_y = min_pan_y(app, host);
                let st = &mut app.st.editor_viewport;
                st.overscroll_y = 0.0;
                st.pan_y = st.pan_y.max(min_y);
                st.pending_create = false;
                update_indicator(app, host);
                update_transform(app, host);
            }
            0
        }
        Event::Timer(T_SAVE) => {
            save_view(app, host);
            0
        }
        Event::Timer(T_GESTURE) => {
            gesture_settle(app, host);
            0
        }
        Event::Timer(T_TRANSITION) => {
            host.out().style(Ui::CANVAS_CONTAINER, "transition", Some(""));
            0
        }
        Event::Timer(T_RESYNC) => {
            app.st.editor_viewport.resync_armed = false;
            if interacting(app, host.now()) {
                // Writing does not change zoom; the resync follows the gesture or the pen lift.
                schedule_resync(app, host, false, false);
            } else {
                let force = std::mem::take(&mut app.st.editor_viewport.resync_force);
                resync_now(app, host, force);
            }
            0
        }
        Event::Life(l) if l.code == life::RESIZE => {
            resized(app, host);
            0
        }
        Event::Life(l) if l.code == life::BLUR => {
            release_gesture(app, host);
            0
        }
        _ => 0,
    }
}

/// `__IHN_TEST_API__.cmd('editor.viewport.<name>')`: `state` → zoom, pan, overscroll; `reads` → layout reads.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    let st = &app.st.editor_viewport;
    match name {
        "state" => Some(format!(
            "{{\"zoom\":{},\"panX\":{},\"panY\":{},\"overscrollY\":{},\"pendingCreate\":{},\"panning\":{},\"inertia\":{},\"reads\":{}}}",
            num(st.zoom),
            num(st.pan_x),
            num(st.pan_y),
            num(st.overscroll_y),
            st.pending_create,
            st.is_panning,
            st.inertia,
            st.reads
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{kind, mode, Config};
    use crate::doc::session;
    use crate::host::FakeHost;
    use ihn_core::codec::Writer;
    use ihn_core::op::{Background, Template};

    fn boot() -> (App, FakeHost) {
        let cfg = Config { version: "6.0.0".into(), url: "http://127.0.0.1/?e2e=1".into(), user_agent: "test".into(), language: "en".into(), flags: mode::E2E | mode::ONLINE, viewport: (1280.0, 800.0), dpr: 1.0, max_touch_points: 0 };
        let (mut app, mut host) = (App::new(cfg), FakeHost::default());
        app.init(&mut host);
        host.rects.insert(Ui::CANVAS_VIEWPORT, Rect { x: 0.0, y: 69.0, w: 1280.0, h: 731.0 });
        session::load(&mut app, &mut host, "", "d", 0);
        timer(&mut app, &mut host, tag(Mod::DocSession, 1));
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
        { let t = host.now; app.frame(&mut host, t); }
        host.take();
        (app, host)
    }

    fn timer(app: &mut App, host: &mut FakeHost, id: u32) {
        let mut w = Writer::default();
        w.u32(id);
        app.event(host, kind::TIMER, &w.buf);
    }

    fn wheel(app: &mut App, host: &mut FakeHost, dy: f64, ctrl: bool) {
        let mut w = Writer::default();
        w.u32(L_WHEEL);
        w.u32(Ui::CANVAS_VIEWPORT.0);
        w.u8(ety::WHEEL);
        w.u32(if ctrl { crate::abi::ev::CTRL } else { 0 });
        w.f64(host.now);
        w.i32(0);
        w.u32(0);
        w.f64(640.0);
        w.f64(400.0);
        w.f64(0.0);
        w.f64(dy);
        w.u32(0);
        w.i32(0);
        w.u8(0);
        w.u32(0);
        for _ in 0..3 {
            w.str("");
        }
        for _ in 0..4 {
            w.u32(0);
        }
        assert_eq!(app.event(host, kind::DOM, &w.buf), ret::PREVENT_DEFAULT);
    }

    fn touch(app: &mut App, host: &mut FakeHost, listener: u32, k: u8, id: i32, x: f64, y: f64) {
        let mut w = Writer::default();
        w.u32(listener);
        w.u32(Ui::CANVAS_VIEWPORT.0);
        w.u8(k);
        w.u8(ptr::TOUCH);
        w.i32(id);
        w.i32(0);
        w.u32(0);
        w.f64(x);
        w.f64(y);
        w.f64(host.now);
        w.f32(0.5);
        w.f32(0.0);
        w.f32(0.0);
        w.u32(1);
        w.u32(0);
        w.u32(0);
        app.event(host, kind::POINTER, &w.buf);
    }

    use crate::abi::ety;

    /// spec/editor.md §10.4: a wheel or pinch gesture reads layout once and resizes no canvas; the resync follows
    /// 90 ms after the gesture settled.
    #[test]
    fn gesture_budget_no_resizes_and_at_most_three_layout_reads() {
        let (mut app, mut host) = boot();
        let active = |app: &App| app.st.editor_pages.rows.iter().filter(|r| r.active.is_some()).count();
        assert_eq!(active(&app), 1, "page 1 fills the viewport at 100 %");
        assert_eq!((app.st.editor_viewport.zoom, app.st.editor_viewport.pan_x, app.st.editor_viewport.pan_y), (1.0, 213.0, 20.0));
        let (reads, resizes) = (app.st.editor_viewport.reads, app.st.editor_pages.resizes);
        for _ in 0..20 {
            host.advance(16.0);
            wheel(&mut app, &mut host, -100.0, true);
            { let t = host.now; app.frame(&mut host, t); }
        }
        assert!((app.st.editor_viewport.zoom - jsmath::pow(1.06, 20.0).min(3.0)).abs() < 1e-9);
        assert_eq!(app.st.editor_pages.resizes, resizes, "no canvas resize mid-gesture");
        assert!(app.st.editor_viewport.reads - reads <= 3, "{} layout reads", app.st.editor_viewport.reads - reads);
        // Pinch with two fingers, then one lifts and the other pans.
        let reads = app.st.editor_viewport.reads;
        touch(&mut app, &mut host, L_DOWN, ptr::DOWN, 1, 500.0, 400.0);
        touch(&mut app, &mut host, L_DOWN, ptr::DOWN, 2, 700.0, 400.0);
        for i in 1..=20 {
            host.advance(16.0);
            touch(&mut app, &mut host, L_MOVE, ptr::MOVE, 1, 500.0 + i as f64 * 3.0, 400.0);
            touch(&mut app, &mut host, L_MOVE, ptr::MOVE, 2, 700.0 - i as f64 * 3.0, 400.0);
            { let t = host.now; app.frame(&mut host, t); }
        }
        assert!(app.st.editor_viewport.zoom < 3.0);
        assert_eq!(app.st.editor_pages.resizes, resizes);
        assert!(app.st.editor_viewport.reads - reads <= 3, "{} layout reads", app.st.editor_viewport.reads - reads);
        touch(&mut app, &mut host, L_UP, ptr::UP, 2, 640.0, 400.0);
        touch(&mut app, &mut host, L_UP, ptr::UP, 1, 560.0, 400.0);
        assert!(app.st.editor_viewport.pointers.is_empty() && !app.st.editor_viewport.is_panning);
        // Settled: the idle deadline passes, then the 90 ms resync re-rasters the active pages.
        host.advance(300.0);
        timer(&mut app, &mut host, T_GESTURE);
        host.advance(100.0);
        timer(&mut app, &mut host, T_RESYNC);
        assert!(app.st.editor_pages.resizes > resizes, "the resync follows the gesture");
    }

    #[test]
    fn clamp_keeps_five_pixels_of_content_and_pull_to_add_needs_the_threshold() {
        let (mut app, mut host) = boot();
        // Far up: clamped so that 5 px of the last page stay visible.
        app.st.editor_viewport.pan_y = -100000.0;
        update_transform(&mut app, &mut host);
        let bottom = layout::metrics(&app).bounds[3];
        assert_eq!(app.st.editor_viewport.pan_y, 5.0 - bottom);
        // Read mode: no overscroll.
        app.st.editor_viewport.pan_y = -2000.0;
        vertical_pan(&mut app, &mut host, -500.0);
        assert_eq!(app.st.editor_viewport.overscroll_y, 0.0);
        // Edit mode: wheel down past the end accumulates overscroll and arms the creation.
        view::set_read_only(&mut app, &mut host, false, false);
        center_on_top(&mut app, &mut host);
        let mut n = 0;
        while !app.st.editor_viewport.pending_create && n < 200 {
            host.advance(16.0);
            wheel(&mut app, &mut host, 120.0, false);
            n += 1;
        }
        assert!(app.st.editor_viewport.pending_create && app.st.editor_viewport.overscroll_y >= OVERSCROLL_THRESHOLD);
        host.take();
        timer(&mut app, &mut host, T_WHEEL);
        assert_eq!(app.st.editor_pages.rows.len(), 4);
        assert_eq!(app.st.editor_viewport.overscroll_y, 0.0);
        assert!(host.take().contains("\"transition\" \"transform 0.35s ease\""));
    }
}
