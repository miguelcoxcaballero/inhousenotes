//! Trace goldens (TESTING §1, §3.4): each `canvas` scene recorded from 5.18.0 by W1.2 is produced again by the
//! Rust renderer and compared call by call, argument bits included. The scene functions play the part of the
//! editor (which canvases exist, when a frame happens); every drawing call comes from `render`.

use super::cmd::{Cmds, Img, Src, Surf};
use super::live::{self, Cursor, Live, PageTarget};
use super::page::{self, Bg, PageView, Staged};
use super::template::{BgUrls, Templates};
use super::thumb::{self, ScrollPreview, ThumbBg};
use super::trace::{strokes_of, Golden, Host, Stroke};
use super::{damage, overlay, pdfink, ImageRef, PageItems, PathCache, StrokeRef};
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
        self.h.dump(scene);
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

#[test]
fn host_table_matches() {
    // host/canvas.js mirrors `cmd::op` and the three keyword tables.
    let js = std::fs::read_to_string(super::trace::fixture("../../../host/canvas.js")).unwrap();
    let table = js.split("export const OP = {").nth(1).unwrap().split('}').next().unwrap();
    let mut theirs: Vec<(String, u8)> = table.split(',').map(|e| e.split_once(':').unwrap()).map(|(k, v)| (k.trim().to_string(), v.trim().parse().unwrap())).collect();
    let rust = include_str!("cmd.rs").split("pub mod op {").nth(1).unwrap().split("\n}").next().unwrap();
    let mut ours: Vec<(String, u8)> = rust.lines().filter_map(|l| l.trim().strip_prefix("pub const ")).map(|l| l.split_once(": u8 = ").unwrap()).map(|(k, v)| (k.to_string(), v.trim_end_matches(';').parse().unwrap())).collect();
    theirs.sort();
    ours.sort();
    assert_eq!(ours, theirs);
    assert_eq!(ours.len(), 54);
    for (name, list) in [("CAPS", &super::cmd::CAPS), ("JOINS", &super::cmd::JOINS), ("COMPOSITES", &super::cmd::COMPOSITES)] {
        let text = js.split(&format!("const {name} = [")).nth(1).unwrap().split(']').next().unwrap();
        assert_eq!(text.split(", ").map(|v| v.trim_matches('\'')).collect::<Vec<_>>(), list.to_vec());
    }
}

// ------------------------------------------------------------------------------- without a recorded scene

fn stroke(tool: Tool, colour: &str, width: f64, xy: &[f64]) -> Stroke { Stroke { id: 0, tool, color: Color::parse(colour), width, xy: xy.to_vec() } }

/// The op names a command batch makes on the canvas `label`, in order.
fn calls(s: &mut Scene, label: &str) -> Vec<String> {
    s.h.run(&mut s.c);
    let all = s.h.surfaces();
    let ops = all.iter().find(|e| e.0.contains(&format!("\"{label}\""))).map(|e| e.1.clone()).unwrap_or_default();
    s.h.reset();
    ops.iter().map(|l| super::trace::name(l).to_string()).collect()
}

#[test]
fn single_tap_dots_survive_repaints() {
    // BEHAVIOUR-CHANGES B-E3: a one-point pen stroke is a filled circle of radius max(0.7, width / 2) wherever
    // strokes are drawn; 5.18.0 only showed it until the next repaint.
    let mut s = Scene::new();
    let p = s.page(0);
    let dots = [stroke(Tool::Pen, "#002FD9", 3.0, &[50.0, 60.0]), stroke(Tool::Pen, "#002fd9", 1.0, &[70.0, 60.0])];
    let items = items(&dots, 0);
    page::repaint(&mut s.c, &mut s.tpl, &mut s.paths, p.page, PAGE_PX, &PageView { bg: Bg::PdfPending, ..a4(&items) }, 2.0);
    s.h.run(&mut s.c);
    let ops = &s.h.surfaces()[0].1;
    let arcs: Vec<Vec<f64>> = ops.iter().filter(|l| l.starts_with("[\"arc\"")).map(|l| super::trace::nums(l)).collect();
    assert_eq!(arcs, vec![vec![50.0, 60.0, 1.5, 0.0, super::TAU], vec![70.0, 60.0, 0.7, 0.0, super::TAU]]);
    // `Color::parse` folds the case of `#rrggbb`; the canvas string is lowercase.
    assert_eq!(ops.iter().filter(|l| l.as_str() == "[\"set:fillStyle\",\"#002fd9\"]").count(), 2);
    assert!(s.paths.is_empty());
}

