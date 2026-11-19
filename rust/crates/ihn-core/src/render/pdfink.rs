//! The ink of one exported page as a PDF content stream (ARCHITECTURE §8.2): the body of the `/IHN6Ink` Form
//! XObject. Same geometry as the screen: pen = the Catmull–Rom Béziers of [`super::pen`], highlighter = the two
//! multiply passes, images in their boxes. Order: legacy covers, images, strokes.
//!
//! Coordinates are page pixels with y down; the stream starts with the flip `1 0 0 -1 0 <page height> cm`, so the
//! form's `/BBox` is `[0 0 w h]` and its `/Matrix` maps page pixels to the PDF page. Resources the form needs:
//! `/ExtGState` [`GS`] and one image XObject `/Im<k>` per entry of [`InkStream::images`].

use super::cmd::{op, Cmds};
use super::{highlighter, or, pen, PageItems, StrokeRef};
use crate::codec::Reader;
use crate::geom::js_max;
use crate::jsmath;
use crate::op::{Color, Tool};
use std::fmt::Write;

/// `/ExtGState` entries: name, opacity (`/CA` and `/ca`), blend mode.
pub const GS: [(&str, f64, &str); 4] = [("GSn", 1.0, "Normal"), ("GSh1", 0.22, "Multiply"), ("GSh2", 0.1, "Multiply"), ("GSh3", 0.24, "Multiply")];

pub struct InkStream {
    pub content: String,
    /// `images[k]` = index into `PageItems::images` drawn as `/Im<k>`.
    pub images: Vec<usize>,
}

/// A coordinate: at most 4 decimals, no exponent, no trailing zeros (1/10,000 page px ≈ 1/13,000 pt).
fn num(out: &mut String, v: f64) {
    let scaled = jsmath::round(v * 1e4);
    if scaled == 0.0 {
        out.push('0');
        return;
    }
    let (neg, n) = (scaled < 0.0, scaled.abs() as u64);
    if neg {
        out.push('-');
    }
    let _ = write!(out, "{}", n / 10_000);
    let frac = n % 10_000;
    if frac != 0 {
        let digits = format!("{frac:04}");
        out.push('.');
        out.push_str(digits.trim_end_matches('0'));
    }
}

fn nums(out: &mut String, v: &[f64], operator: &str) {
    for x in v {
        num(out, *x);
        out.push(' ');
    }
    out.push_str(operator);
    out.push('\n');
}

/// `r g b` in 0..1; anything that is not `#rgb`/`#rrggbb` is black.
fn rgb(color: &Color) -> [f64; 3] {
    let v = match color {
        Color::Hex(v) => *v,
        Color::Css(s) => match s.strip_prefix('#').filter(|h| h.len() == 3).and_then(|h| u32::from_str_radix(h, 16).ok()) {
            Some(v) => (v >> 8 & 15) * 0x11_0000 | (v >> 4 & 15) * 0x1100 | (v & 15) * 0x11,
            None => 0,
        },
    };
    [(v >> 16 & 255) as f64 / 255.0, (v >> 8 & 255) as f64 / 255.0, (v & 255) as f64 / 255.0]
}

/// Replays path commands (`move/line/bezier/quad`) as PDF operators; a quadratic becomes the equal cubic.
fn path_ops(out: &mut String, cmds: &[u8]) {
    let mut r = Reader::new(cmds);
    let (mut x, mut y) = (0.0, 0.0);
    while let Some(code) = r.u8() {
        let n = match code {
            op::MOVE_TO | op::LINE_TO => 2,
            op::QUAD_TO => 4,
            op::BEZIER_TO => 6,
            _ => 0,
        };
        let mut a = [0.0; 6];
        for v in a.iter_mut().take(n) {
            *v = r.f64().unwrap_or(0.0);
        }
        match code {
            op::MOVE_TO => nums(out, &a[..2], "m"),
            op::LINE_TO => nums(out, &a[..2], "l"),
            op::BEZIER_TO => nums(out, &a, "c"),
            op::QUAD_TO => nums(out, &[x + (a[0] - x) * 2.0 / 3.0, y + (a[1] - y) * 2.0 / 3.0, a[2] + (a[0] - a[2]) * 2.0 / 3.0, a[3] + (a[1] - a[3]) * 2.0 / 3.0, a[2], a[3]], "c"),
            _ => {}
        }
        (x, y) = (a[n.max(2) - 2], a[n.max(2) - 1]);
    }
}

