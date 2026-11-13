//! Trace goldens (TESTING §1, §3.4): each `canvas` scene recorded from 5.18.0 by W1.2 is produced again by the
//! Rust renderer and compared call by call, argument bits included. The scene functions play the part of the
//! editor (which canvases exist, when a frame happens); every drawing call comes from `render`.

use super::cmd::{Cmds, Img, Src, Surf};
use super::live::{self, Cursor, Live, PageTarget};
use super::page::{self, Bg, PageView, Staged};
use super::template::{BgUrls, Templates};
use super::thumb::{self, ScrollPreview, ThumbBg};
use super::trace::{strokes_of, Golden, Host, Stroke};
use super::{damage, overlay, ImageRef, PageItems, PathCache, StrokeRef};
use crate::codec::b64_encode;
use crate::geom::{self, Sweep};
use crate::id::Id;
use crate::op::{Color, Template, Tool};
use crate::stamp::Stamp;

const A4: (f64, f64) = (794.0, 1123.0);
/// Backing sizes of an A4 page's canvases at zoom 1 on a desktop (v5 `attachPageCanvases`).
const PAGE_PX: (u32, u32) = (1588, 2246);
const LASSO_PX: (u32, u32) = (1072, 1516);
const CURSOR_PX: (u32, u32) = (993, 1404);
const CURSOR_SCALE: f64 = 993.0 / 794.0;

struct Scene {
    c: Cmds,
    h: Host,
    tpl: Templates,
    paths: PathCache,
    urls: BgUrls,
    nodes: u32,
    reqs: u32,
}

struct PageCanvases {
    page: Surf,
    lasso: Surf,
    cursor: Surf,
    remote: Surf,
    nodes: [u32; 4],
}

impl Scene {
    fn new() -> Scene { Scene { c: Cmds::new(), h: Host::default(), tpl: Templates::new(901, 902), paths: PathCache::default(), urls: BgUrls::default(), nodes: 2000, reqs: 0 } }
    fn req(&mut self) -> u32 {
        self.reqs += 1;
        self.reqs
    }
    fn dom(&mut self, label: String, px: (u32, u32)) -> (Surf, u32) {
        self.nodes += 1;
        self.h.dom.insert(self.nodes, (label, px.0, px.1));
        (self.c.bind(self.nodes), self.nodes)
    }
    /// The four canvases of an active page, already in the document with their sizes.
    fn page(&mut self, index: usize) -> PageCanvases {
        let (page, a) = self.dom(format!("canvas.page-canvas[page={index}]"), PAGE_PX);
        let (lasso, b) = self.dom(format!("canvas.lasso-overlay[page={index}]"), LASSO_PX);
        let (cursor, d) = self.dom(format!("canvas.cursor-overlay[page={index}]"), CURSOR_PX);
        let (remote, e) = self.dom(format!("canvas.remote-live-overlay[page={index}]"), CURSOR_PX);
        PageCanvases { page, lasso, cursor, remote, nodes: [a, b, d, e] }
    }
    /// v5 `attachPageCanvases`: the sizes are set in creation order.
    fn attach(&mut self, index: usize) -> PageCanvases {
        let p = self.page(index);
        self.c.size(p.page, PAGE_PX.0, PAGE_PX.1);
        self.c.size(p.lasso, LASSO_PX.0, LASSO_PX.1);
        self.c.size(p.cursor, CURSOR_PX.0, CURSOR_PX.1);
        self.c.size(p.remote, CURSOR_PX.0, CURSOR_PX.1);
        p
    }
    /// v5 `applyPageDimensionsToWrapper` when no size changes: context defaults + the remote overlay redraw.
    fn apply(&mut self, p: &PageCanvases) {
        page::context_defaults(&mut self.c, p.page, 2.0);
        assert!(!overlay::remote_redraw(&mut self.c, &mut self.paths, p.remote, CURSOR_PX, CURSOR_SCALE, &[]));
    }
    fn staged(&mut self, canvas: Surf, view: &PageView) {
        let mut job = Staged::begin(&mut self.c, &mut self.tpl, PAGE_PX, view, 2.0);
        let mut slices = 0;
        while !job.paint(&mut self.c, &mut self.paths, view, 97) {
            slices += 1;
            self.h.run(&mut self.c); // a slice per host flush, as the editor does
        }
        assert_eq!(slices, view.items.strokes.len() / 97);
        job.blit(&mut self.c, canvas);
    }
    fn scroll_preview(&mut self, view: &PageView) {
        let mut job = ScrollPreview::begin(&mut self.c, view.w, view.h);
        assert_eq!((job.px, job.scale), ((199, 281), 0.25));
        job.background(&mut self.c, &mut self.tpl, &mut self.paths, view);
        while !job.strokes(&mut self.c, &mut self.paths, view, 50) {}
        let req = self.req();
        job.finish(&mut self.c, req);
    }
    fn check(mut self, scene: &str) {
        self.h.run(&mut self.c);
        Golden::load(scene).assert_eq(&self.h);
    }
}

