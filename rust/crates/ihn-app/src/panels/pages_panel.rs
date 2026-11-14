//! Manage-Pages sidebar (`#pages-panel`, spec/pages.md §3.5–§4.4): open/close, the default-size line, the page list
//! with thumbnails, drag reorder, delete with the slide-to-confirm modal, the Photo / Scan (Soon) / Import tools.
//! Owned by W5.1.
//!
//! Edits are single `session::edit` batches (`page_move`, `page_delete`), so they reach collaborators like any other
//! op; as in 5.18.0 they are not undo steps. The list is built only while the panel is visible (or when it opens).
//! A drag measures every row once at pointerdown, then runs one animation-frame tick that auto-scrolls near the
//! edges, moves the ghost and places the insertion line (binary search over the measured rows).
// v5: app-v5.js:openPagesPanel / closePagesPanel / renderPagesList / initPageListDrag / movePage / openDeleteModal /
// confirmDeletePage / getPageSizeInfo / getDocumentBasePageDimensions / getContainedRect

use super::{photo_page, slider};
use crate::abi::{ety, ev, life, listen, mode, ptr, ret};
use crate::Prop;
use crate::doc::{import, session};
use crate::editor::{pages, selection, view, viewport};
use crate::host::Metric;
use crate::shell::{self, Status, StatusOpts};
use crate::{tag, App, Event, Host, Mod, Node, Ui};
use ihn_core::doc::Doc;
use ihn_core::id::Id;
use ihn_core::jsfmt::{fixed, num};
use ihn_core::jsmath::round;
use ihn_core::op::{Background, Body, Tool};
use ihn_core::render::cmd::{Src, Surf};

const L_OPEN: u32 = tag(Mod::PanelsPagesPanel, 1);
const L_CLOSE: u32 = tag(Mod::PanelsPagesPanel, 2);
const L_COVER: u32 = tag(Mod::PanelsPagesPanel, 3);
const L_SCAN: u32 = tag(Mod::PanelsPagesPanel, 4);
const L_IMPORT: u32 = tag(Mod::PanelsPagesPanel, 5);
const L_LIST_CLICK: u32 = tag(Mod::PanelsPagesPanel, 6);
const L_LIST_DOWN: u32 = tag(Mod::PanelsPagesPanel, 7);
const L_MOVE: u32 = tag(Mod::PanelsPagesPanel, 8);
const L_UP: u32 = tag(Mod::PanelsPagesPanel, 9);
const L_CANCEL: u32 = tag(Mod::PanelsPagesPanel, 10);
const L_KEY: u32 = tag(Mod::PanelsPagesPanel, 11);
const L_OVERLAY: u32 = tag(Mod::PanelsPagesPanel, 12);
const L_LOST: u32 = tag(Mod::PanelsPagesPanel, 13);
const T_THUMBS: u32 = tag(Mod::PanelsPagesPanel, 1);

/// v5: A4 in CSS px.
const A4: (f64, f64) = (794.0, 1123.0);

struct Row {
    id: Id,
    item: Node,
    main: Node,
    del: Node,
    handle: Node,
    canvas: Node,
    surf: Surf,
    tw: u32,
    th: u32,
}

struct Drag {
    from: usize,
    page: Id,
    pointer: i32,
    item: Node,
    ghost: Node,
    indicator: Node,
    /// `(top in list content, height)` of every row, measured once.
    rows: Vec<(f64, f64)>,
    x: f64,
    y: f64,
    prev: f64,
    dest: Option<(usize, bool)>,
}

#[derive(Default)]
pub struct State {
    /// `editorView.querySelector('header')`.
    header: Node,
    open: bool,
    rows: Vec<Row>,
    /// Thumbnails waiting for a decode.
    waiting: Vec<usize>,
    drag: Option<Drag>,
    /// The page the slider is confirming the deletion of.
    delete: Id,
    /// Pages moved / deleted (test counters).
    pub moved: u32,
    pub deleted: u32,
}

// ---- geometry and labels ----------------------------------------------------------------------------------------

