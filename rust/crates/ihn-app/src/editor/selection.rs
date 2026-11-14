//! Editor: Lasso, selection box, transforms, cross-page move (5.18.0 § 4.12: `selectStrokesWithLasso`,
//! `updateSelectionBox`, `drawSelectionOutlines`, `setupSelectionBoxEvents`, `startSelectionTransform`,
//! `applySelectionPointerMove`, `onSelectionPointerUp`, `moveSelectionToPage`, `duplicateSelection`,
//! `changeSelectionColor`, `deleteSelection`, `clearSelection`).
//!
//! The lasso is an `input::ToolHandler`: pointer events append the dispatched position (no allocation) and the path
//! is drawn once per frame. A drag of the selection box never edits the model until the lift: once per frame the
//! selected items are transformed from the model with `geom::Affine` into a preview, the damaged rectangle (or the
//! whole page when images move) is repainted with the preview in place of the originals, and the box follows. At
//! the lift the transform is one batch (`Tx::transform`) and one undo step. A drag past ±50 px of the page edge
//! moves the items to the neighbouring page at once (one batch: the transformed bodies shifted by the crossed
//! page's real height, on top of the target page: B-E2), as 5.18.0 does mid-gesture.
//! Selection move, scale and duplicate clamping use the page's real size (B-E1).

use super::input::{self, ToolHandler};
use super::{history, pages, view};
use crate::abi::{ptr, ret, PointerBatch};
use crate::doc::session;
use crate::shell::{self, Status, StatusOpts};
use crate::{tag, App, Event, Host, Mod, Node, Rect, Ui};
use ihn_core::doc::Doc;
use ihn_core::geom::{self, js_max, js_min, Affine};
use ihn_core::id::Id;
use ihn_core::jsfmt::num;
use ihn_core::jsmath;
use ihn_core::op::{Body, Color, Op, Z};
use ihn_core::render::damage::{self, Damage};
use ihn_core::render::overlay;
use ihn_core::render::page::{self as rpage, PageView};
use ihn_core::render::{ImageRef, PageItems, StrokeRef};
use ihn_core::stamp::Stamp;

const L_BOX: u32 = tag(Mod::EditorSelection, 1);
const L_DOC_MOVE: u32 = tag(Mod::EditorSelection, 2);
const L_DOC_UP: u32 = tag(Mod::EditorSelection, 3);
const L_DOC_CANCEL: u32 = tag(Mod::EditorSelection, 4);

/// v5: `crossPageThreshold`.
const CROSS_PAGE_PX: f64 = 50.0;
/// v5: duplicate offset and margin.
const DUPLICATE_OFFSET: f64 = 18.0;
const DUPLICATE_MARGIN: f64 = 6.0;
/// The scale handles (`data-handle`), in the box's document order.
const HANDLES: [&str; 8] = ["n", "s", "e", "w", "ne", "nw", "se", "sw"];
/// Lasso points reserved at boot.
const RESERVE: usize = 512;

/// Handles of the children of the selection box shown (queried once per box).
struct Kids {
    boxn: Node,
    actions: Node,
    rotate: Node,
    color_btn: Node,
    menu: Node,
    swatches: [Node; 3],
    duplicate: Node,
    trash: Node,
    handles: [Node; 8],
}

impl Kids {
    fn all(&self) -> impl Iterator<Item = Node> + '_ {
        [self.actions, self.rotate, self.color_btn, self.menu, self.duplicate, self.trash].into_iter().chain(self.swatches).chain(self.handles)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Move,
    /// Index in [`HANDLES`].
    Scale(usize),
    Rotate,
}

/// v5 `state.selectionTransform`.
struct Transform {
    page: Id,
    mode: Mode,
    bounds: [f64; 4],
    center: (f64, f64),
    start: (f64, f64),
    start_angle: f64,
    pointer_id: i32,
    /// The page wrapper's rect and the page's size (read at the start and after a page change).
    rect: Rect,
    w: f64,
    h: f64,
    /// The newest pointer position not applied yet (v5 `selectionMoveEvent`).
    pending: Option<(f64, f64)>,
    armed: bool,
    /// The transform shown (from the model's bodies).
    affine: Option<Affine>,
    /// The selected items as shown (`None` body: unchanged).
    preview: Vec<(Id, Body)>,
}

pub struct State {
    /// v5 `state.selection.pageIndex` (`Id::NONE`: no selection).
    pub page: Id,
    /// v5 `strokeIndices` + `imageIndices`: the selected items in draw order, with the body stamp last seen.
    items: Vec<(Id, Stamp)>,
    kids: Option<Kids>,
    menu_open: bool,
    /// The lasso of the current contact: page and path `x0, y0, …`; `drawn` = points on the overlay.
    lasso_page: Id,
    lasso: Vec<f64>,
    lasso_active: bool,
    drawn: usize,
    frame_armed: bool,
    tf: Option<Transform>,
    /// A selection edit is being written (its own `STATE` signal is not a foreign change).
    editing: bool,
    /// Batches written by the selection (test counter).
    pub commits: u32,
}

impl Default for State {
    fn default() -> Self {
        State {
            page: Id::NONE,
            items: Vec::new(),
            kids: None,
            menu_open: false,
            lasso_page: Id::NONE,
            lasso: Vec::with_capacity(2 * RESERVE),
            lasso_active: false,
            drawn: 0,
            frame_armed: false,
            tf: None,
            editing: false,
            commits: 0,
        }
    }
}

fn doc(app: &App) -> Option<&Doc> { session::get(app).map(|s| &s.doc) }

fn ids(app: &App) -> Vec<Id> { app.st.editor_selection.items.iter().map(|i| i.0).collect() }

fn is_image(d: &Doc, id: Id) -> bool { d.live(id).is_some_and(|i| matches!(i.body.v, Body::Image { .. })) }

/// The selected items still live on the selection page.
fn live_ids(app: &App) -> Vec<Id> {
    let page = app.st.editor_selection.page;
    let Some(d) = doc(app) else { return Vec::new() };
    app.st.editor_selection.items.iter().filter(|i| d.live(i.0).is_some_and(|it| it.page.v == page)).map(|i| i.0).collect()
}

fn page_size(app: &App, page: Id) -> Option<(f64, f64)> { app.st.editor_pages.rows.iter().find(|r| r.id == page).map(|r| (r.w, r.h)) }

