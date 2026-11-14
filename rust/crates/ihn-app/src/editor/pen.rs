//! Editor: Pen and highlighter tools.
//!
//! A contact appends samples to pre-reserved buffers (no allocation per pointer event); once per animation frame
//! the finished spline segments go to the page bitmap and the newest one to the cursor overlay (5.18.0
//! `updateLiveStroke`; the highlighter draws on the cursor overlay only). At lift the rest is committed to the
//! bitmap, a highlighter is simplified, and the stroke becomes one batch (`session::edit`) and one undo step.
//! While a live sink is attached (`collab`, W4.5) the contact is also described as `LivePreview` packets.

use super::input::{self, Tool as InputTool, ToolHandler};
use super::{history, pages};
use crate::doc::session;
use crate::{App, Event, Host, Mod, PointerBatch};
use ihn_core::doc::LivePreview;
use ihn_core::geom::{js_max, js_min};
use ihn_core::id::Id;
use ihn_core::jsmath;
use ihn_core::op::{Body, Color, Tool};
use ihn_core::render::live::{self, Bounds, Cursor, Live, PageTarget};
use ihn_core::render::StrokeRef;

/// Samples reserved per stroke (a 4-second contact at 240 Hz); longer strokes grow the buffers once.
const RESERVE: usize = 1024;

pub struct State {
    active: bool,
    page: Id,
    tool: Tool,
    color: Color,
    width: f64,
    /// `x0, y0, x1, y1, …` and the pressure of each sample (v5 `points[i].p`).
    xy: Vec<f64>,
    p: Vec<f32>,
    min_d2: f64,
    live: Live,
    bounds: Option<Bounds>,
    frame_armed: bool,
    /// Live preview packets for remote viewers (only while a sink is attached).
    sink: bool,
    out: Vec<LivePreview>,
    live_id: u64,
    live_seq: u64,
    sent: usize,
    /// Frames painted and strokes committed (test counters).
    pub frames: u32,
    pub strokes: u32,
}

impl Default for State {
    fn default() -> Self {
        State {
            active: false,
            page: Id(0),
            tool: Tool::Pen,
            color: Color::Hex(0),
            width: 2.0,
            xy: Vec::with_capacity(2 * RESERVE),
            p: Vec::with_capacity(RESERVE),
            min_d2: 0.0,
            live: Live::default(),
            bounds: None,
            frame_armed: false,
            sink: false,
            out: Vec::new(),
            live_id: 0,
            live_seq: 0,
            sent: 0,
            frames: 0,
            strokes: 0,
        }
    }
}

/// Attaches (or detaches) the consumer of [`drain_live`]; without one no packet is built.
pub fn set_live_sink(app: &mut App, on: bool) {
    let st = &mut app.st.editor_pen;
    st.sink = on;
    if !on {
        st.out.clear();
    }
}

/// The live preview packets produced since the last call (`SyncEngine::live`).
pub fn drain_live(app: &mut App) -> Vec<LivePreview> { std::mem::take(&mut app.st.editor_pen.out) }

/// The pen and highlighter [`ToolHandler`].
pub struct Pen;

/// The page canvas and cursor overlay of `page`: (target, cursor surface, scale, backing size).
fn surfaces(app: &App, page: Id) -> Option<(PageTarget, Cursor)> {
    let row = app.st.editor_pages.rows.iter().find(|r| r.id == page)?;
    let a = row.active.as_ref()?;
    let target = PageTarget { surf: a.surf, sx: a.px.0 as f64 / row.w, sy: a.px.1 as f64 / row.h };
    Some((target, Cursor { surf: a.cursor_surf, scale: a.cursor_scale, px: a.cursor_px, bounds: app.st.editor_pen.bounds }))
}

/// The contact as a stroke (live drawing reads no bounds and caches no path).
fn stroke<'a>(tool: Tool, color: &'a Color, width: f64, xy: &'a [f64]) -> StrokeRef<'a> { StrokeRef { tool, color, width, xy, bounds: [0.0; 4], key: None } }

/// v5: `updateLiveStroke(pageIndex, canvas, stroke, previewCanvas, metrics)`, then `publishLiveStrokePreview`.
fn paint(app: &mut App, host: &mut dyn Host) {
    let page = app.st.editor_pen.page;
    if let Some((target, mut cursor)) = surfaces(app, page) {
        let ink = pages::with_gfx(app, |app, g| {
            let st = &mut app.st.editor_pen;
            let s = stroke(st.tool, &st.color, st.width, &st.xy);
            let ink = live::update(&mut g.cmds, &mut st.live, target, &mut cursor, &s);
            st.bounds = cursor.bounds;
            ink
        });
        if let Some(ink) = ink {
            pages::overlay_ink(app, host, page, 1, ink);
        }
        pages::flush(app, host);
    }
    let st = &mut app.st.editor_pen;
    st.frames += 1;
    let n = st.p.len();
    if st.sink && st.sent < n {
        st.out.push(LivePreview::Points { live: st.live_id, from: st.sent as u32, xy: st.xy[2 * st.sent..].to_vec(), p: Some(st.p[st.sent..].to_vec()) });
        st.sent = n;
    }
}