/// v5 `getContainedRect(sw, sh, mw, mh)` → `(w, h)`.
pub fn contained(sw: f64, sh: f64, mw: f64, mh: f64) -> (f64, f64) {
    let (sw, sh) = (sw.max(1.0), sh.max(1.0));
    let (mw, mh) = (mw.max(1.0), mh.max(1.0));
    let scale = (mw / sw).min(mh / sh);
    (round(sw * scale).max(1.0), round(sh * scale).max(1.0))
}

/// v5 `formatPageMillimetres(px)`.
fn mm(px: f64) -> String {
    let m = px.max(1.0) * 25.4 / 96.0;
    let r = round(m);
    if (m - r).abs() < 0.25 {
        num(r)
    } else {
        let s = fixed(m, 1);
        s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
    }
}

/// v5 `formatPageAspectRatio(w, h)`.
fn ratio(w: f64, h: f64) -> String {
    let (w, h) = (w.max(1.0), h.max(1.0));
    let land = w >= h;
    let s = fixed(if land { w / h } else { h / w }, 3);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if land { format!("{s} : 1") } else { format!("1 : {s}") }
}

/// v5 `getPageSizeInfo(w, h)` → `(standardName, dimensionsLabel, ratioLabel)`.
pub fn size_info(w: f64, h: f64) -> (&'static str, String, String) {
    let w = if w > 0.0 { w } else { A4.0 };
    let h = if h > 0.0 { h } else { A4.1 };
    let (wm, hm) = (w * 25.4 / 96.0, h * 25.4 / 96.0);
    let is = |a: f64, b: f64| (wm - a).abs() <= 1.5 && (hm - b).abs() <= 1.5;
    let name = if is(210.0, 297.0) || is(297.0, 210.0) {
        "A4"
    } else if is(148.0, 210.0) || is(210.0, 148.0) {
        "A5"
    } else if is(215.9, 279.4) || is(279.4, 215.9) {
        "US Letter"
    } else {
        ""
    };
    (name, format!("{} × {} mm", mm(w), mm(h)), ratio(w, h))
}

/// `parts.filter(Boolean).join(sep)`.
fn join(parts: &[&str], sep: &str) -> String { parts.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(sep) }

/// The size line of a page: `A4 · 210 × 297 mm · 1 : 1.414`.
pub fn size_line(w: f64, h: f64) -> String {
    let (n, d, r) = size_info(w, h);
    join(&[n, &d, &r], " · ")
}

fn page_size(d: &Doc, id: Id) -> (f64, f64) { d.pages.get(&id).map_or(A4, |p| (p.size.v.0 as f64, p.size.v.1 as f64)) }

/// v5 `getDocumentBasePageDimensions()`: the most common size of the pages after the first (ties: first seen).
pub fn base_size(d: Option<&Doc>) -> (f64, f64) {
    let Some(d) = d else { return A4 };
    let order = d.order();
    let pages = if order.len() > 1 { &order[1..] } else { order };
    let mut counts: Vec<((f64, f64), u32)> = Vec::new();
    for id in pages {
        let s = page_size(d, *id);
        match counts.iter_mut().find(|c| c.0 == s) {
            Some(c) => c.1 += 1,
            None => counts.push((s, 1)),
        }
    }
    counts.iter().fold(None::<((f64, f64), u32)>, |best, c| if best.is_none_or(|b| c.1 > b.1) { Some(*c) } else { best }).map_or(A4, |b| b.0)
}

/// v5 `getVisibleStrokeCount(page)`: every stroke that is not an eraser stroke.
fn stroke_count(d: &Doc, id: Id) -> usize {
    let l = d.draw_list(id);
    l.covers.iter().chain(l.strokes).filter(|r| d.live(r.id).is_some_and(|i| !matches!(i.body.v, Body::Stroke { tool: Tool::LegacyEraseArea, .. }))).count()
}

fn doc(app: &App) -> Option<&Doc> { session::get(app).map(|s| &s.doc) }

// ---- open / close / render --------------------------------------------------------------------------------------

pub fn is_open(app: &App) -> bool { app.st.panels_pages_panel.open }

