//! Canvas command encoder (ARCHITECTURE §3.3 family 2). Rust decides every call and argument; `host/canvas.js`
//! replays them on real contexts.
//!
//! Wire form: the app wraps what [`Cmds::take`] returns in ONE out-buffer record `[2][REC_BATCH][u32 len]`. Inside,
//! commands are `[u8 op][arguments]` without a length (a dense page repaint is ~50,000 commands). Numbers are f64,
//! handles u32, strings u32 length + UTF-8. Drawing commands act on the selected context: [`Cmds::ctx`] calls
//! `getContext('2d')` exactly where 5.18.0 does (the call is part of the recorded trace) and selects it;
//! [`Cmds::select`] only selects. Between [`Cmds::path_begin`] and [`Cmds::path_end`] the path commands build a
//! `Path2D` instead.
//!
//! Handles are chosen here. Surface numbers are never reused (caches compare them like object identity).

use crate::codec::Writer;

/// Out-buffer record op of family 2 that carries a command batch.
pub const REC_BATCH: u8 = 1;

/// Command bytes. `host/canvas.js` mirrors this table; `tests::host_table_matches` compares the two.
pub mod op {
    /// `u32 surf, u8 kind` (0: `document.createElement('canvas')`; 1: `+ u32 w, u32 h`, 5.18.0
    /// `createProcessingCanvas`: an `OffscreenCanvas` when the browser has it).
    pub const CREATE: u8 = 1;
    /// `u32 surf, u32 dom node`: a canvas element of the document.
    pub const BIND: u8 = 2;
    /// `u32 surf`: forget the surface.
    pub const DROP: u8 = 3;
    /// `u32 surf, u32 px`
    pub const WIDTH: u8 = 4;
    pub const HEIGHT: u8 = 5;
    /// `u32 surf, u8 flags` (1: `{ willReadFrequently: true }`): `getContext('2d')` and select.
    pub const CTX: u8 = 6;
    /// `u32 surf`: select the context obtained earlier.
    pub const SELECT: u8 = 7;
    pub const SAVE: u8 = 8;
    pub const RESTORE: u8 = 9;
    /// 6 × f64
    pub const SET_TRANSFORM: u8 = 10;
    pub const SCALE: u8 = 11;
    pub const TRANSLATE: u8 = 12;
    pub const ROTATE: u8 = 13;
    /// str
    pub const FILL_STYLE: u8 = 14;
    pub const STROKE_STYLE: u8 = 15;
    pub const LINE_WIDTH: u8 = 16;
    /// u8: index into `CAPS`
    pub const LINE_CAP: u8 = 17;
    /// u8: index into `JOINS`
    pub const LINE_JOIN: u8 = 18;
    pub const GLOBAL_ALPHA: u8 = 19;
    /// u8: index into `COMPOSITES`
    pub const COMPOSITE: u8 = 20;
    /// `u8 n, n × f64`
    pub const LINE_DASH: u8 = 21;
    pub const BEGIN_PATH: u8 = 22;
    pub const MOVE_TO: u8 = 23;
    pub const LINE_TO: u8 = 24;
    pub const BEZIER_TO: u8 = 25;
    pub const QUAD_TO: u8 = 26;
    /// `x, y, r, a0, a1`
    pub const ARC: u8 = 27;
    /// `x, y, w, h, r`
    pub const ROUND_RECT: u8 = 28;
    pub const RECT: u8 = 29;
    pub const CLIP: u8 = 30;
    pub const STROKE: u8 = 31;
    pub const FILL: u8 = 32;
    pub const CLEAR_RECT: u8 = 33;
    pub const FILL_RECT: u8 = 34;
    pub const STROKE_RECT: u8 = 35;
    /// `u8 kind (0 surface, 1 image), u32 handle, u8 argc (2, 4 or 8), argc × f64`
    pub const DRAW_IMAGE: u8 = 36;
    /// `u32 path`: start a `Path2D`; `MOVE_TO`, `LINE_TO`, `BEZIER_TO`, `QUAD_TO` go to it until `PATH_END`.
    pub const PATH_BEGIN: u8 = 37;
    pub const PATH_END: u8 = 38;
    /// `u32 path`: `ctx.stroke(path)`
    pub const STROKE_PATH: u8 = 39;
    pub const PATH_DROP: u8 = 40;
    /// str
    pub const FILTER: u8 = 41;
    pub const SHADOW_COLOR: u8 = 42;
    pub const SHADOW_BLUR: u8 = 43;
    /// `x0, y0, r0, x1, y1, r1, u8 n, n × (f64 offset, str colour)`: `fillStyle = createRadialGradient(…)`.
    pub const RADIAL_FILL: u8 = 44;
    /// `u32 surf, u32 req, str type, f64 quality` → `Io(req, 0, bytes = the data URL)`
    pub const TO_DATA_URL: u8 = 45;
    /// `u32 surf, u32 req, str type, f64 quality` → `Io(req, 0, obj = the Blob, bytes = f64 size)`; status 1 = null.
    pub const TO_BLOB: u8 = 46;
    /// `u32 image, u32 req, str url` → `Io(req, 0 | 1, bytes = f64 naturalWidth, f64 naturalHeight)` once decoded.
    pub const IMAGE_URL: u8 = 47;
    /// `u32 image, u32 req, u32 blob object`: same, from a Blob held by the host.
    pub const IMAGE_BLOB: u8 = 48;
    pub const IMAGE_DROP: u8 = 49;
    /// str
    pub const FONT: u8 = 50;
    pub const TEXT_ALIGN: u8 = 51;
    pub const TEXT_BASELINE: u8 = 52;
    /// `str text, x, y`
    pub const FILL_TEXT: u8 = 53;
    /// `x, y, w, h`: `getImageData` whose result is dropped (5.18.0 materialises CPU canvases this way).
    pub const READ_PIXELS: u8 = 54;
}

