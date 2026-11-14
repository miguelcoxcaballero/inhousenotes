//! Photo page modal (`#modal-cover-image`, spec/pages.md §3.4, §5). Owned by W5.1.
//!
//! A photo from the gallery (file chooser or drop) is previewed in the page it will make: "Document" keeps the
//! document's base page size with the photo contained and centred, "Photo ratio" makes a page of the photo's own
//! ratio. "Add Page" renders the page once (≤ 3× the logical size, ≤ 3840 px, never above the photo's own
//! resolution) on white as JPEG 0.94 — or keeps a PNG that exactly fills the page byte for byte — stores it as an
//! asset (SHA-256) and inserts one `Background::Custom` page after the current page in a single batch. As in 5.18.0
//! the undo history is cleared.
// v5: app-v5.js:openCoverModal / closeCoverModal / setCoverModalFile / updateCoverModalPreview /
// getCoverResultLayout / buildCoverImageAsset / confirmCoverModal / insertPhotoPageAt / setCoverModalBusy

use super::pages_panel::{self, base_size, contained, size_line};
use crate::abi::{ety, ev, life, mode, ret};
use crate::Prop;
use crate::doc::session;
use crate::editor::{history, pages, selection, view, viewport};
use crate::host::Metric;
use crate::shell::{self, fx, json::Json, Status, StatusOpts};
use crate::{tag, App, Event, Host, Mod, Node, Ui};
use ihn_core::codec::Reader;
use ihn_core::id::{Hash, Id};
use ihn_core::jsfmt::num;
use ihn_core::jsmath::round;
use ihn_core::op::{Background, Template};
use ihn_core::render::cmd::{Img, Src, Surf};

const L_PICK: u32 = tag(Mod::PanelsPhotoPage, 1);
const L_KEY: u32 = tag(Mod::PanelsPhotoPage, 2);
const L_DRAG: u32 = tag(Mod::PanelsPhotoPage, 3);
const L_LEAVE: u32 = tag(Mod::PanelsPhotoPage, 4);
const L_DROP: u32 = tag(Mod::PanelsPhotoPage, 5);
const L_INPUT: u32 = tag(Mod::PanelsPhotoPage, 6);
const L_BROWSE: u32 = tag(Mod::PanelsPhotoPage, 7);
const L_MODE: u32 = tag(Mod::PanelsPhotoPage, 8);
const L_CANCEL: u32 = tag(Mod::PanelsPhotoPage, 9);
const L_CONFIRM: u32 = tag(Mod::PanelsPhotoPage, 10);
const L_OVERLAY: u32 = tag(Mod::PanelsPhotoPage, 11);
const L_ESCAPE: u32 = tag(Mod::PanelsPhotoPage, 12);
const L_DRAGOVER: u32 = tag(Mod::PanelsPhotoPage, 13);
const L_MODE_PHOTO: u32 = tag(Mod::PanelsPhotoPage, 14);
const T_ACQUIRE: u32 = tag(Mod::PanelsPhotoPage, 32);
/// v5 `acquireLocalPageStructureMutation(timeoutMs = 12_000)`: its poll interval and deadline.
const ACQUIRE_POLL_MS: f64 = 32.0;
const ACQUIRE_TIMEOUT_MS: f64 = 12_000.0;

/// v5 `COVER_RENDER_SCALE`, `COVER_RENDER_MAX_SIDE`, `COVER_JPEG_QUALITY`, `IMAGE_ORIGINAL_MAX_BYTES`,
/// `IMAGE_INSERT_MAX_PIXELS`.
const RENDER_SCALE: f64 = 3.0;
const MAX_SIDE: f64 = 3840.0;
const JPEG_QUALITY: f64 = 0.94;
const PNG_MAX_BYTES: f64 = 4.0 * 1024.0 * 1024.0;
const MAX_PIXELS: f64 = 8.0 * 1024.0 * 1024.0;
/// Object handles reserved for the files of one `change` / `drop`.
const MAX_FILES: u32 = 16;

/// The chosen photo: the file, its decoded image, its size and its preview URL.
struct Sel {
    obj: u32,
    name: String,
    mime: String,
    size: f64,
    img: Img,
    w: f64,
    h: f64,
    url: String,
}