/// v5: `clearEraserPreview(pageIndex)` (how the live tip is removed at lift).
fn clear_cursor(app: &mut App, host: &mut dyn Host, page: Id) {
    if let Some((_, mut cursor)) = surfaces(app, page) {
        pages::with_gfx(app, |_, g| live::clear_cursor(&mut g.cmds, &mut cursor));
        pages::flush(app, host);
        pages::overlay_ink(app, host, page, 1, false);
    }
    app.st.editor_pen.bounds = None;
}

impl ToolHandler for Pen {
    fn down(&self, app: &mut App, host: &mut dyn Host, x: f64, y: f64, pressure: f64) {
        let (tool, color, width) = {
            let i = &app.st.editor_input;
            (if i.tool == InputTool::Highlighter { Tool::Highlighter } else { Tool::Pen }, i.color, i.width)
        };
        let Some(page) = input::contact(app).map(|c| c.page) else { return };
        let time = host.time();
        let st = &mut app.st.editor_pen;
        st.active = true;
        st.page = page;
        st.tool = tool;
        st.color = Color::parse(color);
        st.width = width;
        st.xy.clear();
        st.p.clear();
        st.xy.extend_from_slice(&[x, y]);
        st.p.push(pressure as f32);
        // v5: appendPenSamples' minimum distance (pen 0.001 px; highlighter max(0.6, 0.12·width)).
        let min = if tool == Tool::Highlighter { js_max(0.6, width * 0.12) } else { 0.001 };
        st.min_d2 = min * min;
        st.live = Live::default();
        st.bounds = None;
        st.sent = 0;
        st.live_seq += 1;
        st.live_id = (time as u64) << 16 | (st.live_seq & 0xffff);
        if st.sink {
            st.out.push(LivePreview::Begin { live: st.live_id, page, tool, color: st.color.clone(), width, has_p: true });
        }
        paint(app, host);
    }

    // v5: appendPenSamples(e) + schedulePenPaint().
    fn moved(&self, app: &mut App, host: &mut dyn Host, p: &PointerBatch) {
        let Some(c) = input::contact(app).copied() else { return };
        let st = &mut app.st.editor_pen;
        if !st.active {
            return;
        }
        let before = st.p.len();
        let mut last_t = c.last_t;
        for s in p.samples() {
            if s.t < last_t {
                continue;
            }
            let (x, y) = c.page_xy(&s);
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            last_t = js_max(last_t, s.t);
            let n = st.xy.len();
            let (dx, dy) = (x - st.xy[n - 2], y - st.xy[n - 1]);
            if dx * dx + dy * dy < st.min_d2 {
                continue;
            }
            let last_p = st.p[st.p.len() - 1];
            st.xy.push(x);
            st.xy.push(y);
            st.p.push(if s.pressure != 0.0 && !s.pressure.is_nan() { s.pressure } else if last_p != 0.0 && !last_p.is_nan() { last_p } else { 0.5 });
        }
        let appended = st.p.len() != before;
        if appended && !std::mem::replace(&mut st.frame_armed, true) {
            app.sched.frame(host.out(), Mod::EditorPen);
        }
        if let Some(c) = input::contact_mut(app) {
            c.last_t = last_t;
        }
    }

    // v5: endDrawing(e) for the pen and the highlighter.
    fn end(&self, app: &mut App, host: &mut dyn Host) {
        let st = &mut app.st.editor_pen;
        if !std::mem::replace(&mut st.active, false) {
            return;
        }
        let page = st.page;
        let live_id = st.live_id;
        if !session::get(app).is_some_and(|s| s.doc.order().contains(&page)) {
            // The page was deleted while drawing: drop the stroke.
            if app.st.editor_pen.sink {
                app.st.editor_pen.out.push(LivePreview::Cancel { live: live_id });
            }
            clear_cursor(app, host, page);
            return;
        }
        if let Some((target, _)) = surfaces(app, page) {
            pages::with_gfx(app, |app, g| {
                let st = &mut app.st.editor_pen;
                let s = stroke(st.tool, &st.color, st.width, &st.xy);
                live::commit(&mut g.cmds, &mut g.paths, &mut st.live, target, &s);
            });
            pages::flush(app, host);
        }
        let st = &app.st.editor_pen;
        let (xy, p) = match (st.tool == Tool::Highlighter).then(|| simplify(&st.xy, &st.p)).flatten() {
            Some(kept) => kept,
            None => (st.xy.clone(), st.p.clone()),
        };
        let points = p.len();
        let body = Body::Stroke { tool: st.tool, color: st.color.clone(), width: st.width, xy, p: Some(p) };
        let mut item = Id(0);
        if let Some(e) = session::edit(app, host, |tx| item = tx.add_stroke(page, body)) {
            app.st.editor_pen.strokes += 1;
            committed(app, page);
            history::record(app, host, page, e.inverse, points);
        }
        if app.st.editor_pen.sink {
            app.st.editor_pen.out.push(LivePreview::End { live: live_id, item });
        }
        clear_cursor(app, host, page);
    }