/// v5: `clearLassoCanvas(pageIndex)`.
fn clear_lasso(app: &mut App, host: &mut dyn Host, page: Id) {
    let Some(a) = pages::active(app, page) else { return };
    let (surf, px) = (a.lasso_surf, a.lasso_px);
    pages::with_gfx(app, |_, g| overlay::clear_lasso(&mut g.cmds, surf, px));
    pages::flush(app, host);
    pages::overlay_ink(app, host, page, 0, false);
}

/// The selection box of `page` if it is the one whose children are known (queried when it is not).
fn kids<'a>(app: &'a mut App, host: &mut dyn Host, page: Id) -> Option<&'a Kids> {
    let boxn = pages::active(app, page)?.selection_box;
    let st = &mut app.st.editor_selection;
    if st.kids.as_ref().is_none_or(|k| k.boxn != boxn) {
        let out = host.out();
        if let Some(old) = st.kids.take() {
            old.all().for_each(|n| out.release(n));
        }
        let q = |out: &mut crate::OutBuf, s: &str| out.query(boxn, s);
        st.kids = Some(Kids {
            boxn,
            actions: q(out, ".selection-actions"),
            rotate: q(out, ".rotate-handle"),
            color_btn: q(out, ".selection-color-btn"),
            menu: q(out, ".selection-color-menu"),
            swatches: [0, 1, 2].map(|i| q(out, &format!(".selection-color-swatch:nth-child({})", i + 1))),
            duplicate: q(out, ".selection-duplicate"),
            trash: q(out, ".selection-trash"),
            handles: HANDLES.map(|h| q(out, &format!(".selection-handle[data-handle=\"{h}\"]"))),
        });
        st.menu_open = false;
    }
    st.kids.as_ref()
}

/// v5: `clearSelection()`.
pub fn clear(app: &mut App, host: &mut dyn Host) {
    let page = app.st.editor_selection.page;
    if page != Id::NONE {
        if let Some(k) = kids(app, host, page) {
            let (boxn, menu) = (k.boxn, k.menu);
            let out = host.out();
            out.class(boxn, "hidden", true);
            out.class(boxn, "has-selection", false);
            out.class(boxn, "has-image", false);
            out.class(menu, "open", false);
        }
        clear_lasso(app, host, page);
    }
    abandon(app, host);
    let st = &mut app.st.editor_selection;
    st.page = Id::NONE;
    st.items.clear();
    st.menu_open = false;
    app.st.editor_pages.selection_page = Id::NONE;
}

/// Selects `items` of `page` (the lasso, a duplicate, an inserted image) and shows the box.
pub fn select(app: &mut App, host: &mut dyn Host, page: Id, items: &[Id]) {
    let Some(d) = doc(app) else { return };
    let mut sel: Vec<(Id, Stamp)> = items.iter().filter_map(|&id| d.live(id).filter(|i| i.page.v == page).map(|i| (id, i.body.at))).collect();
    // Draw order: images, then strokes.
    sel.sort_by_key(|&(id, _)| !is_image(d, id));
    let st = &mut app.st.editor_selection;
    st.page = page;
    st.items = sel;
    app.st.editor_pages.selection_page = page;
    update_box(app, host);
}

/// The ids, kinds and body stamps are re-read after a selection edit.
fn refresh_stamps(app: &mut App) {
    let page = app.st.editor_selection.page;
    let Some(d) = session::get(app).map(|s| &s.doc) else { return };
    let items: Vec<(Id, Stamp)> = app.st.editor_selection.items.iter().filter_map(|&(id, _)| d.live(id).filter(|i| i.page.v == page).map(|i| (id, i.body.at))).collect();
    app.st.editor_selection.items = items;
}

/// The bounds, strokes and image flag of the selection as shown (the preview during a drag).
fn shown_bounds(app: &App) -> Option<([f64; 4], bool, bool)> {
    let d = doc(app)?;
    let st = &app.st.editor_selection;
    let mut acc = geom::SelectionBounds::default();
    let (mut strokes, mut images) = (false, false);
    let preview = st.tf.as_ref().map(|t| &t.preview[..]).unwrap_or(&[]);
    for &(id, _) in &st.items {
        let body = match preview.iter().find(|p| p.0 == id) {
            Some(p) => &p.1,
            None => match d.live(id) {
                Some(i) => &i.body.v,
                None => continue,
            },
        };
        match body {
            Body::Stroke { xy, .. } => {
                strokes = true;
                acc.add(xy);
            }
            Body::Image { cx, cy, w, h, rotation, .. } => {
                images = true;
                acc.add(&geom::image_corners(*cx, *cy, *w, *h, *rotation));
            }
            Body::Dropped => {}
        }
    }
    acc.get().map(|b| (b, strokes, images))
}

/// v5: `updateSelectionBox()`.
fn update_box(app: &mut App, host: &mut dyn Host) {
    let page = app.st.editor_selection.page;
    if page == Id::NONE {
        return;
    }
    let Some((b, _, images)) = shown_bounds(app) else { return };
    let Some(k) = kids(app, host, page) else { return };
    let (boxn, actions) = (k.boxn, k.actions);
    let has = !app.st.editor_selection.items.is_empty();
    let out = host.out();
    out.class(boxn, "hidden", false);
    out.class(boxn, "has-selection", has);
    out.class(boxn, "has-image", images);
    let px = |v: f64| format!("{}px", num(v));
    out.style(boxn, "left", Some(&px(b[0])));
    out.style(boxn, "top", Some(&px(b[1])));
    out.style(boxn, "width", Some(&px(b[2] - b[0])));
    out.style(boxn, "height", Some(&px(b[3] - b[1])));
    // v5: the actions go above the box when 34 + 28 px fit above it, else below.
    if b[1] > 34.0 + 28.0 {
        out.style(actions, "bottom", Some("calc(100% + 28px)"));
        out.style(actions, "top", Some("auto"));
    } else {
        out.style(actions, "top", Some("calc(100% + 12px)"));
        out.style(actions, "bottom", Some("auto"));
    }
    out.style(actions, "margin-top", Some("0"));
    out.style(actions, "margin-bottom", Some("0"));
    out.style(actions, "left", Some("50%"));
    out.style(actions, "transform", Some("translateX(-50%)"));
    outlines(app, host);
}