/// v5 `openPagesPanel()`.
pub fn open(app: &mut App, host: &mut dyn Host) {
    app.st.panels_pages_panel.open = true;
    render(app, host);
    let ro = view::read_only(app);
    let out = host.out();
    out.prop(Ui::BTN_SET_COVER, "disabled", Prop::Bool(ro));
    out.prop(Ui::BTN_SCAN_PAGE, "disabled", Prop::Bool(ro));
    let header = app.st.panels_pages_panel.header;
    if !header.is_none() {
        let bottom = round(host.rect(header).bottom()).max(0.0);
        host.out().style(Ui::PAGES_PANEL, "--pages-top", Some(&format!("{}px", num(bottom))));
    }
    let out = host.out();
    out.class(Ui::PAGES_PANEL, "visible", true);
    out.class(Ui::PANEL_OVERLAY, "visible", true);
}

/// v5 `closePagesPanel()`.
pub fn close(app: &mut App, host: &mut dyn Host) {
    finish_drag(app, host, true);
    app.st.panels_pages_panel.open = false;
    let out = host.out();
    out.class(Ui::PAGES_PANEL, "visible", false);
    out.class(Ui::PANEL_OVERLAY, "visible", false);
}

fn drop_rows(app: &mut App, host: &mut dyn Host) {
    let rows = std::mem::take(&mut app.st.panels_pages_panel.rows);
    app.st.panels_pages_panel.waiting.clear();
    pages::with_gfx(app, |_, g| rows.iter().for_each(|r| g.cmds.drop_surf(r.surf)));
    pages::flush(app, host);
    let out = host.out();
    for r in rows {
        for n in [r.item, r.main, r.del, r.handle, r.canvas] {
            if !n.is_none() {
                out.release(n);
            }
        }
    }
}

const HANDLE: &str = "<div class=\"page-drag-handle\" title=\"Drag to reorder\"><svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.5\"><line x1=\"4\" y1=\"8\" x2=\"20\" y2=\"8\"></line><line x1=\"4\" y1=\"16\" x2=\"20\" y2=\"16\"></line></svg></div>";
const TRASH: &str = "<svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\"><polyline points=\"3 6 5 6 21 6\"></polyline><path d=\"M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2\"></path></svg>";

