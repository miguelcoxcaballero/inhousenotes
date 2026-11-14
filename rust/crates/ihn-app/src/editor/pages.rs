//! Editor: Page DOM rows, activation, canvas resolution sync, previews (ARCHITECTURE §6.1).
//!
//! Every page has a `.page-row > .page-wrapper` (5.18.0 `renderAllPages` / `createPageElement`); at most three
//! pages near the viewport are *active*: they carry `canvas.page-canvas`, the three overlays and the selection box
//! (`attachPageCanvases`), sized with 5.18.0's backing-store formulas. Inactive pages show a JPEG preview (or the
//! template URL) as the wrapper's CSS background. The canvas command encoder, the template canvases and the
//! `Path2D` cache of the whole app live here ([`Gfx`]); the ink tools draw through [`with_gfx`] and [`flush`].
//!
//! v6 keeps the whole document in memory, so a page is always "loaded": `ensurePageDataLoaded` and
//! `unloadPageData` have no counterpart; previews still replace released canvases, as in 5.18.0.

use super::{layout, view, viewport};
use crate::abi::{self, ret};
use crate::doc::session;
use crate::shell::{self, Status, StatusOpts};
use crate::{tag, App, Event, Host, Mod, Node, Rect, Ui};
use ihn_core::doc::Doc;
use ihn_core::geom::{js_max, js_min};
use ihn_core::id::{Hash, Id};
use ihn_core::jsfmt::num;
use ihn_core::op::{Background, Template};
use ihn_core::render::cmd::{Cmds, Img, Surf, REC_BATCH};
use ihn_core::render::page::{self as rpage, Bg, PageView, Staged};
use ihn_core::render::template::{self, BgUrls, Templates};
use ihn_core::render::thumb::{self, ScrollPreview};
use ihn_core::render::{PageItems, PathCache};

// v5: app-v5.js constants
const CANVAS_SCALE: f64 = 2.0;
const CANVAS_SCALE_STEP: f64 = 0.25;
const PAGE_ACTIVATION_IDLE_TIMEOUT: f64 = 120.0;
/// Strokes per staged-repaint slice and per scroll-preview slice (5.18.0 measured 4 ms / 3 ms; the commands run
/// after the Rust call returns, so slices are counted).
const STAGED_SLICE: usize = 160;
const PREVIEW_SLICE: usize = 160;

const L_WRAPPER: u32 = tag(Mod::EditorPages, 1);
const T_VISIBLE_RETRY: u32 = tag(Mod::EditorPages, 2);
const T_ACTIVATE: u32 = tag(Mod::EditorPages, 3);
const I_ACTIVATE: u32 = tag(Mod::EditorPages, 4);
const T_STAGED: u32 = tag(Mod::EditorPages, 5);
const T_PREVIEW: u32 = tag(Mod::EditorPages, 6);
const T_NEW_PAGE: u32 = tag(Mod::EditorPages, 7);
const T_REPAINT: u32 = tag(Mod::EditorPages, 8);

/// v5: `SELECTION_ALLOWED_COLORS` (= `ink-palette.js`).
pub const INK_PALETTE: [&str; 3] = ["#4D4D4D", "#002FD9", "#E81010"];

/// The canvases of an active page (v5 `attachPageCanvases`).
pub struct Active {
    pub canvas: Node,
    pub lasso: Node,
    pub cursor: Node,
    pub remote: Node,
    pub selection_box: Node,
    pub surf: Surf,
    pub lasso_surf: Surf,
    pub cursor_surf: Surf,
    pub remote_surf: Surf,
    /// Backing size of the page canvas and its `dataset.renderScale`.
    pub px: (u32, u32),
    pub scale: f64,
    pub lasso_px: (u32, u32),
    pub lasso_scale: f64,
    pub cursor_px: (u32, u32),
    pub cursor_scale: f64,
    /// v5 `dataset.hasPageDraw === '1'`.
    pub drawn: bool,
    /// v5 `overlay.__inkHidden` of lasso / cursor / remote.
    pub ink_hidden: [bool; 3],
}

/// One page row.
pub struct Row {
    pub id: Id,
    pub row: Node,
    pub wrapper: Node,
    pub w: f64,
    pub h: f64,
    pub active: Option<Active>,
    pub needs_redraw: bool,
    /// v5 `page.preview` (JPEG data URL) and `scrollPreviewDirty`.
    preview: Option<String>,
    preview_dirty: bool,
    /// The wrapper's current CSS background (to skip identical writes).
    wrapper_bg: String,
    /// Bumped whenever the page's items or background change (v5 `pageDirtyGeneration`).
    gen: u32,
    preview_failed_at: f64,
}

/// A decoded asset image (and its blob URL for CSS backgrounds).
struct Picture {
    hash: Hash,
    img: Img,
    req: u32,
    ready: bool,
    url: Option<String>,
    url_req: u32,
}

/// Canvas state shared by the editor: one encoder, the template canvases and the `Path2D` cache.
pub struct Gfx {
    pub cmds: Cmds,
    pub tpl: Templates,
    pub paths: PathCache,
    pub urls: BgUrls,
    /// Template CSS JPEG data URLs by kind (v5 `_templateBgUrlCache`) and their pending requests.
    tpl_urls: [Option<String>; 3],
    tpl_url_req: [u32; 3],
    pictures: Vec<Picture>,
    /// PDF page rasters (`doc::backgrounds`): page → (blob URL, image, decoded).
    rasters: Vec<(Id, String, Img, u32, bool)>,
}

struct StagedJob {
    page: Id,
    gen: u32,
    px: (u32, u32),
    staged: Staged,
    done: bool,
}

struct PreviewJob {
    page: Id,
    gen: u32,
    sp: ScrollPreview,
    background_done: bool,
}

#[derive(Default)]
pub struct State {
    gfx: Option<Box<Gfx>>,
    pub rows: Vec<Row>,
    /// v5 `state.activePageIndex` (an id here: indices move).
    active_page: Id,
    desired: Vec<Id>,
    pending: Vec<(Id, usize)>,
    activation_armed: bool,
    visible_frame: bool,
    visible_force: bool,
    retry_armed: bool,
    repaint_armed: bool,
    staged: Option<StagedJob>,
    previews: Vec<(Id, usize)>,
    preview_job: Option<PreviewJob>,
    preview_armed: bool,
    /// Requests of preview data URLs: req → page.
    preview_reqs: Vec<(u32, Id, u32)>,
    new_pages: Vec<Id>,
    is_pc: Option<bool>,
    /// The page holding the lasso selection (set by `editor::selection`): it keeps its canvases first.
    pub selection_page: Id,
}

// ---- graphics ---------------------------------------------------------------------------------------------------

fn new_gfx(app: &mut App) -> Box<Gfx> {
    let (a, d) = (app.req(Mod::EditorPages), app.req(Mod::EditorPages));
    Box::new(Gfx {
        cmds: Cmds::new(),
        tpl: Templates::new(a, d),
        paths: PathCache::default(),
        urls: BgUrls::default(),
        tpl_urls: [None, None, None],
        tpl_url_req: [0; 3],
        pictures: Vec::new(),
        rasters: Vec::new(),
    })
}

/// Runs `f` with the editor's canvas state (the encoder, templates, path cache), the rest of the app borrowable.
pub fn with_gfx<R>(app: &mut App, f: impl FnOnce(&mut App, &mut Gfx) -> R) -> R {
    let mut gfx = match app.st.editor_pages.gfx.take() {
        Some(g) => g,
        None => new_gfx(app),
    };
    let r = f(app, &mut gfx);
    app.st.editor_pages.gfx = Some(gfx);
    r
}

/// Sends the canvas commands encoded so far as one record (they run after the DOM records written before them).
pub fn flush(app: &mut App, host: &mut dyn Host) {
    let Some(g) = app.st.editor_pages.gfx.as_mut() else { return };
    if g.cmds.is_empty() {
        return;
    }
    let bytes = g.cmds.take();
    let out = host.out();
    let m = out.w.rec(abi::fam::CANVAS, REC_BATCH);
    out.w.raw(&bytes);
    out.w.end(m);
}