/// v5: `drawSelectionOutlines(pageIndex)` for the selection page.
fn outlines(app: &mut App, host: &mut dyn Host) {
    let page = app.st.editor_selection.page;
    let Some(a) = pages::active(app, page) else { return };
    let (surf, px, scale) = (a.lasso_surf, a.lasso_px, a.lasso_scale);
    let ink = pages::with_gfx(app, |app, g| {
        let Some(s) = session::get(app) else { return false };
        let st = &app.st.editor_selection;
        let preview = st.tf.as_ref().map(|t| &t.preview[..]).unwrap_or(&[]);
        let refs: Vec<StrokeRef> = st
            .items
            .iter()
            .filter_map(|&(id, _)| match preview.iter().find(|p| p.0 == id).map(|p| &p.1).or_else(|| s.doc.live(id).map(|i| &i.body.v)) {
                Some(Body::Stroke { tool, color, width, xy, .. }) => Some(StrokeRef { tool: *tool, color, width: *width, xy, bounds: [0.0; 4], key: None }),
                _ => None,
            })
            .collect();
        overlay::selection_outlines(&mut g.cmds, surf, px, scale, &refs)
    });
    pages::flush(app, host);
    pages::overlay_ink(app, host, page, 0, ink);
}

/// A change written by the selection: `f` fills the batch; `true` when one was written (and the undo step).
fn write(app: &mut App, host: &mut dyn Host, page: Id, points: usize, f: impl FnOnce(&mut ihn_core::doc::Tx)) -> bool {
    app.st.editor_selection.editing = true;
    let e = session::edit(app, host, f);
    app.st.editor_selection.editing = false;
    let Some(e) = e else { return false };
    app.st.editor_selection.commits += 1;
    history::record(app, host, page, e.inverse, points);
    true
}

fn repaint(app: &mut App, host: &mut dyn Host, page: Id) {
    pages::touch(app, page);
    pages::request_repaint(app, host, page);
}

/// v5: `deleteSelection(pageIndex)`.
fn delete(app: &mut App, host: &mut dyn Host) {
    let page = app.st.editor_selection.page;
    let items = live_ids(app);
    if view::read_only(app) || items.is_empty() {
        return;
    }
    if !write(app, host, page, 0, |tx| tx.erase(&items)) {
        return;
    }
    pages::with_gfx(app, |_, g| items.iter().for_each(|&id| g.paths.forget(&mut g.cmds, id)));
    clear(app, host);
    repaint(app, host, page);
    let text = if items.len() > 1 { "Selection deleted" } else { "Item deleted" };
    shell::status(app, host, text, StatusOpts::from(Status::Info).preserve());
}

/// v5: `duplicateSelection(pageIndex)`: copies 18 px right and down, clamped to the page with a 6 px margin (the
/// page's real size, B-E1); the copies become the selection and the lasso the tool.
fn duplicate(app: &mut App, host: &mut dyn Host) {
    let page = app.st.editor_selection.page;
    let items = live_ids(app);
    if view::read_only(app) || items.is_empty() {
        return;
    }
    let (Some(d), Some((w, h))) = (doc(app), page_size(app, page)) else { return };
    let (mut dx, mut dy) = (DUPLICATE_OFFSET, DUPLICATE_OFFSET);
    if let Some(b) = d.selection_bounds(&items) {
        let clamp = |v: f64, lo: f64, hi: f64| if lo > hi { 0.0 } else { js_min(js_max(v, lo), hi) };
        dx = clamp(dx, DUPLICATE_MARGIN - b[0], (w - DUPLICATE_MARGIN) - b[2]);
        dy = clamp(dy, DUPLICATE_MARGIN - b[1], (h - DUPLICATE_MARGIN) - b[3]);
    }
    // v5 pushes the strokes first, then the images.
    let mut order = items.clone();
    order.sort_by_key(|&id| is_image(d, id));
    let points: usize = order.iter().filter_map(|&id| match d.live(id).map(|i| &i.body.v) {
        Some(Body::Stroke { xy, .. }) => Some(xy.len() / 2),
        _ => None,
    }).sum();
    let mut copies = Vec::new();
    if !write(app, host, page, points, |tx| copies = tx.duplicate(&order, dx, dy)) {
        return;
    }
    select(app, host, page, &copies);
    input::set_tool(app, host, Some(input::Tool::Lasso), None, None);
    repaint(app, host, page);
    shell::status(app, host, "Selection duplicated", StatusOpts::from(Status::Info).preserve());
}

/// v5: `changeSelectionColor(pageIndex, color)`: palette colours only, strokes only.
fn recolor(app: &mut App, host: &mut dyn Host, color: &str) {
    let page = app.st.editor_selection.page;
    let Some(c) = pages::INK_PALETTE.iter().find(|c| c.eq_ignore_ascii_case(color.trim())) else { return };
    let strokes: Vec<Id> = live_ids(app).into_iter().filter(|&id| doc(app).is_some_and(|d| !is_image(d, id))).collect();
    if view::read_only(app) || strokes.is_empty() {
        return;
    }
    write(app, host, page, 0, |tx| tx.recolor(&strokes, Color::parse(c)));
    refresh_stamps(app);
    repaint(app, host, page);
    outlines(app, host);
    shell::status(app, host, "Color changed", StatusOpts::from(Status::Info).preserve());
}

// ---- transforms -------------------------------------------------------------------------------------------------

/// v5: `startSelectionTransform(pageIndex, mode, handle, e)`.
fn start(app: &mut App, host: &mut dyn Host, mode: Mode, p: &PointerBatch) {
    let page = app.st.editor_selection.page;
    let items = live_ids(app);
    let Some(b) = doc(app).and_then(|d| d.selection_bounds(&items)) else { return };
    let Some((w, h, wrapper)) = app.st.editor_pages.rows.iter().find(|r| r.id == page).map(|r| (r.w, r.h, r.wrapper)) else { return };
    let rect = host.rect(wrapper);
    let center = (b[0] + (b[2] - b[0]) / 2.0, b[1] + (b[3] - b[1]) / 2.0);
    let start = pages::to_page(rect, w, h, p.raw.x, p.raw.y);
    let start_angle = jsmath::atan2(start.1 - center.1, start.0 - center.0);
    let out = host.out();
    out.listen(Node::DOCUMENT, "pointermove", L_DOC_MOVE, 0);
    out.listen(Node::DOCUMENT, "pointerup", L_DOC_UP, 0);
    out.listen(Node::DOCUMENT, "pointercancel", L_DOC_CANCEL, 0);
    app.st.editor_selection.tf = Some(Transform {
        page,
        mode,
        bounds: b,
        center,
        start,
        start_angle,
        pointer_id: p.pointer_id,
        rect,
        w,
        h,
        pending: None,
        armed: false,
        affine: None,
        preview: Vec::new(),
    });
}