/// v5 `renderPagesList({force: true})` (callers check visibility).
fn render(app: &mut App, host: &mut dyn Host) {
    finish_drag(app, host, true);
    let scroll = host.metric(Ui::PAGES_LIST, Metric::ScrollTop);
    drop_rows(app, host);
    let ro = view::read_only(app);
    let narrow = host.metric(Node::WINDOW, Metric::InnerWidth) <= 720.0;
    let active = pages::active_index(app);
    let Some(d) = doc(app) else {
        host.out().html(Ui::PAGES_LIST, "");
        return;
    };
    let order = d.order().to_vec();
    let can_edit = !ro && order.len() > 1;
    let (bw, bh) = base_size(Some(d));
    let (name, dims, ratio) = size_info(bw, bh);
    let mut html = String::new();
    let mut sizes = Vec::with_capacity(order.len());
    let bx = if narrow { 100.0 } else { 136.0 };
    for (i, id) in order.iter().enumerate() {
        let (w, h) = page_size(d, *id);
        let k = stroke_count(d, *id);
        let kind = match d.pages.get(id).map(|p| p.background.v) {
            Some(Background::Pdf { .. }) => "PDF",
            Some(Background::Custom { .. }) => "Photo",
            _ => "",
        };
        let (std, pd, pr) = size_info(w, h);
        let (tw, th) = contained(w, h, bx, round(bx * 1.42));
        let strokes = format!("{k} stroke{}", if k != 1 { "s" } else { "" });
        let n = i + 1;
        html.push_str(&format!(
            "<div class=\"page-item{}\" data-page-index=\"{i}\" data-page-id=\"{}\" style=\"cursor: pointer;\">{}<div class=\"page-item-main\" data-action=\"open\" title=\"{}\"><div class=\"page-thumbnail-slot\"><div class=\"page-thumbnail\" style=\"width:{}px;height:{}px\"><canvas id=\"thumb-{i}\" width=\"{}\" height=\"{}\"></canvas></div></div><div class=\"page-info\"><h3 aria-label=\"Page {n}\">{n}</h3><p class=\"page-size\">{}</p><p>{strokes}{}</p></div></div><div class=\"page-actions\"><button class=\"page-action-btn delete\" data-action=\"delete\" data-page=\"{i}\" title=\"Delete\"{}>{TRASH}</button></div></div>",
            if active == Some(i) { " is-current" } else { "" },
            id.hex(),
            if can_edit { HANDLE } else { "" },
            join(&[&format!("Page {n}"), std, &strokes, kind], " · "),
            num(tw),
            num(th),
            num(tw * 2.0),
            num(th * 2.0),
            join(&[std, &pd, &pr], " · "),
            if kind.is_empty() { String::new() } else { format!(" · {kind}") },
            if can_edit { "" } else { " disabled=\"\" aria-disabled=\"true\"" },
        ));
        sizes.push((*id, tw as u32 * 2, th as u32 * 2));
    }
    let out = host.out();
    out.html(Ui::PAGES_DEFAULT_SIZE, &format!("<span>Default page</span><strong>{}</strong>", join(&[name, &dims], " · ")));
    out.prop(Ui::PAGES_DEFAULT_SIZE, "title", Prop::Str(&join(&[name, &dims, &ratio], " · ")));
    out.html(Ui::PAGES_LIST, &html);
    let mut rows = Vec::with_capacity(sizes.len());
    for (i, (id, tw, th)) in sizes.into_iter().enumerate() {
        let item = if i < 256 { out.child(Ui::PAGES_LIST, &[i as u8]) } else { out.query(Ui::PAGES_LIST, &format!(".page-item[data-page-index=\"{i}\"]")) };
        let main = out.query(item, ".page-item-main");
        let del = out.query(item, ".page-action-btn");
        let handle = if can_edit { out.query(item, ".page-drag-handle") } else { Node::NONE };
        let canvas = out.query(item, "canvas");
        rows.push(Row { id, item, main, del, handle, canvas, surf: Surf(0), tw, th });
    }
    out.prop(Ui::PAGES_LIST, "scrollTop", Prop::Num(scroll));
    pages::with_gfx(app, |_, g| rows.iter_mut().for_each(|r| r.surf = g.cmds.bind(r.canvas.0)));
    pages::flush(app, host);
    app.st.panels_pages_panel.rows = rows;
    for i in 0..app.st.panels_pages_panel.rows.len() {
        thumb(app, host, i);
    }
}

/// Paints thumbnail `i`; a thumbnail still decoding is retried shortly.
fn thumb(app: &mut App, host: &mut dyn Host, i: usize) {
    let Some(r) = app.st.panels_pages_panel.rows.get(i) else { return };
    let (id, surf, tw, th) = (r.id, r.surf, r.tw, r.th);
    if !pages::panel_thumb(app, host, id, surf, tw, th) {
        let st = &mut app.st.panels_pages_panel;
        if !st.waiting.contains(&i) {
            st.waiting.push(i);
        }
        app.sched.timer(host.out(), T_THUMBS, 60.0);
    }
}

/// v5 `renderPagesList()`: rebuilds the list when the panel is visible.
pub fn refresh(app: &mut App, host: &mut dyn Host) {
    if is_open(app) {
        render(app, host);
    }
}

/// The document changed while the panel is open: rebuild when the page list or a page's kind changed.
fn changed(app: &mut App, host: &mut dyn Host) {
    if !is_open(app) {
        return;
    }
    let same = doc(app).is_some_and(|d| d.order().len() == app.st.panels_pages_panel.rows.len() && d.order().iter().zip(&app.st.panels_pages_panel.rows).all(|(a, r)| *a == r.id));
    if !same {
        render(app, host);
    }
}

/// After a local structure edit: the editor's rows, the list.
fn after_edit(app: &mut App, host: &mut dyn Host, c: &ihn_core::doc::Changes) {
    pages::changed(app, host, c);
    if is_open(app) {
        render(app, host);
    }
}

// ---- delete -----------------------------------------------------------------------------------------------------