/// v5: `detectDeviceType() === 'pc'` (app-v5.js:24020): tablets and phones use the touch budgets.
pub fn is_pc(app: &mut App) -> bool {
    if let Some(v) = app.st.editor_pages.is_pc {
        return v;
    }
    let ua = &app.cfg.user_agent;
    let has = |s: &str| ua.to_ascii_lowercase().contains(&s.to_ascii_lowercase());
    let android = has("android");
    let tablet = (android && !has("mobile")) || has("ipad") || has("tablet") || has("playbook") || (app.cfg.max_touch_points > 1 && has("macintosh") && app.cfg.viewport.0 >= 768.0);
    let mobile = has("mobi") || (android && has("mobile")) || has("iphone") || has("ipod") || has("blackberry") || has("iemobile") || has("opera mini");
    let pc = !tablet && !mobile;
    app.st.editor_pages.is_pc = Some(pc);
    pc
}

/// v5: `getTargetPageCanvasScale(w, h, zoom)`.
pub fn target_scale(w: f64, h: f64, zoom: f64, pc: bool) -> f64 {
    let (w, h) = (w.max(1.0), h.max(1.0));
    let zoom = if zoom.is_finite() { zoom.max(0.5) } else { 1.0 };
    let desired = CANVAS_SCALE * zoom.max(1.0);
    let (max_scale, max_pixels) = if pc { (6.0, 28.0 * 1024.0 * 1024.0) } else { (4.5, 16.0 * 1024.0 * 1024.0) };
    let bound = (max_pixels / (w * h)).sqrt();
    let capped = js_max(CANVAS_SCALE, js_min(js_min(desired, max_scale), bound));
    let q = (capped / CANVAS_SCALE_STEP).floor() * CANVAS_SCALE_STEP;
    js_max(CANVAS_SCALE, if q == 0.0 || q.is_nan() { CANVAS_SCALE } else { q })
}

/// v5: `getTargetPageCanvasBackingSize(w, h, zoom)`: backing pixels and the applied scale.
pub fn backing(w: f64, h: f64, zoom: f64, pc: bool) -> ((u32, u32), f64) {
    let scale = target_scale(w, h, zoom, pc);
    let tw = js_max(1.0, ihn_core::jsmath::round(w * scale));
    let applied = tw / w;
    let th = js_max(1.0, ihn_core::jsmath::round(h * applied));
    ((tw as u32, th as u32), applied)
}

/// v5: `getCursorOverlayScale(zoom)` / `getLassoOverlayScale(zoom)`.
pub fn overlay_scale(zoom: f64, lasso: bool) -> f64 {
    let z = if zoom.is_finite() { zoom.max(1.0) } else { 1.0 };
    let (base, max, k) = if lasso { (1.35, 2.2, 0.45) } else { (1.25, 1.8, 0.35) };
    js_max(base, js_min(max, base + (z - 1.0) * k))
}

fn overlay_px(w: f64, h: f64, s: f64) -> (u32, u32) {
    (js_max(1.0, ihn_core::jsmath::round(w * s)) as u32, js_max(1.0, ihn_core::jsmath::round(h * s)) as u32)
}

fn kind_index(k: Template) -> usize { k as usize }

// ---- rows -------------------------------------------------------------------------------------------------------

fn doc(app: &App) -> Option<&Doc> { session::get(app).map(|s| &s.doc) }

fn page_bg(app: &App, id: Id) -> Background { doc(app).and_then(|d| d.pages.get(&id)).map_or(Background::Template, |p| p.background.v) }
fn page_template(app: &App, id: Id) -> Template { doc(app).and_then(|d| d.pages.get(&id)).map_or(Template::Default, |p| p.template.v) }

fn index(app: &App, id: Id) -> Option<usize> { app.st.editor_pages.rows.iter().position(|r| r.id == id) }
fn row(app: &App, id: Id) -> Option<&Row> { app.st.editor_pages.rows.iter().find(|r| r.id == id) }
fn row_mut(app: &mut App, id: Id) -> Option<&mut Row> { app.st.editor_pages.rows.iter_mut().find(|r| r.id == id) }

/// The active canvases of a page.
pub fn active(app: &App, id: Id) -> Option<&Active> { row(app, id).and_then(|r| r.active.as_ref()) }
/// Page ids in display order.
pub fn order(app: &App) -> Vec<Id> { app.st.editor_pages.rows.iter().map(|r| r.id).collect() }
/// The page under a wrapper or canvas node.
pub fn page_of_node(app: &App, node: Node) -> Option<Id> {
    app.st.editor_pages.rows.iter().find(|r| r.wrapper == node || r.active.as_ref().is_some_and(|a| a.canvas == node)).map(|r| r.id)
}
/// v5: `state.activePageIndex = i`.
pub fn set_active_page(app: &mut App, i: usize) {
    if let Some(r) = app.st.editor_pages.rows.get(i) {
        app.st.editor_pages.active_page = r.id;
    }
}

/// Client → page coordinates for a rect measured once per contact (v5 `getCanvasPointFromClient`).
pub fn to_page(rect: Rect, w: f64, h: f64, x: f64, y: f64) -> (f64, f64) { ((x - rect.x) * w / rect.w, (y - rect.y) * h / rect.h) }

/// The page canvas rect (one layout read; cache it for the gesture).
pub fn page_rect(app: &App, host: &mut dyn Host, id: Id) -> Option<Rect> {
    let a = active(app, id)?;
    Some(host.rect(a.canvas))
}

fn sync_layout(app: &mut App) {
    let dims = app.st.editor_pages.rows.iter().map(|r| layout::Dim { id: r.id, w: r.w, h: r.h }).collect();
    layout::set_pages(app, dims);
    viewport::forget_geometry(app);
}

fn row_html(i: usize, w: f64, h: f64) -> String {
    format!(
        "<div class=\"page-row\" data-page-row=\"{i}\"><div class=\"page-wrapper\" data-page=\"{i}\" style=\"width: {}px; height: {}px;\"><div class=\"page-number\">{}</div></div></div>",
        num(w),
        num(h),
        i + 1
    )
}

fn make_row(app: &mut App, host: &mut dyn Host, id: Id, i: usize, animate: bool) -> Row {
    let (w, h) = doc(app).and_then(|d| d.pages.get(&id)).map_or((layout::A4_WIDTH, layout::A4_HEIGHT), |p| (p.size.v.0 as f64, p.size.v.1 as f64));
    let before = viewport::indicator(app);
    let out = host.out();
    let row = out.insert_html(Ui::CANVAS_CONTAINER, before, &row_html(i, w, h));
    let wrapper = out.child(row, &[0]);
    if animate {
        out.class(wrapper, "new-page", true);
    }
    out.listen(wrapper, "pointerdown", L_WRAPPER, 0);
    Row { id, row, wrapper, w, h, active: None, needs_redraw: true, preview: None, preview_dirty: false, wrapper_bg: String::new(), gen: 0, preview_failed_at: f64::NEG_INFINITY }
}

fn drop_row(app: &mut App, host: &mut dyn Host, mut r: Row) {
    if let Some(a) = r.active.take() {
        drop_active(app, host, a);
    }
    let out = host.out();
    out.remove(r.row);
    out.release(r.wrapper);
    out.release(r.row);
}

/// Removes every row and canvas (document closed or about to be rebuilt).
pub fn clear(app: &mut App, host: &mut dyn Host) {
    cancel_jobs(app, host);
    for r in std::mem::take(&mut app.st.editor_pages.rows) {
        drop_row(app, host, r);
    }
    let st = &mut app.st.editor_pages;
    st.desired.clear();
    st.pending.clear();
    st.previews.clear();
    st.new_pages.clear();
    st.active_page = Id::NONE;
    with_gfx(app, |_, g| {
        g.paths.clear(&mut g.cmds);
        for p in g.pictures.drain(..) {
            g.cmds.image_drop(p.img);
        }
        for (_, _, img, _, _) in g.rasters.drain(..) {
            g.cmds.image_drop(img);
        }
    });
    flush(app, host);
    sync_layout(app);
}

/// v5: `clearTemplateBackgroundUrlCache()` + `_tplCache.clear()` at the start of an open.
pub fn reset_template_urls(app: &mut App) {
    with_gfx(app, |_, g| {
        g.urls.clear();
        g.tpl_urls = [None, None, None];
    });
}

