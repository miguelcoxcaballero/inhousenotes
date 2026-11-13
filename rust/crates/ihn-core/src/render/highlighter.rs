//! Highlighter ink: two multiply passes over a quadratic mid-point path.

use super::cmd::{Cap, Cmds, Composite, Join};
use super::{or, with_css, StrokeRef, TAU};
use crate::geom::js_max;

/// v5: `buildSmoothPath(ctx, points)` (app-v5.js:12703).
pub fn smooth_path(c: &mut Cmds, xy: &[f64]) {
    let n = xy.len() / 2;
    if n < 2 {
        return;
    }
    c.begin_path();
    c.move_to(xy[0], xy[1]);
    if n == 2 {
        c.line_to(xy[2], xy[3]);
        return;
    }
    for i in 1..n - 1 {
        let (x, y, nx, ny) = (xy[2 * i], xy[2 * i + 1], xy[2 * i + 2], xy[2 * i + 3]);
        c.quad_to(x, y, (x + nx) / 2.0, (y + ny) / 2.0);
    }
    c.line_to(xy[2 * n - 2], xy[2 * n - 1]);
}

/// The pass widths of a highlighter of stored width `w`: `max(4, w || 12)` and `max(2, that * 0.62)`.
pub fn widths(w: f64) -> (f64, f64) {
    let width = js_max(4.0, or(w, 12.0));
    (width, js_max(2.0, width * 0.62))
}

/// v5: `drawFixedHighlighterStroke(ctx, stroke)` (app-v5.js:12668).
pub fn draw(c: &mut Cmds, s: &StrokeRef) {
    if s.is_empty() {
        return;
    }
    let (width, inner) = widths(s.width);
    c.save();
    c.composite(Composite::Multiply);
    with_css(s.color, |css| {
        let css = if css.is_empty() { "#ffde00" } else { css };
        c.stroke_style(css);
        c.fill_style(css);
    });
    c.line_cap(Cap::Round);
    c.line_join(Join::Round);
    if s.len() == 1 {
        c.global_alpha(0.24);
        c.begin_path();
        c.arc(s.xy[0], s.xy[1], width * 0.52, 0.0, TAU);
        c.fill();
        c.restore();
        return;
    }
    c.global_alpha(0.22);
    c.line_width(width);
    smooth_path(c, s.xy);
    c.stroke();
    c.global_alpha(0.1);
    c.line_width(inner);
    smooth_path(c, s.xy);
    c.stroke();
    c.restore();
}