#[test]
fn backgrounds_covers_and_the_path_cache() {
    let mut s = Scene::new();
    let p = s.page(0);
    s.h.images.insert("blob:5".into(), "{\"image\":\"photo\",\"w\":10,\"h\":10}".into());
    let photo = s.c.image_blob(1, 5);
    let ink = [stroke(Tool::Pen, "#111111", 2.0, &[10.0, 10.0, 30.0, 10.0, 30.0, 40.0])];
    let cover = [stroke(Tool::LegacyCover, "#111111", 2.0, &[10.0, 10.0, 30.0, 10.0])];
    let mut all = items(&ink, 0);
    all.covers = items(&cover, 9).strokes;
    all.images = vec![ImageRef { img: Some(photo), cx: 100.0, cy: 100.0, w: 20.0, h: 10.0, rotation: f64::NAN }, ImageRef { img: None, cx: 0.0, cy: 0.0, w: 1.0, h: 1.0, rotation: 0.0 }];
    let view = PageView { bg: Bg::Image(Some(photo)), ..a4(&all) };
    page::repaint(&mut s.c, &mut s.tpl, &mut s.paths, p.page, PAGE_PX, &view, 2.0);
    s.h.run(&mut s.c);
    let ops = s.h.surfaces()[0].1.clone();
    let names: Vec<&str> = ops.iter().map(|l| super::trace::name(l)).collect();
    // white, photo, cover (white, 1.35 × wider), the decoded image (rotation `NaN || 0`), the stroke.
    assert_eq!(names[..9], ["getContext", "save", "setTransform", "clearRect", "set:globalCompositeOperation", "set:globalAlpha", "set:fillStyle", "fillRect", "drawImage"]);
    assert!(ops.contains(&"[\"set:strokeStyle\",\"#ffffff\"]".to_string()) && ops.contains(&format!("[\"set:lineWidth\",{}]", super::trace::num(2.0 * 1.35))));
    assert_eq!(names.iter().filter(|n| **n == "drawImage").count(), 2);
    assert!(ops.contains(&format!("[\"rotate\",{}]", super::trace::num(0.0))));
    assert_eq!((names.iter().filter(|n| **n == "stroke(path)").count(), s.paths.len()), (2, 2));
    // Unchanged geometry: the cached Path2D is stroked again, nothing is rebuilt. A new body stamp rebuilds it.
    s.h.reset();
    page::repaint(&mut s.c, &mut s.tpl, &mut s.paths, p.page, PAGE_PX, &view, 2.0);
    let again = s.c.take();
    assert!(!again.contains(&super::cmd::op::PATH_BEGIN) || again.iter().filter(|b| **b == super::cmd::op::BEZIER_TO).count() < 2);
    let mut moved = items(&ink, 0);
    moved.strokes[0].key = Some((Id(0), Stamp { clock: 7, ..Stamp::ZERO }));
    let before = s.paths.len();
    pen_draw_twice(&mut s, &moved);
    assert_eq!(s.paths.len(), before);
    s.paths.forget(&mut s.c, Id(0));
    assert_eq!(s.paths.len(), before - 1);
    s.paths.clear(&mut s.c);
    assert!(s.paths.is_empty());
}

fn pen_draw_twice(s: &mut Scene, items: &PageItems) {
    let probe = s.c.canvas();
    s.c.ctx(probe, false);
    super::pen::draw(&mut s.c, &mut s.paths, &items.strokes[0]);
    let first = s.c.take().len();
    super::pen::draw(&mut s.c, &mut s.paths, &items.strokes[0]);
    assert!(s.c.take().len() < first, "the second draw reuses the path");
}

#[test]
fn page_resize_keeps_the_last_frame() {
    // v5 `resizePageCanvas`: copy out, resize, draw back scaled, release the copy.
    let mut s = Scene::new();
    let p = s.page(0);
    page::resize(&mut s.c, p.page, Some(PAGE_PX), (1787, 2527));
    assert_eq!(calls(&mut s, "canvas.page-canvas[page=0]"), ["set:width", "set:height", "getContext", "drawImage"]);
    page::resize(&mut s.c, p.page, None, PAGE_PX);
    assert_eq!(calls(&mut s, "canvas.page-canvas[page=0]"), ["set:width", "set:height"]);
}