fn cancel_jobs(app: &mut App, host: &mut dyn Host) {
    if let Some(job) = app.st.editor_pages.staged.take() {
        with_gfx(app, |_, g| job.staged.discard(&mut g.cmds));
    }
    if let Some(job) = app.st.editor_pages.preview_job.take() {
        with_gfx(app, |_, g| job.sp.cancel(&mut g.cmds));
    }
    app.st.editor_pages.preview_reqs.clear();
    flush(app, host);
}

/// v5: `renderAllPages()`: one row per page of the document, previews or template backgrounds, indicator last.
pub fn rebuild(app: &mut App, host: &mut dyn Host) {
    let keep_active = app.st.editor_pages.active_page;
    let previews: Vec<(Id, Option<String>, bool)> = app.st.editor_pages.rows.iter_mut().map(|r| (r.id, r.preview.take(), r.preview_dirty)).collect();
    clear(app, host);
    let ids: Vec<Id> = doc(app).map(|d| d.order().to_vec()).unwrap_or_default();
    for (i, id) in ids.iter().enumerate() {
        let mut r = make_row(app, host, *id, i, false);
        if let Some((_, p, dirty)) = previews.iter().find(|p| p.0 == *id) {
            r.preview = p.clone();
            r.preview_dirty = *dirty;
        }
        app.st.editor_pages.rows.push(r);
    }
    sync_layout(app);
    for id in ids.iter() {
        apply_preview(app, host, *id);
    }
    viewport::update_indicator(app, host);
    let st = &mut app.st.editor_pages;
    st.active_page = if st.rows.iter().any(|r| r.id == keep_active) { keep_active } else { st.rows.first().map_or(Id::NONE, |r| r.id) };
    schedule_visible(app, host, true);
    queue_scroll_previews(app, host, true);
}

/// v5: `addPage(null, animate)`: a page after the last one with the last page's template and default size.
pub fn add_page(app: &mut App, host: &mut dyn Host, animate: bool) -> Option<Id> {
    let last = app.st.editor_pages.rows.last().map(|r| r.id);
    let kind = last.map_or(Template::Default, |id| page_template(app, id));
    let (w, h) = template::default_size(kind);
    let mut new_id = Id::NONE;
    session::edit(app, host, |tx| {
        new_id = tx.page_new(last, kind, (w as u32, h as u32), Background::Template);
    })?;
    session::take_changes(app);
    let i = app.st.editor_pages.rows.len();
    let r = make_row(app, host, new_id, i, animate);
    app.st.editor_pages.rows.push(r);
    sync_layout(app);
    apply_preview(app, host, new_id);
    if animate {
        app.st.editor_pages.new_pages.push(new_id);
        app.sched.timer(host.out(), T_NEW_PAGE, 600.0);
    }
    schedule_visible(app, host, true);
    shell::status(app, host, "Page added", StatusOpts::from(Status::Info).preserve());
    Some(new_id)
}

/// Follows a model change (remote batches, a snapshot merge, local structure edits made elsewhere): rows for a
/// changed page list, repaints for changed pages.
pub fn changed(app: &mut App, host: &mut dyn Host, c: &ihn_core::doc::Changes) {
    if c.order || order(app).as_slice() != doc(app).map_or(&[][..], |d| d.order()) {
        rebuild(app, host);
        viewport::update_transform(app, host);
        return;
    }
    let mut dims_changed = false;
    for id in c.page_props.iter() {
        let Some((w, h)) = doc(app).and_then(|d| d.pages.get(id)).map(|p| (p.size.v.0 as f64, p.size.v.1 as f64)) else { continue };
        if let Some(r) = row_mut(app, *id) {
            if (r.w, r.h) != (w, h) {
                (r.w, r.h) = (w, h);
                let out = host.out();
                out.style(r.wrapper, "width", Some(&format!("{}px", num(w))));
                out.style(r.wrapper, "height", Some(&format!("{}px", num(h))));
                dims_changed = true;
            }
        }
        if index(app, *id).is_some() {
            touch(app, *id);
            if let Some(a) = active(app, *id) {
                let canvas = a.canvas;
                apply_background(app, host, *id, canvas);
            }
            request_repaint(app, host, *id);
        }
    }
    if dims_changed {
        sync_layout(app);
        sync_resolution(app, host, false);
        viewport::update_indicator(app, host);
        schedule_visible(app, host, true);
    }
    for id in c.pages.iter() {
        if index(app, *id).is_some() {
            touch(app, *id);
            request_repaint(app, host, *id);
        }
    }
}

/// The page's content changed: previews are stale, a running staged repaint restarts.
pub fn touch(app: &mut App, id: Id) {
    if let Some(r) = row_mut(app, id) {
        r.gen = r.gen.wrapping_add(1);
        r.needs_redraw = true;
        r.preview_dirty = true;
    }
}

// ---- previews and backgrounds -----------------------------------------------------------------------------------

/// The template URL of a kind at the current zoom, requesting a sharper one when needed
/// (v5 `ensureTemplateBackgroundUrl`).
fn template_url(app: &mut App, kind: Template) -> Option<String> {
    let (zoom, pc) = (app.st.editor_viewport.zoom, is_pc(app));
    let req = app.req(Mod::EditorPages);
    with_gfx(app, |_, g| {
        if let Some(scale) = g.urls.need(kind, zoom, pc) {
            g.tpl_url_req[kind_index(kind)] = req;
            g.tpl.bg_url(&mut g.cmds, kind, scale, req);
        }
        g.tpl_urls[kind_index(kind)].clone()
    })
}

/// v5: `warmTemplateBackgroundUrlsForDoc()`.
pub fn warm_template_urls(app: &mut App, host: &mut dyn Host) {
    let mut kinds = vec![Template::Default];
    for r in &app.st.editor_pages.rows {
        if page_bg(app, r.id) == Background::Template {
            let k = page_template(app, r.id);
            if !kinds.contains(&k) {
                kinds.push(k);
            }
        }
    }
    for k in kinds {
        template_url(app, k);
    }
    flush(app, host);
}

fn set_wrapper_bg(host: &mut dyn Host, r: &mut Row, url: Option<&str>, size: &str) {
    let css = url.map(|u| format!("url(\"{u}\")")).unwrap_or_default();
    if r.wrapper_bg == css {
        return;
    }
    let out = host.out();
    out.style(r.wrapper, "background-image", Some(&css));
    if url.is_some() {
        out.style(r.wrapper, "background-size", Some(size));
        out.style(r.wrapper, "background-repeat", Some("no-repeat"));
        out.style(r.wrapper, "background-position", Some("center"));
    }
    r.wrapper_bg = css;
}

/// v5: `applyPagePreview(i)`: the preview, else the template URL (not for PDF pages), else nothing.
fn apply_preview(app: &mut App, host: &mut dyn Host, id: Id) {
    let bg = page_bg(app, id);
    let tpl = (!matches!(bg, Background::Pdf { .. }) && row(app, id).is_some_and(|r| r.preview.is_none())).then(|| page_template(app, id));
    let url = tpl.and_then(|k| template_url(app, k));
    flush(app, host);
    let Some(r) = row_mut(app, id) else { return };
    if let Some(p) = r.preview.clone() {
        set_wrapper_bg(host, r, Some(&p), "cover");
    } else {
        set_wrapper_bg(host, r, url.as_deref(), "100% 100%");
    }
}

/// v5: `clearPagePreview(i)` once the canvas has painted (PDF pages keep it until their raster exists).
fn clear_preview(app: &mut App, host: &mut dyn Host, id: Id) {
    let keep = matches!(page_bg(app, id), Background::Pdf { .. }) && raster(app, id).is_none();
    if keep {
        return;
    }
    if let Some(r) = row_mut(app, id) {
        set_wrapper_bg(host, r, None, "");
    }
}

fn raster(app: &App, id: Id) -> Option<(String, Img, bool)> {
    app.st.editor_pages.gfx.as_ref()?.rasters.iter().find(|r| r.0 == id).map(|r| (r.1.clone(), r.2, r.4))
}

