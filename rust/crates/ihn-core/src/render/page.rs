//! Page bitmap repaints: background layer, synchronous repaint and the staged repaint of dense pages.

use super::cmd::{Cmds, Composite, Img, Src, Surf};
use super::template::Templates;
use super::{pen, ImageRef, PageItems, PathCache, StrokeRef};
use crate::geom::js_max;
use crate::jsmath;
use crate::op::Template;

/// What lies under the ink of a page (v5 `backgroundSource` + `backgroundImage`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Bg {
    /// The page shows its template.
    Template(Template),
    /// A photo page or a rendered PDF page; `None` while the image is not decoded.
    Image(Option<Img>),
    /// A PDF page whose raster is not there yet: nothing is drawn (the caller queues the pdf.js render).
    PdfPending,
}

/// One page for a repaint: logical size, background and its draw runs.
pub struct PageView<'a> {
    pub w: f64,
    pub h: f64,
    pub bg: Bg,
    pub items: &'a PageItems<'a>,
}

/// v5: `drawImagesOnContext(ctx, page)` (app-v5.js:12759).
pub fn images(c: &mut Cmds, images: &[ImageRef]) {
    for image in images {
        let Some(img) = image.img else { continue };
        c.save();
        c.translate(image.cx, image.cy);
        c.rotate(super::or(image.rotation, 0.0));
        c.draw_image(Src::Img(img), &[-image.w / 2.0, -image.h / 2.0, image.w, image.h]);
        c.restore();
    }
}

/// The strokes run of a repaint, thumbnail or preview: everything drawable, in order.
pub fn strokes(c: &mut Cmds, paths: &mut PathCache, strokes: &[StrokeRef]) {
    for s in strokes.iter().filter(|s| s.drawable()) {
        pen::draw(c, paths, s);
    }
}

/// v5: `drawPageBackgroundLayer(ctx, page, pageIndex, renderScale)` (app-v5.js:12799). `dest` is selected.
pub fn background(c: &mut Cmds, tpl: &mut Templates, dest: Surf, view: &PageView, render_scale: f64) {
    match view.bg {
        Bg::Template(kind) => {
            let scale = if render_scale.is_finite() { js_max(1.0, render_scale) } else { 1.0 };
            let (w, h) = (js_max(1.0, jsmath::round(view.w * scale)), js_max(1.0, jsmath::round(view.h * scale)));
            // The template is drawn in backing pixels, so zoom adds real template detail.
            c.save();
            c.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
            tpl.draw(c, Some(dest), w as u32, h as u32, kind);
            c.restore();
        }
        Bg::PdfPending => {}
        Bg::Image(img) => {
            c.fill_style("white");
            c.fill_rect(0.0, 0.0, view.w, view.h);
            if let Some(img) = img {
                c.draw_image(Src::Img(img), &[0.0, 0.0, view.w, view.h]);
            }
        }
    }
}

/// v5: `staged` test of `redrawPage`: more than 128 strokes, or one with more than 2048 points.
pub fn is_staged(strokes: &[StrokeRef]) -> bool { strokes.len() > 128 || strokes.iter().any(|s| s.len() > 2048) }

/// The common head of both repaints (app-v5.js:12944-12951).
fn head(c: &mut Cmds, tpl: &mut Templates, target: Surf, px: (u32, u32), view: &PageView, render_scale: f64) {
    c.ctx(target, false);
    c.save();
    c.set_transform(px.0 as f64 / view.w, 0.0, 0.0, px.1 as f64 / view.h, 0.0, 0.0);
    c.clear_rect(0.0, 0.0, view.w, view.h);
    c.composite(Composite::SourceOver);
    c.global_alpha(1.0);
    background(c, tpl, target, view, render_scale);
}

/// v5: `redrawPage(pageIndex)`, synchronous branch (app-v5.js:12899). `px` is the backing size of `canvas`,
/// `render_scale` its `dataset.renderScale`.
pub fn repaint(c: &mut Cmds, tpl: &mut Templates, paths: &mut PathCache, canvas: Surf, px: (u32, u32), view: &PageView, render_scale: f64) {
    head(c, tpl, canvas, px, view, render_scale);
    pen::covers(c, paths, &view.items.covers);
    images(c, &view.items.images);
    strokes(c, paths, &view.items.strokes);
    c.restore();
}

