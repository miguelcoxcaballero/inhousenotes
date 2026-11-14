//! Editor: inline images — paste, drop, insert, place (5.18.0 `insertImageBlobsAtClientPoint`,
//! `compressImageForDocument`, `addImageToPage`, spec/editor.md §5.7).
//!
//! A paste on the document or a drop of image files on `#canvas-viewport` takes the files of the event
//! (`fx::files_take`, inside the event) and queues them as one batch, aimed at the page nearest to the point (the
//! last pointer position for a paste, else the viewport centre). The files are handled one at a time, as 5.18.0
//! awaits them: decode (`Cmds::image_blob`) → a PNG ≤ 4 MiB that needs no scaling is kept byte-exact, anything else is
//! drawn once on a processing canvas at `min(1, 3200/max(w,h), √(8 MiPx/(w·h)))` and encoded once (PNG stays PNG,
//! else JPEG 0.94 on white) → SHA-256 → the asset (stored once per hash, `session::put_asset`) → one `ItemPut`
//! batch and one undo step. Moving, scaling, rotating, duplicating and deleting images is the lasso selection's.
//! Placement uses the page's real size (BEHAVIOUR-CHANGES B-E1).

use super::{history, input, pages, selection, view, viewport};
use crate::abi::{ety, ev, listen, ptr, ret};
use crate::doc::session;
use crate::shell::{self, fx, json::Json, StatusOpts};
use crate::{tag, App, Event, Host, Mod, Node, Ui};
use ihn_core::codec::Reader;
use ihn_core::geom::{js_max, js_min};
use ihn_core::id::{Hash, Id};
use ihn_core::op::ImageRect;
use ihn_core::render::cmd::{Img, Surf};
use std::collections::VecDeque;

const L_PASTE: u32 = tag(Mod::EditorImages, 1);
const L_DRAGOVER: u32 = tag(Mod::EditorImages, 2);
const L_DROP: u32 = tag(Mod::EditorImages, 3);
const L_POINTER: u32 = tag(Mod::EditorImages, 4);
const L_MOUSE: u32 = tag(Mod::EditorImages, 5);

/// v5 `IMAGE_INSERT_MAX_DIM`, `IMAGE_INSERT_MAX_PIXELS`, `IMAGE_ORIGINAL_MAX_BYTES`.
pub const MAX_DIM: f64 = 3200.0;
pub const MAX_PIXELS: f64 = 8.0 * 1024.0 * 1024.0;
pub const ORIGINAL_MAX_BYTES: f64 = 4.0 * 1024.0 * 1024.0;
/// Object handles reserved per event for its files (`fx::files_take` numbers them from a base).
const MAX_FILES: u32 = 256;

struct File {
    obj: u32,
    mime: String,
    size: f64,
}

struct Batch {
    doc: String,
    page: Id,
    /// The point in page coordinates.
    pos: (f64, f64),
    /// `files_take` request (0 once its answer arrived) and the first object handle of its files.
    take: u32,
    base: u32,
    files: VecDeque<File>,
    inserted: u32,
}