/// `doc::backgrounds` hands over (or drops, `None`) the rendered pdf.js raster of a PDF page as a blob URL.
pub fn set_pdf_raster(app: &mut App, host: &mut dyn Host, page: Id, url: Option<&str>) {
    let req = app.req(Mod::EditorPages);
    with_gfx(app, |_, g| {
        if let Some(at) = g.rasters.iter().position(|r| r.0 == page) {
            let (_, _, img, _, _) = g.rasters.remove(at);
            g.cmds.image_drop(img);
        }
        if let Some(url) = url {
            let img = g.cmds.image_url(req, url);
            g.rasters.push((page, url.into(), img, req, false));
        }
    });
    flush(app, host);
    if let Some(a) = active(app, page) {
        let canvas = a.canvas;
        apply_background(app, host, page, canvas);
    }
}

/// The decoded image of an asset, starting its decode (and its blob URL) on first use.
fn picture(app: &mut App, hash: Hash) -> Option<Img> {
    if let Some(p) = app.st.editor_pages.gfx.as_ref().and_then(|g| g.pictures.iter().find(|p| p.hash == hash)) {
        return p.ready.then_some(p.img);
    }
    let blob = session::get(app).and_then(|s| s.asset_blob(&hash))?;
    let (req, url_req) = (app.req(Mod::EditorPages), app.req(Mod::EditorPages));
    with_gfx(app, |_, g| {
        let img = g.cmds.image_blob(req, blob);
        g.pictures.push(Picture { hash, img, req, ready: false, url: None, url_req });
    });
    None
}

fn picture_url(app: &App, hash: &Hash) -> Option<String> { app.st.editor_pages.gfx.as_ref()?.pictures.iter().find(|p| p.hash == *hash)?.url.clone() }

/// v5: `applyPageBackground(i, canvas)`: the canvas' CSS background.
fn apply_background(app: &mut App, host: &mut dyn Host, id: Id, canvas: Node) {
    let (url, size) = match page_bg(app, id) {
        Background::Template => (template_url(app, page_template(app, id)), "100% 100%"),
        Background::Custom { asset } => {
            picture(app, asset);
            (picture_url(app, &asset), "cover")
        }
        Background::Pdf { .. } => (raster(app, id).map(|r| r.0), "100% 100%"),
    };
    flush(app, host);
    let out = host.out();
    out.style(canvas, "background-image", Some(&url.as_ref().map(|u| format!("url(\"{u}\")")).unwrap_or_default()));
    if url.is_some() {
        out.style(canvas, "background-size", Some(size));
        out.style(canvas, "background-repeat", Some("no-repeat"));
        out.style(canvas, "background-position", Some("center"));
    }
    out.style(canvas, "background-color", Some("transparent"));
}

/// What a repaint draws under the ink.
fn bg_of(app: &mut App, id: Id) -> Bg {
    match page_bg(app, id) {
        Background::Template => Bg::Template(page_template(app, id)),
        Background::Custom { asset } => Bg::Image(picture(app, asset)),
        Background::Pdf { .. } => match raster(app, id) {
            Some((_, img, true)) => Bg::Image(Some(img)),
            _ => Bg::PdfPending,
        },
    }
}

/// The decoded images of a page's items (decodes start for the others).
fn decoded_images(app: &mut App, id: Id) -> Vec<(Hash, Img)> {
    let hashes: Vec<Hash> = doc(app).map_or(Vec::new(), |d| {
        d.draw_list(id).images.iter().filter_map(|r| match &d.live(r.id)?.body.v {
            ihn_core::op::Body::Image { asset, .. } => Some(*asset),
            _ => None,
        }).collect()
    });
    hashes.into_iter().filter_map(|h| picture(app, h).map(|img| (h, img))).collect()
}

/// Paints page `id` with `f(gfx, view)`; `None` when the page or the document is gone.
fn with_view<R>(app: &mut App, id: Id, f: impl FnOnce(&mut Gfx, &PageView) -> R) -> Option<R> {
    let bg = bg_of(app, id);
    let images = decoded_images(app, id);
    let (w, h) = row(app, id).map(|r| (r.w, r.h))?;
    let mut gfx = app.st.editor_pages.gfx.take().unwrap_or_else(|| new_gfx(app));
    let r = doc(app).map(|d| {
        let items = PageItems::collect(d, id, &|hash| images.iter().find(|e| e.0 == hash).map(|e| e.1));
        f(&mut gfx, &PageView { w, h, bg, items: &items })
    });
    app.st.editor_pages.gfx = Some(gfx);
    r
}

// ---- canvases ---------------------------------------------------------------------------------------------------

const SELECTION_BOX: &str = r##"
                <div class="selection-rotate-line"></div>
                <div class="rotate-handle" data-handle="rotate"></div>
                <div class="selection-actions">
                    <button class="selection-color-btn" type="button" title="Change color" aria-label="Change color">
                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <circle cx="12" cy="12" r="9"/>
                            <circle cx="12" cy="7.5" r="2" fill="#E81010" stroke="none"/>
                            <circle cx="7.5" cy="14" r="2" fill="#002FD9" stroke="none"/>
                            <circle cx="16.5" cy="14" r="2" fill="#4D4D4D" stroke="none"/>
                        </svg>
                        <div class="selection-color-menu">
                            SWATCHES
                        </div>
                    </button>
                    <button class="selection-duplicate" type="button" title="Duplicate selection" aria-label="Duplicate selection">
                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                            <rect x="9" y="9" width="10" height="10" rx="2"/>
                            <rect x="5" y="5" width="10" height="10" rx="2"/>
                        </svg>
                    </button>
                    <button class="selection-trash" type="button" title="Delete selection" aria-label="Delete selection">
                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                            <polyline points="3 6 5 6 21 6"/>
                            <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>
                        </svg>
                    </button>
                </div>
                <div class="selection-handle edge handle-n" data-handle="n"></div>
                <div class="selection-handle edge handle-s" data-handle="s"></div>
                <div class="selection-handle edge handle-e" data-handle="e"></div>
                <div class="selection-handle edge handle-w" data-handle="w"></div>
                <div class="selection-handle handle-ne" data-handle="ne"></div>
                <div class="selection-handle handle-nw" data-handle="nw"></div>
                <div class="selection-handle handle-se" data-handle="se"></div>
                <div class="selection-handle handle-sw" data-handle="sw"></div>
            "##;

fn selection_box_html(i: usize) -> String {
    let swatches: String = INK_PALETTE.iter().map(|c| format!("<div class=\"selection-color-swatch\" data-swatch-color=\"{c}\" style=\"background:{c}\"></div>")).collect();
    format!("<div class=\"selection-box hidden\" data-page=\"{i}\">{}</div>", SELECTION_BOX.replace("SWATCHES", &swatches))
}

fn overlay_html(class: &str, i: usize, px: (u32, u32), w: f64) -> String {
    format!(
        "<canvas class=\"{class}\" data-page=\"{i}\" width=\"{}\" height=\"{}\" data-overlay-scale=\"{}\" style=\"width: 100%; height: 100%; visibility: hidden;\"></canvas>",
        px.0,
        px.1,
        num(px.0 as f64 / w.max(1.0))
    )
}