/// A step of the selection or of the confirm.
enum Step {
    Idle,
    Take { base: u32, drop: bool },
    Decode { obj: u32, name: String, mime: String, size: f64, img: Img },
    Url { obj: u32, name: String, mime: String, size: f64, img: Img, w: f64, h: f64 },
    Encode { surf: Surf, page: (f64, f64) },
    Digest { blob: u32, mime: String, size: f64, page: (f64, f64) },
    /// The page is ready; the structure edit waits for the running interaction to settle.
    Acquire { hash: Hash, blob: u32, mime: String, size: f64, page: (f64, f64), deadline: f64 },
}

pub struct State {
    open: bool,
    busy: bool,
    /// "Photo ratio" is checked.
    photo: bool,
    sel: Option<Sel>,
    step: Step,
    req: u32,
    /// The document and the page the photo page goes after (anchor by id, as 5.18.0).
    doc: String,
    after: Id,
    /// The `T_ACQUIRE` poll is armed.
    acquiring: bool,
    /// Photo pages added (test counter).
    pub added: u32,
}

impl Default for State {
    fn default() -> Self { State { open: false, busy: false, photo: false, sel: None, step: Step::Idle, req: 0, doc: String::new(), after: Id::NONE, acquiring: false, added: 0 } }
}

fn photo_mode(app: &App) -> bool { app.st.panels_photo_page.photo }

/// v5 `normalizePageDimension(value)`.
fn dim(v: f64) -> f64 { round(v).max(64.0) }

/// v5 `getCoverResultLayout(w, h, mode)` → `(page, image)` sizes.
pub fn layout(doc: (f64, f64), img: (f64, f64), photo: bool) -> ((f64, f64), (f64, f64)) {
    let fitted = contained(img.0, img.1, doc.0, doc.1);
    if photo {
        let page = (dim(fitted.0), dim(fitted.1));
        (page, contained(img.0, img.1, page.0, page.1))
    } else {
        (doc, fitted)
    }
}

fn base(app: &App) -> (f64, f64) { base_size(session::get(app).map(|s| &s.doc)) }

/// v5 `updateCoverModalConfirmState()`.
fn confirm_state(app: &mut App, host: &mut dyn Host) {
    let st = &app.st.panels_photo_page;
    let disabled = st.busy || st.sel.is_none();
    host.out().prop(Ui::COVER_MODAL_CONFIRM, "disabled", Prop::Bool(disabled));
}

/// v5 `setCoverModalBusy(busy)`.
fn set_busy(app: &mut App, host: &mut dyn Host, busy: bool) {
    app.st.panels_photo_page.busy = busy;
    let out = host.out();
    for n in [Ui::BTN_COVER_BROWSE, Ui::COVER_MODAL_CANCEL, Ui::COVER_IMAGE_INPUT, Ui::COVER_LAYOUT_FIT, Ui::COVER_LAYOUT_PHOTO] {
        out.prop(n, "disabled", Prop::Bool(busy));
    }
    out.attr(Ui::COVER_UPLOAD_DROPZONE, "aria-disabled", Some(if busy { "true" } else { "false" }));
    confirm_state(app, host);
}