/// v5: the staged branch of `redrawPage`: the page is painted on a processing canvas in slices and copied to
/// the visible canvas in one `drawImage`. The caller owns what 5.18.0 keeps in the job: the generation guard,
/// the pen/pointer pauses and the slice length (commands are replayed by the host after the Rust call returns,
/// so the caller bounds the number of strokes per slice instead of measuring 4 ms here).
pub struct Staged {
    pub target: Surf,
    px: (u32, u32),
    phase: u8,
    index: usize,
}

impl Staged {
    /// Creates the processing canvas and paints the background layer.
    pub fn begin(c: &mut Cmds, tpl: &mut Templates, px: (u32, u32), view: &PageView, render_scale: f64) -> Staged {
        let target = c.processing(px.0.max(1), px.1.max(1));
        head(c, tpl, target, px, view, render_scale);
        Staged { target, px, phase: 0, index: 0 }
    }
    /// Draws up to `limit` more covers/strokes of the same `view`. True when the page is complete.
    pub fn paint(&mut self, c: &mut Cmds, paths: &mut PathCache, view: &PageView, limit: usize) -> bool {
        c.select(self.target);
        let mut left = limit;
        while self.phase < 2 {
            let run = if self.phase == 0 { &view.items.covers } else { &view.items.strokes };
            while self.index < run.len() {
                if left == 0 {
                    return false;
                }
                let s = &run[self.index];
                self.index += 1;
                left -= 1;
                if self.phase == 0 {
                    pen::covers(c, paths, std::slice::from_ref(s));
                } else {
                    pen::draw(c, paths, s);
                }
            }
            if self.phase == 0 {
                images(c, &view.items.images);
            }
            self.phase += 1;
            self.index = 0;
        }
        true
    }
    /// The atomic copy to the visible canvas, then the processing canvas is released.
    pub fn blit(self, c: &mut Cmds, canvas: Surf) {
        c.ctx(canvas, false);
        c.save();
        c.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
        c.clear_rect(0.0, 0.0, self.px.0 as f64, self.px.1 as f64);
        c.draw_image(Src::Surf(self.target), &[0.0, 0.0]);
        c.restore();
        self.discard(c);
    }
    /// v5: `discard()` — `target.width = target.height = 0`.
    pub fn discard(self, c: &mut Cmds) {
        c.height(self.target, 0);
        c.width(self.target, 0);
        c.drop_surf(self.target);
    }
}

/// v5: `resizePageCanvas(canvas, …)` when the size changes (app-v5.js:2265): the last complete frame is kept
/// visible, scaled, while the sharper paint is prepared. `old` is the current backing size and whether the
/// canvas holds a complete paint (`dataset.hasPageDraw`).
pub fn resize(c: &mut Cmds, canvas: Surf, old: Option<(u32, u32)>, new: (u32, u32)) {
    let previous = old.map(|(w, h)| {
        let p = c.processing(w.max(1), h.max(1));
        c.ctx(p, false);
        c.draw_image(Src::Surf(canvas), &[0.0, 0.0]);
        p
    });
    c.size(canvas, new.0, new.1);
    if let Some(p) = previous {
        c.ctx(canvas, false);
        c.draw_image(Src::Surf(p), &[0.0, 0.0, new.0 as f64, new.1 as f64]);
        c.height(p, 0);
        c.width(p, 0);
        c.drop_surf(p);
    }
}

/// v5: the context defaults `applyPageDimensionsToWrapper` sets on a page canvas (app-v5.js:2393).
pub fn context_defaults(c: &mut Cmds, canvas: Surf, scale: f64) {
    c.ctx(canvas, false);
    c.set_transform(scale, 0.0, 0.0, scale, 0.0, 0.0);
    c.line_cap(super::cmd::Cap::Round);
    c.line_join(super::cmd::Join::Round);
}
