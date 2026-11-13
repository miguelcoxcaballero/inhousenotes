//! Pen ink: the centripetal Catmull–Rom spline of 5.18.0 and the context set-up shared by every stroke draw.

use super::cmd::{Cap, Cmds, Composite, Join};
use super::{highlighter, with_css, PathCache, StrokeRef, TAU};
use crate::geom::js_max;
use crate::op::Tool;

type P = (f64, f64);

/// v5: `appendPenSplineSegment(target, p0, p1, p2, p3)` — the segment p1→p2 (app-v5.js:4459).
pub fn segment(c: &mut Cmds, p0: P, p1: P, p2: P, p3: P) {
    let (ax, ay) = (p2.0 - p1.0, p2.1 - p1.1);
    let length2 = ax * ax + ay * ay;
    if length2 < 4.0 {
        c.line_to(p2.0, p2.1);
        return;
    }
    let (bx, by, cx, cy) = (p1.0 - p0.0, p1.1 - p0.1, p3.0 - p2.0, p3.1 - p2.1);
    let d1 = (bx * bx + by * by).sqrt().sqrt();
    let d2 = length2.sqrt().sqrt();
    let d3 = (cx * cx + cy * cy).sqrt().sqrt();
    let (mut c1x, mut c1y, mut c2x, mut c2y) = (p1.0, p1.1, p2.0, p2.1);
    if d1 >= 1e-3 {
        let (a, b) = (d1 * d1, d2 * d2);
        let (k, n) = (2.0 * a + 3.0 * d1 * d2 + b, 3.0 * d1 * (d1 + d2));
        c1x = (a * p2.0 - b * p0.0 + k * p1.0) / n;
        c1y = (a * p2.1 - b * p0.1 + k * p1.1) / n;
    }
    if d3 >= 1e-3 {
        let (a, b) = (d3 * d3, d2 * d2);
        let (k, n) = (2.0 * a + 3.0 * d3 * d2 + b, 3.0 * d3 * (d3 + d2));
        c2x = (a * p1.0 - b * p3.0 + k * p2.0) / n;
        c2y = (a * p1.1 - b * p3.1 + k * p2.1) / n;
    }
    c.bezier_to(c1x, c1y, c2x, c2y, p2.0, p2.1);
}

fn pt(xy: &[f64], i: usize) -> P { (xy[2 * i], xy[2 * i + 1]) }

/// Segments `from..to` of the spline through `xy` (segment i joins points i and i + 1).
pub fn segments(c: &mut Cmds, xy: &[f64], from: usize, to: usize) {
    let end = xy.len() / 2 - 1;
    for i in from..to {
        segment(c, pt(xy, i.saturating_sub(1)), pt(xy, i), pt(xy, i + 1), pt(xy, (i + 2).min(end)));
    }
}

/// The whole stroke: `moveTo(first)` + every segment (the `Path2D` body of v5 `drawSmoothStroke`). ≥ 2 points.
pub fn path(c: &mut Cmds, xy: &[f64]) {
    c.move_to(xy[0], xy[1]);
    segments(c, xy, 0, xy.len() / 2 - 1);
}

/// v5: `setupContext(ctx, tool, color, width)` + `setStrokeCaps(ctx, tool)` (app-v5.js:12784, 12659).
pub fn setup(c: &mut Cmds, s: &StrokeRef) {
    c.composite(Composite::SourceOver);
    c.global_alpha(1.0);
    match s.tool {
        Tool::LegacyEraseArea => {
            c.composite(Composite::DestinationOut);
            c.line_width(s.width);
            c.stroke_style("rgba(0,0,0,1)");
        }
        Tool::Highlighter => {
            c.composite(Composite::SourceOver);
            c.global_alpha(0.35);
            with_css(s.color, |css| c.stroke_style(css));
            c.line_width(s.width);
        }
        _ => {
            with_css(s.color, |css| c.stroke_style(css));
            c.line_width(s.width);
        }
    }
    c.line_cap(if s.tool == Tool::Highlighter { Cap::Butt } else { Cap::Round });
    c.line_join(Join::Round);
}

/// v5: `drawSmoothStroke(ctx, stroke)` (app-v5.js:12726).
pub fn draw(c: &mut Cmds, paths: &mut PathCache, s: &StrokeRef) {
    if s.is_empty() {
        return;
    }
    if s.tool == Tool::Highlighter {
        highlighter::draw(c, s);
        return;
    }
    if s.len() < 2 {
        // BEHAVIOUR-CHANGES B-E3: a single-tap dot stays (5.18.0 drops it at the next repaint).
        dot(c, s);
        return;
    }
    setup(c, s);
    c.line_width(s.width);
    let (p, temporary) = paths.path(c, s);
    c.stroke_path(p);
    if temporary {
        c.path_drop(p);
    }
}

/// v5: `drawSinglePointStroke(ctx, stroke)` (app-v5.js:4545).
pub fn dot(c: &mut Cmds, s: &StrokeRef) {
    if s.is_empty() {
        return;
    }
    if s.tool == Tool::Highlighter {
        highlighter::draw(c, s);
        return;
    }
    setup(c, s);
    let radius = js_max(0.7, s.width * 0.5);
    with_css(s.color, |css| c.fill_style(css));
    c.begin_path();
    c.arc(s.xy[0], s.xy[1], radius, 0.0, TAU);
    c.fill();
}

/// v5: `drawStrokeTailSegment(ctx, stroke, final)` (app-v5.js:4485). `done` is `stroke.liveCurveIndex`.
pub fn tail(c: &mut Cmds, s: &StrokeRef, done: &mut Option<usize>, last_too: bool) {
    if s.len() < 2 {
        return;
    }
    let last = s.len() - 1;
    let (from, to) = (done.unwrap_or(0), if last_too { last } else { last - 1 });
    if from >= to {
        return;
    }
    setup(c, s);
    c.line_width(s.width);
    c.begin_path();
    let (x, y) = s.pt(from);
    c.move_to(x, y);
    segments(c, s.xy, from, to);
    c.stroke();
    *done = Some(to);
}

/// v5: `drawLegacyCoverStrokes(ctx, page)` (app-v5.js:12874): each cover again, opaque white and 1.35× wider.
pub fn covers(c: &mut Cmds, paths: &mut PathCache, covers: &[StrokeRef]) {
    if covers.is_empty() {
        return;
    }
    const WHITE: crate::op::Color = crate::op::Color::Hex(0xff_ffff);
    c.save();
    for s in covers.iter().filter(|s| s.len() >= 2) {
        draw(c, paths, &StrokeRef { tool: Tool::Pen, color: &WHITE, width: js_max(1.0, super::or(s.width, 1.0) * 1.35), ..*s });
    }
    c.restore();
}