/// The draw runs of page number `page` of a fixture (the number keeps the `Path2D` cache keys of pages apart).
fn items(strokes: &[Stroke], page: u32) -> PageItems<'_> {
    let key = |s: &Stroke| Some((Id((page as u128) << 32 | s.id as u128), Stamp::ZERO));
    PageItems { covers: Vec::new(), images: Vec::new(), strokes: strokes.iter().map(|s| StrokeRef { key: key(s), ..StrokeRef::new(s.tool, &s.color, s.width, &s.xy) }).collect() }
}
fn a4<'a>(items: &'a PageItems<'a>) -> PageView<'a> { PageView { w: A4.0, h: A4.1, bg: Bg::Template(Template::Default), items } }

/// `trace-init.js` `hash` (cyrb53) of an image `src`.
fn cyrb53(text: &str) -> String {
    let (mut h1, mut h2) = (0xdead_beef_u32, 0x41c6_ce57_u32);
    for unit in text.encode_utf16() {
        h1 = (h1 ^ unit as u32).wrapping_mul(2_654_435_761);
        h2 = (h2 ^ unit as u32).wrapping_mul(1_597_334_677);
    }
    h1 = (h1 ^ h1 >> 16).wrapping_mul(2_246_822_507) ^ (h2 ^ h2 >> 13).wrapping_mul(3_266_489_909);
    h2 = (h2 ^ h2 >> 16).wrapping_mul(2_246_822_507) ^ (h1 ^ h1 >> 13).wrapping_mul(3_266_489_909);
    format!("{:014x}", ((h2 & 0x1f_ffff) as u64) << 32 | h1 as u64)
}

/// The pen samples of an ink scene in page coordinates. Chromium delivers client coordinates as f32; the page's
/// top-left corner is at an integer client position (`top`), so only y loses bits.
fn samples(input: &str, top: f64) -> Vec<f64> {
    let field = |row: &str, key: &str| -> f64 { row.split(key).nth(1).unwrap().split([',', '}']).next().unwrap().parse().unwrap() };
    input.split('{').skip(1).flat_map(|row| [field(row, "\"x\":"), (top + field(row, "\"y\":")) as f32 as f64 - top]).collect()
}