/// Ends the drag listeners and forgets the transform.
fn stop(app: &mut App, host: &mut dyn Host) -> Option<Transform> {
    let tf = app.st.editor_selection.tf.take()?;
    let out = host.out();
    for l in [L_DOC_MOVE, L_DOC_UP, L_DOC_CANCEL] {
        out.unlisten(l);
    }
    Some(tf)
}

/// A drag ends without being written: the page shows the model again.
fn abandon(app: &mut App, host: &mut dyn Host) {
    if let Some(tf) = stop(app, host) {
        if tf.affine.is_some() {
            repaint(app, host, tf.page);
        }
    }
}

/// The selected items' bodies under `m` (images: centre, size and rotation).
fn transformed(d: &Doc, items: &[(Id, Stamp)], m: &Affine) -> Vec<(Id, Body)> {
    items
        .iter()
        .filter_map(|&(id, _)| {
            let body = match &d.live(id)?.body.v {
                Body::Stroke { tool, color, width, xy, p } => Body::Stroke { tool: *tool, color: color.clone(), width: *width, xy: m.points(xy), p: p.clone() },
                Body::Image { asset, cx, cy, w, h, rotation } => {
                    let (cx, cy, w, h, rotation) = m.image(*cx, *cy, *w, *h, *rotation);
                    Body::Image { asset: *asset, cx, cy, w, h, rotation }
                }
                Body::Dropped => return None,
            };
            Some((id, body))
        })
        .collect()
}

/// The page's items with the preview in place of the originals.
fn preview_items<'a>(items: &PageItems<'a>, preview: &'a [(Id, Body)], images: &[Id]) -> PageItems<'a> {
    let strokes = items
        .strokes
        .iter()
        .map(|s| match s.key.and_then(|(id, _)| preview.iter().find(|p| p.0 == id)) {
            Some((_, Body::Stroke { tool, color, width, xy, .. })) => StrokeRef::new(*tool, color, *width, xy),
            _ => *s,
        })
        .collect();
    let images = items
        .images
        .iter()
        .zip(images.iter().copied().chain(std::iter::repeat(Id::NONE)))
        .map(|(r, id)| match preview.iter().find(|p| p.0 == id) {
            Some((_, Body::Image { cx, cy, w, h, rotation, .. })) => ImageRef { img: r.img, cx: *cx, cy: *cy, w: *w, h: *h, rotation: *rotation },
            _ => *r,
        })
        .collect();
    PageItems { covers: items.covers.clone(), images, strokes }
}

/// Shows `preview` on the page: the damaged rectangle (v5 `scheduleEraserRedraw`), or the whole page when images
/// move or the canvas has no complete paint (v5 `redrawPage(…, {duringErase: true})`).
fn paint_preview(app: &mut App, page: Id, preview: &[(Id, Body)], before: &[(Id, Body)]) {
    let Some(d) = doc(app) else { return };
    let Some((surf, px, scale, drawn)) = pages::active(app, page).map(|a| (a.surf, a.px, a.scale, a.drawn)) else { return };
    let images: Vec<Id> = d.draw_list(page).images.iter().filter(|r| is_image(d, r.id)).map(|r| r.id).collect();
    let moved_images = preview.iter().any(|p| matches!(p.1, Body::Image { .. }));
    let mut dmg = Damage::default();
    for (id, body) in preview {
        let old = before.iter().find(|b| b.0 == *id).map(|b| &b.1).or_else(|| d.live(*id).map(|i| &i.body.v));
        for b in [old, Some(body)].into_iter().flatten() {
            if let Body::Stroke { xy, width, .. } = b {
                dmg.add(geom::bounds(xy), *width);
            }
        }
    }
    pages::with_view(app, page, |g, view| {
        let items = preview_items(view.items, preview, &images);
        let v = PageView { w: view.w, h: view.h, bg: view.bg, items: &items };
        if moved_images || !drawn {
            rpage::repaint(&mut g.cmds, &mut g.tpl, &mut g.paths, surf, px, &v, scale);
        } else if let Some(r) = dmg.take() {
            damage::repaint(&mut g.cmds, &mut g.tpl, &mut g.paths, surf, &v, scale, r);
        }
    });
}

/// v5: `applySelectionPointerMove(e)` for the newest pointer position.
fn apply_move(app: &mut App, host: &mut dyn Host) {
    let Some(tf) = app.st.editor_selection.tf.as_mut() else { return };
    tf.armed = false;
    let Some((cx, cy)) = tf.pending.take() else { return };
    if view::read_only(app) {
        return;
    }
    let tf = app.st.editor_selection.tf.as_ref().expect("checked above");
    let pos = pages::to_page(tf.rect, tf.w, tf.h, cx, cy);
    let (c, b) = (tf.center, tf.bounds);
    let (mut tx, mut ty, mut sx, mut sy, mut rot) = (0.0, 0.0, 1.0, 1.0, 0.0);
    match tf.mode {
        Mode::Move => {
            tx = pos.0 - tf.start.0;
            ty = pos.1 - tf.start.1;
            let new_cy = c.1 + ty;
            let order = pages::order(app);
            let i = order.iter().position(|&p| p == tf.page).unwrap_or(0);
            let target = if new_cy < -CROSS_PAGE_PX && i > 0 {
                Some((order[i - 1], false))
            } else if new_cy > tf.h + CROSS_PAGE_PX && i + 1 < order.len() {
                Some((order[i + 1], true))
            } else {
                None
            };
            if let Some((target, down)) = target {
                cross(app, host, target, Affine::new(c, 1.0, 1.0, 0.0, tx, ty), pos, down);
                return;
            }
        }
        Mode::Scale(handle) => {
            let half_w = js_max(10.0, (b[2] - b[0]) / 2.0);
            let half_h = js_max(10.0, (b[3] - b[1]) / 2.0);
            let (dx, dy) = ((pos.0 - c.0) / half_w, (pos.1 - c.1) / half_h);
            match HANDLES[handle] {
                "ne" | "nw" | "se" | "sw" => {
                    let uniform = js_max(0.2, jsmath::hypot(pos.0 - c.0, pos.1 - c.1) / jsmath::hypot(half_w, half_h));
                    (sx, sy) = (uniform, uniform);
                }
                "e" | "w" => sx = js_max(0.2, dx.abs()),
                _ => sy = js_max(0.2, dy.abs()),
            }
        }
        Mode::Rotate => rot = jsmath::atan2(pos.1 - c.1, pos.0 - c.0) - tf.start_angle,
    }
    let m = Affine::new(c, sx, sy, rot, tx, ty);
    let page = tf.page;
    let Some(d) = doc(app) else { return };
    let preview = transformed(d, &app.st.editor_selection.items, &m);
    let tf = app.st.editor_selection.tf.as_mut().expect("checked above");
    let before = std::mem::replace(&mut tf.preview, Vec::new());
    tf.affine = Some(m);
    paint_preview(app, page, &preview, &before);
    if let Some(tf) = app.st.editor_selection.tf.as_mut() {
        tf.preview = preview;
    }
    pages::flush(app, host);
    update_box(app, host);
}