/// A full circle as four Béziers.
fn circle(out: &mut String, cx: f64, cy: f64, r: f64) {
    let k = 0.552_284_749_830_793_6 * r;
    nums(out, &[cx + r, cy], "m");
    nums(out, &[cx + r, cy + k, cx + k, cy + r, cx, cy + r], "c");
    nums(out, &[cx - k, cy + r, cx - r, cy + k, cx - r, cy], "c");
    nums(out, &[cx - r, cy - k, cx - k, cy - r, cx, cy - r], "c");
    nums(out, &[cx + k, cy - r, cx + r, cy - k, cx + r, cy], "c");
}

fn stroke(out: &mut String, scratch: &mut Cmds, s: &StrokeRef, color: [f64; 3], width: f64) {
    if s.is_empty() || s.xy.iter().any(|v| !v.is_finite()) || !width.is_finite() {
        return;
    }
    if s.tool == Tool::Highlighter {
        let (wide, inner) = highlighter::widths(s.width);
        nums(out, &color, "RG");
        if s.len() == 1 {
            out.push_str("/GSh3 gs\n");
            nums(out, &color, "rg");
            circle(out, s.xy[0], s.xy[1], wide * 0.52);
            out.push_str("f\n");
            return;
        }
        scratch.take();
        highlighter::smooth_path(scratch, s.xy);
        // Skip the `beginPath` byte: the rest is move/quad/line commands.
        let cmds = scratch.take();
        for (gs, w) in [("GSh1", wide), ("GSh2", inner)] {
            let _ = writeln!(out, "/{gs} gs");
            nums(out, &[w], "w");
            path_ops(out, &cmds[1..]);
            out.push_str("S\n");
        }
        return;
    }
    out.push_str("/GSn gs\n");
    if s.len() == 1 {
        // BEHAVIOUR-CHANGES B-E3: the single-tap dot.
        nums(out, &color, "rg");
        circle(out, s.xy[0], s.xy[1], js_max(0.7, width * 0.5));
        out.push_str("f\n");
        return;
    }
    nums(out, &color, "RG");
    nums(out, &[width], "w");
    scratch.take();
    pen::path(scratch, s.xy);
    path_ops(out, &scratch.take());
    out.push_str("S\n");
}

/// The content stream of a page's ink. `None` when the page has nothing to draw (no stream is appended).
/// Images whose asset is unknown to the exporter must be filtered out by the caller beforehand; every entry of
/// `items.images` is drawn (`img` is not consulted).
pub fn content_stream(page_h: f64, items: &PageItems) -> Option<InkStream> {
    if items.covers.is_empty() && items.images.is_empty() && items.strokes.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(64 + items.strokes.iter().map(|s| s.xy.len() * 22).sum::<usize>());
    let mut images = Vec::new();
    let mut scratch = Cmds::new();
    out.push_str("q\n");
    nums(&mut out, &[1.0, 0.0, 0.0, -1.0, 0.0, page_h], "cm");
    out.push_str("1 J 1 j\n");
    for s in items.covers.iter().filter(|s| s.len() >= 2) {
        stroke(&mut out, &mut scratch, &StrokeRef { tool: Tool::Pen, ..*s }, [1.0; 3], js_max(1.0, or(s.width, 1.0) * 1.35));
    }
    for (index, image) in items.images.iter().enumerate() {
        let (sin, cos) = match or(image.rotation, 0.0) {
            r if r == 0.0 => (0.0, 1.0),
            r => (jsmath::sin(r), jsmath::cos(r)),
        };
        if ![image.cx, image.cy, image.w, image.h, sin, cos].iter().all(|v| v.is_finite()) {
            continue;
        }
        // translate(cx, cy) · rotate · translate(-w/2, h/2) · scale(w, -h): the unit image square, upright.
        let (w, h) = (image.w, image.h);
        let (e, f) = (image.cx + cos * (-w / 2.0) - sin * (h / 2.0), image.cy + sin * (-w / 2.0) + cos * (h / 2.0));
        out.push_str("q\n/GSn gs\n");
        nums(&mut out, &[cos * w, sin * w, sin * h, -cos * h, e, f], "cm");
        let _ = writeln!(out, "/Im{} Do\nQ", images.len());
        images.push(index);
    }
    for s in &items.strokes {
        if s.tool == Tool::LegacyEraseArea {
            continue; // never reaches an export: 5.x converts these strokes when a document is opened
        }
        stroke(&mut out, &mut scratch, s, rgb(s.color), s.width);
    }
    out.push_str("Q\n");
    Some(InkStream { content: out, images })
}