/// v5 `openDeleteModal(i)`.
fn ask_delete(app: &mut App, host: &mut dyn Host, i: usize) {
    let n = doc(app).map_or(0, |d| d.order().len());
    if view::read_only(app) || n <= 1 {
        return;
    }
    let Some(id) = doc(app).and_then(|d| d.order().get(i).copied()) else { return };
    app.st.panels_pages_panel.delete = id;
    let message = format!("This will delete page {} and all its strokes. This cannot be undone.", i + 1);
    slider::show(app, host, "Delete page?", &message, "Slide to delete", delete_answer);
}

fn delete_answer(app: &mut App, host: &mut dyn Host, ok: bool) {
    let id = std::mem::replace(&mut app.st.panels_pages_panel.delete, Id::NONE);
    if ok {
        delete_page(app, host, id);
    }
}

/// v5 `confirmDeletePage()`.
pub fn delete_page(app: &mut App, host: &mut dyn Host, id: Id) {
    if view::read_only(app) {
        return;
    }
    let Some(order) = doc(app).map(|d| d.order().to_vec()) else { return };
    let Some(at) = order.iter().position(|p| *p == id) else { return };
    if order.len() <= 1 {
        return;
    }
    if app.st.editor_pages.selection_page == id {
        selection::clear(app, host);
    }
    let Some(e) = session::edit(app, host, |tx| tx.page_delete(id)) else { return };
    app.st.panels_pages_panel.deleted += 1;
    pages::changed(app, host, &e.changes);
    viewport::go_to_page(app, host, at.min(order.len() - 2));
    if is_open(app) {
        render(app, host);
    }
    shell::status(app, host, "Page deleted", StatusOpts::from(Status::Info).preserve());
}

// ---- reorder ----------------------------------------------------------------------------------------------------

/// v5 `movePage(from, to, {movingPageId, targetPageId, insertBeforeTarget})`.
pub fn move_page(app: &mut App, host: &mut dyn Host, page: Id, target: Id, before: bool) -> bool {
    if view::read_only(app) || page == target {
        return false;
    }
    let Some(order) = doc(app).map(|d| d.order().to_vec()) else { return false };
    let Some(t) = order.iter().position(|p| *p == target) else { return false };
    let next = if before { Some(target) } else { order.get(t + 1).copied() };
    let Some(e) = session::edit(app, host, |tx| tx.page_move(page, next)) else { return false };
    app.st.panels_pages_panel.moved += 1;
    after_edit(app, host, &e.changes);
    true
}

fn drag_start(app: &mut App, host: &mut dyn Host, p: &crate::abi::PointerBatch) -> u32 {
    let st = &app.st.panels_pages_panel;
    let Some(from) = st.rows.iter().position(|r| !r.handle.is_none() && p.path.contains(r.handle)) else { return 0 };
    if p.button > 0 || p.flags & ev::PRIMARY == 0 || view::read_only(app) || st.drag.is_some() {
        return 0;
    }
    let (page, item) = (st.rows[from].id, st.rows[from].item);
    let list = host.rect(Ui::PAGES_LIST);
    let scroll = host.metric(Ui::PAGES_LIST, Metric::ScrollTop);
    let items: Vec<Node> = app.st.panels_pages_panel.rows.iter().map(|r| r.item).collect();
    let rows = items.iter().map(|n| {
        let r = host.rect(*n);
        (r.y - list.y + scroll, r.h)
    }).collect();
    let (tw, th) = (app.st.panels_pages_panel.rows[from].tw, app.st.panels_pages_panel.rows[from].th);
    let source = app.st.panels_pages_panel.rows[from].surf;
    let out = host.out();
    let ghost = out.insert_html(Node::BODY, Node::NONE, &format!("<div class=\"page-reorder-preview\" aria-hidden=\"true\"><canvas width=\"{tw}\" height=\"{th}\"></canvas><span>{}</span></div>", from + 1));
    let indicator = out.insert_html(Node::BODY, Node::NONE, "<div class=\"page-reorder-indicator\" aria-hidden=\"true\"></div>");
    let copy = out.query(ghost, "canvas");
    pages::with_gfx(app, |_, g| {
        let c = &mut g.cmds;
        let s = c.bind(copy.0);
        c.ctx(s, false);
        c.draw_image(Src::Surf(source), &[0.0, 0.0]);
        c.drop_surf(s);
    });
    pages::flush(app, host);
    let out = host.out();
    out.release(copy);
    out.class(item, "is-dragging", true);
    out.class(Ui::PAGES_LIST, "is-reordering", true);
    out.pointer_capture(Ui::PAGES_LIST, p.pointer_id, true);
    out.listen(Node::DOCUMENT, "pointermove", L_MOVE, listen::ACTIVE);
    out.listen(Node::DOCUMENT, "pointerup", L_UP, 0);
    out.listen(Node::DOCUMENT, "pointercancel", L_CANCEL, 0);
    out.listen(Ui::PAGES_LIST, "lostpointercapture", L_LOST, 0);
    out.listen(Node::DOCUMENT, "keydown", L_KEY, 0);
    app.sched.frame(out, Mod::PanelsPagesPanel);
    app.st.panels_pages_panel.drag = Some(Drag { from, page, pointer: p.pointer_id, item, ghost, indicator, rows, x: p.raw.x, y: p.raw.y, prev: 0.0, dest: None });
    ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION
}

