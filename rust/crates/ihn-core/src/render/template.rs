//! Page templates: the bullet-journal template (5.18.0 `bullet-template.js`), the agenda/diary SVG templates, and
//! the three caches 5.18.0 keeps around them (`_tplCache`, `_tplThumbCache`, `dotStamps`).

use super::cmd::{Cap, Cmds, Composite, Img, Join, Src, Surf};
use super::TAU;
use crate::geom::{js_max, js_min};
use crate::jsmath;
use crate::op::Template;

/// URLs of the SVG templates (bytes = 5.18.0 `_AGENDA_SVG_B64` / `_DIARY_SVG_B64` decoded).
pub const AGENDA_SVG: &str = "icons/agenda.svg";
pub const DIARY_SVG: &str = "icons/diary.svg";
/// v5: `TEMPLATE_BG_QUALITY`.
pub const BG_QUALITY: f64 = 0.86;

/// v5: `getTemplateKindDefaults(kind)` — A5 for agenda and diary, A4 otherwise.
pub fn default_size(kind: Template) -> (f64, f64) {
    match kind {
        Template::Default => (794.0, 1123.0),
        _ => (559.0, 794.0),
    }
}

/// v5: `getTargetTemplateBackgroundScale(zoom)` (app-v5.js:2179).
pub fn bg_scale(zoom: f64, is_pc: bool) -> f64 {
    let (base, max) = if is_pc { (3.6, 4.8) } else { (3.0, 4.2) };
    let safe_zoom = if zoom.is_finite() { js_max(1.0, zoom) } else { 1.0 };
    js_max(base, js_min(max, base * (1.0 + (safe_zoom - 1.0) * 0.35)))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Svg {
    /// `_getSvgImage` was never called.
    Unrequested,
    Loading(Img),
    Ready(Img),
}

struct Thumb {
    kind: Template,
    size: (u32, u32),
    thumb: (u32, u32),
    source: Surf,
    canvas: Surf,
}

/// Template canvases. One per app.
pub struct Templates {
    /// Request ids of the two SVG image loads; the app routes their `Io` to [`Templates::svg_loaded`].
    pub svg_req: [u32; 2],
    svg: [Svg; 2],
    /// v5 `_tplCache`: insertion order, at most 6.
    cache: Vec<(Template, u32, u32, Surf)>,
    /// v5 `_tplThumbCache`: at most 6.
    thumbs: Vec<Thumb>,
    /// v5 `dotStamps`: `u` bits → (canvas, centre); at most 4.
    stamps: Vec<(u64, Surf, f64)>,
}

impl Templates {
    pub fn new(agenda_req: u32, diary_req: u32) -> Templates {
        Templates { svg_req: [agenda_req, diary_req], svg: [Svg::Unrequested; 2], cache: Vec::new(), thumbs: Vec::new(), stamps: Vec::new() }
    }

    /// v5: `dotStamp(u)` (bullet-template.js:10).
    fn stamp(&mut self, c: &mut Cmds, u: f64) -> (Surf, f64) {
        if let Some(&(_, s, centre)) = self.stamps.iter().find(|e| e.0 == u.to_bits()) {
            return (s, centre);
        }
        let (dot_r, blur) = (0.035 * u, 0.02 * u);
        let stamp_r = dot_r + blur;
        let size = (stamp_r * 2.0).ceil() + 2.0;
        let s = c.canvas();
        c.size(s, size as u32, size as u32);
        c.ctx(s, false);
        let centre = size / 2.0;
        c.radial_fill([centre, centre, 0.0, centre, centre, stamp_r], &[(0.0, "#a8a8a8"), (js_max(0.0, (dot_r - blur) / stamp_r), "#a8a8a8"), (1.0, "rgba(168,168,168,0)")]);
        c.begin_path();
        c.arc(centre, centre, stamp_r, 0.0, TAU);
        c.fill();
        if self.stamps.len() >= 4 {
            c.drop_surf(self.stamps.remove(0).1);
        }
        self.stamps.push((u.to_bits(), s, centre));
        (s, centre)
    }

    /// v5: `_renderTemplateToCanvas(w, h)` = a new canvas + `InhouseBulletTemplate.render` (bullet-template.js:37).
    pub fn bullet(&mut self, c: &mut Cmds, w: u32, h: u32) -> Surf {
        let s = c.canvas();
        c.size(s, w, h);
        c.ctx(s, false);
        let (width, height) = (w as f64, h as f64);
        c.save();
        c.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
        c.global_alpha(1.0);
        c.composite(Composite::SourceOver);
        c.filter("none");
        c.shadow_color("transparent");
        c.shadow_blur(0.0);
        c.line_cap(Cap::Butt);
        c.line_join(Join::Miter);
        c.line_dash(&[]);
        c.fill_style("white");
        c.fill_rect(0.0, 0.0, width, height);

        let u = width / 21.0;
        let corner = 0.03 * u;
        c.stroke_style("#f0db4c");
        c.line_width(0.06 * u);
        c.begin_path();
        c.round_rect(1.5 * u, 1.43 * u, 18.0 * u, 27.0 * u, corner);
        c.stroke();

        let (stamp, centre) = self.stamp(c, u);
        c.select(s);
        for col in 0..35 {
            let cx = (2.0 + col as f64 * 0.5) * u;
            for row in 0..51 {
                let cy = (1.93 + row as f64 * 0.5) * u;
                c.draw_image(Src::Surf(stamp), &[cx - centre, cy - centre]);
            }
        }

        let (line_y, line_h) = (27.43 * u, 0.5 * u);
        c.begin_path();
        c.round_rect(2.0 * u, line_y, 8.0 * u, line_h, corner);
        c.stroke();
        c.begin_path();
        c.round_rect(11.0 * u, line_y, 8.0 * u, line_h, corner);
        c.stroke();
        c.begin_path();
        for start in [2.5, 11.5] {
            let mut x: f64 = start;
            while x <= start + 7.0 + 1e-9 {
                c.move_to(x * u, line_y);
                c.line_to(x * u, line_y + line_h);
                x += 0.5;
            }
        }
        c.stroke();

        let (circle_y, circle_r) = (27.68 * u, 0.125 * u);
        let mut circle_x = 10.125 * u;
        for colour in ["#ff0000", "#000000", "#0000ff", "#6eff12"] {
            c.fill_style(colour);
            c.begin_path();
            c.arc(circle_x, circle_y, circle_r, 0.0, TAU);
            c.fill();
            c.stroke();
            circle_x += 0.25 * u;
        }
        c.restore();
        s
    }

    fn svg_index(kind: Template) -> usize { (kind == Template::Diary) as usize }

    /// The cached canvas of a template at a pixel size, if any (an export encodes it).
    pub fn cached(&self, kind: Template, w: u32, h: u32) -> Option<Surf> { self.cache.iter().find(|e| (e.0, e.1, e.2) == (kind, w, h)).map(|e| e.3) }

    /// v5: `_getSvgImage(kind)` — starts the load on first use. `Some` once it may be drawn.
    pub fn svg_image(&mut self, c: &mut Cmds, kind: Template) -> Option<Img> {
        let i = Self::svg_index(kind);
        match self.svg[i] {
            Svg::Ready(img) => Some(img),
            Svg::Loading(_) => None,
            Svg::Unrequested => {
                self.svg[i] = Svg::Loading(c.image_url(self.svg_req[i], if i == 0 { AGENDA_SVG } else { DIARY_SVG }));
                None
            }
        }
    }

    /// The `Io` of an SVG load arrived (v5: `img.onload` of `_getSvgImage`): the template canvases are flushed.
    /// The caller then clears its template URLs ([`BgUrls::clear`]), warms them again and updates the page
    /// wrappers, as 5.18.0 does. Returns false for a request that is not ours or a failed load.
    pub fn svg_loaded(&mut self, c: &mut Cmds, req: u32, ok: bool) -> bool {
        let Some(i) = self.svg_req.iter().position(|r| *r == req) else { return false };
        let Svg::Loading(img) = self.svg[i] else { return false };
        if !ok {
            return false;
        }
        self.svg[i] = Svg::Ready(img);
        for (_, _, _, s) in self.cache.drain(..) {
            c.drop_surf(s);
        }
        true
    }

    /// v5: `_renderSvgTemplateToCanvas(kind, w, h)` (app-v5.js:193).
    pub fn svg(&mut self, c: &mut Cmds, kind: Template, w: u32, h: u32) -> Surf {
        let s = c.canvas();
        c.size(s, w, h);
        c.ctx(s, false);
        c.fill_style("#ffffff");
        c.fill_rect(0.0, 0.0, w as f64, h as f64);
        if let Some(img) = self.svg_image(c, kind) {
            c.draw_image(Src::Img(img), &[0.0, 0.0, w as f64, h as f64]);
        }
        s
    }

    /// The cached template canvas of `kind` at `w × h` pixels, rendering it on a miss (v5 `_tplCache`).
    fn canvas(&mut self, c: &mut Cmds, kind: Template, w: u32, h: u32) -> Surf {
        if let Some(e) = self.cache.iter().find(|e| (e.0, e.1, e.2) == (kind, w, h)) {
            return e.3;
        }
        let s = if kind == Template::Default { self.bullet(c, w, h) } else { self.svg(c, kind, w, h) };
        if self.cache.len() >= 6 {
            c.drop_surf(self.cache.remove(0).3);
        }
        self.cache.push((kind, w, h, s));
        s
    }

    /// v5: `drawTemplateBackground(ctx, width, height, 1, kind)` (app-v5.js:211): the template at `w × h`
    /// pixels, drawn at the origin of `dest` (`None`: only make sure it is cached).
    pub fn draw(&mut self, c: &mut Cmds, dest: Option<Surf>, w: u32, h: u32, kind: Template) {
        let cached = self.canvas(c, kind, w, h);
        if let Some(dest) = dest {
            c.select(dest);
            c.draw_image(Src::Surf(cached), &[0.0, 0.0]);
        }
    }

    /// v5: `drawTemplateBackgroundThumbnail(ctx, width, height, kind, pixelWidth, pixelHeight)` (app-v5.js:233):
    /// a CPU copy of the template reduced to the thumbnail's pixel size, drawn over `width × height`.
    pub fn draw_thumbnail(&mut self, c: &mut Cmds, dest: Surf, width: f64, height: f64, kind: Template, pixel_w: f64, pixel_h: f64) {
        let size = (width as u32, height as u32);
        let source = self.canvas(c, kind, size.0, size.1);
        let thumb = (js_max(1.0, jsmath::round(pixel_w)) as u32, js_max(1.0, jsmath::round(pixel_h)) as u32);
        let found = self.thumbs.iter().position(|t| (t.kind, t.size, t.thumb) == (kind, size, thumb));
        let canvas = match found {
            Some(i) if self.thumbs[i].source == source => self.thumbs[i].canvas,
            _ => {
                let canvas = c.canvas();
                c.size(canvas, thumb.0, thumb.1);
                c.ctx(canvas, true);
                c.draw_image(Src::Surf(source), &[0.0, 0.0, thumb.0 as f64, thumb.1 as f64]);
                c.read_pixels(0.0, 0.0, 1.0, 1.0);
                if let Some(i) = found {
                    c.drop_surf(self.thumbs.remove(i).canvas);
                }
                if self.thumbs.len() >= 6 {
                    c.drop_surf(self.thumbs.remove(0).canvas);
                }
                self.thumbs.push(Thumb { kind, size, thumb, source, canvas });
                canvas
            }
        };
        c.select(dest);
        c.draw_image(Src::Surf(canvas), &[0.0, 0.0, width, height]);
    }

    /// The canvas part of v5 `ensureTemplateBackgroundUrl(kind)` (app-v5.js:2192): the template at `scale`,
    /// encoded as JPEG 0.86. `Io(req)` carries the data URL (the CSS background of page wrappers and canvases).
    pub fn bg_url(&mut self, c: &mut Cmds, kind: Template, scale: f64, req: u32) {
        let (base_w, base_h) = default_size(kind);
        let (w, h) = (js_max(1.0, jsmath::round(base_w * scale)) as u32, js_max(1.0, jsmath::round(base_h * scale)) as u32);
        let canvas = c.canvas();
        c.size(canvas, w, h);
        c.ctx(canvas, false);
        self.draw(c, Some(canvas), w, h, kind);
        c.to_data_url(canvas, req, "image/jpeg", BG_QUALITY);
        c.drop_surf(canvas);
    }
}

/// v5 `_templateBgUrlCache` + `_templateBgScaleCache`: which template URLs exist and at what scale. The URL text
/// itself arrives asynchronously and is kept by the caller.
#[derive(Default)]
pub struct BgUrls {
    scales: [f64; 3],
}

impl BgUrls {
    /// v5: the cache test of `ensureTemplateBackgroundUrl` — `Some(scale)` when a new URL must be rendered at
    /// `scale` (and is now recorded), `None` when the cached one is sharp enough.
    pub fn need(&mut self, kind: Template, zoom: f64, is_pc: bool) -> Option<f64> {
        let target = bg_scale(zoom, is_pc);
        let slot = &mut self.scales[kind as usize];
        if *slot != 0.0 && *slot >= target - 0.01 {
            return None;
        }
        *slot = target;
        Some(target)
    }
    /// v5: `clearTemplateBackgroundUrlCache()`.
    pub fn clear(&mut self) { self.scales = [0.0; 3] }
}