#[test]
fn template_backgrounds() {
    let mut s = Scene::new();
    let golden = Golden::load("template-backgrounds");
    // The recorded sizes: `"w":1588,"h":2246` per row of the input.
    let recorded: Vec<(u32, u32)> = golden.input.split("\"w\":").skip(1).map(|r| (r.split(',').next().unwrap().parse().unwrap(), r.split("\"h\":").nth(1).unwrap().split('}').next().unwrap().parse().unwrap())).collect();
    let size = |width: f64, height: f64, scale: f64| {
        let w = geom::js_max(1.0, crate::jsmath::round(width * scale));
        (w as u32, geom::js_max(1.0, crate::jsmath::round(height * (w / width))) as u32)
    };
    let mut done = Vec::new();
    for step in 0..=16 {
        let (w, h) = size(A4.0, A4.1, 2.0 + step as f64 * 0.25);
        s.tpl.bullet(&mut s.c, w, h);
        done.push((w, h));
    }
    for (kind, file, req) in [(Template::Agenda, "icons/agenda.svg", 901), (Template::Diary, "icons/diary.svg", 902)] {
        let bytes = std::fs::read(super::trace::fixture("../../..").join(file)).unwrap();
        let src = format!("data:image/svg+xml;base64,{}", b64_encode(&bytes, false));
        s.h.images.insert(file.into(), format!("{{\"image\":\"{}\",\"w\":106,\"h\":150}}", cyrb53(&src)));
        // v5 `_getSvgImage(kind)` + waiting for the load: its `onload` flushes the template canvases and warms
        // the default template URL again.
        assert!(s.tpl.svg_image(&mut s.c, kind).is_none());
        s.h.run(&mut s.c);
        assert_eq!(s.h.loads.last().unwrap(), &(req, file.to_string()));
        assert!(s.tpl.svg_loaded(&mut s.c, req, true));
        s.urls.clear();
        let scale = s.urls.need(Template::Default, 1.0, true).unwrap();
        assert_eq!((scale, s.urls.need(Template::Default, 1.0, true)), (3.6, None));
        s.tpl.bg_url(&mut s.c, Template::Default, scale, 7);
        for scale in [2.0, 3.0] {
            let (w, h) = size(559.0, 794.0, scale);
            s.tpl.svg(&mut s.c, kind, w, h);
            done.push((w, h));
        }
    }
    assert_eq!(done, recorded);
    s.check("template-backgrounds");
}

#[test]
fn dense_page_staged_repaint() {
    let mut s = Scene::new();
    let strokes = strokes_of(Golden::load("dense-page").surface("offscreen#5"));
    assert_eq!((strokes.len(), strokes.iter().filter(|s| s.tool == Tool::Highlighter).count()), (2400, 25));
    let items = items(&strokes, 0);
    let view = a4(&items);
    assert!(page::is_staged(&items.strokes));
    // Not the renderer's: a context probe and a Home card's cover crop (W2.5) that 5.18.0 made in this window.
    // Emitted by hand so that the whole file compares, and to show the command set covers them.
    let probe = s.c.canvas();
    s.c.ctx(probe, false);
    let crop = s.c.canvas();
    s.c.size(crop, 24, 16);
    s.c.ctx(crop, true);
    s.c.fill_style("#fff");
    s.c.fill_rect(0.0, 0.0, 24.0, 16.0);
    s.h.images.insert("blob:77".into(), "{\"image\":\"blob\",\"w\":8,\"h\":8}".into());
    let cover = s.c.image_blob(1, 77);
    s.c.draw_image(Src::Img(cover), &[0.32, 5.28, 7.36, 2.4, 0.0, 0.0, 24.0, 16.0]);
    s.c.height(crop, 0);
    s.c.width(crop, 0);
    // The editor opens: template URL, page canvases, three layout passes, the staged repaint, one more pass.
    let scale = s.urls.need(Template::Default, 1.0, true).unwrap();
    s.tpl.bg_url(&mut s.c, Template::Default, scale, 2);
    let p = s.attach(0);
    for _ in 0..3 {
        s.apply(&p);
    }
    s.staged(p.page, &view);
    s.apply(&p);
    s.scroll_preview(&view);
    assert_eq!(s.paths.len(), 2375);
    let _ = (p.lasso, p.cursor);
    s.check("dense-page");
}