/// v5 `updateTarget()` of the drag.
fn update_target(app: &mut App, host: &mut dyn Host) {
    let r = host.rect(Ui::PAGES_LIST);
    let scroll = host.metric(Ui::PAGES_LIST, Metric::ScrollTop);
    let Some(dr) = app.st.panels_pages_panel.drag.as_mut() else { return };
    let (x, y) = (dr.x, dr.y);
    let within = x >= r.x - 48.0 && x <= r.right() + 48.0 && y >= r.y - 64.0 && y <= r.bottom() + 64.0;
    dr.dest = None;
    let indicator = dr.indicator;
    if !within || dr.rows.is_empty() {
        host.out().prop(indicator, "hidden", Prop::Bool(true));
        return;
    }
    let cy = (y - r.y).min(r.h).max(0.0) + scroll;
    let (mut lo, mut hi) = (0, dr.rows.len());
    while lo < hi {
        let mid = (lo + hi) >> 1;
        if cy < dr.rows[mid].0 + dr.rows[mid].1 / 2.0 { hi = mid } else { lo = mid + 1 }
    }
    let before = lo < dr.rows.len();
    let at = if before { lo } else { dr.rows.len() - 1 };
    if at != dr.from {
        dr.dest = Some((at, before));
    }
    let row = dr.rows[at];
    let line = r.y + row.0 + if before { 0.0 } else { row.1 } - scroll;
    let hidden = dr.dest.is_none();
    let out = host.out();
    out.style(indicator, "width", Some(&format!("{}px", num((r.w - 20.0).max(0.0)))));
    out.style(indicator, "transform", Some(&format!("translate3d({}px,{}px,0)", num(r.x + 10.0), num(r.y.max(line.min(r.bottom() - 3.0))))));
    out.prop(indicator, "hidden", Prop::Bool(hidden));
}

/// v5 `tick(time)` of the drag.
fn tick(app: &mut App, host: &mut dyn Host, t: f64) {
    let Some(item) = app.st.panels_pages_panel.drag.as_ref().map(|d| d.item) else { return };
    if view::read_only(app) || !app.st.panels_pages_panel.open || host.metric(item, Metric::Connected) == 0.0 {
        return finish_drag(app, host, true);
    }
    let dr = app.st.panels_pages_panel.drag.as_mut().expect("drag");
    let dt = if dr.prev != 0.0 { (t - dr.prev).min(40.0) } else { 0.0 };
    dr.prev = t;
    let (x, y, ghost) = (dr.x, dr.y, dr.ghost);
    let r = host.rect(Ui::PAGES_LIST);
    let edge = 88f64.min(r.h / 3.0);
    let mut speed = 0.0;
    if edge > 0.0 && x >= r.x - 48.0 && x <= r.right() + 48.0 {
        if y < r.y + edge {
            speed = -(1f64.min((r.y + edge - y) / edge));
        } else if y > r.bottom() - edge {
            speed = 1f64.min((y - r.bottom() + edge) / edge);
        }
    }
    if speed != 0.0 {
        let top = host.metric(Ui::PAGES_LIST, Metric::ScrollTop);
        let step = speed.signum() * speed * speed * 1050.0 * dt / 1000.0;
        host.out().prop(Ui::PAGES_LIST, "scrollTop", Prop::Num(top + step));
    }
    host.out().style(ghost, "transform", Some(&format!("translate3d({}px,{}px,0)", num(x + 14.0), num(y - 48.0))));
    update_target(app, host);
    app.sched.frame(host.out(), Mod::PanelsPagesPanel);
}

