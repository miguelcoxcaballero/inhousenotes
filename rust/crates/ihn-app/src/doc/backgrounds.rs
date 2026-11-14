//! pdf.js page rasters: scale policy, LRU, interaction deferral (ARCHITECTURE §6.3). Owned by W3.3.
//!
//! This module decides *which* PDF page is rendered and at *what* scale; `host/pdf.js` opens the source asset and
//! renders (white, JPEG 0.92, blob URL); `editor::pages` shows the result (`pages::set_pdf_raster`). 5.18.0 rules:
//! only active pages are rendered, one page per animation frame, nothing while the viewport is interacting or within
//! 240 ms of user input (a result that lands then is discarded and the page queued again), and at most 6 rasters are
//! kept: the least recently used non-active ones are dropped first.
//!
//! A source asset that this device does not hold is asked for through the export worker (W5.4: rebuild from the
//! latest export, ARCHITECTURE §4.5); until then [`take_missing`] lists it and [`asset_arrived`] resumes its pages.
// v5: app-v5.js getPdfBackgroundScale / applyPageBackground / queuePdfPageRender / processPdfRenderQueue /
// schedulePdfRenderResume / renderPdfPageToBackground / touchPdfBackgroundCache / prunePdfBackgroundCache

use super::open::pdf;
use super::session;
use crate::abi::{Event, Io};
use crate::app::Mod;
use crate::editor::{pages, viewport};
use crate::shell::fx;
use crate::{tag, App, Host};
use ihn_core::codec::Reader;
use ihn_core::id::{Hash, Id};
use ihn_core::op::Background;
use std::collections::VecDeque;

// v5: app-v5.js constants
const MAX_PDF_BACKGROUND_CACHE: usize = 6;
const PDF_RENDER_SCALE_LARGE: f64 = 1.5;
const PDF_RENDER_SCALE_HUGE: f64 = 1.2;
const CANVAS_SCALE: f64 = 2.0;
const RECENT_ACTIVITY_MS: f64 = 240.0;
const MB: f64 = 1024.0 * 1024.0;

/// `requestIdleCallback(resume, {timeout: 600})` of `schedulePdfRenderResume`.
const I_RESUME: u32 = tag(Mod::DocBackgrounds, 1);

/// v5: `getPdfBackgroundScale(pageCount, fileSize)`.
pub fn scale_for(page_count: usize, file_size: f64) -> f64 {
    if page_count >= 500 {
        return PDF_RENDER_SCALE_HUGE;
    }
    if page_count >= 200 {
        return PDF_RENDER_SCALE_LARGE;
    }
    if file_size > 80.0 * MB {
        return 0.9;
    }
    if file_size > 30.0 * MB {
        return 1.25;
    }
    if file_size > 10.0 * MB || page_count >= 30 {
        return 1.6;
    }
    (CANVAS_SCALE * 1.5).min(3.0)
}

/// An opened source PDF (`activePdfDocument`); `doc == 0` while pdf.js opens it.
struct Source {
    hash: Hash,
    req: u32,
    doc: u32,
    scale: f64,
    failed: bool,
}

/// One raster of `pdfBackgroundCache`: the page, what it shows, its blob URL and handle, last use.
struct Entry {
    page: Id,
    src: (Hash, u32),
    url: String,
    blob: u32,
    used: f64,
}

struct Render {
    req: u32,
    page: Id,
    src: (Hash, u32),
}

#[derive(Default)]
pub struct State {
    /// The document the state belongs to.
    doc: String,
    sources: Vec<Source>,
    cache: Vec<Entry>,
    queue: VecDeque<Id>,
    render: Option<Render>,
    /// Waiting for the frame that follows a render (one page per frame).
    frame_wait: bool,
    resume_armed: bool,
    /// Requests of a previous document still in flight: (req, is a render).
    orphans: Vec<(u32, bool)>,
    /// Source assets this device lacks (rebuild requested, W5.4).
    missing: Vec<Hash>,
    /// Rasters handed to the editor (test counter).
    renders: u32,
    /// `reset` is handing the rasters back: `apply` must not queue the pages again.
    resetting: bool,
}

fn st(app: &mut App) -> &mut State { &mut app.st.doc_backgrounds }

/// The PDF page a page shows: (source asset, 1-based page).
fn pdf_of(app: &App, page: Id) -> Option<(Hash, u32)> {
    match session::get(app)?.doc.pages.get(&page)?.background.v {
        Background::Pdf { asset, page } => Some((asset, page)),
        _ => None,
    }
}