/// v5: the cross-page branch of `applySelectionPointerMove` + `moveSelectionToPage(from, to, yOffset)`: the
/// moved items land on top of `target`, shifted by the crossed page's height (B-E2), in one batch and one step.
fn cross(app: &mut App, host: &mut dyn Host, target: Id, m: Affine, pos: (f64, f64), down: bool) {
    let Some(tf) = app.st.editor_selection.tf.as_ref() else { return };
    let from = tf.page;
    if pages::active(app, target).is_none() {
        // v5: ensurePageCanvas(target) and wait for the next move.
        pages::schedule_visible(app, host, true);
        return;
    }
    let Some((th, tw_h)) = page_size(app, target).map(|s| (s.1, s)) else { return };
    let dy = if down { -tf.h } else { th };
    let Some(d) = doc(app) else { return };
    let ops: Vec<Op> = transformed(d, &app.st.editor_selection.items, &m)
        .into_iter()
        .map(|(item, body)| {
            let body = match body {
                Body::Stroke { tool, color, width, xy, p } => Body::Stroke { tool, color, width, xy: xy.chunks_exact(2).flat_map(|q| [q[0], q[1] + dy]).collect(), p },
                Body::Image { asset, cx, cy, w, h, rotation } => Body::Image { asset, cx, cy: cy + dy, w, h, rotation },
                Body::Dropped => Body::Dropped,
            };
            Op::ItemPut { item, page: target, z: Z::TOP, body }
        })
        .collect();
    if ops.is_empty() || !write(app, host, from, 0, |tx| ops.into_iter().for_each(|op| tx.push(op))) {
        return;
    }
    // v5 hides the old box (its overlay keeps the outlines drawn last) and redraws both pages.
    if let Some(k) = kids(app, host, from) {
        let boxn = k.boxn;
        let out = host.out();
        out.class(boxn, "hidden", true);
        out.class(boxn, "has-selection", false);
        out.class(boxn, "has-image", false);
    }
    repaint(app, host, from);
    repaint(app, host, target);
    app.st.editor_selection.page = target;
    app.st.editor_pages.selection_page = target;
    refresh_stamps(app);
    let items = live_ids(app);
    let bounds = doc(app).and_then(|d| d.selection_bounds(&items));
    let wrapper = app.st.editor_pages.rows.iter().find(|r| r.id == target).map(|r| r.wrapper);
    let rect = wrapper.map(|w| host.rect(w));
    if let Some(tf) = app.st.editor_selection.tf.as_mut() {
        tf.page = target;
        tf.preview.clear();
        tf.affine = None;
        (tf.w, tf.h) = tw_h;
        if let Some(r) = rect {
            tf.rect = r;
        }
        if let Some(b) = bounds {
            tf.bounds = b;
            tf.center = (b[0] + (b[2] - b[0]) / 2.0, b[1] + (b[3] - b[1]) / 2.0);
            tf.start = (pos.0, pos.1 + dy);
        }
    }
    update_box(app, host);
}

/// v5: `onSelectionPointerUp(e)`: the transform shown becomes one batch and one undo step.
fn end(app: &mut App, host: &mut dyn Host) {
    if view::read_only(app) {
        abandon(app, host);
        return;
    }
    apply_move(app, host);
    let Some(tf) = stop(app, host) else { return };
    let Some(m) = tf.affine else { return };
    let page = tf.page;
    let items = live_ids(app);
    let changed = doc(app).is_some_and(|d| tf.preview.iter().any(|(id, b)| d.live(*id).is_some_and(|i| i.body.v != *b)));
    if changed && write(app, host, page, 0, |tx| tx.transform(&items, m)) {
        // The bitmap already shows the transform: only the previews are stale.
        pages::touch(app, page);
        if let Some(r) = app.st.editor_pages.rows.iter_mut().find(|r| r.id == page) {
            if r.active.as_ref().is_some_and(|a| a.drawn) {
                r.needs_redraw = false;
            }
        }
        refresh_stamps(app);
    } else {
        repaint(app, host, page);
    }
    update_box(app, host);
}

/// v5: the selection box's `pointerdown` (`setupSelectionBoxEvents`).
fn box_down(app: &mut App, host: &mut dyn Host, p: &PointerBatch) -> u32 {
    let page = app.st.editor_selection.page;
    if page == Id::NONE || view::read_only(app) || app.st.editor_input.tool != input::Tool::Lasso || app.st.editor_selection.tf.is_some() {
        return 0;
    }
    let Some(k) = app.st.editor_selection.kids.as_ref().filter(|k| pages::active(app, page).is_some_and(|a| a.selection_box == k.boxn)) else { return 0 };
    if !p.path.iter().any(|n| n == k.boxn) {
        return 0;
    }
    let has = |n: Node| p.path.iter().any(|x| x == n);
    let first = p.path.iter().next();
    let menu = k.menu;
    if let Some(i) = k.swatches.iter().position(|&s| has(s)) {
        recolor(app, host, pages::INK_PALETTE[i]);
        host.out().class(menu, "open", false);
        app.st.editor_selection.menu_open = false;
        return ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION;
    }
    if has(k.color_btn) {
        let st = &mut app.st.editor_selection;
        st.menu_open = !st.menu_open;
        host.out().class(menu, "open", st.menu_open);
        return ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION;
    }
    let (dup, trash, rotate, handles) = (has(k.duplicate), has(k.trash), first == Some(k.rotate), k.handles.iter().position(|&h| first == Some(h)));
    host.out().class(menu, "open", false);
    app.st.editor_selection.menu_open = false;
    if dup {
        duplicate(app, host);
    } else if trash {
        delete(app, host);
    } else if rotate {
        start(app, host, Mode::Rotate, p);
    } else if let Some(h) = handles {
        start(app, host, Mode::Scale(h), p);
    } else {
        start(app, host, Mode::Move, p);
    }
    ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION
}