enum Step {
    Idle,
    Decode { img: Img, file: File },
    Encode { surf: Surf, w: f64, h: f64, mime: &'static str },
    Digest { blob: u32, mime: String, size: f64, w: f64, h: f64 },
}

pub struct State {
    /// v5 `lastPointerClientX/Y`.
    last: (f64, f64),
    batches: VecDeque<Batch>,
    step: Step,
    /// The request of the running step.
    req: u32,
    /// Images placed (test counter) and the newest one (page, item).
    pub added: u32,
    newest: (Id, Id),
}

impl Default for State {
    fn default() -> Self { State { last: (0.0, 0.0), batches: VecDeque::new(), step: Step::Idle, req: 0, added: 0, newest: (Id::NONE, Id::NONE) } }
}

/// v5 `clamp(value, min, max)`.
fn clamp(v: f64, lo: f64, hi: f64) -> f64 { js_min(hi, js_max(lo, v)) }

/// v5 `compressImageForDocument`: the scale of a decoded `w × h` image.
pub fn compress_scale(w: f64, h: f64) -> f64 { js_min(js_min(1.0, MAX_DIM / js_max(w, h)), (MAX_PIXELS / (w * h)).sqrt()) }

/// The PNG is kept byte-exact (5.18.0: `type === 'image/png' && scale === 1 && size <= 4 MiB`).
pub fn keep_original(mime: &str, size: f64, scale: f64) -> bool { mime == "image/png" && scale == 1.0 && size <= ORIGINAL_MAX_BYTES }

/// v5 `addImageToPage` → `placeImage`: centre and size of the `n`-th image of a batch on a `pw × ph` page.
pub fn place_rect(natural: (f64, f64), pos: (f64, f64), n: u32, pw: f64, ph: f64) -> ImageRect {
    let (max_w, max_h) = (pw * 0.6, ph * 0.45);
    let nw = if natural.0 > 0.0 { natural.0 } else { max_w };
    let nh = if natural.1 > 0.0 { natural.1 } else { max_h };
    let scale = js_min(js_min(max_w / nw, max_h / nh), 1.0);
    let (w, h) = (nw * scale, nh * scale);
    let offset = n as f64 * 18.0;
    ImageRect { cx: clamp(pos.0 + offset, w / 2.0, pw - w / 2.0), cy: clamp(pos.1 + offset, h / 2.0, ph - h / 2.0), w, h, rotation: 0.0 }
}

/// v5 `getPageIndexFromClientPoint` + `getPagePosFromClient`: the page whose wrapper is nearest to the point and
/// the point in its coordinates.
fn page_at(app: &App, host: &mut dyn Host, x: f64, y: f64) -> Option<(Id, (f64, f64))> {
    let mut best: Option<(f64, Id, crate::Rect, f64, f64)> = None;
    for r in &app.st.editor_pages.rows {
        let rect = host.rect(r.wrapper);
        let dx = js_max(js_max(rect.x - x, 0.0), x - (rect.x + rect.w));
        let dy = js_max(js_max(rect.y - y, 0.0), y - (rect.y + rect.h));
        let d = ihn_core::jsmath::hypot(dx, dy);
        if best.as_ref().is_none_or(|b| d < b.0) {
            best = Some((d, r.id, rect, r.w, r.h));
        }
    }
    let (_, id, rect, w, h) = best?;
    let pos = if rect.w > 0.0 && rect.h > 0.0 { pages::to_page(rect, w, h, x, y) } else { (0.0, 0.0) };
    Some((id, pos))
}

/// v5 `getBestInsertClientPoint`.
fn insert_point(app: &App, host: &mut dyn Host) -> (f64, f64) {
    let (x, y) = app.st.editor_images.last;
    if x != 0.0 || y != 0.0 {
        return (x, y);
    }
    let r = host.rect(Ui::CANVAS_VIEWPORT);
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

/// v5 `insertImageBlobsAtClientPoint` (start): takes the image files of event object `obj` for the page nearest to
/// the client point. Must run while the paste or drop event is dispatched.
fn take(app: &mut App, host: &mut dyn Host, obj: u32, x: f64, y: f64) {
    let target = session::get(app).filter(|s| s.writable()).and_then(|_| page_at(app, host, x, y));
    let Some((page, pos)) = target else {
        host.out().obj_release(obj);
        return;
    };
    let base = fx::new_obj(app);
    for _ in 1..MAX_FILES {
        fx::new_obj(app);
    }
    let req = app.req(Mod::EditorImages);
    let out = host.out();
    fx::files_take(out, req, obj, base, true);
    out.obj_release(obj);
    let doc = session::doc_id(app).to_string();
    app.st.editor_images.batches.push_back(Batch { doc, page, pos, take: req, base, files: VecDeque::new(), inserted: 0 });
}

fn files_taken(app: &mut App, req: u32, status: i32, data: &[u8]) {
    let Some(b) = app.st.editor_images.batches.iter_mut().find(|b| b.take == req) else { return };
    b.take = 0;
    if status <= 0 {
        return;
    }
    let list = Json::parse(std::str::from_utf8(data).unwrap_or("")).unwrap_or(Json::Null);
    for (i, f) in list.items().iter().take(MAX_FILES as usize).enumerate() {
        let size = f.get("size").and_then(Json::as_f64).unwrap_or(0.0);
        b.files.push_back(File { obj: b.base.wrapping_add(i as u32), mime: f.text("type").into(), size });
    }
}

/// The next step: the head batch's next file, or the end of the batch (its status line).
fn pump(app: &mut App, host: &mut dyn Host) {
    while matches!(app.st.editor_images.step, Step::Idle) {
        let current = session::doc_id(app).to_string();
        let Some(b) = app.st.editor_images.batches.front_mut() else { return };
        if b.take != 0 {
            return;
        }
        let stale = b.doc != current;
        let next = b.files.pop_front();
        match next {
            Some(file) if stale => host.out().obj_release(file.obj),
            Some(file) => {
                let req = app.req(Mod::EditorImages);
                let img = pages::with_gfx(app, |_, g| g.cmds.image_blob(req, file.obj));
                pages::flush(app, host);
                let st = &mut app.st.editor_images;
                st.req = req;
                st.step = Step::Decode { img, file };
            }
            None => {
                let b = app.st.editor_images.batches.pop_front().expect("head batch");
                if b.inserted > 0 && !stale {
                    let text = if b.inserted == 1 { "Image added".to_string() } else { format!("{} images added", b.inserted) };
                    shell::status(app, host, &text, StatusOpts::default().preserve());
                }
            }
        }
    }
}

fn f64s(data: &[u8]) -> (f64, f64) {
    let mut r = Reader::new(data);
    (r.f64().unwrap_or(0.0), r.f64().unwrap_or(0.0))
}

/// One step finished (`io.req == st.req`).
fn step_done(app: &mut App, host: &mut dyn Host, ok: bool, obj: u32, data: &[u8]) {
    let step = std::mem::replace(&mut app.st.editor_images.step, Step::Idle);
    app.st.editor_images.req = 0;
    match step {
        Step::Idle => {}
        Step::Decode { img, file } => {
            let (w, h) = if ok { f64s(data) } else { (0.0, 0.0) };
            if !(w > 0.0 && h > 0.0) {
                pages::with_gfx(app, |_, g| g.cmds.image_drop(img));
                pages::flush(app, host);
                host.out().obj_release(file.obj);
            } else {
                let scale = compress_scale(w, h);
                if keep_original(&file.mime, file.size, scale) {
                    pages::with_gfx(app, |_, g| g.cmds.image_drop(img));
                    pages::flush(app, host);
                    digest(app, host, file.obj, file.mime, file.size, w, h);
                } else {
                    // v5 `createProcessingCanvas(w·scale, h·scale)`, one draw, one encode.
                    let png = file.mime == "image/png";
                    let cw = js_max(1.0, ihn_core::jsmath::round(w * scale));
                    let ch = js_max(1.0, ihn_core::jsmath::round(h * scale));
                    let mime = if png { "image/png" } else { "image/jpeg" };
                    let req = app.req(Mod::EditorImages);
                    let surf = pages::with_gfx(app, |_, g| {
                        let c = &mut g.cmds;
                        let surf = c.processing(cw as u32, ch as u32);
                        c.ctx(surf, false);
                        if !png {
                            c.fill_style("#FFFFFF");
                            c.fill_rect(0.0, 0.0, cw, ch);
                        }
                        c.draw_image(ihn_core::render::cmd::Src::Img(img), &[0.0, 0.0, cw, ch]);
                        c.to_blob(surf, req, mime, 0.94);
                        c.image_drop(img);
                        surf
                    });
                    pages::flush(app, host);
                    host.out().obj_release(file.obj);
                    let st = &mut app.st.editor_images;
                    st.req = req;
                    st.step = Step::Encode { surf, w: cw, h: ch, mime };
                }
            }
        }
        Step::Encode { surf, w, h, mime } => {
            pages::with_gfx(app, |_, g| g.cmds.drop_surf(surf));
            pages::flush(app, host);
            if ok && obj != 0 {
                digest(app, host, obj, mime.into(), f64s(data).0, w, h);
            } else {
                host.out().obj_release(obj);
            }
        }
        Step::Digest { blob, mime, size, w, h } => {
            let hash: Option<Hash> = if ok { data.get(..32).and_then(|d| d.try_into().ok()) } else { None };
            match hash {
                Some(hash) => placed(app, host, hash, blob, &mime, size, (w, h)),
                None => host.out().obj_release(blob),
            }
        }
    }
    pump(app, host);
}

fn digest(app: &mut App, host: &mut dyn Host, blob: u32, mime: String, size: f64, w: f64, h: f64) {
    let req = app.req(Mod::EditorImages);
    fx::digest(host.out(), req, fx::Src::blob(blob));
    let st = &mut app.st.editor_images;
    st.req = req;
    st.step = Step::Digest { blob, mime, size, w, h };
}

/// The asset is known: store it once and place the image (v5 `addImageToPage` → `placeImage`).
fn placed(app: &mut App, host: &mut dyn Host, hash: Hash, blob: u32, mime: &str, size: f64, natural: (f64, f64)) {
    let current = session::doc_id(app).to_string();
    let Some(b) = app.st.editor_images.batches.front() else {
        host.out().obj_release(blob);
        return;
    };
    let (page, pos, n) = (b.page, b.pos, b.inserted);
    let page_size = session::get(app).filter(|_| b.doc == current).and_then(|s| s.doc.order().contains(&page).then(|| s.doc.pages.get(&page)).flatten()).map(|p| (p.size.v.0 as f64, p.size.v.1 as f64));
    let Some((pw, ph)) = page_size else {
        host.out().obj_release(blob);
        return;
    };
    if session::get(app).is_some_and(|s| s.asset_blob(&hash).is_some()) {
        // Stored once by hash: this copy is not needed.
        host.out().obj_release(blob);
    } else {
        session::put_asset(app, host, hash, blob, mime, size);
    }
    let rect = place_rect(natural, pos, n, pw, ph);
    let mut item = Id::NONE;
    let Some(e) = session::edit(app, host, |tx| item = tx.add_image(page, hash, rect)) else { return };
    if let Some(b) = app.st.editor_images.batches.front_mut() {
        b.inserted += 1;
    }
    app.st.editor_images.added += 1;
    app.st.editor_images.newest = (page, item);
    history::record(app, host, page, e.inverse, 0);
    selection::clear(app, host);
    selection::select(app, host, page, &[item]);
    input::set_tool(app, host, Some(input::Tool::Lasso), None, None);
    pages::touch(app, page);
    pages::request_repaint(app, host, page);
}

/// Drops the queue (the document closed): every handle held is released.
fn abandon(app: &mut App, host: &mut dyn Host) {
    let st = &mut app.st.editor_images;
    let step = std::mem::replace(&mut st.step, Step::Idle);
    st.req = 0;
    let batches = std::mem::take(&mut st.batches);
    let out = host.out();
    for b in &batches {
        b.files.iter().for_each(|f| out.obj_release(f.obj));
    }
    match step {
        Step::Idle => {}
        Step::Decode { img, file } => {
            host.out().obj_release(file.obj);
            pages::with_gfx(app, |_, g| g.cmds.image_drop(img));
        }
        Step::Encode { surf, .. } => pages::with_gfx(app, |_, g| g.cmds.drop_surf(surf)),
        Step::Digest { blob, .. } => host.out().obj_release(blob),
    }
    pages::flush(app, host);
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(app: &mut App, host: &mut dyn Host) {
    if app.cfg.has(crate::abi::mode::HEADLESS) {
        return;
    }
    let out = host.out();
    out.listen(Node::DOCUMENT, "paste", L_PASTE, 0);
    out.listen(Ui::CANVAS_VIEWPORT, "dragover", L_DRAGOVER, 0);
    out.listen(Ui::CANVAS_VIEWPORT, "drop", L_DROP, 0);
    // v5 `rememberPointerClientPoint`: pointer downs, and mouse/pen hover (a `mousemove` costs no coalesced list).
    out.listen(Node::DOCUMENT, "pointerdown", L_POINTER, listen::CAPTURE | listen::PASSIVE);
    out.listen(Node::DOCUMENT, "mousemove", L_MOUSE, listen::CAPTURE | listen::PASSIVE);
}

fn remember(app: &mut App, x: f64, y: f64) {
    if x.is_finite() && y.is_finite() {
        app.st.editor_images.last = (x, y);
    }
}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match event {
        Event::Pointer(p) if p.listener == L_POINTER && p.kind == ptr::DOWN => {
            remember(app, p.raw.x, p.raw.y);
            0
        }
        Event::Dom(e) if e.listener == L_MOUSE => {
            remember(app, e.x, e.y);
            0
        }
        Event::Dom(e) if e.listener == L_PASTE && e.ty == ety::PASTE => {
            // v5: read-only, or no image in the clipboard → the browser's paste.
            if view::read_only(app) || !e.has(ev::DT_FILES) {
                host.out().obj_release(e.obj);
                return 0;
            }
            viewport::note_interaction(app, host, e.t);
            let (x, y) = insert_point(app, host);
            take(app, host, e.obj, x, y);
            ret::PREVENT_DEFAULT
        }
        Event::Dom(e) if e.listener == L_DRAGOVER && e.ty == ety::DRAGOVER => {
            if view::read_only(app) || !e.has(ev::DT_FILES) {
                return 0;
            }
            viewport::note_interaction(app, host, e.t);
            ret::PREVENT_DEFAULT | ret::DROP_COPY
        }
        Event::Dom(e) if e.listener == L_DROP && e.ty == ety::DROP => {
            if view::read_only(app) {
                host.out().obj_release(e.obj);
                return 0;
            }
            viewport::note_interaction(app, host, e.t);
            take(app, host, e.obj, e.x, e.y);
            ret::PREVENT_DEFAULT
        }
        Event::Io(io) if io.req != 0 && io.req == app.st.editor_images.req => {
            step_done(app, host, io.status >= 0, io.obj, io.data);
            0
        }
        Event::Io(io) if app.st.editor_images.batches.iter().any(|b| b.take == io.req) => {
            files_taken(app, io.req, io.status, io.data);
            pump(app, host);
            0
        }
        Event::Io(io) => {
            // A step abandoned when the document closed: its result is not needed.
            host.out().obj_release(io.obj);
            0
        }
        Event::Msg(m) if m.chan == session::sig::CLOSED => {
            abandon(app, host);
            0
        }
        _ => 0,
    }
}

/// `__IHN_TEST_API__.cmd('editor.images.<name>', args)`:
/// * `state` → `{added, queued, busy, last: [x, y]}`;
/// * `list` → the live images of the document: `[{page (index), id, asset (hex), cx, cy, w, h, rotation}]`;
/// * `transform` `{sx, sy, rot, tx, ty}` → the commit of a selection-box transform of the newest placed image about
///   its centre (one batch, one undo step; the box itself is `editor::selection`'s) → `true` when written.
pub fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, args: &str) -> Option<String> {
    let st = &app.st.editor_images;
    match name {
        "transform" => {
            let (page, item) = st.newest;
            let a = Json::parse(args)?;
            let n = |k: &str, d: f64| a.get(k).and_then(Json::as_f64).unwrap_or(d);
            let centre = match &session::get(app)?.doc.live(item)?.body.v {
                ihn_core::op::Body::Image { cx, cy, .. } => (*cx, *cy),
                _ => return Some("false".into()),
            };
            let m = ihn_core::geom::Affine::new(centre, n("sx", 1.0), n("sy", 1.0), n("rot", 0.0), n("tx", 0.0), n("ty", 0.0));
            let Some(e) = session::edit(app, host, |tx| tx.transform(&[item], m)) else { return Some("false".into()) };
            history::record(app, host, page, e.inverse, 0);
            pages::touch(app, page);
            pages::request_repaint(app, host, page);
            Some("true".into())
        }
        "state" => Some(format!(
            "{{\"added\":{},\"queued\":{},\"busy\":{},\"last\":[{},{}]}}",
            st.added,
            st.batches.iter().map(|b| b.files.len() + (b.take != 0) as usize).sum::<usize>(),
            !matches!(st.step, Step::Idle),
            ihn_core::jsfmt::num(st.last.0),
            ihn_core::jsfmt::num(st.last.1)
        )),
        "list" => {
            let d = &session::get(app)?.doc;
            let mut out = Vec::new();
            for (i, &page) in d.order().iter().enumerate() {
                for r in d.draw_list(page).images {
                    if let Some(ihn_core::op::Body::Image { asset, cx, cy, w, h, rotation }) = d.live(r.id).map(|it| &it.body.v) {
                        let hex: String = asset.iter().map(|b| format!("{b:02x}")).collect();
                        let n = ihn_core::jsfmt::num;
                        out.push(format!(
                            "{{\"page\":{i},\"id\":\"{}\",\"asset\":\"{hex}\",\"cx\":{},\"cy\":{},\"w\":{},\"h\":{},\"rotation\":{}}}",
                            r.id.0,
                            n(*cx),
                            n(*cy),
                            n(*w),
                            n(*h),
                            n(*rotation)
                        ));
                    }
                }
            }
            Some(format!("[{}]", out.join(",")))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{kind, mode, Config};
    use crate::host::FakeHost;
    use crate::Rect;
    use ihn_core::codec::Writer;
    use ihn_core::op::{Background, Body, Template};

    struct T {
        app: App,
        host: FakeHost,
        pages: Vec<Id>,
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
            let a = tx.page_new(None, Template::Default, (794, 1123), Background::Template);
            tx.page_new(Some(a), Template::Default, (595, 842), Background::Template);
        })
        .expect("a memory session is writable");
        shell::show_view(&mut app, &mut host, shell::View::Editor);
        shell::signal(&mut app, &mut host, session::sig::OPENED, "d");
        host.advance(500.0);
        let t = host.now;
        app.frame(&mut host, t);
        view::set_read_only(&mut app, &mut host, false, false);
        let pages = session::get(&app).unwrap().doc.order().to_vec();
        // Page 1 at (100, 50) at zoom 1; page 2 below it at half size.
        let rows: Vec<Node> = app.st.editor_pages.rows.iter().map(|r| r.wrapper).collect();
        host.rects.insert(rows[0], Rect { x: 100.0, y: 50.0, w: 794.0, h: 1123.0 });
        host.rects.insert(rows[1], Rect { x: 100.0, y: 1200.0, w: 297.5, h: 421.0 });
        host.take();
        T { app, host, pages }
    }

    fn dom(t: &mut T, listener: u32, ty: u8, flags: u32, x: f64, y: f64, obj: u32) -> u32 {
        let mut w = Writer::default();
        w.u32(listener);
        w.u32(Node::DOCUMENT.0);
        w.u8(ty);
        w.u32(flags);
        w.f64(t.host.now);
        w.i32(0);
        w.u32(0);
        for v in [x, y, 0.0, 0.0] {
            w.f64(v);
        }
        w.u32(0);
        w.i32(0);
        w.u8(0);
        w.u32(obj);
        for _ in 0..3 {
            w.str("");
        }
        for _ in 0..4 {
            w.u32(0);
        }
        t.app.event(&mut t.host, kind::DOM, &w.buf)
    }

    fn io(t: &mut T, req: u32, status: i32, obj: u32, data: &[u8]) {
        let mut w = Writer::default();
        w.u32(req);
        w.i32(status);
        w.u32(obj);
        w.raw(data);
        t.app.event(&mut t.host, kind::IO, &w.buf);
    }

    fn take_req(t: &T) -> (u32, u32) {
        let b = t.app.st.editor_images.batches.back().expect("a batch");
        (b.take, b.base)
    }

    fn nat(w: f64, h: f64) -> Vec<u8> {
        let mut x = Writer::default();
        x.f64(w);
        x.f64(h);
        x.buf
    }

    /// Runs one file through decode → (encode) → digest.
    fn run_file(t: &mut T, natural: (f64, f64), encoded: Option<u32>, hash: u8) {
        let req = t.app.st.editor_images.req;
        assert!(matches!(t.app.st.editor_images.step, Step::Decode { .. }));
        io(t, req, 0, 0, &nat(natural.0, natural.1));
        if let Some(blob) = encoded {
            assert!(matches!(t.app.st.editor_images.step, Step::Encode { .. }));
            let req = t.app.st.editor_images.req;
            io(t, req, 0, blob, &nat(1234.0, 0.0));
        }
        assert!(matches!(t.app.st.editor_images.step, Step::Digest { .. }));
        let req = t.app.st.editor_images.req;
        io(t, req, 0, 0, &[hash; 32]);
    }

    fn images(t: &T) -> Vec<(Id, Hash, ImageRect)> {
        let d = &session::get(&t.app).unwrap().doc;
        t.pages.iter().flat_map(|&p| d.draw_list(p).images.iter().map(move |r| r.id)).filter_map(|id| match &d.live(id)?.body.v {
            Body::Image { asset, cx, cy, w, h, rotation } => Some((d.live(id)?.page.v, *asset, ImageRect { cx: *cx, cy: *cy, w: *w, h: *h, rotation: *rotation })),
            _ => None,
        }).collect()
    }

    #[test]
    fn scale_and_placement_rules() {
        assert_eq!(compress_scale(800.0, 600.0), 1.0);
        assert_eq!(compress_scale(6400.0, 100.0), 0.5);
        assert_eq!(compress_scale(4000.0, 3000.0), js_min(0.8, (MAX_PIXELS / 12e6).sqrt()));
        assert!(keep_original("image/png", 4.0 * 1024.0 * 1024.0, 1.0));
        assert!(!keep_original("image/png", 4.0 * 1024.0 * 1024.0 + 1.0, 1.0));
        assert!(!keep_original("image/jpeg", 10.0, 1.0));
        assert!(!keep_original("image/png", 10.0, 0.5));
        // A4: max 476.4 × 505.35; centred at the point, the n-th image 18·n further, clamped inside the page.
        let r = place_rect((1000.0, 500.0), (300.0, 300.0), 0, 794.0, 1123.0);
        assert_eq!((r.w, r.h, r.cx, r.cy, r.rotation), (794.0 * 0.6, 794.0 * 0.6 / 2.0, 300.0, 300.0, 0.0));
        let r = place_rect((100.0, 50.0), (10.0, 1120.0), 2, 794.0, 1123.0);
        assert_eq!((r.w, r.h, r.cx, r.cy), (100.0, 50.0, 50.0, 1123.0 - 25.0));
        // The page's real size (B-E1): an A5-sized page clamps at its own edge.
        let r = place_rect((100.0, 100.0), (590.0, 10.0), 0, 595.0, 842.0);
        assert_eq!((r.cx, r.cy), (545.0, 50.0));
    }

    #[test]
    fn paste_places_one_image_per_file_and_stores_each_asset_once() {
        let mut t = boot();
        // The last pointer position aims the paste: page 1 at (300, 250) page px.
        let mut p = Writer::default();
        p.u32(L_POINTER);
        p.u32(Node::DOCUMENT.0);
        p.u8(ptr::DOWN);
        p.u8(ptr::MOUSE);
        p.i32(1);
        p.i32(0);
        p.u32(0);
        p.f64(400.0);
        p.f64(300.0);
        p.f64(0.0);
        p.f32(0.0);
        p.f32(0.0);
        p.f32(0.0);
        p.u32(1);
        p.u32(0);
        p.u32(0);
        t.app.event(&mut t.host, kind::POINTER, &p.buf);
        assert_eq!(t.app.st.editor_images.last, (400.0, 300.0));

        // A text paste is the browser's.
        assert_eq!(dom(&mut t, L_PASTE, ety::PASTE, 0, 0.0, 0.0, 7), 0);
        assert!(t.app.st.editor_images.batches.is_empty());

        assert_eq!(dom(&mut t, L_PASTE, ety::PASTE, ev::DT_FILES, 0.0, 0.0, 7), ret::PREVENT_DEFAULT);
        t.host.take();
        let (req, base) = take_req(&t);
        let list = r#"[{"name":"a.png","type":"image/png","size":2000,"lastModified":0},{"name":"b.jpg","type":"image/jpeg","size":90000,"lastModified":0},{"name":"c.png","type":"image/png","size":2000,"lastModified":0}]"#;
        io(&mut t, req, 3, 0, list.as_bytes());
        // a.png: kept byte-exact (no encode), the file itself is the asset.
        run_file(&mut t, (1000.0, 500.0), None, 1);
        // b.jpg: one JPEG encode.
        run_file(&mut t, (400.0, 300.0), Some(base + 1000), 2);
        // c.png: the same bytes as a.png → the same hash, stored once.
        run_file(&mut t, (1000.0, 500.0), None, 1);
        assert!(matches!(t.app.st.editor_images.step, Step::Idle) && t.app.st.editor_images.batches.is_empty());

        let got = images(&t);
        assert_eq!(got.len(), 3);
        assert!(got.iter().all(|g| g.0 == t.pages[0]));
        assert_eq!((got[0].1, got[1].1, got[2].1), ([1; 32], [2; 32], [1; 32]));
        assert_eq!((got[0].2.cx, got[0].2.cy, got[0].2.w), (300.0, 250.0, 794.0 * 0.6));
        assert_eq!((got[1].2.cx, got[1].2.cy, got[1].2.w, got[1].2.h), (318.0, 268.0, 400.0, 300.0));
        assert_eq!((got[2].2.cx, got[2].2.cy), (336.0, 286.0));
        let s = session::get(&t.app).unwrap();
        assert_eq!(s.referenced_assets().len(), 2);
        assert_eq!(s.asset_blob(&[1; 32]), Some(base));
        assert_eq!(s.asset_blob(&[2; 32]), Some(base + 1000));
        assert_eq!(history::depth(&t.app), (3, 3));
        assert_eq!(t.app.st.editor_input.tool, input::Tool::Lasso);
        assert_eq!(t.app.st.editor_images.added, 3);
        let out = t.host.take();
        assert!(out.contains("3 images added"), "{out}");
        // The selection-box commit of a transform; undo restores, then removes the newest image.
        let r = test_cmd(&mut t.app, &mut t.host, "transform", r#"{"sx":0.5,"sy":0.5,"rot":1,"tx":-100,"ty":0}"#);
        assert_eq!(r.as_deref(), Some("true"));
        let last = images(&t)[2].2;
        assert_eq!((last.cx, last.cy, last.w, last.h, last.rotation), (236.0, 286.0, 794.0 * 0.3, 794.0 * 0.15, 1.0));
        history::undo(&mut t.app, &mut t.host);
        assert_eq!(images(&t)[2].2.w, 794.0 * 0.6);
        history::undo(&mut t.app, &mut t.host);
        assert_eq!(images(&t).len(), 2);
    }

    #[test]
    fn drop_targets_the_nearest_page_and_skips_undecodable_files() {
        let mut t = boot();
        // Read mode: neither dragover nor drop is taken.
        view::set_read_only(&mut t.app, &mut t.host, true, false);
        assert_eq!(dom(&mut t, L_DRAGOVER, ety::DRAGOVER, ev::DT_FILES, 0.0, 0.0, 0), 0);
        assert_eq!(dom(&mut t, L_DROP, ety::DROP, ev::DT_FILES, 200.0, 1300.0, 9), 0);
        view::set_read_only(&mut t.app, &mut t.host, false, false);
        assert_eq!(dom(&mut t, L_DRAGOVER, ety::DRAGOVER, 0, 0.0, 0.0, 0), 0);
        assert_eq!(dom(&mut t, L_DRAGOVER, ety::DRAGOVER, ev::DT_FILES, 0.0, 0.0, 0), ret::PREVENT_DEFAULT | ret::DROP_COPY);
        // Next to page 2 (shown at half size): the nearest wrapper.
        assert_eq!(dom(&mut t, L_DROP, ety::DROP, ev::DT_FILES, 160.0, 1300.0, 9), ret::PREVENT_DEFAULT);
        let (req, _) = take_req(&t);
        io(&mut t, req, 2, 0, br#"[{"type":"image/gif","size":5},{"type":"image/webp","size":5}]"#);
        let r = t.app.st.editor_images.req;
        io(&mut t, r, 1, 0, &[]);
        // The second file decodes; WebP is re-encoded as JPEG.
        run_file(&mut t, (100.0, 80.0), Some(4242), 9);
        let got = images(&t);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, t.pages[1]);
        // (160 − 100) · 595 / 297.5 = 120; (1300 − 1200) · 842 / 421 = 200.
        assert_eq!((got[0].2.cx, got[0].2.cy, got[0].2.w, got[0].2.h), (120.0, 200.0, 100.0, 80.0));
        assert!(t.host.take().contains("Image added"));
    }

    #[test]
    fn closing_the_document_abandons_the_queue() {
        let mut t = boot();
        dom(&mut t, L_DROP, ety::DROP, ev::DT_FILES, 300.0, 300.0, 9);
        let (req, _) = take_req(&t);
        io(&mut t, req, 2, 0, br#"[{"type":"image/png","size":5},{"type":"image/png","size":5}]"#);
        assert!(matches!(t.app.st.editor_images.step, Step::Decode { .. }));
        let r = t.app.st.editor_images.req;
        shell::signal(&mut t.app, &mut t.host, session::sig::CLOSED, "");
        assert!(matches!(t.app.st.editor_images.step, Step::Idle) && t.app.st.editor_images.batches.is_empty());
        io(&mut t, r, 0, 0, &nat(10.0, 10.0));
        assert!(images(&t).is_empty());
    }
}