/// v5: `ensurePageCanvas(i)` + `attachPageCanvases(wrapper, i)`.
fn attach(app: &mut App, host: &mut dyn Host, id: Id) {
    let Some(i) = index(app, id) else { return };
    if app.st.editor_pages.rows[i].active.is_some() {
        return;
    }
    let (w, h, wrapper) = {
        let r = &app.st.editor_pages.rows[i];
        (r.w, r.h, r.wrapper)
    };
    let (zoom, pc) = (app.st.editor_viewport.zoom, is_pc(app));
    let (px, scale) = backing(w, h, zoom, pc);
    let (ls, cs) = (overlay_scale(zoom, true), overlay_scale(zoom, false));
    let (lpx, cpx) = (overlay_px(w, h, ls), overlay_px(w, h, cs));
    let out = host.out();
    let first = out.child(wrapper, &[0]); // the page number (or the first child)
    let canvas = out.insert_html(
        wrapper,
        first,
        &format!("<canvas class=\"page-canvas\" width=\"{}\" height=\"{}\" style=\"width: 100%; height: 100%;\" data-render-scale=\"{}\" data-page=\"{i}\"></canvas>", px.0, px.1, num(px.0 as f64 / w.max(1.0))),
    );
    let lasso = out.insert_html(wrapper, first, &overlay_html("lasso-overlay", i, lpx, w));
    let cursor = out.insert_html(wrapper, first, &overlay_html("cursor-overlay", i, cpx, w));
    let remote = out.insert_html(wrapper, first, &overlay_html("remote-live-overlay", i, cpx, w));
    let selection_box = out.insert_html(wrapper, first, &selection_box_html(i));
    out.release(first);
    let (surf, lasso_surf, cursor_surf, remote_surf) = with_gfx(app, |_, g| {
        let s = g.cmds.bind(canvas.0);
        rpage::context_defaults(&mut g.cmds, s, px.0 as f64 / w.max(1.0));
        (s, g.cmds.bind(lasso.0), g.cmds.bind(cursor.0), g.cmds.bind(remote.0))
    });
    app.st.editor_pages.rows[i].active = Some(Active {
        canvas,
        lasso,
        cursor,
        remote,
        selection_box,
        surf,
        lasso_surf,
        cursor_surf,
        remote_surf,
        px,
        scale: px.0 as f64 / w.max(1.0),
        lasso_px: lpx,
        lasso_scale: lpx.0 as f64 / w.max(1.0),
        cursor_px: cpx,
        cursor_scale: cpx.0 as f64 / w.max(1.0),
        drawn: false,
        ink_hidden: [true; 3],
    });
    let _ = scale;
    flush(app, host);
    apply_background(app, host, id, canvas);
    repaint(app, host, id);
}

fn drop_active(app: &mut App, host: &mut dyn Host, a: Active) {
    with_gfx(app, |_, g| {
        for s in [a.surf, a.lasso_surf, a.cursor_surf, a.remote_surf] {
            g.cmds.drop_surf(s);
        }
    });
    flush(app, host);
    let out = host.out();
    for n in [a.canvas, a.lasso, a.cursor, a.remote, a.selection_box] {
        out.remove(n);
        out.release(n);
    }
}

/// v5: `releasePageCanvas(i)`: the preview replaces the canvases.
fn release(app: &mut App, host: &mut dyn Host, id: Id) {
    let Some(i) = index(app, id) else { return };
    let Some(a) = app.st.editor_pages.rows[i].active.take() else { return };
    let st = &mut app.st.editor_pages;
    st.desired.retain(|d| *d != id);
    st.pending.retain(|p| p.0 != id);
    if st.staged.as_ref().is_some_and(|j| j.page == id) {
        if let Some(job) = st.staged.take() {
            with_gfx(app, |_, g| job.staged.discard(&mut g.cmds));
        }
    }
    // v5: unloadPageData renders a preview when it is missing or dirty.
    let r = &app.st.editor_pages.rows[i];
    if r.preview.is_none() || r.preview_dirty {
        request_page_preview(app, id);
    }
    drop_active(app, host, a);
    apply_preview(app, host, id);
    if let Some(r) = row_mut(app, id) {
        r.needs_redraw = true;
    }
}

fn request_page_preview(app: &mut App, id: Id) {
    let req = app.req(Mod::EditorPages);
    let gen = row(app, id).map_or(0, |r| r.gen);
    let made = with_view(app, id, |g, view| thumb::page_preview(&mut g.cmds, &mut g.tpl, &mut g.paths, view, req)).unwrap_or(false);
    if made {
        app.st.editor_pages.preview_reqs.push((req, id, gen));
    }
}

/// v5: `applyPageDimensionsToWrapper(i)` for an active page whose backing size changed.
fn resize(app: &mut App, host: &mut dyn Host, id: Id, force: bool) {
    let Some(i) = index(app, id) else { return };
    let (w, h) = (app.st.editor_pages.rows[i].w, app.st.editor_pages.rows[i].h);
    let (zoom, pc) = (app.st.editor_viewport.zoom, is_pc(app));
    let (px, _) = backing(w, h, zoom, pc);
    let Some(a) = app.st.editor_pages.rows[i].active.as_ref() else { return };
    if !force && a.px == px {
        return;
    }
    let (old, drawn, canvas, surf) = (a.px, a.drawn, a.canvas, a.surf);
    let (lpx, cpx) = (overlay_px(w, h, overlay_scale(zoom, true)), overlay_px(w, h, overlay_scale(zoom, false)));
    let (lasso, cursor, remote) = (a.lasso_surf, a.cursor_surf, a.remote_surf);
    let (lnode, cnode, rnode, lold, cold) = (a.lasso, a.cursor, a.remote, a.lasso_px, a.cursor_px);
    let resized = old != px;
    let scale = px.0 as f64 / w.max(1.0);
    with_gfx(app, |_, g| {
        if resized {
            rpage::resize(&mut g.cmds, surf, drawn.then_some(old), px);
        }
        rpage::context_defaults(&mut g.cmds, surf, scale);
        if lold != lpx {
            g.cmds.size(lasso, lpx.0, lpx.1);
        }
        if cold != cpx {
            g.cmds.size(cursor, cpx.0, cpx.1);
            g.cmds.size(remote, cpx.0, cpx.1);
        }
    });
    flush(app, host);
    let out = host.out();
    out.attr(canvas, "data-render-scale", Some(&num(scale)));
    out.attr(lnode, "data-overlay-scale", Some(&num(lpx.0 as f64 / w.max(1.0))));
    for n in [cnode, rnode] {
        out.attr(n, "data-overlay-scale", Some(&num(cpx.0 as f64 / w.max(1.0))));
    }
    if let Some(a) = app.st.editor_pages.rows[i].active.as_mut() {
        a.px = px;
        a.scale = scale;
        a.lasso_px = lpx;
        a.lasso_scale = lpx.0 as f64 / w.max(1.0);
        a.cursor_px = cpx;
        a.cursor_scale = cpx.0 as f64 / w.max(1.0);
    }
    if resized {
        app.st.editor_pages.rows[i].needs_redraw = true;
        repaint(app, host, id);
    }
}

/// v5: `syncActivePageCanvasResolution({force})`.
pub fn sync_resolution(app: &mut App, host: &mut dyn Host, force: bool) {
    let ids: Vec<Id> = app.st.editor_pages.rows.iter().filter(|r| r.active.is_some()).map(|r| r.id).collect();
    for id in ids {
        resize(app, host, id, force);
    }
}

/// Sets an overlay's visibility once per change (v5 `setOverlayCanvasInk`); `which`: 0 lasso, 1 cursor, 2 remote.
pub fn overlay_ink(app: &mut App, host: &mut dyn Host, id: Id, which: usize, has_ink: bool) {
    let Some(a) = row_mut(app, id).and_then(|r| r.active.as_mut()) else { return };
    if a.ink_hidden[which] == !has_ink {
        return;
    }
    a.ink_hidden[which] = !has_ink;
    let node = [a.lasso, a.cursor, a.remote][which];
    host.out().style(node, "visibility", Some(if has_ink { "" } else { "hidden" }));
}

// ---- repaint ----------------------------------------------------------------------------------------------------

/// Asks for a repaint of page `id` (immediately if it is active and nothing writes; otherwise when it is).
pub fn request_repaint(app: &mut App, host: &mut dyn Host, id: Id) {
    if let Some(r) = row_mut(app, id) {
        r.needs_redraw = true;
    }
    if active(app, id).is_some() {
        repaint(app, host, id);
    }
}

