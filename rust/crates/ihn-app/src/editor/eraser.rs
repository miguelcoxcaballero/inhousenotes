//! Editor: Stroke and area erasers (5.18.0 `setupCanvasEvents` eraser branches, `eraseStrokeAtPoint`,
//! `eraseAreaAtPoint`, `drawEraserPreview`, `scheduleEraserRedraw` / `repaintEraserDamage`).
//!
//! A gesture never edits the model until the lift: pointer events only append eraser positions to a buffer
//! reserved at boot; once per animation frame the positions are tested (the segment swept between consecutive
//! positions, B-E6), the erased strokes become a pending *cut* (hidden items, or the surviving point ranges of an
//! area-erased stroke), the damaged rectangle is repainted from the model minus the cut, and the cursor is drawn.
//! At lift the cut becomes one batch (`Tx::erase` / `Tx::split_ranges`, fragments at the original z: B-E4) and one
//! undo step. While a live sink is attached (`collab`, W4.5) the cut is also published as `LivePreview::Erase`
//! (at most every 50 ms) and `EraseEnd` before the batch.

use super::input::{self, Tool as InputTool, ToolHandler};
use super::{history, pages};
use crate::abi::ptr;
use crate::doc::session;
use crate::{App, Event, Host, Mod, PointerBatch};
use ihn_core::doc::{Doc, LivePreview};
use ihn_core::geom::{self, js_max, Sweep};
use ihn_core::id::Id;
use ihn_core::jsmath;
use ihn_core::op::{Body, Tool};
use ihn_core::render::damage::{self, Damage};
use ihn_core::render::live::{self, Bounds, Cursor};
use ihn_core::render::overlay;
use ihn_core::render::page::PageView;
use ihn_core::render::{PageItems, StrokeRef};
use ihn_core::stamp::Stamp;

/// v5: `ERASER_MIN_STEP_FACTOR`.
const MIN_STEP_FACTOR: f64 = 0.18;
/// SYNC §10.6: live erase packets at most every 50 ms.
const LIVE_MS: f64 = 50.0;
/// Positions reserved per frame (coalesced samples of a fast swipe); more grow the buffer once.
const RESERVE: usize = 256;

/// One erased stroke of the gesture: the body version it was cut from and, for the area eraser, the surviving
/// half-open point ranges (empty = the whole stroke goes; the stroke eraser always removes whole strokes).
struct Cut {
    item: Id,
    at: Stamp,
    ranges: Vec<(u32, u32)>,
}

pub struct State {
    active: bool,
    area: bool,
    page: Id,
    /// `state.eraserWidth` at pointerdown (radius = width / 2).
    width: f64,
    /// Positions not tested yet: `x0, y0, x1, y1, …`.
    pending: Vec<f64>,
    /// v5 `lastEraserPoint` (spacing filter) and the start of the next swept segment.
    kept: (f64, f64),
    swept: (f64, f64),
    /// Sorted by item id.
    cuts: Vec<Cut>,
    damage: Damage,
    cursor: Option<Bounds>,
    cursor_at: Option<(f64, f64)>,
    frame_armed: bool,
    /// Live erase packets (only while a sink is attached).
    sink: bool,
    out: Vec<LivePreview>,
    gesture: u64,
    seq: u64,
    unsent: bool,
    sent_at: f64,
    /// Frames painted, damage repaints and gestures committed (test counters).
    pub frames: u32,
    pub repaints: u32,
    pub commits: u32,
}

impl Default for State {
    fn default() -> Self {
        State {
            active: false,
            area: false,
            page: Id(0),
            width: 12.0,
            pending: Vec::with_capacity(2 * RESERVE),
            kept: (0.0, 0.0),
            swept: (0.0, 0.0),
            cuts: Vec::new(),
            damage: Damage::default(),
            cursor: None,
            cursor_at: None,
            frame_armed: false,
            sink: false,
            out: Vec::new(),
            gesture: 0,
            seq: 0,
            unsent: false,
            sent_at: f64::NEG_INFINITY,
            frames: 0,
            repaints: 0,
            commits: 0,
        }
    }
}