/// v5 `finishDrag(cancelled)`.
fn finish_drag(app: &mut App, host: &mut dyn Host, cancelled: bool) {
    let Some(dr) = app.st.panels_pages_panel.drag.take() else { return };
    let out = host.out();
    for l in [L_MOVE, L_UP, L_CANCEL, L_LOST, L_KEY] {
        out.unlisten(l);
    }
    out.pointer_capture(Ui::PAGES_LIST, dr.pointer, false);
    out.remove(dr.ghost);
    out.remove(dr.indicator);
    out.release(dr.ghost);
    out.release(dr.indicator);
    out.class(dr.item, "is-dragging", false);
    out.class(Ui::PAGES_LIST, "is-reordering", false);
    let Some((at, before)) = dr.dest.filter(|_| !cancelled) else { return };
    let Some(target) = app.st.panels_pages_panel.rows.get(at).map(|r| r.id) else { return };
    if !move_page(app, host, dr.page, target, before) {
        if doc(app).is_some() {
            shell::status(app, host, "Could not move the page. Try again.", StatusOpts::from(Status::Error).preserve());
        }
        return;
    }
    if !app.cfg.has(mode::REDUCED_MOTION) {
        if let Some(r) = app.st.panels_pages_panel.rows.iter().find(|r| r.id == dr.page) {
            let item = r.item;
            let anim = host.out().animate(item, r#"[{"opacity":0.55,"transform":"scale(.97)"},{"opacity":1,"transform":"scale(1)"}]"#, r#"{"duration":180,"easing":"ease-out"}"#);
            host.out().release(anim);
        }
    }
}

// ---- module contract --------------------------------------------------------------------------------------------

pub fn init(app: &mut App, host: &mut dyn Host) {
    if app.cfg.has(mode::HEADLESS) {
        return;
    }
    let out = host.out();
    app.st.panels_pages_panel.header = out.query(Ui::EDITOR_VIEW, "header");
    out.listen(Ui::BTN_EDIT_PAGES, "click", L_OPEN, 0);
    out.listen(Ui::PANEL_CLOSE, "click", L_CLOSE, 0);
    out.listen(Ui::PANEL_OVERLAY, "click", L_OVERLAY, 0);
    out.listen(Ui::BTN_SET_COVER, "click", L_COVER, 0);
    out.listen(Ui::BTN_SCAN_PAGE, "click", L_SCAN, 0);
    out.listen(Ui::BTN_IMPORT_PDF, "click", L_IMPORT, 0);
    out.listen(Ui::PAGES_LIST, "click", L_LIST_CLICK, 0);
    out.listen(Ui::PAGES_LIST, "pointerdown", L_LIST_DOWN, listen::ACTIVE);
}

pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match event {
        Event::Dom(d) => match d.listener {
            L_OPEN => open(app, host),
            L_CLOSE | L_OVERLAY => close(app, host),
            L_COVER => photo_page::open(app, host),
            L_SCAN => shell::soon(app, host),
            L_IMPORT => import::pick(app, host),
            L_LIST_CLICK if d.ty == ety::CLICK => {
                let st = &app.st.panels_pages_panel;
                if let Some(i) = st.rows.iter().position(|r| d.path.contains(r.del)) {
                    ask_delete(app, host, i);
                    return ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION;
                }
                if let Some(i) = st.rows.iter().position(|r| d.path.contains(r.main)) {
                    viewport::go_to_page(app, host, i);
                    close(app, host);
                }
            }
            L_KEY if d.key == "Escape" && app.st.panels_pages_panel.drag.is_some() => {
                finish_drag(app, host, true);
                return ret::PREVENT_DEFAULT;
            }
            _ => {}
        },
        Event::Pointer(p) => match p.listener {
            L_LIST_DOWN if p.kind == ptr::DOWN => return drag_start(app, host, p),
            L_MOVE => {
                if let Some(dr) = app.st.panels_pages_panel.drag.as_mut().filter(|d| d.pointer == p.pointer_id) {
                    (dr.x, dr.y) = (p.raw.x, p.raw.y);
                    return ret::PREVENT_DEFAULT;
                }
            }
            L_UP => {
                if let Some(dr) = app.st.panels_pages_panel.drag.as_mut().filter(|d| d.pointer == p.pointer_id) {
                    (dr.x, dr.y) = (p.raw.x, p.raw.y);
                    update_target(app, host);
                    finish_drag(app, host, false);
                }
            }
            L_CANCEL | L_LOST => {
                if app.st.panels_pages_panel.drag.as_ref().is_some_and(|d| d.pointer == p.pointer_id) {
                    finish_drag(app, host, true);
                }
            }
            _ => {}
        },
        Event::Frame(t) => tick(app, host, *t),
        Event::Timer(T_THUMBS) => {
            let waiting = std::mem::take(&mut app.st.panels_pages_panel.waiting);
            if is_open(app) {
                waiting.into_iter().for_each(|i| thumb(app, host, i));
            }
        }
        Event::Life(l) if (l.code == life::BLUR || l.code == life::RESIZE) && app.st.panels_pages_panel.drag.is_some() => finish_drag(app, host, true),
        Event::Msg(m) => match (m.chan, m.data) {
            (view::sig::MODE, b"read") if is_open(app) => close(app, host),
            (session::sig::CHANGED, _) => changed(app, host),
            (session::sig::CLOSED, _) => {
                if is_open(app) {
                    close(app, host);
                }
                drop_rows(app, host);
            }
            _ => {}
        },
        _ => {}
    }
    0
}