/// v5: `redrawPage(i)`: synchronous, or staged on a processing canvas for dense pages.
pub fn repaint(app: &mut App, host: &mut dyn Host, id: Id) {
    let Some(a) = active(app, id) else { return };
    let (surf, px, scale) = (a.surf, a.px, a.scale);
    if app.st.editor_viewport.pen_writing {
        // v5: a repaint waits for the pen to lift (polled every 40 ms).
        if !std::mem::replace(&mut app.st.editor_pages.repaint_armed, true) {
            app.sched.timer(host.out(), T_REPAINT, 40.0);
        }
        return;
    }
    let gen = row(app, id).map_or(0, |r| r.gen);
    if let Some(job) = app.st.editor_pages.staged.as_ref() {
        if job.page == id && job.gen == gen && job.px == px {
            return; // the same paint is already running
        }
    }
    if let Some(job) = app.st.editor_pages.staged.take() {
        with_gfx(app, |_, g| job.staged.discard(&mut g.cmds));
        if job.page != id {
            if let Some(r) = row_mut(app, job.page) {
                r.needs_redraw = true;
            }
            app.sched.timer(host.out(), T_REPAINT, 0.0);
            app.st.editor_pages.repaint_armed = true;
        }
    }
    let staged = with_view(app, id, |g, view| {
        if rpage::is_staged(&view.items.strokes) {
            Some(Staged::begin(&mut g.cmds, &mut g.tpl, px, view, scale))
        } else {
            rpage::repaint(&mut g.cmds, &mut g.tpl, &mut g.paths, surf, px, view, scale);
            None
        }
    });
    flush(app, host);
    match staged {
        Some(Some(staged)) => {
            app.st.editor_pages.staged = Some(StagedJob { page: id, gen, px, staged, done: false });
            app.sched.timer(host.out(), T_STAGED, 0.0);
        }
        Some(None) => painted(app, host, id),
        None => {}
    }
}

fn painted(app: &mut App, host: &mut dyn Host, id: Id) {
    let Some(r) = row_mut(app, id) else { return };
    r.needs_redraw = false;
    let Some(a) = r.active.as_mut() else { return };
    if !std::mem::replace(&mut a.drawn, true) {
        host.out().attr(a.canvas, "data-has-page-draw", Some("1"));
    }
    clear_preview(app, host, id);
}

fn staged_slice(app: &mut App, host: &mut dyn Host) {
    let Some(mut job) = app.st.editor_pages.staged.take() else { return };
    let id = job.page;
    let valid = row(app, id).is_some_and(|r| r.gen == job.gen) && active(app, id).is_some_and(|a| a.px == job.px);
    if !valid {
        with_gfx(app, |_, g| job.staged.discard(&mut g.cmds));
        flush(app, host);
        if active(app, id).is_some() {
            repaint(app, host, id);
        }
        return;
    }
    if app.st.editor_viewport.pen_writing || (job.done && viewport::interacting(app, host.now()) && app.sched.is_busy()) {
        app.st.editor_pages.staged = Some(job);
        app.sched.timer(host.out(), T_STAGED, 40.0);
        return;
    }
    if !job.done {
        job.done = with_view(app, id, |g, view| job.staged.paint(&mut g.cmds, &mut g.paths, view, STAGED_SLICE)).unwrap_or(true);
        flush(app, host);
        app.st.editor_pages.staged = Some(job);
        app.sched.timer(host.out(), T_STAGED, 0.0);
        return;
    }
    let surf = active(app, id).map(|a| a.surf);
    with_gfx(app, |_, g| match surf {
        Some(s) => job.staged.blit(&mut g.cmds, s),
        None => job.staged.discard(&mut g.cmds),
    });
    flush(app, host);
    painted(app, host, id);
}

// ---- visible pages ----------------------------------------------------------------------------------------------

/// v5: `scheduleVisiblePageUpdate(force)`: one pass in the next frame.
pub fn schedule_visible(app: &mut App, host: &mut dyn Host, force: bool) {
    let st = &mut app.st.editor_pages;
    st.visible_force |= force;
    if !std::mem::replace(&mut st.visible_frame, true) {
        app.sched.frame(host.out(), Mod::EditorPages);
    }
}

/// v5: `updateVisiblePages(force)`.
fn update_visible(app: &mut App, host: &mut dyn Host, force: bool) {
    let now = host.now();
    let defer = !force && viewport::interacting(app, now);
    if defer && app.st.editor_viewport.pen_writing {
        if !std::mem::replace(&mut app.st.editor_pages.retry_armed, true) {
            app.sched.timer(host.out(), T_VISIBLE_RETRY, 140.0);
        }
        return;
    }
    queue_scroll_previews(app, host, false);
    let n = app.st.editor_pages.rows.len();
    if n == 0 {
        return;
    }
    if defer {
        if !std::mem::replace(&mut app.st.editor_pages.retry_armed, true) {
            app.sched.timer(host.out(), T_VISIBLE_RETRY, 140.0);
        }
        return;
    }
    if std::mem::take(&mut app.st.editor_pages.retry_armed) {
        app.sched.clear(host.out(), T_VISIBLE_RETRY);
    }
    let r = viewport::rect(app, host);
    let (zoom, pan_x, pan_y) = (app.st.editor_viewport.zoom, app.st.editor_viewport.pan_x, app.st.editor_viewport.pan_y);
    let direct_dom = layout::viewport_visible(app, r.w, r.h, pan_x, pan_y, zoom);
    let center = (-pan_y + r.h / 2.0) / zoom;
    let dist = |app: &App, i: usize| (layout::page_center(app, i) - center).abs();
    let (visible, direct): (Vec<usize>, Vec<usize>);
    if !direct_dom.is_empty() {
        let mut d = direct_dom;
        d.sort_by(|a, b| dist(app, *a).partial_cmp(&dist(app, *b)).unwrap_or(std::cmp::Ordering::Equal));
        d.truncate(layout::MAX_ACTIVE_PAGES);
        visible = layout::expand(n, &d, layout::VISIBLE_PAGE_BUFFER);
        direct = d;
    } else {
        let (s, e) = layout::visible_range(app, r.h, pan_y, zoom, layout::VISIBLE_PAGE_BUFFER);
        if e < s {
            return;
        }
        visible = (s as usize..=e as usize).collect();
        let (ds, de) = layout::visible_range(app, r.h, pan_y, zoom, 0);
        direct = if de >= ds { (ds as usize..=de as usize).collect() } else { Vec::new() };
    }
    if visible.is_empty() {
        return;
    }
    let (start, end) = (visible[0], visible[visible.len() - 1]);
    let mut candidates: Vec<usize> = visible.clone();
    candidates.sort_by(|a, b| dist(app, *a).partial_cmp(&dist(app, *b)).unwrap_or(std::cmp::Ordering::Equal));
    let ids: Vec<Id> = app.st.editor_pages.rows.iter().map(|r| r.id).collect();
    let mut ordered: Vec<usize> = Vec::new();
    let sel = ids.iter().position(|x| *x == app.st.editor_pages.selection_page && *x != Id::NONE);
    let act = ids.iter().position(|x| *x == app.st.editor_pages.active_page);
    for i in [sel, act].into_iter().flatten() {
        if i >= start && i <= end && !ordered.contains(&i) {
            ordered.push(i);
        }
    }
    for &i in direct.iter().chain(candidates.iter()) {
        if ordered.len() >= layout::MAX_ACTIVE_PAGES {
            break;
        }
        if !ordered.contains(&i) {
            ordered.push(i);
        }
    }
    let st = &mut app.st.editor_pages;
    st.desired = ordered.iter().map(|&i| ids[i]).collect();
    let desired = st.desired.clone();
    st.pending.retain(|p| desired.contains(&p.0));
    for &i in &direct {
        if i < n {
            attach(app, host, ids[i]);
            let needs = row(app, ids[i]).is_some_and(|r| r.needs_redraw || !r.active.as_ref().is_some_and(|a| a.drawn));
            if needs {
                repaint(app, host, ids[i]);
            }
        }
    }
    for (order, &i) in ordered.iter().enumerate() {
        if i < start || i > end {
            continue;
        }
        let id = ids[i];
        apply_preview_if_inactive(app, host, id);
        if active(app, id).is_some() {
            let needs = row(app, id).is_some_and(|r| r.needs_redraw || !r.active.as_ref().is_some_and(|a| a.drawn));
            if needs {
                repaint(app, host, id);
            }
            continue;
        }
        queue_activation(app, id, order);
    }
    for &i in &visible {
        if !ordered.contains(&i) {
            apply_preview(app, host, ids[i]);
        }
    }
    let stale: Vec<Id> = app.st.editor_pages.rows.iter().enumerate().filter(|(i, r)| r.active.is_some() && !ordered.contains(i)).map(|(_, r)| r.id).collect();
    for id in stale {
        release(app, host, id);
    }
    schedule_activation(app, host);
}