/// Attaches (or detaches) the consumer of [`drain_live`]; without one no packet is built.
pub fn set_live_sink(app: &mut App, on: bool) {
    let st = &mut app.st.editor_eraser;
    st.sink = on;
    if !on {
        st.out.clear();
    }
}

/// The live erase packets produced since the last call (`SyncEngine::live`).
pub fn drain_live(app: &mut App) -> Vec<LivePreview> { std::mem::take(&mut app.st.editor_eraser.out) }

/// The eraser [`ToolHandler`] (`eraser-stroke` and `eraser-area`).
pub struct Eraser;

/// A stroke the erasers may touch (legacy eraser strokes never are).
fn stroke_xy(doc: &Doc, id: Id) -> Option<(&[f64], Stamp)> {
    let item = doc.live(id)?;
    match &item.body.v {
        Body::Stroke { tool, xy, .. } if *tool != Tool::LegacyEraseArea => Some((xy, item.body.at)),
        _ => None,
    }
}

/// v5: `eraseStrokeAtPoint` / `eraseAreaAtPoint` for the segment `sweep` (B-E6): the stroke eraser adds the strokes
/// it touches to `cuts`, the area eraser splits them (or their fragments of this gesture). `true` when `cuts` changed.
fn apply(doc: &Doc, page: Id, area: bool, sweep: &Sweep, r: f64, cuts: &mut Vec<Cut>, damage: &mut Damage) -> bool {
    let mut changed = false;
    let mut tmp = Vec::new();
    for d in doc.draw_list(page).strokes {
        if !sweep.may_touch(&d.bounds, geom::eraser_pad(r, d.width)) {
            continue;
        }
        let Some((xy, at)) = stroke_xy(doc, d.id) else { continue };
        let found = cuts.binary_search_by_key(&d.id, |c| c.item);
        if !area {
            if let Err(i) = found {
                if geom::polyline_hit(xy, sweep, r) {
                    cuts.insert(i, Cut { item: d.id, at, ranges: Vec::new() });
                    damage.add(d.bounds, d.width);
                    changed = true;
                }
            }
            continue;
        }
        match found {
            Ok(i) => {
                // A fragment of this gesture is split again: its runs are ranges of the original (unless another
                // device replaced the stroke meanwhile; the commit then leaves it alone).
                let cut = &mut cuts[i];
                if cut.at != at {
                    continue;
                }
                let mut next = Vec::with_capacity(cut.ranges.len() + 1);
                let mut any = false;
                for &(a, b) in &cut.ranges {
                    let sub = &xy[2 * a as usize..2 * b as usize];
                    tmp.clear();
                    if geom::split_ranges(sub, sweep, r, &mut tmp) {
                        any = true;
                        damage.add(geom::bounds(sub), d.width);
                        next.extend(tmp.iter().map(|&(s, e)| (a + s, a + e)));
                    } else {
                        next.push((a, b));
                    }
                }
                if any {
                    cut.ranges = next;
                    changed = true;
                }
            }
            Err(i) => {
                tmp.clear();
                if xy.len() >= 4 && geom::split_ranges(xy, sweep, r, &mut tmp) {
                    cuts.insert(i, Cut { item: d.id, at, ranges: tmp.clone() });
                    damage.add(d.bounds, d.width);
                    changed = true;
                }
            }
        }
    }
    changed
}

/// Tests the buffered positions, each as the segment swept from the previous one; `true` when the cut changed.
fn drain_positions(app: &mut App) -> bool {
    let st = &mut app.st.editor_eraser;
    if st.pending.is_empty() {
        return false;
    }
    let (pending, mut cuts, mut damage) = (std::mem::take(&mut st.pending), std::mem::take(&mut st.cuts), st.damage);
    let (page, area, r, mut from) = (st.page, st.area, st.width / 2.0, st.swept);
    let mut changed = false;
    if let Some(s) = session::get(app) {
        for p in pending.chunks_exact(2) {
            let sweep = Sweep { x0: from.0, y0: from.1, x1: p[0], y1: p[1] };
            from = (p[0], p[1]);
            changed |= apply(&s.doc, page, area, &sweep, r, &mut cuts, &mut damage);
        }
    }
    let st = &mut app.st.editor_eraser;
    st.cursor_at = Some(from);
    (st.swept, st.cuts, st.damage, st.pending) = (from, cuts, damage, pending);
    st.pending.clear();
    st.unsent |= changed && st.sink;
    changed
}