/// `panels.pages_panel.state` → `{open, items, current, dragging, moved, deleted}`; `panels.pages_panel.labels` →
/// `{defaultSize, size}` for a page size given as `{w, h}`.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, args: &str) -> Option<String> {
    let st = &app.st.panels_pages_panel;
    match name {
        "state" => Some(format!(
            "{{\"open\":{},\"items\":{},\"current\":{},\"dragging\":{},\"moved\":{},\"deleted\":{},\"order\":[{}]}}",
            st.open,
            st.rows.len(),
            pages::active_index(app).map_or(-1, |i| i as i64),
            st.drag.is_some(),
            st.moved,
            st.deleted,
            doc(app).map_or(String::new(), |d| d.order().iter().map(|id| format!("\"{}\"", id.hex())).collect::<Vec<_>>().join(","))
        )),
        "labels" => {
            let j = crate::shell::json::Json::parse(args).unwrap_or(crate::shell::json::Json::Null);
            let (w, h) = (j.get("w").and_then(|v| v.as_f64()).unwrap_or(0.0), j.get("h").and_then(|v| v.as_f64()).unwrap_or(0.0));
            Some(format!("{{\"size\":{}}}", crate::testapi::json_str(&size_line(w, h))))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_follow_5_18_0() {
        assert_eq!(size_line(794.0, 1123.0), "A4 · 210 × 297 mm · 1 : 1.414");
        assert_eq!(size_line(559.0, 794.0), "A5 · 148 × 210 mm · 1 : 1.42");
        assert_eq!(size_line(595.0, 842.0), "157.4 × 223 mm · 1 : 1.415");
        assert_eq!(size_line(800.0, 400.0), "211.7 × 106 mm · 2 : 1");
        assert_eq!(contained(794.0, 1123.0, 136.0, 193.0), (136.0, 192.0));
        assert_eq!(contained(794.0, 1123.0, 100.0, 142.0), (100.0, 141.0));
    }
}