// ---- the lasso tool ---------------------------------------------------------------------------------------------

/// The lasso [`ToolHandler`] (v5: the `isLassoing` branches of `setupCanvasEvents`).
pub struct Lasso;

fn draw_lasso(app: &mut App, host: &mut dyn Host) {
    let st = &app.st.editor_selection;
    let page = st.lasso_page;
    if !st.lasso_active || st.drawn == st.lasso.len() / 2 {
        return;
    }
    let Some(a) = pages::active(app, page) else { return };
    let (surf, px, scale) = (a.lasso_surf, a.lasso_px, a.lasso_scale);
    pages::with_gfx(app, |app, g| overlay::lasso_path(&mut g.cmds, surf, px, scale, &app.st.editor_selection.lasso));
    pages::flush(app, host);
    pages::overlay_ink(app, host, page, 0, true);
    let st = &mut app.st.editor_selection;
    st.drawn = st.lasso.len() / 2;
}

impl ToolHandler for Lasso {
    fn down(&self, app: &mut App, host: &mut dyn Host, x: f64, y: f64, _pressure: f64) {
        let Some(page) = input::contact(app).map(|c| c.page) else { return };
        clear(app, host);
        let st = &mut app.st.editor_selection;
        st.lasso_active = true;
        st.lasso_page = page;
        st.lasso.clear();
        st.lasso.extend_from_slice(&[x, y]);
        st.drawn = 0;
        draw_lasso(app, host);
    }

    // v5: one point per dispatched event (the release adds none); drawn once per frame here.
    fn moved(&self, app: &mut App, host: &mut dyn Host, p: &PointerBatch) {
        if p.kind == ptr::UP || !app.st.editor_selection.lasso_active {
            return;
        }
        let Some(c) = input::contact(app) else { return };
        let (x, y) = c.page_xy(&p.raw);
        let st = &mut app.st.editor_selection;
        st.lasso.push(x);
        st.lasso.push(y);
        if !std::mem::replace(&mut st.frame_armed, true) {
            app.sched.frame(host.out(), Mod::EditorSelection);
        }
    }

    fn end(&self, app: &mut App, host: &mut dyn Host) {
        let st = &mut app.st.editor_selection;
        if !std::mem::replace(&mut st.lasso_active, false) {
            return;
        }
        let page = st.lasso_page;
        let found = if st.lasso.len() / 2 > 2 { doc(app).map(|d| d.lasso(page, &app.st.editor_selection.lasso)).unwrap_or_default() } else { Vec::new() };
        app.st.editor_selection.lasso.clear();
        clear_lasso(app, host, page);
        if found.is_empty() {
            clear(app, host);
        } else {
            select(app, host, page, &found);
        }
    }

    fn discard(&self, app: &mut App, host: &mut dyn Host) {
        let st = &mut app.st.editor_selection;
        if std::mem::replace(&mut st.lasso_active, false) {
            st.lasso.clear();
            let page = st.lasso_page;
            clear_lasso(app, host, page);
        }
    }
}

// ---- module contract --------------------------------------------------------------------------------------------

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(app: &mut App, host: &mut dyn Host) {
    if app.cfg.has(crate::abi::mode::HEADLESS) {
        return;
    }
    host.out().listen(Ui::CANVAS_CONTAINER, "pointerdown", L_BOX, 0);
}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Pointer(p) => match p.listener {
            L_BOX if p.kind == ptr::DOWN => box_down(app, host, &p),
            L_DOC_MOVE | L_DOC_UP | L_DOC_CANCEL => {
                let Some(tf) = app.st.editor_selection.tf.as_mut().filter(|t| t.pointer_id == p.pointer_id) else { return 0 };
                tf.pending = Some((p.raw.x, p.raw.y));
                if p.listener != L_DOC_MOVE {
                    end(app, host);
                    return 0;
                }
                if !std::mem::replace(&mut tf.armed, true) {
                    app.sched.frame(host.out(), Mod::EditorSelection);
                }
                ret::PREVENT_DEFAULT
            }
            _ => 0,
        },
        Event::Frame(_) => {
            app.st.editor_selection.frame_armed = false;
            draw_lasso(app, host);
            if app.st.editor_selection.tf.as_ref().is_some_and(|t| t.pending.is_some()) {
                apply_move(app, host);
            }
            0
        }
        Event::Msg(m) => {
            match m.chan {
                session::sig::CLOSED => {
                    abandon(app, host);
                    let st = &mut app.st.editor_selection;
                    (st.page, st.lasso_active) = (Id::NONE, false);
                    st.items.clear();
                    app.st.editor_pages.selection_page = Id::NONE;
                }
                // A selected item changed or vanished through another path (undo/redo, another device): 5.18.0
                // clears the selection after its undo of a selection action; a remote change is followed.
                session::sig::STATE | session::sig::CHANGED => {
                    let st = &app.st.editor_selection;
                    if st.editing || st.tf.is_some() || st.page == Id::NONE {
                        return 0;
                    }
                    let page = st.page;
                    let stale = doc(app).is_none_or(|d| st.items.iter().any(|&(id, at)| !d.live(id).is_some_and(|i| i.page.v == page && i.body.at == at)));
                    if stale {
                        clear(app, host);
                    }
                }
                _ => {}
            }
            0
        }
        _ => 0,
    }
}

/// `__IHN_TEST_API__.cmd('editor.selection.state')` → page index, selected strokes and images, bounds, whether a
/// drag is running, batches written.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    match name {
        "state" => {
            let st = &app.st.editor_selection;
            let page = pages::order(app).iter().position(|&p| p == st.page).map_or(-1, |i| i as i64);
            let (images, strokes) = doc(app).map_or((0, 0), |d| {
                let n = st.items.iter().filter(|i| is_image(d, i.0)).count();
                (n, st.items.len() - n)
            });
            let b = shown_bounds(app).map_or("null".to_string(), |(b, ..)| format!("[{},{},{},{}]", num(b[0]), num(b[1]), num(b[2]), num(b[3])));
            Some(format!(
                "{{\"page\":{page},\"strokes\":{strokes},\"images\":{images},\"bounds\":{b},\"transforming\":{},\"lasso\":{},\"commits\":{}}}",
                st.tf.is_some(),
                st.lasso.len() / 2,
                st.commits
            ))
        }
        _ => None,
    }
}