/// The page's items with the gesture's cut applied (erased strokes gone, area-erased ones as their fragments).
fn cut_items<'a>(items: &PageItems<'a>, cuts: &[Cut]) -> PageItems<'a> {
    let mut strokes = Vec::with_capacity(items.strokes.len());
    for s in &items.strokes {
        match s.key.and_then(|(id, at)| cuts.binary_search_by_key(&id, |c| c.item).ok().filter(|&i| cuts[i].at == at)) {
            None => strokes.push(*s),
            Some(i) => {
                for &(a, b) in &cuts[i].ranges {
                    let xy = &s.xy[2 * a as usize..2 * b as usize];
                    strokes.push(StrokeRef { xy, bounds: geom::bounds(xy), key: None, ..*s });
                }
            }
        }
    }
    PageItems { covers: items.covers.clone(), images: items.images.clone(), strokes }
}

/// v5: `repaintEraserDamage(pageIndex, damage)` — even while the pen is down; a canvas without a complete paint
/// is repainted whole (v5: `redrawPage(…, {duringErase: true})`).
fn repaint_damage(app: &mut App) {
    let page = app.st.editor_eraser.page;
    let Some(d) = app.st.editor_eraser.damage.take() else { return };
    let Some((surf, scale, drawn)) = pages::active(app, page).map(|a| (a.surf, a.scale, a.drawn)) else { return };
    let cuts = std::mem::take(&mut app.st.editor_eraser.cuts);
    pages::with_view(app, page, |g, view| {
        let items = cut_items(view.items, &cuts);
        let rect = if drawn { d } else { [0.0, 0.0, view.w, view.h] };
        damage::repaint(&mut g.cmds, &mut g.tpl, &mut g.paths, surf, &PageView { w: view.w, h: view.h, bg: view.bg, items: &items }, scale, rect);
    });
    app.st.editor_eraser.cuts = cuts;
    app.st.editor_eraser.repaints += 1;
}

/// v5: `drawEraserPreview(pageIndex, lastPos)`.
fn draw_cursor(app: &mut App, host: &mut dyn Host) {
    let st = &app.st.editor_eraser;
    let (page, width) = (st.page, st.width);
    let Some((x, y)) = st.cursor_at else { return };
    let Some(a) = pages::active(app, page) else { return };
    let mut cursor = Cursor { surf: a.cursor_surf, scale: a.cursor_scale, px: a.cursor_px, bounds: st.cursor };
    pages::with_gfx(app, |_, g| overlay::eraser_cursor(&mut g.cmds, &mut cursor, x, y, width));
    app.st.editor_eraser.cursor = cursor.bounds;
    app.st.editor_eraser.cursor_at = None;
    pages::overlay_ink(app, host, page, 1, true);
}

/// v5: `clearEraserPreview(pageIndex)`.
fn clear_cursor(app: &mut App, host: &mut dyn Host) {
    let page = app.st.editor_eraser.page;
    if let Some(a) = pages::active(app, page) {
        let mut cursor = Cursor { surf: a.cursor_surf, scale: a.cursor_scale, px: a.cursor_px, bounds: app.st.editor_eraser.cursor };
        pages::with_gfx(app, |_, g| live::clear_cursor(&mut g.cmds, &mut cursor));
        pages::flush(app, host);
        pages::overlay_ink(app, host, page, 1, false);
    }
    app.st.editor_eraser.cursor = None;
    app.st.editor_eraser.cursor_at = None;
}