#[test]
fn dense_page_erase_damage_repaint() {
    let mut s = Scene::new();
    let golden = Golden::load("dense-page-erase");
    let mut strokes = strokes_of(Golden::load("dense-page").surface("offscreen#5"));
    let p = s.page(0);
    // Before the recording the page was painted and previewed: its template canvases exist.
    s.tpl.draw(&mut s.c, None, PAGE_PX.0, PAGE_PX.1, Template::Default);
    s.scroll_preview(&a4(&PageItems::default()));
    s.h.run(&mut s.c);
    s.h.reset();
    let mut cursor = Cursor { surf: p.cursor, scale: CURSOR_SCALE, px: CURSOR_PX, bounds: None };
    let pen = samples(&golden.input, 105.0);
    let (mut hurt, mut erased) = (damage::Damage::default(), 0);
    for (k, at) in pen.chunks_exact(2).enumerate() {
        // v5 `eraseStrokeAtPoint` at the sample (stroke eraser, width 12), then the eraser cursor.
        let sweep = Sweep::point(at[0], at[1]);
        strokes.retain(|st| {
            let bounds = geom::bounds(&st.xy);
            let hit = sweep.may_touch(&bounds, geom::eraser_pad(6.0, st.width)) && geom::polyline_hit(&st.xy, &sweep, 6.0);
            if hit {
                hurt.add(bounds, st.width);
                erased += 1;
            }
            !hit
        });
        overlay::eraser_cursor(&mut s.c, &mut cursor, at[0], at[1], 12.0);
        // One animation frame per two samples (8 ms apart): `scheduleEraserRedraw`.
        if k % 2 == 0 {
            if let Some(d) = hurt.take() {
                let items = items(&strokes, 0);
                damage::repaint(&mut s.c, &mut s.tpl, &mut s.paths, p.page, &a4(&items), 2.0, d);
            }
        }
    }
    assert_eq!((erased, strokes.len()), (18, 2382));
    live::clear_cursor(&mut s.c, &mut cursor);
    live::clear_cursor(&mut s.c, &mut cursor);
    // Not the renderer's: the 5.x export that follows an edit encodes the template canvas and makes a 1 × 1
    // processing canvas (v6 exports ink as a form XObject instead, ARCHITECTURE §8.2).
    let template = s.tpl.cached(Template::Default, PAGE_PX.0, PAGE_PX.1).unwrap();
    s.c.to_blob(template, 3, "image/jpeg", 0.85);
    let one = s.c.processing(1, 1);
    s.c.ctx(one, false);
    s.c.size(one, 0, 0);
    // The full repaint after the gesture, and the page's new scroll preview.
    let items = items(&strokes, 0);
    s.staged(p.page, &a4(&items));
    s.scroll_preview(&a4(&items));
    s.check("dense-page-erase");
}

fn live_ink(scene: &str, tool: Tool, colour: &str, width: f64) {
    let mut s = Scene::new();
    let golden = Golden::load(scene);
    let p = s.page(0);
    // The page had its scroll preview once before the recording: the template copies exist.
    let empty = PageItems::default();
    s.scroll_preview(&a4(&empty));
    s.h.run(&mut s.c);
    s.h.reset();

    let xy = samples(&golden.input, 96.0);
    let colour = Color::Css(colour.into());
    let target = PageTarget { surf: p.page, sx: PAGE_PX.0 as f64 / A4.0, sy: PAGE_PX.1 as f64 / A4.1 };
    let mut cursor = Cursor { surf: p.cursor, scale: CURSOR_SCALE, px: CURSOR_PX, bounds: None };
    let mut live = Live::default();
    // Pointer down, then one frame per two samples.
    let mut ink = Vec::new();
    for n in std::iter::once(1).chain((2..40).step_by(2)) {
        ink.push(live::update(&mut s.c, &mut live, target, &mut cursor, &StrokeRef::new(tool, &colour, width, &xy[..2 * n])));
    }
    assert_eq!(ink[0], if tool == Tool::Pen { None } else { Some(true) });
    assert!(ink[1..].iter().all(|v| *v == Some(true)));
    live::commit(&mut s.c, &mut s.paths, &mut live, target, &StrokeRef::new(tool, &colour, width, &xy));
    live::clear_cursor(&mut s.c, &mut cursor);
    assert!(cursor.bounds.is_none());
    // 5.18.0 simplifies a highlighter's points after the commit (`simplifyStrokePoints`, the pen tool's
    // business — W3.5); the stored points are read from the recorded preview. A pen stroke keeps every sample.
    let mut committed = vec![Stroke { id: 0, tool, color: colour.clone(), width, xy }];
    if tool == Tool::Highlighter {
        committed = strokes_of(golden.surface("offscreen#0"));
        assert_eq!((committed.len(), committed[0].xy.len() / 2, committed[0].width), (1, 27, width));
    }
    let items = items(&committed, 0);
    s.scroll_preview(&a4(&items));
    s.check(scene);
}

#[test]
fn ink_pen() { live_ink("ink-pen", Tool::Pen, "#4D4D4D", 2.0) }

#[test]
fn ink_highlighter() { live_ink("ink-highlighter", Tool::Highlighter, "#ffde00", 16.0) }

