//! Canvas command stream (ARCHITECTURE §6.2): every Canvas 2D call 5.18.0 makes for a page, an overlay, a
//! thumbnail or a template, with the same arguments in the same order. Each function names the 5.18.0 function it
//! ports. Nothing here measures time or touches the DOM: callers (`ihn-app::editor`) own canvases, scheduling and
//! overlay visibility, and send [`cmd::Cmds::take`] to `host/canvas.js`.

pub mod cmd;
pub mod damage;
pub mod highlighter;
pub mod live;
pub mod overlay;
pub mod page;
pub mod pdfink;
pub mod pen;
pub mod template;
pub mod thumb;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod trace;

use crate::doc::{Doc, Item};
use crate::geom;
use crate::id::{Hash, Id, IdMap};
use crate::op::{Body, Color, Tool};
use crate::stamp::Stamp;
use cmd::{Cmds, Img, PathId};

/// `Math.PI * 2`.
pub const TAU: f64 = 2.0 * std::f64::consts::PI;

/// A stroke as the renderer sees it (5.18.0 `{tool, color, width, points}`).
#[derive(Clone, Copy, Debug)]
pub struct StrokeRef<'a> {
    pub tool: Tool,
    pub color: &'a Color,
    pub width: f64,
    /// `x0, y0, x1, y1, …`
    pub xy: &'a [f64],
    /// `[minX, minY, maxX, maxY]` of the points (5.18.0 `getStrokeBounds`).
    pub bounds: [f64; 4],
    /// Identity of the geometry for the `Path2D` cache: item id + stamp of its body. `None`: never cached.
    pub key: Option<(Id, Stamp)>,
}

impl<'a> StrokeRef<'a> {
    pub fn new(tool: Tool, color: &'a Color, width: f64, xy: &'a [f64]) -> StrokeRef<'a> { StrokeRef { tool, color, width, xy, bounds: geom::bounds(xy), key: None } }
    /// The stroke of a live document item (`None` for images and dropped bodies).
    pub fn of(id: Id, item: &'a Item, bounds: [f64; 4]) -> Option<StrokeRef<'a>> {
        match &item.body.v {
            Body::Stroke { tool, color, width, xy, .. } => Some(StrokeRef { tool: *tool, color, width: *width, xy, bounds, key: Some((id, item.body.at)) }),
            _ => None,
        }
    }
    pub fn len(&self) -> usize { self.xy.len() / 2 }
    pub fn is_empty(&self) -> bool { self.xy.len() < 2 }
    pub fn pt(&self, i: usize) -> (f64, f64) { (self.xy[2 * i], self.xy[2 * i + 1]) }
    /// Repaint filter. 5.18.0 wants two points of anything but a highlighter; v6 keeps single-tap dots
    /// (BEHAVIOUR-CHANGES B-E3), so every stroke with a point is drawn.
    pub fn drawable(&self) -> bool { !self.is_empty() }
}

/// An image on a page; `img` is `None` until its asset is decoded (5.18.0 `!img.complete`: skipped).
#[derive(Clone, Copy, Debug)]
pub struct ImageRef {
    pub img: Option<Img>,
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
    pub rotation: f64,
}

/// The draw runs of one page in 5.18.0 `redrawPage` order: legacy covers, images, strokes (`Doc::draw_list`).
#[derive(Default)]
pub struct PageItems<'a> {
    pub covers: Vec<StrokeRef<'a>>,
    pub images: Vec<ImageRef>,
    pub strokes: Vec<StrokeRef<'a>>,
}

impl<'a> PageItems<'a> {
    /// `decoded` maps an asset to its image once it may be drawn.
    pub fn collect(doc: &'a Doc, page: Id, decoded: &dyn Fn(Hash) -> Option<Img>) -> PageItems<'a> {
        let list = doc.draw_list(page);
        let stroke = |r: &crate::doc::DrawRef| doc.live(r.id).and_then(|item| StrokeRef::of(r.id, item, r.bounds));
        PageItems {
            covers: list.covers.iter().filter_map(stroke).collect(),
            images: list
                .images
                .iter()
                .filter_map(|r| match &doc.live(r.id)?.body.v {
                    Body::Image { asset, cx, cy, w, h, rotation } => Some(ImageRef { img: decoded(*asset), cx: *cx, cy: *cy, w: *w, h: *h, rotation: *rotation }),
                    _ => None,
                })
                .collect(),
            strokes: list.strokes.iter().filter_map(stroke).collect(),
        }
    }
}

/// 5.18.0 `penPathCache`: one `Path2D` per pen stroke geometry, built on first draw and reused by every repaint.
#[derive(Default)]
pub struct PathCache {
    map: IdMap<(Stamp, PathId)>,
}

impl PathCache {
    pub fn len(&self) -> usize { self.map.len() }
    pub fn is_empty(&self) -> bool { self.map.is_empty() }
    /// The path of `s`, building it when the cache has none for this geometry. `.1`: drop it after use.
    fn path(&mut self, c: &mut Cmds, s: &StrokeRef) -> (PathId, bool) {
        let build = |c: &mut Cmds| {
            let p = c.path_begin();
            pen::path(c, s.xy);
            c.path_end();
            p
        };
        let Some((id, at)) = s.key else { return (build(c), true) };
        if let Some((have, p)) = self.map.get(&id).copied() {
            if have == at {
                return (p, false);
            }
            c.path_drop(p);
        }
        let p = build(c);
        self.map.insert(id, (at, p));
        (p, false)
    }
    /// Call when an item is deleted or leaves memory.
    pub fn forget(&mut self, c: &mut Cmds, id: Id) {
        if let Some((_, p)) = self.map.remove(&id) {
            c.path_drop(p);
        }
    }
    pub fn clear(&mut self, c: &mut Cmds) {
        for (_, (_, p)) in self.map.drain() {
            c.path_drop(p);
        }
    }
}

/// The canvas colour string: lowercase `#rrggbb` or the stored text, without allocating for the common case.
fn with_css<R>(color: &Color, f: impl FnOnce(&str) -> R) -> R {
    match color {
        Color::Css(s) => f(s),
        Color::Hex(v) => {
            let mut buf = *b"#000000";
            for (i, b) in buf[1..].iter_mut().enumerate() {
                *b = b"0123456789abcdef"[(v >> (20 - 4 * i) & 15) as usize];
            }
            f(std::str::from_utf8(&buf).unwrap_or("#000000"))
        }
    }
}

/// JS `value || fallback` for numbers.
fn or(value: f64, fallback: f64) -> f64 {
    if value == 0.0 || value.is_nan() {
        fallback
    } else {
        value
    }
}

/// Test command hook: `__IHN_TEST_API__.cmd('core.render.<name>', args)` (JSON text in, JSON text out).
pub fn test_cmd(name: &str, _args: &str) -> Option<String> {
    match name {
        // The command table, for the host self-test: `[["CREATE",1], …]` is not needed; the count is enough to
        // detect a host built against another ABI.
        "abi" => Some(format!("{{\"batch\":{},\"last\":{}}}", cmd::REC_BATCH, cmd::op::READ_PIXELS)),
        _ => None,
    }
}
