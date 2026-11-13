//! Thumbnails: the preview a released page keeps, the scroll-preview queue's picture and the Pages panel.
//! All are CPU canvases (`willReadFrequently`) encoded with `toDataURL`, as 5.18.0.

use super::cmd::{Cmds, Img, Src, Surf};
use super::page::{self, Bg, PageView};
use super::template::Templates;
use super::{pen, PathCache};
use crate::geom::{js_max, js_min};

/// v5: `PAGE_PREVIEW_SCALE`.
pub const PAGE_PREVIEW_SCALE: f64 = 0.18;

/// v5: `renderPagePreview(pageIndex)` (app-v5.js:20034): `Io(req)` carries the JPEG 0.6 data URL. Returns false
/// (nothing emitted) for a PDF page whose raster is not decoded, where 5.18.0 returns null.
pub fn page_preview(c: &mut Cmds, tpl: &mut Templates, paths: &mut PathCache, view: &PageView, req: u32) -> bool {
    if matches!(view.bg, Bg::PdfPending) {
        return false;
    }
    let scale = PAGE_PREVIEW_SCALE;
    let canvas = c.canvas();
    c.size(canvas, js_max(1.0, (view.w * scale).floor()) as u32, js_max(1.0, (view.h * scale).floor()) as u32);
    c.ctx(canvas, true);
    c.scale(scale, scale);
    c.fill_style("white");
    c.fill_rect(0.0, 0.0, view.w, view.h);
    match view.bg {
        Bg::Image(Some(img)) => c.draw_image(Src::Img(img), &[0.0, 0.0, view.w, view.h]),
        Bg::Template(kind) => tpl.draw_thumbnail(c, canvas, view.w, view.h, kind, view.w * scale, view.h * scale),
        _ => {}
    }
    page::images(c, &view.items.images);
    page::strokes(c, paths, &view.items.strokes);
    c.to_data_url(canvas, req, "image/jpeg", 0.6);
    c.drop_surf(canvas);
    true
}

/// v5: `processScrollPagePreview()` (app-v5.js:19972), the canvas side. The caller keeps the queue, the validity
/// checks between steps, the pdf.js render of a PDF page without raster (into [`ScrollPreview::surf`], between
/// `begin` and `background`) and the slice length.
pub struct ScrollPreview {
    pub surf: Surf,
    pub scale: f64,
    /// Backing size: the pdf.js viewport is fitted into it.
    pub px: (u32, u32),
    index: usize,
}

impl ScrollPreview {
    /// The white canvas of `min(0.25, 320 / w, 440 / h)` times the page.
    pub fn begin(c: &mut Cmds, w: f64, h: f64) -> ScrollPreview {
        let scale = js_min(js_min(0.25, 320.0 / w), 440.0 / h);
        let px = ((w * scale).ceil() as u32, (h * scale).ceil() as u32);
        let surf = c.canvas();
        c.size(surf, px.0, px.1);
        c.ctx(surf, true);
        c.fill_style("#fff");
        c.fill_rect(0.0, 0.0, px.0 as f64, px.1 as f64);
        ScrollPreview { surf, scale, px, index: 0 }
    }
    /// Background, legacy covers and images, once every image of the page is decoded.
    pub fn background(&mut self, c: &mut Cmds, tpl: &mut Templates, paths: &mut PathCache, view: &PageView) {
        c.select(self.surf);
        c.set_transform(self.scale, 0.0, 0.0, self.scale, 0.0, 0.0);
        match view.bg {
            Bg::Image(Some(img)) => c.draw_image(Src::Img(img), &[0.0, 0.0, view.w, view.h]),
            Bg::Template(kind) => tpl.draw_thumbnail(c, self.surf, view.w, view.h, kind, view.w * self.scale, view.h * self.scale),
            _ => {}
        }
        pen::covers(c, paths, &view.items.covers);
        page::images(c, &view.items.images);
    }
    /// Up to `limit` more strokes. True when all are drawn.
    pub fn strokes(&mut self, c: &mut Cmds, paths: &mut PathCache, view: &PageView, limit: usize) -> bool {
        c.select(self.surf);
        let run = &view.items.strokes;
        let end = run.len().min(self.index.saturating_add(limit));
        page::strokes(c, paths, &run[self.index..end]);
        self.index = end;
        end == run.len()
    }
    /// `Io(req)` carries the JPEG 0.65 data URL; the canvas is released.
    pub fn finish(self, c: &mut Cmds, req: u32) {
        c.to_data_url(self.surf, req, "image/jpeg", 0.65);
        self.cancel(c);
    }
    /// v5: the `finally` of the task — `canvas.width = 0; canvas.height = 0`.
    pub fn cancel(self, c: &mut Cmds) {
        c.size(self.surf, 0, 0);
        c.drop_surf(self.surf);
    }
}

/// What a Pages-panel thumbnail shows under the ink.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ThumbBg {
    /// The page's template at thumbnail size.
    Template(crate::op::Template),
    /// The page's background image, decoded (v5 draws it in the image's `onload`).
    Image(Img),
    /// A PDF page without raster: the grey `PDF` placeholder.
    PdfPlaceholder,
    /// A page shown by its stored preview only (v5: `page.unloaded && page.preview`): no ink is drawn.
    Preview(Img),
    /// A background or preview image that is not decoded yet: white only; call again when it is.
    Waiting,
}

/// v5: `generateThumbnail(pageIndex)` + `drawThumbnailStrokes` (app-v5.js:17741, 17790) on `canvas#thumb-<i>` of
/// `tw × th` pixels.
pub fn panel(c: &mut Cmds, tpl: &mut Templates, paths: &mut PathCache, thumb: Surf, tw: u32, th: u32, bg: ThumbBg, view: &PageView) {
    let (w, h) = (tw as f64, th as f64);
    c.ctx(thumb, false);
    c.fill_style("white");
    c.fill_rect(0.0, 0.0, w, h);
    match bg {
        ThumbBg::Waiting => return,
        ThumbBg::Preview(img) => {
            c.draw_image(Src::Img(img), &[0.0, 0.0, w, h]);
            return;
        }
        ThumbBg::Image(img) => c.draw_image(Src::Img(img), &[0.0, 0.0, w, h]),
        ThumbBg::PdfPlaceholder => {
            let small = js_min(w, h);
            let inset = js_max(2.0, crate::jsmath::round(small * 0.04));
            c.fill_style("#f2f2f2");
            c.fill_rect(inset, inset, w - inset * 2.0, h - inset * 2.0);
            c.stroke_style("#d0d0d0");
            c.stroke_rect(inset, inset, w - inset * 2.0, h - inset * 2.0);
            c.fill_style("#9a9a9a");
            c.font(&format!("bold {}px sans-serif", crate::jsfmt::num(js_max(9.0, crate::jsmath::round(small * 0.18)))));
            c.text_align("center");
            c.text_baseline("middle");
            c.fill_text("PDF", w / 2.0, h / 2.0);
        }
        ThumbBg::Template(kind) => tpl.draw(c, Some(thumb), tw, th, kind),
    }
    c.save();
    c.scale(w / view.w, h / view.h);
    page::images(c, &view.items.images);
    page::strokes(c, paths, &view.items.strokes);
    c.restore();
}