/// The selected item ids (images first, then strokes, in draw order).
pub fn selected(app: &App) -> Vec<Id> { ids(app) }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{kind, mode, Config};
    use crate::host::FakeHost;
    use ihn_core::codec::Writer;
    use ihn_core::op::{Background, Template, Tool};

    const CANVAS: Rect = Rect { x: 100.0, y: 50.0, w: 794.0, h: 1123.0 };

    struct T {
        app: App,
        host: FakeHost,
        pages: Vec<Id>,
    }

    fn boot(sizes: &[(u32, u32)]) -> T {
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
            for &s in sizes {
                after = Some(tx.page_new(after, Template::Default, s, Background::Template));
            }
        })
        .expect("a memory session is writable");
        shell::show_view(&mut app, &mut host, shell::View::Editor);
        shell::signal(&mut app, &mut host, session::sig::OPENED, "d");
        host.advance(500.0);
        let t = host.now;
        app.frame(&mut host, t);
        view::set_read_only(&mut app, &mut host, false, false);
        let pages = session::get(&app).unwrap().doc.order().to_vec();
        let mut top = CANVAS.y;
        for (i, &p) in pages.iter().enumerate() {
            let (w, h) = (sizes[i].0 as f64, sizes[i].1 as f64);
            // Rows touch (no gap): a crossing keeps the pointer's place on the next page.
            let r = Rect { x: CANVAS.x, y: top, w, h };
            top += h;
            if let Some(a) = pages::active(&app, p) {
                host.rects.insert(a.canvas, r);
            }
            let wrapper = app.st.editor_pages.rows.iter().find(|r| r.id == p).unwrap().wrapper;
            host.rects.insert(wrapper, r);
        }
        host.take();
        T { app, host, pages }
    }

    fn stroke(t: &mut T, page: Id, tool: Tool, xy: &[f64]) -> Id {
        let mut id = Id::NONE;
        session::edit(&mut t.app, &mut t.host, |tx| id = tx.add_stroke(page, Body::Stroke { tool, color: Color::parse("#002FD9"), width: 2.0, xy: xy.to_vec(), p: None })).unwrap();
        id
    }

    fn send(t: &mut T, listener: u32, k: u8, x: f64, y: f64, path: &[Node]) -> u32 {
        let mut w = Writer::default();
        w.u32(listener);
        w.u32(Ui::CANVAS_CONTAINER.0);
        w.u8(k);
        w.u8(ptr::PEN);
        w.i32(1);
        w.i32(0);
        w.u32(0);
        for v in [x, y, t.host.now] {
            w.f64(v);
        }
        w.f32(if k == ptr::UP { 0.0 } else { 0.5 });
        w.f32(0.0);
        w.f32(0.0);
        w.u32(if k == ptr::UP { 0 } else { 1 });
        w.u32(0);
        w.u32(path.len() as u32);
        for n in path {
            w.u32(n.0);
        }
        t.host.advance(8.0);
        t.app.event(&mut t.host, kind::POINTER, &w.buf)
    }

    fn frame(t: &mut T) {
        t.host.advance(16.0);
        let now = t.host.now;
        t.app.frame(&mut t.host, now);
    }

    /// A lasso around page-1 rectangle (x0, y0)–(x1, y1), drawn with the pen.
    fn lasso(t: &mut T, x0: f64, y0: f64, x1: f64, y1: f64) {
        input::set_tool(&mut t.app, &mut t.host, Some(input::Tool::Lasso), None, None);
        let canvas = pages::active(&t.app, t.pages[0]).unwrap().canvas;
        let l_down = tag(Mod::EditorInput, 1);
        let (l_move, l_up) = (tag(Mod::EditorInput, 2), tag(Mod::EditorInput, 3));
        let c = |x: f64, y: f64| (x + CANVAS.x, y + CANVAS.y);
        let (x, y) = c(x0, y0);
        send(t, l_down, ptr::DOWN, x, y, &[canvas]);
        for (px, py) in [(x1, y0), (x1, y1), (x0, y1)] {
            let (x, y) = c(px, py);
            send(t, l_move, ptr::MOVE, x, y, &[]);
        }
        frame(t);
        let (x, y) = c(x0, y1);
        send(t, l_up, ptr::UP, x, y, &[]);
    }

    fn kid(t: &T, f: impl Fn(&Kids) -> Node) -> Node { f(t.app.st.editor_selection.kids.as_ref().expect("box children")) }

    /// Drags the selection box from page-1 (x0, y0) to (x1, y1) holding `node`.
    fn drag(t: &mut T, node: Node, from: (f64, f64), to: (f64, f64)) {
        let boxn = kid(t, |k| k.boxn);
        let path: Vec<Node> = if node == boxn { vec![boxn] } else { vec![node, boxn] };
        send(t, L_BOX, ptr::DOWN, from.0 + CANVAS.x, from.1 + CANVAS.y, &path);
        send(t, L_DOC_MOVE, ptr::MOVE, to.0 + CANVAS.x, to.1 + CANVAS.y, &[]);
        frame(t);
        send(t, L_DOC_UP, ptr::UP, to.0 + CANVAS.x, to.1 + CANVAS.y, &[]);
    }

    fn body(t: &T, id: Id) -> Body { session::get(&t.app).unwrap().doc.live(id).unwrap().body.v.clone() }
    fn xy(t: &T, id: Id) -> Vec<f64> {
        match body(t, id) {
            Body::Stroke { xy, .. } => xy,
            _ => panic!("not a stroke"),
        }
    }
    fn live(t: &T) -> usize { session::get(&t.app).unwrap().doc.order().iter().map(|&p| session::get(&t.app).unwrap().doc.page_items(p).len()).sum() }

    #[test]
    fn lasso_selects_strokes_with_a_point_inside() {
        let mut t = boot(&[(794, 1123)]);
        let page = t.pages[0];
        let a = stroke(&mut t, page, Tool::Pen, &[200.0, 200.0, 260.0, 220.0]);
        let _b = stroke(&mut t, page, Tool::Pen, &[600.0, 600.0, 650.0, 620.0]);
        lasso(&mut t, 150.0, 150.0, 300.0, 300.0);
        assert_eq!(t.app.st.editor_selection.page, page);
        assert_eq!(selected(&t.app), vec![a]);
        assert_eq!(t.app.st.editor_pages.selection_page, page);
        let s = test_cmd(&mut t.app, &mut t.host, "state", "").unwrap();
        assert!(s.contains("\"strokes\":1") && s.contains("[200,200,260,220]"), "{s}");
        // A lasso around nothing clears it; a pen down clears it too.
        lasso(&mut t, 400.0, 900.0, 450.0, 950.0);
        assert_eq!(t.app.st.editor_selection.page, Id::NONE);
    }

    #[test]
    fn move_scale_rotate_write_one_batch_each() {
        let mut t = boot(&[(794, 1123)]);
        let page = t.pages[0];
        let a = stroke(&mut t, page, Tool::Pen, &[200.0, 200.0, 260.0, 220.0]);
        lasso(&mut t, 150.0, 150.0, 300.0, 300.0);
        let (boxn, before) = (kid(&t, |k| k.boxn), live(&t));
        drag(&mut t, boxn, (230.0, 210.0), (330.0, 260.0));
        assert_eq!(xy(&t, a), vec![300.0, 250.0, 360.0, 270.0]);
        assert_eq!(t.app.st.editor_selection.commits, 1);
        assert_eq!(history::depth(&t.app), (1, 1), "one undo step");
        assert_eq!(live(&t), before);
        // Corner scale about the centre: twice the distance.
        let se = kid(&t, |k| k.handles[6]);
        drag(&mut t, se, (360.0, 270.0), (390.0, 280.0));
        assert_eq!(xy(&t, a), vec![270.0, 240.0, 390.0, 280.0]);
        // Rotate a half turn around the centre (330, 260).
        let rot = kid(&t, |k| k.rotate);
        drag(&mut t, rot, (330.0, 200.0), (330.0, 320.0));
        let p = xy(&t, a);
        assert!((p[0] - 390.0).abs() < 1e-9 && (p[1] - 280.0).abs() < 1e-9, "{p:?}");
        assert_eq!(t.app.st.editor_selection.commits, 3);
        // Undo returns to the scaled stroke, the selection is cleared (its item changed elsewhere).
        history::undo(&mut t.app, &mut t.host);
        assert_eq!(xy(&t, a), vec![270.0, 240.0, 390.0, 280.0]);
        assert_eq!(t.app.st.editor_selection.page, Id::NONE);
    }

    #[test]
    fn preview_until_the_lift() {
        let mut t = boot(&[(794, 1123)]);
        let page = t.pages[0];
        let a = stroke(&mut t, page, Tool::Pen, &[200.0, 200.0, 260.0, 220.0]);
        lasso(&mut t, 150.0, 150.0, 300.0, 300.0);
        let boxn = kid(&t, |k| k.boxn);
        send(&mut t, L_BOX, ptr::DOWN, 330.0, 260.0, &[boxn]);
        send(&mut t, L_DOC_MOVE, ptr::MOVE, 340.0, 270.0, &[]);
        frame(&mut t);
        assert_eq!(xy(&t, a), vec![200.0, 200.0, 260.0, 220.0], "the model waits for the lift");
        let s = test_cmd(&mut t.app, &mut t.host, "state", "").unwrap();
        assert!(s.contains("[210,210,270,230]") && s.contains("\"transforming\":true"), "{s}");
        send(&mut t, L_DOC_UP, ptr::UP, 340.0, 270.0, &[]);
        assert_eq!(xy(&t, a), vec![210.0, 210.0, 270.0, 230.0]);
    }

    #[test]
    fn duplicate_recolor_delete() {
        let mut t = boot(&[(420, 595)]);
        let page = t.pages[0];
        let a = stroke(&mut t, page, Tool::Pen, &[300.0, 500.0, 405.0, 580.0]);
        lasso(&mut t, 250.0, 450.0, 419.0, 594.0);
        let dup = kid(&t, |k| k.duplicate);
        let boxn = kid(&t, |k| k.boxn);
        send(&mut t, L_BOX, ptr::DOWN, 0.0, 0.0, &[dup, boxn]);
        // Clamped to the A5 page (B-E1): x by 420 − 6 − 405 = 9, y by 595 − 6 − 580 = 9.
        let copy = selected(&t.app)[0];
        assert_ne!(copy, a);
        assert_eq!(xy(&t, copy), vec![309.0, 509.0, 414.0, 589.0]);
        let red = kid(&t, |k| k.swatches[2]);
        let btn = kid(&t, |k| k.color_btn);
        send(&mut t, L_BOX, ptr::DOWN, 0.0, 0.0, &[red, btn, boxn]);
        assert!(matches!(body(&t, copy), Body::Stroke { color, .. } if color == Color::parse("#E81010")));
        assert_eq!(selected(&t.app), vec![copy], "a recolour keeps the selection");
        let trash = kid(&t, |k| k.trash);
        send(&mut t, L_BOX, ptr::DOWN, 0.0, 0.0, &[trash, boxn]);
        assert!(session::get(&t.app).unwrap().doc.live(copy).is_none());
        assert_eq!(t.app.st.editor_selection.page, Id::NONE);
        assert_eq!(history::depth(&t.app).1, 3);
    }

    #[test]
    fn cross_page_drag_lands_on_top_shifted_by_the_real_height() {
        let mut t = boot(&[(420, 595), (420, 595)]);
        let (p0, p1) = (t.pages[0], t.pages[1]);
        let old = stroke(&mut t, p0, Tool::Pen, &[200.0, 500.0, 260.0, 520.0]);
        let hl = stroke(&mut t, p1, Tool::Highlighter, &[100.0, 50.0, 300.0, 60.0]);
        lasso(&mut t, 150.0, 450.0, 300.0, 560.0);
        let boxn = kid(&t, |k| k.boxn);
        // Centre y 510 → 510 + 140 = 650 > 595 + 50: the items go to page 2, y − 595.
        drag(&mut t, boxn, (230.0, 510.0), (230.0, 650.0));
        let d = &session::get(&t.app).unwrap().doc;
        assert_eq!(d.live(old).unwrap().page.v, p1);
        assert_eq!(xy(&t, old), vec![200.0, 45.0, 260.0, 65.0]);
        let order = d.page_items(p1);
        assert_eq!(order, vec![hl, old], "the moved stroke draws over the newer highlighter");
        assert_eq!(t.app.st.editor_selection.page, p1);
        history::undo(&mut t.app, &mut t.host);
        assert_eq!(session::get(&t.app).unwrap().doc.live(old).unwrap().page.v, p0);
        assert_eq!(xy(&t, old), vec![200.0, 500.0, 260.0, 520.0]);
    }
}