/// v5: `publishLiveErasePreview(pageId, gestureId, changeSet)` — the complete hidden set and fragments.
fn publish(app: &mut App, host: &mut dyn Host) {
    let now = host.now();
    let st = &app.st.editor_eraser;
    if !st.sink || !st.unsent || now - st.sent_at < LIVE_MS {
        return;
    }
    let Some(doc) = session::get(app).map(|s| &s.doc) else { return };
    let mut hidden = Vec::with_capacity(st.cuts.len());
    let mut fragments = Vec::new();
    for (ci, c) in st.cuts.iter().enumerate() {
        hidden.push(c.item);
        let Some(Body::Stroke { tool, color, width, xy, p }) = doc.live(c.item).filter(|i| i.body.at == c.at).map(|i| &i.body.v) else { continue };
        for (k, &(a, b)) in c.ranges.iter().enumerate() {
            let (a, b) = (a as usize, b as usize);
            let body = Body::Stroke { tool: *tool, color: color.clone(), width: *width, xy: xy[2 * a..2 * b].to_vec(), p: p.as_ref().and_then(|p| p.get(a..b)).map(<[f32]>::to_vec) };
            fragments.push(((ci as u64) << 32 | k as u64, body));
        }
    }
    let st = &mut app.st.editor_eraser;
    st.out.push(LivePreview::Erase { gesture: st.gesture, page: st.page, hidden, fragments });
    st.unsent = false;
    st.sent_at = now;
    crate::collab::wake(app, host);
}

/// One frame of the gesture: test, repaint the damage, draw the cursor, publish.
fn paint(app: &mut App, host: &mut dyn Host) {
    drain_positions(app);
    repaint_damage(app);
    draw_cursor(app, host);
    pages::flush(app, host);
    publish(app, host);
    let st = &mut app.st.editor_eraser;
    st.frames += 1;
    // A change not yet published waits for the 50 ms window.
    if st.unsent && !std::mem::replace(&mut st.frame_armed, true) {
        app.sched.frame(host.out(), Mod::EditorEraser);
    }
}

/// Ends the gesture without committing: the bitmap is restored from the model.
fn abandon(app: &mut App, host: &mut dyn Host) {
    let st = &mut app.st.editor_eraser;
    st.pending.clear();
    st.damage = Damage::default();
    let had = !std::mem::take(&mut st.cuts).is_empty();
    if st.sink {
        st.out.push(LivePreview::EraseEnd { gesture: st.gesture });
    }
    clear_cursor(app, host);
    crate::collab::wake(app, host);
    let page = app.st.editor_eraser.page;
    if had {
        pages::touch(app, page);
        pages::request_repaint(app, host, page);
    }
}

impl ToolHandler for Eraser {
    // v5: the eraser branches of the canvas' pointerdown: erase at the down point, draw the cursor.
    fn down(&self, app: &mut App, host: &mut dyn Host, x: f64, y: f64, _pressure: f64) {
        let Some(c) = input::contact(app).copied() else { return };
        let width = app.st.editor_input.eraser_width;
        let time = host.time();
        let st = &mut app.st.editor_eraser;
        st.active = true;
        st.area = c.tool == InputTool::EraserArea;
        st.page = c.page;
        st.width = width;
        st.pending.clear();
        st.cuts.clear();
        st.damage = Damage::default();
        st.kept = (x, y);
        st.swept = (x, y);
        st.unsent = false;
        st.sent_at = f64::NEG_INFINITY;
        st.seq += 1;
        st.gesture = (time as u64) << 16 | (st.seq & 0xffff);
        st.pending.extend_from_slice(&[x, y]);
        paint(app, host);
    }

    // v5: `collectEraserPositions(samplesForEvent(e))`: a sample at least max(0.8, 0.18·width) from the last kept
    // one, and always the event's last.
    fn moved(&self, app: &mut App, host: &mut dyn Host, p: &PointerBatch) {
        if p.kind == ptr::UP {
            return; // v5: the release's samples are the pen's only
        }
        let Some(c) = input::contact(app).copied() else { return };
        let st = &mut app.st.editor_eraser;
        if !st.active {
            return;
        }
        let min = js_max(0.8, st.width * MIN_STEP_FACTOR);
        let before = st.pending.len();
        let mut samples = p.samples().peekable();
        while let Some(s) = samples.next() {
            let (x, y) = c.page_xy(&s);
            let last = samples.peek().is_none();
            if last || jsmath::hypot(x - st.kept.0, y - st.kept.1) >= min {
                st.pending.push(x);
                st.pending.push(y);
                st.kept = (x, y);
            }
        }
        if st.pending.len() != before && !std::mem::replace(&mut st.frame_armed, true) {
            app.sched.frame(host.out(), Mod::EditorEraser);
        }
    }