#[test]
fn thumbnails_pages_panel() {
    let mut s = Scene::new();
    let golden = Golden::load("thumbnails");
    for index in 0..3 {
        let label = format!("#thumb-{index}");
        let strokes = strokes_of(golden.surface(&label));
        assert_eq!(strokes.len(), [15, 3, 0][index]);
        let (thumb, _) = s.dom(label, (272, 384));
        let items = items(&strokes, index as u32);
        thumb::panel(&mut s.c, &mut s.tpl, &mut s.paths, thumb, 272, 384, ThumbBg::Template(Template::Default), &a4(&items));
    }
    s.check("thumbnails");
}

#[test]
fn page_preview() {
    let mut s = Scene::new();
    // The 20 pages of the profile document hold the same 120 strokes; they are read from the other recording.
    let strokes = strokes_of(Golden::load("scroll-preview").surface("offscreen#0"));
    assert_eq!(strokes.len(), 120);
    s.tpl.draw(&mut s.c, None, 794, 1123, Template::Default);
    s.h.run(&mut s.c);
    s.h.reset();
    for page in [0, 1, 2, 9, 19] {
        let items = items(&strokes, page);
        let req = s.req();
        assert!(thumb::page_preview(&mut s.c, &mut s.tpl, &mut s.paths, &a4(&items), req));
    }
    s.check("page-preview");
}

#[test]
fn page_preview_with_images() {
    let mut s = Scene::new();
    let golden = Golden::load("page-preview-images");
    let strokes = strokes_of(golden.surface("offscreen#0"));
    assert_eq!(strokes.len(), 1);
    s.tpl.draw(&mut s.c, None, 794, 1123, Template::Default);
    // The two image assets of the `inline-images` fixture (trace identity = hash of their data URL).
    s.h.images.insert("blob:1".into(), "{\"image\":\"168798a4dea1ba\",\"w\":64,\"h\":48}".into());
    s.h.images.insert("blob:2".into(), "{\"image\":\"1cfae403635686\",\"w\":40,\"h\":40}".into());
    let (a, b): (Img, Img) = (s.c.image_blob(1, 1), s.c.image_blob(2, 2));
    s.h.run(&mut s.c);
    s.h.reset();
    let image = |img, cx, cy, w, h| ImageRef { img: Some(img), cx, cy, w, h, rotation: 0.0 };
    let first = PageItems { images: vec![image(a, 120.0, 160.0, 64.0, 48.0), image(b, 438.0, 318.0, 40.0, 40.0)], ..items(&strokes, 0) };
    let second = PageItems { images: vec![image(a, 200.0, 500.0, 64.0, 48.0)], ..PageItems::default() };
    for items in [&first, &second] {
        assert!(thumb::page_preview(&mut s.c, &mut s.tpl, &mut s.paths, &a4(items), 9));
    }
    s.check("page-preview-images");
}

#[test]
fn scroll_preview_queue() {
    let mut s = Scene::new();
    let golden = Golden::load("scroll-preview");
    let strokes = strokes_of(golden.surface("offscreen#0"));
    let items = items(&strokes, 0);
    let view = a4(&items);
    assert!(!page::is_staged(&items.strokes));
    // Before the recording: the first pages were painted and previewed, so both template canvases exist.
    s.tpl.draw(&mut s.c, None, PAGE_PX.0, PAGE_PX.1, Template::Default);
    s.scroll_preview(&a4(&PageItems::default()));
    s.h.run(&mut s.c);
    s.h.reset();
    // A page gets its preview; the next ones come into view, are activated and painted, and get theirs. The
    // first of them is released again before the recording ends (its canvases leave the document).
    let activate = |s: &mut Scene, index: usize| {
        let p = s.attach(index);
        s.apply(&p);
        s.apply(&p);
        page::repaint(&mut s.c, &mut s.tpl, &mut s.paths, p.page, PAGE_PX, &view, 2.0);
        p
    };
    s.scroll_preview(&view);
    let released = activate(&mut s, 2);
    for index in 3..=5 {
        activate(&mut s, index);
    }
    for _ in 0..3 {
        s.scroll_preview(&view);
    }
    s.h.run(&mut s.c);
    for node in released.nodes {
        s.h.detach(node);
    }
    s.check("scroll-preview");
}