/// 5.18.0 applies the preview to every ordered page; on an active, painted page `clearPagePreview` already ran.
fn apply_preview_if_inactive(app: &mut App, host: &mut dyn Host, id: Id) {
    if active(app, id).is_some_and(|a| a.drawn) {
        return;
    }
    apply_preview(app, host, id);
}

fn queue_activation(app: &mut App, id: Id, priority: usize) {
    let st = &mut app.st.editor_pages;
    if !st.desired.contains(&id) {
        return;
    }
    match st.pending.iter_mut().find(|p| p.0 == id) {
        Some(p) if p.1 <= priority => {}
        Some(p) => p.1 = priority,
        None => st.pending.push((id, priority)),
    }
}

/// v5: `schedulePageActivationProcessing()`: an idle callback (timeout 120), or 90 ms while interacting.
fn schedule_activation(app: &mut App, host: &mut dyn Host) {
    if app.st.editor_pages.activation_armed || app.st.editor_pages.pending.is_empty() {
        return;
    }
    app.st.editor_pages.activation_armed = true;
    if viewport::interacting(app, host.now()) {
        app.sched.timer(host.out(), T_ACTIVATE, 90.0);
    } else {
        app.sched.idle(host.out(), I_ACTIVATE, PAGE_ACTIVATION_IDLE_TIMEOUT);
    }
}

/// v5: `processPendingPageActivations()`: one page per turn, best priority first.
fn process_activation(app: &mut App, host: &mut dyn Host) {
    app.st.editor_pages.activation_armed = false;
    if app.st.editor_pages.pending.is_empty() {
        return;
    }
    if viewport::interacting(app, host.now()) {
        schedule_activation(app, host);
        return;
    }
    let st = &mut app.st.editor_pages;
    let desired = st.desired.clone();
    st.pending.retain(|p| desired.contains(&p.0));
    let Some(at) = st.pending.iter().enumerate().min_by_key(|(_, p)| p.1).map(|(i, _)| i) else { return };
    let (id, _) = st.pending.remove(at);
    attach(app, host, id);
    if row(app, id).is_some_and(|r| r.needs_redraw) {
        repaint(app, host, id);
    }
    schedule_activation(app, host);
}

// ---- scroll previews --------------------------------------------------------------------------------------------

/// v5: `queueScrollPagePreviews(all)`: pages in the visible range ±1 without a fresh preview.
pub fn queue_scroll_previews(app: &mut App, host: &mut dyn Host, all: bool) {
    let n = app.st.editor_pages.rows.len();
    if n == 0 || app.cfg.has(abi::mode::HEADLESS) {
        return;
    }
    let (s, e) = if all {
        (0, n as isize - 1)
    } else {
        let vh = viewport::rect(app, host).h;
        let v = &app.st.editor_viewport;
        let (pan_y, zoom) = (v.pan_y, v.zoom);
        layout::visible_range(app, vh, pan_y, zoom, 1)
    };
    let (first, last) = (s.max(0) as usize, (e.min(n as isize - 1)).max(-1));
    let range = if all { None } else { Some((s, e)) };
    let now = host.time();
    let mut i = first as isize;
    while i <= last {
        let k = i as usize;
        let r = &app.st.editor_pages.rows[k];
        let id = r.id;
        let skip = (r.preview.is_some() && !r.preview_dirty) || now - r.preview_failed_at < 3000.0 || matches!(page_bg(app, id), Background::Pdf { .. }) && raster(app, id).is_none();
        if !skip {
            let pri = if range.is_some_and(|(s, e)| i >= s && i <= e) { 0 } else { 1000 + k };
            let q = &mut app.st.editor_pages.previews;
            match q.iter_mut().find(|p| p.0 == id) {
                Some(p) => p.1 = p.1.min(pri),
                None => q.push((id, pri)),
            }
        }
        i += 1;
    }
    schedule_previews(app, host, 16.0);
}

fn schedule_previews(app: &mut App, host: &mut dyn Host, ms: f64) {
    let st = &mut app.st.editor_pages;
    if st.preview_armed || (st.previews.is_empty() && st.preview_job.is_none()) {
        return;
    }
    st.preview_armed = true;
    app.sched.timer(host.out(), T_PREVIEW, ms);
}

/// v5: `processScrollPagePreview()`: one page at a time; waits while the viewport is interacting.
fn preview_step(app: &mut App, host: &mut dyn Host) {
    app.st.editor_pages.preview_armed = false;
    if viewport::interacting(app, host.now()) {
        schedule_previews(app, host, 240.0);
        return;
    }
    if let Some(mut job) = app.st.editor_pages.preview_job.take() {
        let id = job.page;
        if row(app, id).is_none_or(|r| r.gen != job.gen) {
            with_gfx(app, |_, g| job.sp.cancel(&mut g.cmds));
            flush(app, host);
            schedule_previews(app, host, 16.0);
            return;
        }
        if !job.background_done {
            // Every image of the page (and a photo background) must be decoded first.
            let waiting = images_pending(app, id);
            if waiting {
                app.st.editor_pages.preview_job = Some(job);
                schedule_previews(app, host, 50.0);
                return;
            }
            with_view(app, id, |g, view| job.sp.background(&mut g.cmds, &mut g.tpl, &mut g.paths, view));
            job.background_done = true;
        }
        let done = with_view(app, id, |g, view| job.sp.strokes(&mut g.cmds, &mut g.paths, view, PREVIEW_SLICE)).unwrap_or(true);
        if done {
            let req = app.req(Mod::EditorPages);
            app.st.editor_pages.preview_reqs.push((req, id, job.gen));
            with_gfx(app, |_, g| job.sp.finish(&mut g.cmds, req));
        } else {
            app.st.editor_pages.preview_job = Some(job);
        }
        flush(app, host);
        schedule_previews(app, host, if done { 16.0 } else { 0.0 });
        return;
    }
    let st = &mut app.st.editor_pages;
    let Some(at) = st.previews.iter().enumerate().min_by_key(|(_, p)| p.1).map(|(i, _)| i) else { return };
    let (id, _) = st.previews.remove(at);
    let Some(r) = row(app, id) else { return schedule_previews(app, host, 16.0) };
    if r.preview.is_some() && !r.preview_dirty {
        return schedule_previews(app, host, 16.0);
    }
    let (w, h, gen) = (r.w, r.h, r.gen);
    let sp = with_gfx(app, |_, g| ScrollPreview::begin(&mut g.cmds, w, h));
    app.st.editor_pages.preview_job = Some(PreviewJob { page: id, gen, sp, background_done: false });
    flush(app, host);
    schedule_previews(app, host, 0.0);
}

fn images_pending(app: &mut App, id: Id) -> bool {
    let mut hashes: Vec<Hash> = doc(app).map_or(Vec::new(), |d| {
        d.draw_list(id).images.iter().filter_map(|r| match &d.live(r.id)?.body.v {
            ihn_core::op::Body::Image { asset, .. } => Some(*asset),
            _ => None,
        }).collect()
    });
    if let Background::Custom { asset } = page_bg(app, id) {
        hashes.push(asset);
    }
    let mut pending = false;
    for h in hashes {
        let known = session::get(app).is_some_and(|s| s.asset_blob(&h).is_some());
        pending |= known && picture(app, h).is_none();
    }
    pending
}

// ---- events -----------------------------------------------------------------------------------------------------