#[test]
fn overlays() {
    let mut s = Scene::new();
    let p = s.page(0);
    let lasso_scale = LASSO_PX.0 as f64 / A4.0;
    overlay::lasso_path(&mut s.c, p.lasso, LASSO_PX, lasso_scale, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    assert_eq!(calls(&mut s, "canvas.lasso-overlay[page=0]"), ["getContext", "setTransform", "clearRect", "set:strokeStyle", "set:lineWidth", "setLineDash", "beginPath", "moveTo", "lineTo", "lineTo", "stroke", "setLineDash"]);
    overlay::lasso_path(&mut s.c, p.lasso, LASSO_PX, lasso_scale, &[]);
    assert!(s.c.is_empty());

    let picked = [stroke(Tool::Pen, "#111111", 2.0, &[1.0, 1.0, 2.0, 2.0, 3.0, 1.0]), stroke(Tool::Pen, "#111111", 2.0, &[9.0, 9.0])];
    let picked = items(&picked, 0);
    assert!(overlay::selection_outlines(&mut s.c, p.lasso, LASSO_PX, lasso_scale, &picked.strokes));
    assert_eq!(calls(&mut s, "canvas.lasso-overlay[page=0]"), ["getContext", "setTransform", "clearRect", "setTransform", "set:strokeStyle", "set:lineWidth", "setLineDash", "set:lineCap", "set:lineJoin", "beginPath", "moveTo", "lineTo", "lineTo", "stroke", "setLineDash"]);
    assert!(!overlay::selection_outlines(&mut s.c, p.lasso, LASSO_PX, lasso_scale, &[]));
    assert_eq!(calls(&mut s, "canvas.lasso-overlay[page=0]"), ["getContext", "setTransform", "clearRect"]);

    // Other devices' ink: appended as straight segments, redrawn as the spline.
    let remote = [stroke(Tool::Pen, "#e81010", 2.0, &[0.0, 0.0, 10.0, 0.0, 20.0, 5.0, 30.0, 5.0])];
    let remote = items(&remote, 0);
    let mut rendered = 0;
    let head = StrokeRef { xy: &remote.strokes[0].xy[..2], ..remote.strokes[0] };
    assert!(overlay::remote_append(&mut s.c, p.remote, CURSOR_SCALE, &head, &mut rendered));
    assert_eq!((rendered, calls(&mut s, "canvas.remote-live-overlay[page=0]").last().unwrap().as_str()), (1, "fill"));
    assert!(overlay::remote_append(&mut s.c, p.remote, CURSOR_SCALE, &remote.strokes[0], &mut rendered));
    assert_eq!(calls(&mut s, "canvas.remote-live-overlay[page=0]")[9..], ["beginPath", "moveTo", "lineTo", "lineTo", "lineTo", "stroke"]);
    assert!(!overlay::remote_append(&mut s.c, p.remote, CURSOR_SCALE, &remote.strokes[0], &mut rendered) && s.c.is_empty());
    assert!(overlay::remote_redraw(&mut s.c, &mut s.paths, p.remote, CURSOR_PX, CURSOR_SCALE, &remote.strokes));
    assert_eq!(calls(&mut s, "canvas.remote-live-overlay[page=0]").last().unwrap(), "stroke(path)");
    assert!(s.paths.is_empty(), "live ink is never cached");

    // Timeline preview highlights on the selected context.
    s.c.ctx(p.remote, false);
    overlay::timeline_highlights(&mut s.c, &[overlay::Highlight { removed: true, x: 1.0, y: 2.0, w: 3.0, h: 4.0 }, overlay::Highlight { removed: false, x: 1.0, y: 2.0, w: 3.0, h: 4.0 }]);
    assert_eq!(calls(&mut s, "canvas.remote-live-overlay[page=0]")[1..], ["save", "set:lineJoin", "set:strokeStyle", "set:fillStyle", "set:lineWidth", "setLineDash", "fillRect", "strokeRect", "set:strokeStyle", "set:fillStyle", "set:lineWidth", "setLineDash", "fillRect", "strokeRect", "restore"]);
}

#[test]
fn panel_thumbnail_backgrounds() {
    let mut s = Scene::new();
    let (thumb, _) = s.dom("#thumb-7".into(), (272, 384));
    let none = PageItems::default();
    let view = PageView { w: 595.0, h: 842.0, bg: Bg::PdfPending, items: &none };
    thumb::panel(&mut s.c, &mut s.tpl, &mut s.paths, thumb, 272, 384, ThumbBg::PdfPlaceholder, &view);
    s.h.run(&mut s.c);
    let ops = s.h.surfaces()[0].1.clone();
    // inset = max(2, round(272 × 0.04)) = 11; font = max(9, round(272 × 0.18)) = 49 px.
    assert!(ops.contains(&format!("[\"fillRect\",{0},{0},{1},{2}]", super::trace::num(11.0), super::trace::num(250.0), super::trace::num(362.0))));
    assert!(ops.contains(&"[\"set:font\",\"bold 49px sans-serif\"]".to_string()) && ops.contains(&format!("[\"fillText\",\"PDF\",{},{}]", super::trace::num(136.0), super::trace::num(192.0))));
    assert!(ops.contains(&format!("[\"scale\",{},{}]", super::trace::num(272.0 / 595.0), super::trace::num(384.0 / 842.0))));
    s.h.reset();
    thumb::panel(&mut s.c, &mut s.tpl, &mut s.paths, thumb, 272, 384, ThumbBg::Waiting, &view);
    assert_eq!(calls(&mut s, "#thumb-7"), ["getContext", "set:fillStyle", "fillRect"]);
    assert!(!thumb::page_preview(&mut s.c, &mut s.tpl, &mut s.paths, &view, 1) && s.c.is_empty());
    // A5 template pages draw the SVG once it is loaded; before that the canvas stays white and is not cached
    // past the load (v5 flushes `_tplCache` in the image's onload).
    s.h.images.insert("icons/diary.svg".into(), "{\"image\":\"diary\",\"w\":106,\"h\":150}".into());
    s.tpl.draw(&mut s.c, None, 1118, 1588, Template::Diary);
    s.h.run(&mut s.c);
    assert!(s.tpl.cached(Template::Diary, 1118, 1588).is_some() && s.h.loads == [(902, "icons/diary.svg".to_string())]);
    assert!(!s.tpl.svg_loaded(&mut s.c, 902, false) && !s.tpl.svg_loaded(&mut s.c, 55, true));
    assert!(s.tpl.svg_loaded(&mut s.c, 902, true) && s.tpl.cached(Template::Diary, 1118, 1588).is_none());
    assert_eq!((super::template::bg_scale(1.0, false), super::template::bg_scale(3.0, false), super::template::bg_scale(f64::NAN, true), super::template::bg_scale(2.0, true)), (3.0, 4.2, 3.6, 4.8));
}

#[test]
fn pdf_ink_streams_parse_and_keep_every_sample() {
    let pen: Vec<f64> = (0..40).flat_map(|k| [10.0 + k as f64 * 5.5, 300.0 + crate::jsmath::sin(k as f64 / 2.5) * 22.0]).collect();
    let dense: Vec<f64> = (0..30).flat_map(|k| [400.0 + k as f64 * 0.37, 500.0 + k as f64 * 0.11]).collect();
    let list = [
        stroke(Tool::Highlighter, "#ffde00", 16.0, &pen[..20]),
        stroke(Tool::Pen, "#002FD9", 2.0, &pen),
        stroke(Tool::Pen, "#f00", 1.5, &dense),
        stroke(Tool::Pen, "#1a1a1a", 3.0, &[77.0, 88.0]),
        stroke(Tool::Highlighter, "#ffde00", 0.0, &[90.0, 91.0]),
        stroke(Tool::Pen, "#1a1a1a", 3.0, &[1.0, f64::NAN, 2.0, 3.0]),
    ];
    let mut all = items(&list, 0);
    all.images = vec![ImageRef { img: None, cx: 200.0, cy: 100.0, w: 64.0, h: 48.0, rotation: std::f64::consts::FRAC_PI_2 }];
    let ink = pdfink::content_stream(1123.0, &all).unwrap();
    assert_eq!(ink.images, [0]);
    assert!(pdfink::content_stream(1123.0, &PageItems::default()).is_none());

    // A small content-stream reader: operands are numbers or names, every operator has its operand count.
    let arity = |operator: &str| -> usize {
        match operator {
            "q" | "Q" | "S" | "f" => 0,
            "w" | "J" | "j" | "gs" | "Do" => 1,
            "m" | "l" => 2,
            "RG" | "rg" => 3,
            "c" | "cm" => 6,
            other => panic!("operator {other}"),
        }
    };
    let (mut stack, mut depth, mut ends, mut ops): (Vec<&str>, i32, Vec<(f64, f64)>, Vec<String>) = (Vec::new(), 0, Vec::new(), Vec::new());
    for token in ink.content.split_ascii_whitespace() {
        if token.parse::<f64>().is_ok() || token.starts_with('/') {
            assert!(!token.contains(['e', 'E']) || token.starts_with('/'), "no exponents in PDF numbers: {token}");
            stack.push(token);
            continue;
        }
        assert_eq!(stack.len(), arity(token), "{token} after {stack:?}");
        match token {
            "q" => depth += 1,
            "Q" => depth -= 1,
            "m" | "l" | "c" => ends.push((stack[stack.len() - 2].parse().unwrap(), stack[stack.len() - 1].parse().unwrap())),
            _ => {}
        }
        assert!(depth >= 0);
        ops.push(format!("{} {token}", stack.join(" ")).trim().to_string());
        stack.clear();
    }
    assert!(stack.is_empty() && depth == 0);
    assert_eq!(ops[..4], ["q", "1 0 0 -1 0 1123 cm", "1 J", "1 j"]);
    // The image box, rotated a quarter turn about its centre, upright in the flipped space.
    assert!(ops.contains(&"0 64 48 0 176 68 cm".to_string()) && ops.contains(&"/Im0 Do".to_string()));
    // Highlighter: two multiply passes; pen: colour, width, one path; dots: filled circles; NaN stroke: skipped.
    let count = |needle: &str| ops.iter().filter(|o| o.as_str() == needle).count();
    assert_eq!((count("/GSh1 gs"), count("/GSh2 gs"), count("/GSh3 gs"), count("16 w"), count("9.92 w")), (1, 1, 1, 1, 1));
    assert_eq!((count("0 0.1843 0.851 RG"), count("1 0 0 RG"), count("S"), count("f")), (1, 1, 4, 2));
    // Every pen sample is the end point of a segment, in order (4 decimals).
    let near = |a: (f64, f64), b: &[f64]| (a.0 - b[0]).abs() <= 5.1e-5 && (a.1 - b[1]).abs() <= 5.1e-5;
    for xy in [&pen, &dense] {
        let start = ends.windows(xy.len() / 2).position(|w| w.iter().zip(xy.chunks_exact(2)).all(|(e, p)| near(*e, p)));
        assert!(start.is_some(), "samples lost");
    }
    // Short segments are lines, long ones curves — the screen's rule. Lines: 29 dense pen segments + the last
    // point of each highlighter pass; curves: 39 pen segments + 8 quadratics per pass + two dots of 4 arcs.
    assert_eq!((count_ops(&ops, " l"), count_ops(&ops, " c")), (29 + 2, 39 + 2 * 8 + 2 * 4));
}

fn count_ops(ops: &[String], suffix: &str) -> usize { ops.iter().filter(|o| o.ends_with(suffix)).count() }

#[test]
fn page_items_follow_the_document_draw_order() {
    use crate::doc::{Author, Doc, Tx};
    use crate::id::{ActorId, IdGen};
    use crate::op::{Background, Body, ImageRect};
    let mut doc = Doc::default();
    let mut author = Author::new(IdGen::new(ActorId(0x700), 0), 0);
    let mut tx = Tx::new(&doc, &mut author, 1000);
    let page = tx.page_new(None, Template::Default, (794, 1123), Background::Template);
    let body = |tool, x: f64| Body::Stroke { tool, color: Color::parse("#111111"), width: 2.0, xy: vec![x, 1.0, x + 5.0, 9.0], p: None };
    tx.add_stroke(page, body(Tool::Pen, 10.0));
    tx.add_image(page, [7u8; 32], ImageRect { cx: 50.0, cy: 60.0, w: 20.0, h: 10.0, rotation: 0.0 });
    tx.add_stroke(page, body(Tool::LegacyCover, 30.0));
    let gone = tx.add_stroke(page, body(Tool::Highlighter, 40.0));
    let batch = tx.commit();
    doc.apply(&batch);
    let mut tx = Tx::new(&doc, &mut author, 1001);
    tx.erase(&[gone]);
    let batch = tx.commit();
    doc.apply(&batch);
    let items = PageItems::collect(&doc, page, &|hash| (hash == [7u8; 32]).then_some(Img(3)));
    assert_eq!((items.covers.len(), items.images.len(), items.strokes.len()), (1, 1, 1));
    assert_eq!((items.images[0].img, items.strokes[0].bounds, items.covers[0].xy[0]), (Some(Img(3)), [10.0, 1.0, 15.0, 9.0], 30.0));
    assert!(items.strokes[0].key.is_some() && items.strokes[0].key != items.covers[0].key);
}