/// v5 `updateCoverModalPreview()`.
fn update_preview(app: &mut App, host: &mut dyn Host) {
    let doc = base(app);
    let aw = host.metric(Ui::COVER_PREVIEW_STAGE, Metric::ClientWidth);
    let ah = host.metric(Ui::COVER_PREVIEW_STAGE, Metric::ClientHeight);
    let (aw, ah) = (if aw > 0.0 { aw } else { 460.0 }, if ah > 0.0 { ah } else { 320.0 });
    let photo = photo_mode(app);
    let px = |v: f64| format!("{}px", num(v));
    let Some(sel) = app.st.panels_photo_page.sel.as_ref() else {
        let (pw, ph) = contained(doc.0, doc.1, aw, ah);
        let out = host.out();
        out.class(Ui::COVER_PREVIEW_STAGE, "is-empty", true);
        out.class(Ui::COVER_PREVIEW_EMPTY, "hidden", false);
        out.style(Ui::COVER_PREVIEW_PAGE, "width", Some(&px(pw)));
        out.style(Ui::COVER_PREVIEW_PAGE, "height", Some(&px(ph)));
        out.attr(Ui::COVER_PREVIEW_IMAGE, "src", None);
        out.style(Ui::COVER_PREVIEW_IMAGE, "width", Some("0"));
        out.style(Ui::COVER_PREVIEW_IMAGE, "height", Some("0"));
        out.text(Ui::COVER_MODAL_META, &size_line(doc.0, doc.1));
        out.text(Ui::COVER_BROWSE_LABEL, "Choose photo");
        out.attr(Ui::COVER_UPLOAD_DROPZONE, "aria-label", Some("Choose photo"));
        return confirm_state(app, host);
    };
    let (page, image) = layout(doc, (sel.w, sel.h), photo);
    let (pw, ph) = contained(page.0, page.1, aw, ah);
    let scale = pw / page.0;
    let (iw, ih) = (round(image.0 * scale).max(1.0), round(image.1 * scale).max(1.0));
    let (url, label) = (sel.url.clone(), format!("Replace {}", sel.name));
    let out = host.out();
    out.class(Ui::COVER_PREVIEW_STAGE, "is-empty", false);
    out.class(Ui::COVER_PREVIEW_EMPTY, "hidden", true);
    out.style(Ui::COVER_PREVIEW_PAGE, "width", Some(&px(pw)));
    out.style(Ui::COVER_PREVIEW_PAGE, "height", Some(&px(ph)));
    out.attr(Ui::COVER_PREVIEW_IMAGE, "src", Some(&url));
    out.style(Ui::COVER_PREVIEW_IMAGE, "width", Some(&px(iw)));
    out.style(Ui::COVER_PREVIEW_IMAGE, "height", Some(&px(ih)));
    out.text(Ui::COVER_MODAL_META, &size_line(page.0, page.1));
    out.text(Ui::COVER_BROWSE_LABEL, "Replace photo");
    out.attr(Ui::COVER_UPLOAD_DROPZONE, "aria-label", Some(&label));
    confirm_state(app, host);
}

fn drop_step(app: &mut App, host: &mut dyn Host) {
    let step = std::mem::replace(&mut app.st.panels_photo_page.step, Step::Idle);
    app.st.panels_photo_page.req = 0;
    match step {
        Step::Idle | Step::Take { .. } => {}
        Step::Decode { obj, img, .. } | Step::Url { obj, img, .. } => {
            host.out().obj_release(obj);
            pages::with_gfx(app, |_, g| g.cmds.image_drop(img));
        }
        Step::Encode { surf, .. } => pages::with_gfx(app, |_, g| g.cmds.drop_surf(surf)),
        Step::Digest { blob, .. } | Step::Acquire { blob, .. } => host.out().obj_release(blob),
    }
    if std::mem::take(&mut app.st.panels_photo_page.acquiring) {
        app.sched.clear(host.out(), T_ACQUIRE);
    }
    pages::flush(app, host);
}

/// v5 `clearCoverModalSelection()`.
fn clear_selection(app: &mut App, host: &mut dyn Host) {
    if !matches!(app.st.panels_photo_page.step, Step::Encode { .. } | Step::Digest { .. } | Step::Acquire { .. }) {
        drop_step(app, host);
    }
    if let Some(sel) = app.st.panels_photo_page.sel.take() {
        pages::with_gfx(app, |_, g| g.cmds.image_drop(sel.img));
        pages::flush(app, host);
        let out = host.out();
        fx::url_revoke(out, &sel.url);
        out.obj_release(sel.obj);
    }
    let out = host.out();
    out.prop(Ui::COVER_IMAGE_INPUT, "value", Prop::Str(""));
    out.class(Ui::COVER_UPLOAD_DROPZONE, "dragover", false);
    update_preview(app, host);
}

/// v5 `openCoverModal()`.
pub fn open(app: &mut App, host: &mut dyn Host) {
    if view::read_only(app) {
        return;
    }
    app.st.panels_photo_page.photo = false;
    let out = host.out();
    out.prop(Ui::COVER_LAYOUT_FIT, "checked", Prop::Bool(true));
    out.prop(Ui::COVER_LAYOUT_PHOTO, "checked", Prop::Bool(false));
    set_busy(app, host, false);
    clear_selection(app, host);
    app.st.panels_photo_page.open = true;
    host.out().class(Ui::MODAL_COVER_IMAGE, "visible", true);
    app.sched.frame(host.out(), Mod::PanelsPhotoPage);
}

/// v5 `closeCoverModal()`.
pub fn close(app: &mut App, host: &mut dyn Host) {
    if !app.st.panels_photo_page.open || app.st.panels_photo_page.busy {
        return;
    }
    app.st.panels_photo_page.open = false;
    host.out().class(Ui::MODAL_COVER_IMAGE, "visible", false);
    clear_selection(app, host);
}