/// `lineCap` values by index.
pub const CAPS: [&str; 3] = ["butt", "round", "square"];
/// `lineJoin` values by index.
pub const JOINS: [&str; 3] = ["round", "miter", "bevel"];
/// `globalCompositeOperation` values by index.
pub const COMPOSITES: [&str; 3] = ["source-over", "multiply", "destination-out"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cap {
    Butt = 0,
    Round = 1,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Join {
    Round = 0,
    Miter = 1,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Composite {
    SourceOver = 0,
    Multiply = 1,
    DestinationOut = 2,
}

/// A canvas (element, detached element or OffscreenCanvas) held by `host/canvas.js`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Surf(pub u32);
/// A decoded `HTMLImageElement`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Img(pub u32);
/// A `Path2D`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PathId(pub u32);

/// What `drawImage` draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Src {
    Surf(Surf),
    Img(Img),
}

/// The encoder. One per app; it lives as long as the page because it owns the handle counters.
#[derive(Default)]
pub struct Cmds {
    pub w: Writer,
    surfs: u32,
    imgs: u32,
    paths: u32,
    free_paths: Vec<u32>,
    cur: u32,
}

impl Cmds {
    pub fn new() -> Cmds { Cmds::default() }
    /// The commands encoded since the last call: the payload of one `[2][REC_BATCH]` record.
    pub fn take(&mut self) -> Vec<u8> { std::mem::take(&mut self.w.buf) }
    pub fn is_empty(&self) -> bool { self.w.buf.is_empty() }

    fn o(&mut self, op: u8) { self.w.u8(op); }
    fn f(&mut self, op: u8, v: &[f64]) {
        self.w.u8(op);
        for x in v {
            self.w.f64(*x);
        }
    }
    fn s(&mut self, op: u8, v: &str) {
        self.w.u8(op);
        self.w.str(v);
    }
    fn surf(&mut self) -> Surf {
        self.surfs += 1;
        Surf(self.surfs)
    }

    // ---------------------------------------------------------------------------------------- surfaces
    /// `document.createElement('canvas')` (300 × 150 until sized).
    pub fn canvas(&mut self) -> Surf {
        let s = self.surf();
        self.o(op::CREATE);
        self.w.u32(s.0);
        self.w.u8(0);
        s
    }
    /// 5.18.0 `createProcessingCanvas(w, h)`; the caller passes sizes already rounded and clamped to ≥ 1.
    pub fn processing(&mut self, w: u32, h: u32) -> Surf {
        let s = self.surf();
        self.o(op::CREATE);
        self.w.u32(s.0);
        self.w.u8(1);
        self.w.u32(w);
        self.w.u32(h);
        s
    }
    /// A canvas element of the document (DOM handle of `ihn-app::dom`).
    pub fn bind(&mut self, node: u32) -> Surf {
        let s = self.surf();
        self.o(op::BIND);
        self.w.u32(s.0);
        self.w.u32(node);
        s
    }
    pub fn drop_surf(&mut self, s: Surf) {
        if self.cur == s.0 {
            self.cur = 0;
        }
        self.o(op::DROP);
        self.w.u32(s.0);
    }
    pub fn width(&mut self, s: Surf, px: u32) {
        self.o(op::WIDTH);
        self.w.u32(s.0);
        self.w.u32(px);
    }
    pub fn height(&mut self, s: Surf, px: u32) {
        self.o(op::HEIGHT);
        self.w.u32(s.0);
        self.w.u32(px);
    }
    /// `canvas.width = w; canvas.height = h`.
    pub fn size(&mut self, s: Surf, w: u32, h: u32) {
        self.width(s, w);
        self.height(s, h);
    }
    /// `getContext('2d')` (or `('2d', { willReadFrequently: true })`) and select.
    pub fn ctx(&mut self, s: Surf, will_read: bool) {
        self.o(op::CTX);
        self.w.u32(s.0);
        self.w.u8(will_read as u8);
        self.cur = s.0;
    }
    /// Continue on a context obtained earlier (no `getContext` call).
    pub fn select(&mut self, s: Surf) {
        if self.cur != s.0 {
            self.o(op::SELECT);
            self.w.u32(s.0);
            self.cur = s.0;
        }
    }
    /// The surface drawing commands go to (`None` before the first `ctx`).
    pub fn selected(&self) -> Option<Surf> { (self.cur != 0).then_some(Surf(self.cur)) }

    // ------------------------------------------------------------------------------------------- state
    pub fn save(&mut self) { self.o(op::SAVE) }
    pub fn restore(&mut self) { self.o(op::RESTORE) }
    pub fn set_transform(&mut self, a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) { self.f(op::SET_TRANSFORM, &[a, b, c, d, e, f]) }
    pub fn scale(&mut self, x: f64, y: f64) { self.f(op::SCALE, &[x, y]) }
    pub fn translate(&mut self, x: f64, y: f64) { self.f(op::TRANSLATE, &[x, y]) }
    pub fn rotate(&mut self, a: f64) { self.f(op::ROTATE, &[a]) }
    pub fn fill_style(&mut self, v: &str) { self.s(op::FILL_STYLE, v) }
    pub fn stroke_style(&mut self, v: &str) { self.s(op::STROKE_STYLE, v) }
    pub fn line_width(&mut self, v: f64) { self.f(op::LINE_WIDTH, &[v]) }
    pub fn line_cap(&mut self, v: Cap) {
        self.o(op::LINE_CAP);
        self.w.u8(v as u8);
    }
    pub fn line_join(&mut self, v: Join) {
        self.o(op::LINE_JOIN);
        self.w.u8(v as u8);
    }
    pub fn global_alpha(&mut self, v: f64) { self.f(op::GLOBAL_ALPHA, &[v]) }
    pub fn composite(&mut self, v: Composite) {
        self.o(op::COMPOSITE);
        self.w.u8(v as u8);
    }
    pub fn line_dash(&mut self, v: &[f64]) {
        self.o(op::LINE_DASH);
        self.w.u8(v.len() as u8);
        for x in v {
            self.w.f64(*x);
        }
    }
    pub fn filter(&mut self, v: &str) { self.s(op::FILTER, v) }
    pub fn shadow_color(&mut self, v: &str) { self.s(op::SHADOW_COLOR, v) }
    pub fn shadow_blur(&mut self, v: f64) { self.f(op::SHADOW_BLUR, &[v]) }
    /// `fillStyle = createRadialGradient(x0, y0, r0, x1, y1, r1)` with `stops`.
    pub fn radial_fill(&mut self, circles: [f64; 6], stops: &[(f64, &str)]) {
        self.f(op::RADIAL_FILL, &circles);
        self.w.u8(stops.len() as u8);
        for (at, colour) in stops {
            self.w.f64(*at);
            self.w.str(colour);
        }
    }
    pub fn font(&mut self, v: &str) { self.s(op::FONT, v) }
    pub fn text_align(&mut self, v: &str) { self.s(op::TEXT_ALIGN, v) }
    pub fn text_baseline(&mut self, v: &str) { self.s(op::TEXT_BASELINE, v) }

    // ------------------------------------------------------------------------------------------- paths
    pub fn begin_path(&mut self) { self.o(op::BEGIN_PATH) }
    pub fn move_to(&mut self, x: f64, y: f64) { self.f(op::MOVE_TO, &[x, y]) }
    pub fn line_to(&mut self, x: f64, y: f64) { self.f(op::LINE_TO, &[x, y]) }
    pub fn bezier_to(&mut self, c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64) { self.f(op::BEZIER_TO, &[c1x, c1y, c2x, c2y, x, y]) }
    pub fn quad_to(&mut self, cx: f64, cy: f64, x: f64, y: f64) { self.f(op::QUAD_TO, &[cx, cy, x, y]) }
    pub fn arc(&mut self, x: f64, y: f64, r: f64, a0: f64, a1: f64) { self.f(op::ARC, &[x, y, r, a0, a1]) }
    pub fn round_rect(&mut self, x: f64, y: f64, w: f64, h: f64, r: f64) { self.f(op::ROUND_RECT, &[x, y, w, h, r]) }
    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) { self.f(op::RECT, &[x, y, w, h]) }
    pub fn clip(&mut self) { self.o(op::CLIP) }
    pub fn stroke(&mut self) { self.o(op::STROKE) }
    pub fn fill(&mut self) { self.o(op::FILL) }
    /// `new Path2D()`: the path commands that follow build it, until [`Cmds::path_end`].
    pub fn path_begin(&mut self) -> PathId {
        let p = PathId(self.free_paths.pop().unwrap_or_else(|| {
            self.paths += 1;
            self.paths
        }));
        self.o(op::PATH_BEGIN);
        self.w.u32(p.0);
        p
    }
    pub fn path_end(&mut self) { self.o(op::PATH_END) }
    pub fn stroke_path(&mut self, p: PathId) {
        self.o(op::STROKE_PATH);
        self.w.u32(p.0);
    }
    pub fn path_drop(&mut self, p: PathId) {
        self.o(op::PATH_DROP);
        self.w.u32(p.0);
        self.free_paths.push(p.0);
    }

    // ----------------------------------------------------------------------------------------- drawing
    pub fn clear_rect(&mut self, x: f64, y: f64, w: f64, h: f64) { self.f(op::CLEAR_RECT, &[x, y, w, h]) }
    pub fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64) { self.f(op::FILL_RECT, &[x, y, w, h]) }
    pub fn stroke_rect(&mut self, x: f64, y: f64, w: f64, h: f64) { self.f(op::STROKE_RECT, &[x, y, w, h]) }
    pub fn fill_text(&mut self, text: &str, x: f64, y: f64) {
        self.s(op::FILL_TEXT, text);
        self.w.f64(x);
        self.w.f64(y);
    }
    /// `drawImage(src, …args)` with 2, 4 or 8 numbers.
    pub fn draw_image(&mut self, src: Src, args: &[f64]) {
        debug_assert!(matches!(args.len(), 2 | 4 | 8));
        self.o(op::DRAW_IMAGE);
        let (kind, handle) = match src {
            Src::Surf(s) => (0, s.0),
            Src::Img(i) => (1, i.0),
        };
        self.w.u8(kind);
        self.w.u32(handle);
        self.w.u8(args.len() as u8);
        for x in args {
            self.w.f64(*x);
        }
    }
    pub fn read_pixels(&mut self, x: f64, y: f64, w: f64, h: f64) { self.f(op::READ_PIXELS, &[x, y, w, h]) }

    // ------------------------------------------------------------------------------------ async results
    pub fn to_data_url(&mut self, s: Surf, req: u32, mime: &str, quality: f64) { self.encode(op::TO_DATA_URL, s, req, mime, quality) }
    pub fn to_blob(&mut self, s: Surf, req: u32, mime: &str, quality: f64) { self.encode(op::TO_BLOB, s, req, mime, quality) }
    fn encode(&mut self, op: u8, s: Surf, req: u32, mime: &str, quality: f64) {
        self.o(op);
        self.w.u32(s.0);
        self.w.u32(req);
        self.w.str(mime);
        self.w.f64(quality);
    }
    fn img(&mut self) -> Img {
        self.imgs += 1;
        Img(self.imgs)
    }
    /// `new Image(); img.src = url`. The image may be drawn once `Io(req, 0)` arrived (5.18.0 `img.complete`).
    pub fn image_url(&mut self, req: u32, url: &str) -> Img {
        let i = self.img();
        self.o(op::IMAGE_URL);
        self.w.u32(i.0);
        self.w.u32(req);
        self.w.str(url);
        i
    }
    /// The same from a Blob object handle (an asset).
    pub fn image_blob(&mut self, req: u32, blob: u32) -> Img {
        let i = self.img();
        self.o(op::IMAGE_BLOB);
        self.w.u32(i.0);
        self.w.u32(req);
        self.w.u32(blob);
        i
    }
    pub fn image_drop(&mut self, i: Img) {
        self.o(op::IMAGE_DROP);
        self.w.u32(i.0);
    }
}