    // v5: endDrawing(e) for the erasers: history `erase-strokes` / `erase-area`, the final live packet.
    fn end(&self, app: &mut App, host: &mut dyn Host) {
        if !std::mem::replace(&mut app.st.editor_eraser.active, false) {
            return;
        }
        let page = app.st.editor_eraser.page;
        if !session::get(app).is_some_and(|s| s.doc.order().contains(&page)) {
            abandon(app, host);
            return;
        }
        drain_positions(app);
        repaint_damage(app);
        pages::flush(app, host);
        let cuts = std::mem::take(&mut app.st.editor_eraser.cuts);
        if app.st.editor_eraser.sink {
            let g = app.st.editor_eraser.gesture;
            app.st.editor_eraser.out.push(LivePreview::EraseEnd { gesture: g });
        }
        clear_cursor(app, host);
        crate::collab::wake(app, host);
        if cuts.is_empty() {
            return;
        }
        let area = app.st.editor_eraser.area;
        // A stroke another device changed during the gesture is left alone.
        let valid: Vec<&Cut> = match session::get(app) {
            Some(s) => cuts.iter().filter(|c| s.doc.live(c.item).is_some_and(|i| i.body.at == c.at)).collect(),
            None => Vec::new(),
        };
        let points = if area { valid.iter().flat_map(|c| c.ranges.iter()).map(|&(a, b)| (b - a) as usize).sum() } else { 0 };
        let edit = session::edit(app, host, |tx| {
            for c in &valid {
                if area {
                    tx.split_ranges(c.item, &c.ranges);
                } else {
                    tx.erase(&[c.item]);
                }
            }
        });
        pages::with_gfx(app, |_, g| cuts.iter().for_each(|c| g.paths.forget(&mut g.cmds, c.item)));
        pages::flush(app, host);
        pages::touch(app, page);
        if let Some(e) = edit {
            app.st.editor_eraser.commits += 1;
            history::record(app, host, page, e.inverse, points);
        }
        // v5 `repaintEraserDamage` ends with `page.needsRedraw = true` ("final gesture rendering still refreshes the
        // complete cache"): the damage repaints' clipped antialiasing is replaced by a full repaint.
        pages::request_repaint(app, host, page);
    }

    fn discard(&self, app: &mut App, host: &mut dyn Host) {
        if std::mem::replace(&mut app.st.editor_eraser.active, false) {
            abandon(app, host);
        }
    }
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(_app: &mut App, _host: &mut dyn Host) {}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    if let Event::Frame(_) = event {
        app.st.editor_eraser.frame_armed = false;
        if app.st.editor_eraser.active && input::contact(app).is_some() {
            paint(app, host);
        }
    }
    0
}