/// v5 `openCoverImagePicker()`.
fn picker(app: &mut App, host: &mut dyn Host) {
    if view::read_only(app) || app.st.panels_photo_page.busy {
        return;
    }
    let out = host.out();
    out.prop(Ui::COVER_IMAGE_INPUT, "value", Prop::Str(""));
    out.click(Ui::COVER_IMAGE_INPUT);
}

/// Takes the files of a `change` / `drop` event (`obj`).
fn take(app: &mut App, host: &mut dyn Host, obj: u32, drop: bool) {
    drop_step(app, host);
    let base = fx::new_obj(app);
    for _ in 1..MAX_FILES {
        fx::new_obj(app);
    }
    let req = app.req(Mod::PanelsPhotoPage);
    let out = host.out();
    fx::files_take(out, req, obj, base, drop);
    out.obj_release(obj);
    let st = &mut app.st.panels_photo_page;
    st.req = req;
    st.step = Step::Take { base, drop };
}

/// v5 `setCoverModalFile(file)` (start): the file is decoded, then gets its preview URL.
fn set_file(app: &mut App, host: &mut dyn Host, obj: u32, name: String, mime: String, size: f64) {
    if !mime.starts_with("image/") {
        host.out().obj_release(obj);
        shell::status(app, host, "Choose an image file", StatusOpts::from(Status::Error).preserve());
        return;
    }
    host.out().text(Ui::COVER_BROWSE_LABEL, "Loading…");
    confirm_state(app, host);
    let req = app.req(Mod::PanelsPhotoPage);
    let img = pages::with_gfx(app, |_, g| g.cmds.image_blob(req, obj));
    pages::flush(app, host);
    let st = &mut app.st.panels_photo_page;
    st.req = req;
    st.step = Step::Decode { obj, name, mime, size, img };
}

fn read_failed(app: &mut App, host: &mut dyn Host) {
    shell::status(app, host, "Could not read that photo", StatusOpts::from(Status::Error).preserve());
    update_preview(app, host);
}

/// v5 `confirmCoverModal()`.
fn confirm(app: &mut App, host: &mut dyn Host) {
    let st = &app.st.panels_photo_page;
    if st.sel.is_none() || st.busy || view::read_only(app) || !matches!(st.step, Step::Idle) {
        return;
    }
    let Some(order) = session::get(app).map(|s| s.doc.order().to_vec()) else { return };
    let after = pages::active_index(app).or(order.len().checked_sub(1)).and_then(|i| order.get(i).copied()).unwrap_or(Id::NONE);
    app.st.panels_photo_page.doc = session::doc_id(app).to_string();
    app.st.panels_photo_page.after = after;
    set_busy(app, host, true);
    shell::loading(app, host, "Adding photo page...");
    build(app, host);
}

/// v5 `buildCoverImageAsset(file, mode)`.
fn build(app: &mut App, host: &mut dyn Host) {
    let photo = photo_mode(app);
    let doc = base(app);
    let req = app.req(Mod::PanelsPhotoPage);
    let Some(sel) = app.st.panels_photo_page.sel.as_ref() else { return failed(app, host) };
    let (page, image) = layout(doc, (sel.w, sel.h), photo);
    if sel.mime == "image/png" && sel.size <= PNG_MAX_BYTES && sel.w.max(sel.h) <= MAX_SIDE && sel.w * sel.h <= MAX_PIXELS && image == page {
        // The original PNG is the page: kept byte for byte (a second handle to the same file).
        let (obj, size) = (sel.obj, sel.size);
        let copy = fx::new_obj(app);
        fx::blob_slice(host.out(), copy, obj, 0.0, -1.0, "");
        return digest(app, host, copy, "image/png".into(), size, page, req);
    }
    let mut scale = RENDER_SCALE.min(MAX_SIDE / page.0.max(page.1)).min(sel.w / image.0).min(sel.h / image.1);
    if !scale.is_finite() || scale <= 0.0 {
        scale = 1.0;
    }
    scale = scale.max(0.05);
    let (tw, th) = (round(page.0 * scale).max(1.0), round(page.1 * scale).max(1.0));
    let (iw, ih) = (round(image.0 * scale).max(1.0), round(image.1 * scale).max(1.0));
    let (ox, oy) = (round((tw - iw) / 2.0), round((th - ih) / 2.0));
    let img = sel.img;
    let surf = pages::with_gfx(app, |_, g| {
        let c = &mut g.cmds;
        let s = c.processing(tw as u32, th as u32);
        c.ctx(s, false);
        c.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
        c.fill_style("#FFFFFF");
        c.fill_rect(0.0, 0.0, tw, th);
        c.draw_image(Src::Img(img), &[ox, oy, iw, ih]);
        c.to_blob(s, req, "image/jpeg", JPEG_QUALITY);
        s
    });
    pages::flush(app, host);
    let st = &mut app.st.panels_photo_page;
    st.req = req;
    st.step = Step::Encode { surf, page };
}