    fn discard(&self, app: &mut App, host: &mut dyn Host) {
        let st = &mut app.st.editor_pen;
        if !std::mem::replace(&mut st.active, false) {
            return;
        }
        let (page, live_id) = (st.page, st.live_id);
        if st.sink {
            st.out.push(LivePreview::Cancel { live: live_id });
        }
        clear_cursor(app, host, page);
    }
}

/// A local stroke landed on `page`: previews are stale, but the bitmap already shows it (no repaint).
fn committed(app: &mut App, page: Id) {
    pages::touch(app, page);
    if let Some(r) = app.st.editor_pages.rows.iter_mut().find(|r| r.id == page) {
        if r.active.as_ref().is_some_and(|a| a.drawn) {
            r.needs_redraw = false;
        }
    }
}

/// v5: `simplifyPoints(points, epsilon)` over `idx` (indices of points in `xy`): the kept indices, or `None` when
/// the work budget overflows (all are kept).
fn douglas_peucker(xy: &[f64], idx: &[usize], epsilon: f64) -> Option<Vec<usize>> {
    let m = idx.len();
    if m < 3 {
        return None;
    }
    let pt = |k: usize| (xy[2 * idx[k]], xy[2 * idx[k] + 1]);
    let mut keep = vec![false; m];
    keep[0] = true;
    keep[m - 1] = true;
    let mut stack = vec![0, m - 1];
    let mut work = 0usize;
    let budget = (m * 40).min(250_000);
    while let (Some(end), Some(start)) = (stack.pop(), stack.pop()) {
        let ((ax, ay), (bx, by)) = (pt(start), pt(end));
        let (dx, dy) = (bx - ax, by - ay);
        let length = dx * dx + dy * dy;
        let mut maximum = epsilon * epsilon;
        let mut index = None;
        for i in start + 1..end {
            work += 1;
            if work > budget {
                return None;
            }
            let (px, py) = pt(i);
            let t = if length != 0.0 && !length.is_nan() { js_max(0.0, js_min(1.0, ((px - ax) * dx + (py - ay) * dy) / length)) } else { 0.0 };
            let error = jsmath::pow(px - ax - t * dx, 2.0) + jsmath::pow(py - ay - t * dy, 2.0);
            if error > maximum {
                maximum = error;
                index = Some(i);
            }
        }
        if let Some(i) = index {
            keep[i] = true;
            stack.extend_from_slice(&[start, i, i, end]);
        }
    }
    Some((0..m).filter(|&k| keep[k]).map(|k| idx[k]).collect())
}

/// v5: `simplifyStrokePoints(stroke)` for a highlighter (pens keep every sample): `None` = unchanged.
pub fn simplify(xy: &[f64], p: &[f32]) -> Option<(Vec<f64>, Vec<f32>)> {
    let n = xy.len() / 2;
    if n < 6 {
        return None;
    }
    let mut filtered = vec![0usize];
    for i in 1..n - 1 {
        let l = filtered[filtered.len() - 1];
        if jsmath::hypot(xy[2 * i] - xy[2 * l], xy[2 * i + 1] - xy[2 * l + 1]) >= 0.4 {
            filtered.push(i);
        }
    }
    filtered.push(n - 1);
    let min_keep = js_max(3.0, (filtered.len() as f64 * 0.5).ceil()) as usize;
    let kept = match douglas_peucker(xy, &filtered, 0.45 * 1.3) {
        Some(s) if s.len() >= min_keep => s,
        _ => filtered,
    };
    Some((kept.iter().flat_map(|&i| [xy[2 * i], xy[2 * i + 1]]).collect(), kept.iter().map(|&i| p[i]).collect()))
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(_app: &mut App, _host: &mut dyn Host) {}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    if let Event::Frame(_) = event {
        app.st.editor_pen.frame_armed = false;
        // v5: the frame is cancelled at lift (`cancelAnimationFrame(paintFrame)`).
        if app.st.editor_pen.active && input::contact(app).is_some() {
            paint(app, host);
        }
    }
    0
}

/// `__IHN_TEST_API__.cmd('editor.pen.<name>')`: `state` → points of the contact, frames painted, strokes
/// committed, whether a contact is live.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    let st = &app.st.editor_pen;
    match name {
        "state" => Some(format!("{{\"active\":{},\"points\":{},\"frames\":{},\"strokes\":{}}}", st.active, st.p.len(), st.frames, st.strokes)),
        _ => None,
    }
}