fn io(app: &mut App, host: &mut dyn Host, io: &abi::Io) -> u32 {
    let ok = io.status >= 0;
    let text = std::str::from_utf8(io.data).unwrap_or("");
    // Template SVG loads (v5 `_getSvgImage` onload): caches flushed, URLs warmed again, pages repainted.
    let svg = with_gfx(app, |_, g| {
        if g.tpl.svg_req.contains(&io.req) {
            Some(g.tpl.svg_loaded(&mut g.cmds, io.req, ok))
        } else {
            None
        }
    });
    if let Some(loaded) = svg {
        if loaded {
            with_gfx(app, |_, g| {
                g.urls.clear();
                g.tpl_urls = [None, None, None];
            });
            warm_template_urls(app, host);
            let ids: Vec<Id> = app.st.editor_pages.rows.iter().map(|r| r.id).collect();
            for id in ids {
                touch(app, id);
                request_repaint(app, host, id);
            }
        }
        flush(app, host);
        return 0;
    }
    // Template CSS URLs.
    let kind = app.st.editor_pages.gfx.as_ref().and_then(|g| g.tpl_url_req.iter().position(|r| *r == io.req));
    if let Some(k) = kind {
        if ok {
            with_gfx(app, |_, g| g.tpl_urls[k] = Some(text.to_string()));
            let ids: Vec<Id> = app.st.editor_pages.rows.iter().map(|r| r.id).collect();
            for id in ids {
                if page_bg(app, id) == Background::Template && kind_index(page_template(app, id)) == k {
                    let r = row(app, id);
                    let (has_preview, act) = (r.is_some_and(|r| r.preview.is_some()), active(app, id).map(|a| (a.canvas, a.drawn)));
                    match act {
                        Some((canvas, drawn)) => {
                            apply_background(app, host, id, canvas);
                            if !drawn && !has_preview {
                                apply_preview(app, host, id);
                            }
                        }
                        None if !has_preview => apply_preview(app, host, id),
                        None => {}
                    }
                }
            }
        }
        return 0;
    }
    // Page previews (release) and scroll previews.
    if let Some(at) = app.st.editor_pages.preview_reqs.iter().position(|p| p.0 == io.req) {
        let (_, id, gen) = app.st.editor_pages.preview_reqs.remove(at);
        let now = host.time();
        let Some(r) = row_mut(app, id) else { return 0 };
        if !ok || text.is_empty() {
            r.preview_failed_at = now;
            return 0;
        }
        r.preview = Some(text.to_string());
        r.preview_dirty = r.gen != gen;
        let show = r.active.as_ref().is_none_or(|a| !a.drawn);
        if show {
            apply_preview(app, host, id);
        }
        return 0;
    }
    // Decoded asset images and their blob URLs.
    let mut repaint_all = false;
    let mut url_for: Option<Hash> = None;
    let mut blob_url: Option<(u32, u32)> = None;
    with_gfx(app, |_, g| {
        if let Some(p) = g.pictures.iter_mut().find(|p| p.req == io.req) {
            p.ready = ok;
            repaint_all = ok;
            if ok && p.url.is_none() {
                blob_url = Some((p.url_req, 0));
            }
        } else if let Some(p) = g.pictures.iter_mut().find(|p| p.url_req == io.req) {
            if ok {
                p.url = Some(text.to_string());
                url_for = Some(p.hash);
            }
        } else if let Some(r) = g.rasters.iter_mut().find(|r| r.3 == io.req) {
            r.4 = ok;
            repaint_all = ok;
        }
    });
    if let Some((req, _)) = blob_url {
        let blob = with_gfx(app, |_, g| g.pictures.iter().find(|p| p.url_req == req).map(|p| p.hash)).and_then(|h| session::get(app).and_then(|s| s.asset_blob(&h)));
        if let Some(blob) = blob {
            crate::shell::fx::blob_url(host.out(), req, blob);
        }
    }
    if url_for.is_some() || repaint_all {
        let ids: Vec<Id> = app.st.editor_pages.rows.iter().filter(|r| r.active.is_some()).map(|r| r.id).collect();
        for id in ids {
            if url_for.is_some() {
                if let Some(canvas) = active(app, id).map(|a| a.canvas) {
                    apply_background(app, host, id, canvas);
                }
            }
            if repaint_all {
                touch(app, id);
                request_repaint(app, host, id);
            }
        }
        if repaint_all {
            schedule_previews(app, host, 16.0);
        }
    }
    0
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(app: &mut App, _host: &mut dyn Host) {
    let g = new_gfx(app);
    app.st.editor_pages.gfx = Some(g);
}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Frame(_) => {
            let st = &mut app.st.editor_pages;
            if std::mem::take(&mut st.visible_frame) {
                let force = std::mem::take(&mut st.visible_force);
                update_visible(app, host, force);
            }
            0
        }
        Event::Timer(T_VISIBLE_RETRY) => {
            app.st.editor_pages.retry_armed = false;
            schedule_visible(app, host, false);
            0
        }
        Event::Timer(T_ACTIVATE) | Event::Idle { id: I_ACTIVATE, .. } => {
            process_activation(app, host);
            0
        }
        Event::Timer(T_STAGED) => {
            staged_slice(app, host);
            0
        }
        Event::Timer(T_PREVIEW) => {
            preview_step(app, host);
            0
        }
        Event::Timer(T_REPAINT) => {
            app.st.editor_pages.repaint_armed = false;
            let ids: Vec<Id> = app.st.editor_pages.rows.iter().filter(|r| r.active.is_some() && r.needs_redraw).map(|r| r.id).collect();
            for id in ids {
                repaint(app, host, id);
            }
            0
        }
        Event::Timer(T_NEW_PAGE) => {
            for id in std::mem::take(&mut app.st.editor_pages.new_pages) {
                if let Some(w) = row(app, id).map(|r| r.wrapper) {
                    host.out().class(w, "new-page", false);
                }
            }
            0
        }
        Event::Pointer(p) if p.listener == L_WRAPPER => {
            // v5: the wrapper's pointerdown — the touched page gets activation priority.
            if view::read_only(app) {
                return 0;
            }
            let Some(id) = page_of_node(app, p.node) else { return 0 };
            app.st.editor_pages.active_page = id;
            if active(app, id).is_none() {
                attach(app, host, id);
                schedule_visible(app, host, true);
            }
            0
        }
        Event::Io(ref i) => io(app, host, i),
        _ => {
            let _ = ret::PREVENT_DEFAULT;
            0
        }
    }
}

/// `__IHN_TEST_API__.cmd('editor.pages.state')`: rows, active canvases and their backing sizes.
pub fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    match name {
        "state" => {
            let rows: Vec<String> = app
                .st
                .editor_pages
                .rows
                .iter()
                .map(|r| match &r.active {
                    Some(a) => format!(
                        "{{\"w\":{},\"h\":{},\"active\":true,\"px\":[{},{}],\"scale\":{},\"lasso\":[{},{}],\"cursor\":[{},{}],\"drawn\":{},\"preview\":{}}}",
                        num(r.w), num(r.h), a.px.0, a.px.1, num(a.scale), a.lasso_px.0, a.lasso_px.1, a.cursor_px.0, a.cursor_px.1, a.drawn, r.preview.is_some()
                    ),
                    None => format!("{{\"w\":{},\"h\":{},\"active\":false,\"preview\":{}}}", num(r.w), num(r.h), r.preview.is_some()),
                })
                .collect();
            Some(format!("{{\"rows\":[{}],\"staged\":{}}}", rows.join(","), app.st.editor_pages.staged.is_some()))
        }
        "add" => {
            add_page(app, host, false);
            Some("{}".into())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backing_sizes_follow_v5() {
        // A4 at 100 %: 1588 × 2246; zoom 3 on PC is capped by the 28 MiB pixel budget.
        assert_eq!(backing(794.0, 1123.0, 1.0, true), ((1588, 2246), 2.0));
        assert_eq!(backing(794.0, 1123.0, 0.5, true).0, (1588, 2246));
        assert_eq!(target_scale(794.0, 1123.0, 2.0, true), 4.0);
        assert_eq!(target_scale(794.0, 1123.0, 3.0, true), 5.5);
        assert_eq!(target_scale(794.0, 1123.0, 3.0, false), 4.25);
        assert_eq!(target_scale(559.0, 794.0, 3.0, false), 4.5);
        assert_eq!(overlay_scale(1.0, false), 1.25);
        assert_eq!(overlay_scale(3.0, false), 1.8);
        assert_eq!(overlay_scale(2.0, true), 1.8);
        assert_eq!(overlay_px(794.0, 1123.0, 1.25), (993, 1404));
    }

    #[test]
    fn selection_box_has_the_palette_swatches() {
        let html = selection_box_html(2);
        assert!(html.starts_with("<div class=\"selection-box hidden\" data-page=\"2\">"));
        assert!(html.contains("data-swatch-color=\"#002FD9\" style=\"background:#002FD9\""));
        assert!(!html.contains("SWATCHES"));
    }
}