fn digest(app: &mut App, host: &mut dyn Host, blob: u32, mime: String, size: f64, page: (f64, f64), req: u32) {
    fx::digest(host.out(), req, fx::Src::blob(blob));
    let st = &mut app.st.panels_photo_page;
    st.req = req;
    st.step = Step::Digest { blob, mime, size, page };
}

/// v5 `acquireLocalPageStructureMutation()`: a page structure edit waits while a smooth interaction runs (the tap
/// on Add Page itself is one until its gesture settles), polling every 32 ms for at most 12 s.
fn acquire(app: &mut App, host: &mut dyn Host, hash: Hash, blob: u32, mime: String, size: f64, page: (f64, f64), deadline: f64) {
    if viewport::smooth(app) && session::doc_id(app) == app.st.panels_photo_page.doc {
        if host.time() >= deadline {
            // v5: "Still syncing page order…", then the insert's failure replaces it.
            host.out().obj_release(blob);
            return failed(app, host);
        }
        let st = &mut app.st.panels_photo_page;
        st.step = Step::Acquire { hash, blob, mime, size, page, deadline };
        st.acquiring = true;
        app.sched.timer(host.out(), T_ACQUIRE, ACQUIRE_POLL_MS);
        return;
    }
    insert(app, host, hash, blob, &mime, size, page);
}

/// v5 `insertPhotoPageAt(asset, insertAfter)`.
fn insert(app: &mut App, host: &mut dyn Host, hash: Hash, blob: u32, mime: &str, size: f64, page: (f64, f64)) {
    let st = &app.st.panels_photo_page;
    if session::doc_id(app) != st.doc || view::read_only(app) {
        host.out().obj_release(blob);
        return abort(app, host);
    }
    let after = st.after;
    if session::get(app).is_some_and(|s| s.asset_blob(&hash).is_some()) {
        host.out().obj_release(blob);
    } else {
        session::put_asset(app, host, hash, blob, mime, size);
    }
    selection::clear(app, host);
    let mut id = Id::NONE;
    let anchor = (after != Id::NONE).then_some(after);
    let Some(e) = session::edit(app, host, |tx| id = tx.page_new(anchor, Template::Default, (page.0 as u32, page.1 as u32), Background::Custom { asset: hash })) else {
        return failed(app, host);
    };
    history::clear(app, host);
    pages::changed(app, host, &e.changes);
    let at = pages::order(app).iter().position(|p| *p == id).unwrap_or(0);
    pages::set_active_page(app, at);
    if pages_panel::is_open(app) {
        pages_panel::refresh(app, host);
    }
    viewport::go_to_page(app, host, at);
    app.st.panels_photo_page.added += 1;
    shell::status(app, host, "Photo page added", StatusOpts::from(Status::Info).preserve());
    shell::hide_loading(app, host);
    set_busy(app, host, false);
    close(app, host);
}

fn failed(app: &mut App, host: &mut dyn Host) {
    shell::status(app, host, "Failed to add photo page", StatusOpts::from(Status::Error).preserve());
    shell::hide_loading(app, host);
    set_busy(app, host, false);
}

/// The document changed under the confirm: silent abort (v5 `AbortError`).
fn abort(app: &mut App, host: &mut dyn Host) {
    app.st.panels_photo_page.busy = false;
    let _ = host;
}

fn f64s(data: &[u8]) -> (f64, f64) {
    let mut r = Reader::new(data);
    (r.f64().unwrap_or(0.0), r.f64().unwrap_or(0.0))
}