/// v5: the PDF branch of `applyPageBackground(i, canvas)`, called by `editor::pages` before it reads the raster:
/// a cached raster is touched (and handed back if the editor dropped it), otherwise the page is queued.
pub fn apply(app: &mut App, host: &mut dyn Host, page: Id) {
    if st(app).resetting {
        return;
    }
    let Some(src) = pdf_of(app, page) else { return };
    let now = host.now();
    if let Some(at) = st(app).cache.iter().position(|e| e.page == page) {
        if st(app).cache[at].src == src {
            st(app).cache[at].used = now;
            if !pages::has_pdf_raster(app, page) {
                let url = st(app).cache[at].url.clone();
                pages::set_pdf_raster(app, host, page, Some(&url));
            }
            prune(app, host);
            return;
        }
        // The page shows another PDF page now (a background edit): the old raster goes.
        drop_entry(app, host, at);
    }
    queue(app, host, page);
}

/// v5: `queuePdfPageRender(i)`.
fn queue(app: &mut App, host: &mut dyn Host, page: Id) {
    if let Some((hash, _)) = pdf_of(app, page) {
        source(app, host, hash);
    }
    if !st(app).queue.contains(&page) {
        st(app).queue.push_back(page);
    }
    process(app, host);
}

/// The opened document of a source asset (`None` while it opens, failed or missing); starts the open.
fn source(app: &mut App, host: &mut dyn Host, hash: Hash) -> Option<u32> {
    if let Some(s) = st(app).sources.iter().find(|s| s.hash == hash) {
        return (s.doc != 0).then_some(s.doc);
    }
    let Some(blob) = session::get(app).and_then(|s| s.asset_blob(&hash)) else {
        if !st(app).missing.contains(&hash) {
            st(app).missing.push(hash);
            host.log(1, "PDF background source missing; rebuild from the latest export requested");
        }
        return None;
    };
    // v5: pdfBackgroundScale = getPdfBackgroundScale(pdf.numPages, blob.size) — the Drive file's pages and size.
    let (pages, size) = session::get(app).map_or((0, 0.0), |s| (s.doc.order().len(), s.export_size));
    let req = app.req(Mod::DocBackgrounds);
    pdf::open(host.out(), req, blob);
    st(app).sources.push(Source { hash, req, doc: 0, scale: scale_for(pages, size), failed: false });
    None
}

/// v5: `processPdfRenderQueue()` + the checks at the start of `renderPdfPageToBackground`.
fn process(app: &mut App, host: &mut dyn Host) {
    if st(app).render.is_some() || st(app).frame_wait {
        return;
    }
    if viewport::interacting(app, host.now()) {
        schedule_resume(app, host);
        return;
    }
    while let Some(page) = st(app).queue.pop_front() {
        if pages::active(app, page).is_none() || st(app).cache.iter().any(|e| e.page == page) {
            continue;
        }
        let Some(src) = pdf_of(app, page) else { continue };
        let Some(s) = st(app).sources.iter().find(|s| s.hash == src.0) else { continue };
        if s.failed {
            continue;
        }
        if s.doc == 0 {
            // pdf.js is still opening the source: the open's answer resumes the queue.
            st(app).queue.push_front(page);
            return;
        }
        let (doc, scale) = (s.doc, s.scale);
        let req = app.req(Mod::DocBackgrounds);
        pdf::render(host.out(), req, doc, src.1, scale.max(0.6));
        st(app).render = Some(Render { req, page, src });
        return;
    }
}

/// v5: `schedulePdfRenderResume()`.
fn schedule_resume(app: &mut App, host: &mut dyn Host) {
    if !std::mem::replace(&mut st(app).resume_armed, true) {
        app.sched.idle(host.out(), I_RESUME, 600.0);
    }
}

fn release(host: &mut dyn Host, url: &str, blob: u32) {
    fx::url_revoke(host.out(), url);
    host.out().obj_release(blob);
}

fn drop_entry(app: &mut App, host: &mut dyn Host, at: usize) {
    let e = st(app).cache.remove(at);
    release(host, &e.url, e.blob);
    pages::set_pdf_raster(app, host, e.page, None);
}