/// `__IHN_TEST_API__.cmd('editor.eraser.<name>')`: `state` → contact, erased items pending, frames, damage
/// repaints, gestures committed, mode; `mode` with `"area"` / `"stroke"` sets the eraser mode (what the eraser
/// menu's `#eraser-stroke-toggle` does, W4.4) and selects the eraser.
pub fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, args: &str) -> Option<String> {
    match name {
        "state" => {
            let st = &app.st.editor_eraser;
            Some(format!(
                "{{\"active\":{},\"cut\":{},\"frames\":{},\"repaints\":{},\"commits\":{},\"stroke\":{},\"width\":{}}}",
                st.active,
                st.cuts.len(),
                st.frames,
                st.repaints,
                st.commits,
                app.st.editor_input.eraser_stroke,
                ihn_core::jsfmt::num(app.st.editor_input.eraser_width)
            ))
        }
        "mode" => {
            app.st.editor_input.eraser_stroke = !args.contains("area");
            input::set_tool(app, host, None, None, None);
            Some("true".into())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{kind, mode, Config};
    use crate::editor::view;
    use crate::host::FakeHost;
    use crate::{shell, tag, Node, Rect, Ui};
    use ihn_core::codec::Writer;
    use ihn_core::op::{Background, Template};

    const L_DOWN: u32 = tag(Mod::EditorInput, 1);
    const L_MOVE: u32 = tag(Mod::EditorInput, 2);
    const L_UP: u32 = tag(Mod::EditorInput, 3);

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
            tx.page_new(None, Template::Default, (794, 1123), Background::Template);
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
        host.rects.insert(canvas, Rect { x: 0.0, y: 0.0, w: 794.0, h: 1123.0 });
        host.take();
        T { app, host, canvas, page }
    }

    impl T {
        /// A pointer event at page coordinates (the canvas is at the client origin, scale 1).
        fn send(&mut self, listener: u32, k: u8, samples: &[(f64, f64)], buttons: u32) {
            let mut w = Writer::default();
            let t = self.host.now;
            let sample = |w: &mut Writer, s: &(f64, f64)| {
                w.f64(s.0);
                w.f64(s.1);
                w.f64(t);
                w.f32(if buttons != 0 { 0.5 } else { 0.0 });
                w.f32(0.0);
                w.f32(0.0);
                w.u32(buttons);
            };
            w.u32(listener);
            w.u32(Ui::CANVAS_CONTAINER.0);
            w.u8(k);
            w.u8(ptr::PEN);
            w.i32(1);
            w.i32(0);
            w.u32(0);
            sample(&mut w, samples.last().unwrap());
            let coalesced = if k == ptr::MOVE { samples } else { &[] };
            w.u32(coalesced.len() as u32);
            for s in coalesced {
                sample(&mut w, s);
            }
            let path = if k == ptr::DOWN { vec![self.canvas, Ui::CANVAS_CONTAINER] } else { vec![] };
            w.u32(path.len() as u32);
            for n in path {
                w.u32(n.0);
            }
            self.app.event(&mut self.host, kind::POINTER, &w.buf);
        }
        fn frame(&mut self) {
            self.host.advance(16.0);
            let t = self.host.now;
            self.app.frame(&mut self.host, t);
        }
        fn gesture(&mut self, pts: &[(f64, f64)]) {
            self.send(L_DOWN, ptr::DOWN, &pts[..1], 1);
            for p in &pts[1..] {
                self.send(L_MOVE, ptr::MOVE, &[*p], 1);
                self.frame();
            }
            self.send(L_UP, ptr::UP, &pts[pts.len() - 1..], 0);
        }
        fn pen(&mut self, tool: InputTool, pts: &[(f64, f64)]) {
            input::set_tool(&mut self.app, &mut self.host, Some(tool), None, None);
            self.gesture(pts);
        }
        fn eraser(&mut self, area: bool) {
            test_cmd(&mut self.app, &mut self.host, "mode", if area { "\"area\"" } else { "\"stroke\"" });
        }
        /// Live strokes in draw order: (id, points).
        fn strokes(&self) -> Vec<(Id, Vec<f64>)> {
            let d = &session::get(&self.app).unwrap().doc;
            d.draw_list(self.page).strokes.iter().filter_map(|r| match &d.live(r.id)?.body.v {
                Body::Stroke { xy, .. } => Some((r.id, xy.clone())),
                _ => None,
            }).collect()
        }
    }

    fn line(y: f64, x0: f64, x1: f64, step: f64) -> Vec<(f64, f64)> {
        let n = ((x1 - x0) / step) as usize;
        (0..=n).map(|i| (x0 + i as f64 * step, y)).collect()
    }

    #[test]
    fn stroke_eraser_sweeps_between_samples_and_repaints_before_lift() {
        let mut t = boot();
        t.pen(InputTool::Pen, &line(100.0, 10.0, 200.0, 10.0));
        t.pen(InputTool::Pen, &line(300.0, 10.0, 200.0, 10.0));
        let before = t.strokes();
        t.eraser(false);
        assert_eq!(t.app.st.editor_input.tool, InputTool::EraserStroke);
        // Down 50 px above the first line, then one sample 50 px below it: no sample is within the 6 px radius,
        // the swept segment crosses the line (B-E6).
        t.send(L_DOWN, ptr::DOWN, &[(100.0, 50.0)], 1);
        let out = t.host.take();
        assert!(out.contains("visibility") || !out.is_empty());
        t.send(L_MOVE, ptr::MOVE, &[(100.0, 150.0)], 1);
        t.frame();
        let st = &t.app.st.editor_eraser;
        assert!(st.active && st.cuts.len() == 1 && st.repaints == 1, "cut {} repaints {}", st.cuts.len(), st.repaints);
        assert_eq!(t.strokes().len(), 2, "the model changes at the lift only");
        assert!(st.cursor.is_some());
        t.send(L_UP, ptr::UP, &[(100.0, 150.0)], 0);
        assert_eq!(t.strokes(), before[1..].to_vec());
        assert_eq!(history::depth(&t.app), (3, 3));
        assert!(t.app.st.editor_eraser.cursor.is_none());
        // Undo puts it back below the second line (its original z).
        history::undo(&mut t.app, &mut t.host);
        assert_eq!(t.strokes(), before);
        history::redo(&mut t.app, &mut t.host);
        assert_eq!(t.strokes().len(), 1);
    }

    #[test]
    fn area_eraser_splits_at_the_original_z_and_undo_restores_it_there() {
        let mut t = boot();
        t.pen(InputTool::Pen, &line(200.0, 10.0, 300.0, 10.0));
        t.pen(InputTool::Highlighter, &line(210.0, 10.0, 300.0, 10.0));
        let before = t.strokes();
        let (pen, marker) = (before[0].0, before[1].0);
        t.eraser(true);
        // One gesture, two places on the pen line (and the highlighter beside it).
        t.send(L_DOWN, ptr::DOWN, &[(100.0, 195.0)], 1);
        t.send(L_UP, ptr::UP, &[(100.0, 195.0)], 0);
        assert_eq!(t.app.st.editor_eraser.commits, 1);
        let after = t.strokes();
        // x = 100 removed: [10..90] and [110..300] remain, both below the highlighter (B-E4).
        assert_eq!(after.len(), 3);
        assert_eq!(after[0].1, line(200.0, 10.0, 90.0, 10.0).iter().flat_map(|p| [p.0, p.1]).collect::<Vec<_>>());
        assert_eq!(after[1].1[..2], [110.0, 200.0]);
        assert_eq!(after[2].0, marker);
        assert!(after.iter().all(|s| s.0 != pen));
        history::undo(&mut t.app, &mut t.host);
        assert_eq!(t.strokes(), before);
        history::redo(&mut t.app, &mut t.host);
        assert_eq!(t.strokes().len(), 3);
        assert_eq!(t.strokes()[2].0, marker);

        // A gesture that cuts a fragment of its own again: one batch, one step, three pieces of the first.
        history::undo(&mut t.app, &mut t.host);
        let steps = history::depth(&t.app).0;
        t.gesture(&[(60.0, 200.0), (60.0, 199.0), (200.0, 180.0), (200.0, 199.0)]);
        let pieces: Vec<_> = t.strokes().into_iter().filter(|s| s.0 != marker).collect();
        assert_eq!(pieces.len(), 3, "{pieces:?}");
        assert_eq!(history::depth(&t.app).0, steps);
        assert_eq!(t.strokes().last().unwrap().0, marker);
    }

    #[test]
    fn read_mode_mid_gesture_discards_and_live_packets_follow_the_gesture() {
        let mut t = boot();
        t.pen(InputTool::Pen, &line(100.0, 10.0, 200.0, 10.0));
        let before = t.strokes();
        t.eraser(false);
        set_live_sink(&mut t.app, true);
        t.send(L_DOWN, ptr::DOWN, &[(50.0, 100.0)], 1);
        t.frame();
        view::set_read_only(&mut t.app, &mut t.host, true, false);
        t.send(L_UP, ptr::UP, &[(50.0, 100.0)], 0);
        assert_eq!(t.strokes(), before);
        assert!(t.app.st.editor_eraser.cuts.is_empty());
        let kinds: Vec<&str> = drain_live(&mut t.app).iter().map(|p| match p {
            LivePreview::Erase { hidden, .. } if hidden.len() == 1 => "erase",
            LivePreview::EraseEnd { .. } => "end",
            _ => "other",
        }).collect();
        assert_eq!(kinds, ["erase", "end"]);
        // Edit mode again: the gesture commits after its EraseEnd.
        view::set_read_only(&mut t.app, &mut t.host, false, false);
        t.gesture(&[(50.0, 100.0), (60.0, 100.0)]);
        assert!(t.strokes().is_empty());
        assert!(matches!(drain_live(&mut t.app).last(), Some(LivePreview::EraseEnd { .. })));
    }
}