/// One step finished.
fn step_done(app: &mut App, host: &mut dyn Host, status: i32, obj: u32, data: &[u8]) {
    let step = std::mem::replace(&mut app.st.panels_photo_page.step, Step::Idle);
    app.st.panels_photo_page.req = 0;
    let ok = status >= 0;
    match step {
        Step::Idle => {}
        Step::Acquire { blob, .. } => host.out().obj_release(blob),
        Step::Take { base, drop } => {
            let list = Json::parse(std::str::from_utf8(data).unwrap_or("")).unwrap_or(Json::Null);
            let files = list.items();
            let n = if ok { (status as usize).min(files.len()).min(MAX_FILES as usize) } else { 0 };
            // v5: the first file of `change`, the first image of a drop.
            let pick = (0..n).find(|&i| !drop || files[i].text("type").starts_with("image/"));
            for i in 0..n {
                if Some(i) != pick {
                    host.out().obj_release(base + i as u32);
                }
            }
            if let Some(i) = pick {
                let f = &files[i];
                let (name, mime, size) = (f.text("name").to_string(), f.text("type").to_string(), f.get("size").and_then(Json::as_f64).unwrap_or(0.0));
                set_file(app, host, base + i as u32, name, mime, size);
            }
        }
        Step::Decode { obj, name, mime, size, img } => {
            let (w, h) = if ok { f64s(data) } else { (0.0, 0.0) };
            if !(w > 0.0 && h > 0.0) {
                host.out().obj_release(obj);
                pages::with_gfx(app, |_, g| g.cmds.image_drop(img));
                pages::flush(app, host);
                return read_failed(app, host);
            }
            let req = app.req(Mod::PanelsPhotoPage);
            fx::blob_url(host.out(), req, obj);
            let st = &mut app.st.panels_photo_page;
            st.req = req;
            st.step = Step::Url { obj, name, mime, size, img, w: round(w).max(1.0), h: round(h).max(1.0) };
        }
        Step::Url { obj, name, mime, size, img, w, h } => {
            let url = std::str::from_utf8(data).unwrap_or("").to_string();
            if !ok || url.is_empty() {
                host.out().obj_release(obj);
                pages::with_gfx(app, |_, g| g.cmds.image_drop(img));
                pages::flush(app, host);
                return read_failed(app, host);
            }
            if let Some(old) = app.st.panels_photo_page.sel.take() {
                pages::with_gfx(app, |_, g| g.cmds.image_drop(old.img));
                pages::flush(app, host);
                fx::url_revoke(host.out(), &old.url);
                host.out().obj_release(old.obj);
            }
            app.st.panels_photo_page.sel = Some(Sel { obj, name, mime, size, img, w, h, url });
            update_preview(app, host);
        }
        Step::Encode { surf, page } => {
            pages::with_gfx(app, |_, g| g.cmds.drop_surf(surf));
            pages::flush(app, host);
            if !ok || obj == 0 {
                host.out().obj_release(obj);
                return failed(app, host);
            }
            let req = app.req(Mod::PanelsPhotoPage);
            digest(app, host, obj, "image/jpeg".into(), f64s(data).0, page, req);
        }
        Step::Digest { blob, mime, size, page } => match data.get(..32).and_then(|d| <Hash>::try_from(d).ok()).filter(|_| ok) {
            Some(hash) => {
                let deadline = host.time() + ACQUIRE_TIMEOUT_MS;
                acquire(app, host, hash, blob, mime, size, page, deadline)
            }
            None => {
                host.out().obj_release(blob);
                failed(app, host);
            }
        },
    }
}

pub fn init(app: &mut App, host: &mut dyn Host) {
    if app.cfg.has(mode::HEADLESS) {
        return;
    }
    let out = host.out();
    let z = Ui::COVER_UPLOAD_DROPZONE;
    out.listen(z, "click", L_PICK, 0);
    out.listen(z, "keydown", L_KEY, 0);
    out.listen(z, "dragenter", L_DRAG, 0);
    out.listen(z, "dragover", L_DRAGOVER, 0);
    out.listen(z, "dragleave", L_LEAVE, 0);
    out.listen(z, "drop", L_DROP, 0);
    out.listen(Ui::COVER_IMAGE_INPUT, "change", L_INPUT, 0);
    out.listen(Ui::BTN_COVER_BROWSE, "click", L_BROWSE, 0);
    out.listen(Ui::COVER_LAYOUT_FIT, "change", L_MODE, 0);
    out.listen(Ui::COVER_LAYOUT_PHOTO, "change", L_MODE_PHOTO, 0);
    out.listen(Ui::COVER_MODAL_CANCEL, "click", L_CANCEL, 0);
    out.listen(Ui::COVER_MODAL_CONFIRM, "click", L_CONFIRM, 0);
    out.listen(Ui::MODAL_COVER_IMAGE, "click", L_OVERLAY, 0);
    out.listen(Node::DOCUMENT, "keydown", L_ESCAPE, 0);
}

pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    let busy = app.st.panels_photo_page.busy;
    match event {
        Event::Dom(d) => match d.listener {
            L_PICK => picker(app, host),
            L_KEY if d.key == "Enter" || d.key == " " => {
                picker(app, host);
                return ret::PREVENT_DEFAULT;
            }
            L_DRAG | L_DRAGOVER if !busy => {
                host.out().class(Ui::COVER_UPLOAD_DROPZONE, "dragover", true);
                return ret::PREVENT_DEFAULT | if d.ty == ety::DRAGOVER { ret::DROP_COPY } else { 0 };
            }
            L_LEAVE if !d.related.contains(Ui::COVER_UPLOAD_DROPZONE) => host.out().class(Ui::COVER_UPLOAD_DROPZONE, "dragover", false),
            L_DROP => {
                if busy {
                    host.out().obj_release(d.obj);
                    return 0;
                }
                host.out().class(Ui::COVER_UPLOAD_DROPZONE, "dragover", false);
                take(app, host, d.obj, true);
                return ret::PREVENT_DEFAULT;
            }
            L_INPUT => {
                if d.obj != 0 {
                    take(app, host, d.obj, false);
                }
            }
            L_BROWSE => {
                picker(app, host);
                return ret::PREVENT_DEFAULT | ret::STOP_PROPAGATION;
            }
            L_MODE | L_MODE_PHOTO => {
                app.st.panels_photo_page.photo = d.listener == L_MODE_PHOTO;
                update_preview(app, host);
            }
            L_CANCEL => close(app, host),
            L_CONFIRM => confirm(app, host),
            L_OVERLAY if d.flags & ev::EXACT != 0 && d.path.first() == Ui::MODAL_COVER_IMAGE => close(app, host),
            L_ESCAPE if d.key == "Escape" && app.st.panels_photo_page.open => close(app, host),
            _ => {}
        },
        Event::Frame(_) if app.st.panels_photo_page.open => {
            update_preview(app, host);
            host.out().focus(Ui::COVER_UPLOAD_DROPZONE, false);
        }
        Event::Timer(T_ACQUIRE) => {
            app.st.panels_photo_page.acquiring = false;
            if let Step::Acquire { hash, blob, mime, size, page, deadline } = std::mem::replace(&mut app.st.panels_photo_page.step, Step::Idle) {
                acquire(app, host, hash, blob, mime, size, page, deadline);
            }
        }
        Event::Io(io) if io.req != 0 && io.req == app.st.panels_photo_page.req => step_done(app, host, io.status, io.obj, io.data),
        Event::Io(io) => host.out().obj_release(io.obj),
        Event::Life(l) if l.code == life::RESIZE && app.st.panels_photo_page.open => update_preview(app, host),
        Event::Msg(m) if m.chan == session::sig::CLOSED => {
            if app.st.panels_photo_page.busy {
                drop_step(app, host);
                app.st.panels_photo_page.busy = false;
            }
            close(app, host);
        }
        _ => {}
    }
    0
}

/// `panels.photo_page.state` → `{open, busy, selected, photo, added, size: [w, h]}`.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    let st = &app.st.panels_photo_page;
    match name {
        "state" => Some(format!(
            "{{\"open\":{},\"busy\":{},\"selected\":{},\"photo\":{},\"added\":{},\"size\":[{},{}]}}",
            st.open,
            st.busy,
            st.sel.is_some(),
            st.photo,
            st.added,
            num(st.sel.as_ref().map_or(0.0, |s| s.w)),
            num(st.sel.as_ref().map_or(0.0, |s| s.h))
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_follow_5_18_0() {
        // Document: the A4 page, the photo contained.
        assert_eq!(layout((794.0, 1123.0), (4000.0, 3000.0), false), ((794.0, 1123.0), (794.0, 596.0)));
        // Photo ratio: the page takes the contained photo's size; the photo fills it.
        assert_eq!(layout((794.0, 1123.0), (4000.0, 3000.0), true), ((794.0, 596.0), (794.0, 596.0)));
        // Tiny pages are at least 64 px on each side.
        assert_eq!(layout((794.0, 1123.0), (10000.0, 100.0), true).0, (794.0, 64.0));
    }
}