/// v5: `prunePdfBackgroundCache()`: least recently used non-active rasters go while more than 6 are kept.
fn prune(app: &mut App, host: &mut dyn Host) {
    while st(app).cache.len() > MAX_PDF_BACKGROUND_CACHE {
        let victim = (0..st(app).cache.len())
            .filter(|i| pages::active(app, app.st.doc_backgrounds.cache[*i].page).is_none())
            .min_by(|a, b| app.st.doc_backgrounds.cache[*a].used.total_cmp(&app.st.doc_backgrounds.cache[*b].used));
        let Some(at) = victim else { return };
        drop_entry(app, host, at);
    }
}

/// The answer of `pdf_render`: kept and shown, or (input in the last 240 ms, page gone) discarded and requeued.
fn rendered(app: &mut App, host: &mut dyn Host, r: Render, io: &Io) {
    let mut rd = Reader::new(io.data);
    let url = (io.status >= 0).then(|| rd.str().map(str::to_string)).flatten();
    let Some(url) = url else {
        host.log(2, &format!("Failed to render PDF page background: {}", String::from_utf8_lossy(io.data)));
        host.out().obj_release(io.obj);
        process(app, host);
        return;
    };
    let now = host.now();
    let still = pdf_of(app, r.page) == Some(r.src);
    if !still || app.sched.quiet_for(now) < RECENT_ACTIVITY_MS {
        release(host, &url, io.obj);
        // v5: requeuePdfPageRenderAfterInteraction — only for an active page that still needs its raster.
        if still && pages::active(app, r.page).is_some() && !st(app).queue.contains(&r.page) {
            st(app).queue.push_front(r.page);
        }
        schedule_resume(app, host);
        return;
    }
    st(app).cache.push(Entry { page: r.page, src: r.src, url: url.clone(), blob: io.obj, used: now });
    st(app).renders += 1;
    pages::set_pdf_raster(app, host, r.page, Some(&url));
    prune(app, host);
    // v5: `await new Promise(requestAnimationFrame)` between two pages.
    st(app).frame_wait = true;
    app.sched.frame(host.out(), Mod::DocBackgrounds);
}

/// Drops every raster and source (a document closed or another one opened).
fn reset(app: &mut App, host: &mut dyn Host) {
    let s = std::mem::take(st(app));
    st(app).resetting = true;
    for e in s.cache {
        release(host, &e.url, e.blob);
        pages::set_pdf_raster(app, host, e.page, None);
    }
    let mut orphans = s.orphans;
    for src in s.sources {
        if src.doc != 0 {
            pdf::close(host.out(), src.doc);
        } else if !src.failed {
            orphans.push((src.req, false));
        }
    }
    if let Some(r) = s.render {
        orphans.push((r.req, true));
    }
    if s.resume_armed {
        app.sched.idle_cancel(host.out(), I_RESUME);
    }
    let st = st(app);
    st.resetting = false;
    st.orphans = orphans;
    st.renders = s.renders;
}

/// Source assets this device lacks; the export worker rebuilds them from the latest export (W5.4).
pub fn take_missing(app: &mut App) -> Vec<Hash> { std::mem::take(&mut st(app).missing) }

/// A source asset became available (`session::put_asset` by sync or a rebuild): its active pages are queued.
pub fn asset_arrived(app: &mut App, host: &mut dyn Host, hash: Hash) {
    st(app).missing.retain(|h| *h != hash);
    let ids: Vec<Id> = pages::order(app).into_iter().filter(|id| pages::active(app, *id).is_some() && pdf_of(app, *id).is_some_and(|s| s.0 == hash)).collect();
    for id in ids {
        apply(app, host, id);
    }
}

pub fn init(_app: &mut App, _host: &mut dyn Host) {}

pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Io(ref io) => {
            if let Some(at) = st(app).orphans.iter().position(|o| o.0 == io.req) {
                let (_, is_render) = st(app).orphans.remove(at);
                if is_render {
                    if let Some(url) = Reader::new(io.data).str().filter(|_| io.status >= 0) {
                        fx::url_revoke(host.out(), url);
                    }
                    host.out().obj_release(io.obj);
                } else if io.status >= 0 && io.obj != 0 {
                    pdf::close(host.out(), io.obj);
                }
                return 0;
            }
            if st(app).render.as_ref().is_some_and(|r| r.req == io.req) {
                let r = st(app).render.take().expect("checked");
                rendered(app, host, r, io);
            } else if let Some(s) = st(app).sources.iter_mut().find(|s| s.req == io.req && s.doc == 0) {
                if io.status >= 0 && io.obj != 0 {
                    s.doc = io.obj;
                } else {
                    s.failed = true;
                    host.log(2, &format!("PDF background source could not be opened: {}", String::from_utf8_lossy(io.data)));
                }
                process(app, host);
            }
            0
        }
        Event::Frame(_) => {
            if std::mem::take(&mut st(app).frame_wait) {
                process(app, host);
            }
            0
        }
        Event::Idle { id: I_RESUME, .. } => {
            st(app).resume_armed = false;
            if !viewport::interacting(app, host.now()) {
                process(app, host);
            } else if !st(app).queue.is_empty() {
                schedule_resume(app, host);
            }
            0
        }
        Event::Msg(m) if m.chan == session::sig::CLOSED => {
            reset(app, host);
            0
        }
        Event::Msg(m) if m.chan == session::sig::OPENED => {
            let id = session::doc_id(app).to_string();
            if !m.data.is_empty() && m.data == id.as_bytes() && app.st.doc_backgrounds.doc != id {
                reset(app, host);
                st(app).doc = id;
            }
            0
        }
        _ => 0,
    }
}

pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    let s = &app.st.doc_backgrounds;
    let ids = |v: &mut dyn Iterator<Item = Id>| v.map(|id| format!("\"{}\"", id.hex())).collect::<Vec<_>>().join(",");
    match name {
        "state" => Some(format!(
            "{{\"cache\":[{}],\"queue\":[{}],\"rendering\":{},\"sources\":[{}],\"missing\":{},\"renders\":{}}}",
            ids(&mut s.cache.iter().map(|e| e.page)),
            ids(&mut s.queue.iter().copied()),
            s.render.as_ref().map_or("null".into(), |r| format!("\"{}\"", r.page.hex())),
            s.sources.iter().map(|x| format!("{{\"open\":{},\"failed\":{},\"scale\":{}}}", x.doc != 0, x.failed, ihn_core::jsfmt::num(x.scale))).collect::<Vec<_>>().join(","),
            s.missing.len(),
            s.renders
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{kind, mode, Config};
    use crate::host::FakeHost;
    use crate::shell;
    use crate::{Rect, Ui};
    use ihn_core::codec::Writer;
    use ihn_core::op::Template;

    const SRC: Hash = [7; 32];

    fn boot(n: u32) -> (App, FakeHost) {
        let cfg = Config { version: "6.0.0".into(), url: "http://127.0.0.1/?e2e=1".into(), user_agent: "test".into(), language: "en".into(), flags: mode::E2E | mode::ONLINE, viewport: (1280.0, 800.0), dpr: 1.0, max_touch_points: 0 };
        let (mut app, mut host) = (App::new(cfg), FakeHost::default());
        app.init(&mut host);
        host.rects.insert(Ui::CANVAS_VIEWPORT, Rect { x: 0.0, y: 69.0, w: 1280.0, h: 731.0 });
        session::load(&mut app, &mut host, "", "d", 0);
        timer(&mut app, &mut host, tag(Mod::DocSession, 1));
        session::put_asset(&mut app, &mut host, SRC, 900, "application/pdf", 1000.0);
        let _ = session::edit(&mut app, &mut host, |tx| {
            let mut after = None;
            for i in 0..n {
                after = Some(tx.page_new(after, Template::Default, (595, 200), Background::Pdf { asset: SRC, page: i + 1 }));
            }
        });
        shell::show_view(&mut app, &mut host, shell::View::Editor);
        shell::signal(&mut app, &mut host, session::sig::OPENED, "d");
        host.advance(500.0);
        frame(&mut app, &mut host);
        host.take();
        (app, host)
    }

    fn timer(app: &mut App, host: &mut FakeHost, id: u32) {
        let mut w = Writer::default();
        w.u32(id);
        app.event(host, kind::TIMER, &w.buf);
    }
    fn frame(app: &mut App, host: &mut FakeHost) {
        let t = host.now;
        app.frame(host, t);
    }
    fn io(app: &mut App, host: &mut FakeHost, req: u32, status: i32, obj: u32, data: &[u8]) {
        let mut w = Writer::default();
        w.u32(req);
        w.i32(status);
        w.u32(obj);
        w.raw(data);
        app.event(host, kind::IO, &w.buf);
    }
    fn raster(url: &str) -> Vec<u8> {
        let mut w = Writer::default();
        w.str(url);
        w.f64(1785.0);
        w.f64(2526.0);
        w.buf
    }
    fn render_req(app: &App) -> Option<(u32, Id)> { app.st.doc_backgrounds.render.as_ref().map(|r| (r.req, r.page)) }

    #[test]
    fn scale_policy_is_5_18_0s() {
        assert_eq!(scale_for(2, 1e5), 3.0);
        assert_eq!(scale_for(30, 1e5), 1.6);
        assert_eq!(scale_for(10, 11.0 * MB), 1.6);
        assert_eq!(scale_for(10, 31.0 * MB), 1.25);
        assert_eq!(scale_for(10, 81.0 * MB), 0.9);
        assert_eq!(scale_for(200, 0.0), 1.5);
        assert_eq!(scale_for(500, 90.0 * MB), 1.2);
    }

    #[test]
    fn active_pages_render_one_per_frame_and_input_defers() {
        let (mut app, mut host) = boot(3);
        // The active pages asked for their source: one open, the pages queued behind it.
        assert_eq!(app.st.doc_backgrounds.sources.len(), 1);
        assert!(render_req(&app).is_none());
        let open_req = app.st.doc_backgrounds.sources[0].req;
        io(&mut app, &mut host, open_req, 0, 41, &[]);
        let (req, first) = render_req(&app).expect("the first active page renders");
        assert_eq!(app.st.doc_backgrounds.sources[0].scale, 3.0);
        assert!(pages::active(&app, first).is_some());
        io(&mut app, &mut host, req, 0, 50, &raster("blob:a"));
        assert!(pages::has_pdf_raster(&app, first));
        // One page per frame: the next render waits for the frame.
        assert!(render_req(&app).is_none() && app.st.doc_backgrounds.frame_wait);
        frame(&mut app, &mut host);
        let (req, second) = render_req(&app).expect("the second page renders after the frame");
        assert_ne!(second, first);
        // Input within 240 ms of the answer: the raster is discarded and the page waits for an idle resume.
        app.sched.activity(host.now);
        host.take();
        io(&mut app, &mut host, req, 0, 51, &raster("blob:b"));
        let out = host.take();
        assert!(out.contains("obj_release"), "{out}");
        assert!(!pages::has_pdf_raster(&app, second));
        assert_eq!(app.st.doc_backgrounds.queue.front(), Some(&second));
        assert!(app.st.doc_backgrounds.resume_armed);
        host.advance(300.0);
        let mut w = Writer::default();
        w.u32(I_RESUME);
        w.f64(10.0);
        w.u8(0);
        app.event(&mut host, kind::IDLE, &w.buf);
        assert_eq!(render_req(&app).map(|r| r.1), Some(second));
    }

    #[test]
    fn lru_keeps_six_and_spares_active_pages() {
        let (mut app, mut host) = boot(2);
        let now = host.now;
        let active = pages::order(&app)[0];
        for i in 0..8u32 {
            let page = if i == 0 { active } else { Id(1000 + i as u128) };
            app.st.doc_backgrounds.cache.push(Entry { page, src: (SRC, 1), url: format!("blob:{i}"), blob: 60 + i, used: now + i as f64 });
        }
        prune(&mut app, &mut host);
        let kept: Vec<u32> = app.st.doc_backgrounds.cache.iter().map(|e| e.blob).collect();
        // The active page is the oldest but stays; the two oldest non-active rasters go.
        assert_eq!(kept, vec![60, 63, 64, 65, 66, 67]);
    }

    #[test]
    fn closing_releases_rasters_and_sources() {
        let (mut app, mut host) = boot(1);
        let open_req = app.st.doc_backgrounds.sources[0].req;
        io(&mut app, &mut host, open_req, 0, 41, &[]);
        let (req, page) = render_req(&app).unwrap();
        io(&mut app, &mut host, req, 0, 50, &raster("blob:a"));
        host.take();
        shell::signal(&mut app, &mut host, session::sig::CLOSED, "d");
        let out = host.take();
        assert!(out.contains("obj_release"), "{out}");
        assert!(app.st.doc_backgrounds.cache.is_empty() && app.st.doc_backgrounds.sources.is_empty());
        assert!(!pages::has_pdf_raster(&app, page));
    }

    #[test]
    fn missing_source_is_listed_for_a_rebuild() {
        let (mut app, mut host) = boot(0);
        let page = {
            let mut id = Id::NONE;
            session::edit(&mut app, &mut host, |tx| id = tx.page_new(None, Template::Default, (595, 842), Background::Pdf { asset: [9; 32], page: 1 })).expect("writable");
            id
        };
        queue(&mut app, &mut host, page);
        assert_eq!(take_missing(&mut app), vec![[9; 32]]);
    }
}
