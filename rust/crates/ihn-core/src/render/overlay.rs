//! Overlay canvases: lasso and selection outlines, eraser cursor, other devices' live ink, timeline highlights.
//! Visibility of an overlay (`setOverlayCanvasInk`) is the caller's: each function documents what 5.18.0 sets.

use super::cmd::{Cap, Cmds, Join, Surf};
use super::live::{Bounds, Cursor};
use super::{pen, PathCache, StrokeRef, TAU};

/// v5: `clearLassoCanvas(pageIndex)` (app-v5.js:4419); ink → false.
pub fn clear_lasso(c: &mut Cmds, lasso: Surf, px: (u32, u32)) {
    c.ctx(lasso, false);
    c.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
    c.clear_rect(0.0, 0.0, px.0 as f64, px.1 as f64);
}

/// v5: `drawLassoPath(lassoCanvas, points)` (app-v5.js:4428); ink → true. `scale` = the overlay's
/// `dataset.overlayScale`. Nothing happens for an empty polygon.
pub fn lasso_path(c: &mut Cmds, lasso: Surf, px: (u32, u32), scale: f64, xy: &[f64]) {
    if xy.len() < 2 {
        return;
    }
    c.ctx(lasso, false);
    c.set_transform(scale, 0.0, 0.0, scale, 0.0, 0.0);
    c.clear_rect(0.0, 0.0, px.0 as f64 / scale, px.1 as f64 / scale);
    c.stroke_style("#002FD9");
    c.line_width(1.5);
    c.line_dash(&[6.0, 4.0]);
    c.begin_path();
    c.move_to(xy[0], xy[1]);
    for p in xy[2..].chunks_exact(2) {
        c.line_to(p[0], p[1]);
    }
    c.stroke();
    c.line_dash(&[]);
}

/// v5: `drawSelectionOutlines(pageIndex)` (app-v5.js:5183): clears the lasso overlay (ink → false) and, when
/// strokes are selected on this page, outlines them (ink → true). Returns whether the overlay has ink.
pub fn selection_outlines(c: &mut Cmds, lasso: Surf, px: (u32, u32), scale: f64, selected: &[StrokeRef]) -> bool {
    clear_lasso(c, lasso, px);
    if selected.is_empty() {
        return false;
    }
    c.set_transform(scale, 0.0, 0.0, scale, 0.0, 0.0);
    c.stroke_style("rgba(0, 47, 217, 0.45)");
    c.line_width(1.5);
    c.line_dash(&[3.0, 3.0]);
    c.line_cap(Cap::Round);
    c.line_join(Join::Round);
    for s in selected.iter().filter(|s| s.len() >= 2) {
        c.begin_path();
        c.move_to(s.xy[0], s.xy[1]);
        for p in s.xy[2..].chunks_exact(2) {
            c.line_to(p[0], p[1]);
        }
        c.stroke();
    }
    c.line_dash(&[]);
    true
}

/// v5: `drawEraserPreview(pageIndex, pos)` (app-v5.js:4680); ink → true. `width` = `state.eraserWidth`.
pub fn eraser_cursor(c: &mut Cmds, cursor: &mut Cursor, x: f64, y: f64, width: f64) {
    c.ctx(cursor.surf, false);
    c.set_transform(cursor.scale, 0.0, 0.0, cursor.scale, 0.0, 0.0);
    if let Some(b) = cursor.bounds {
        c.clear_rect(b.x, b.y, b.w, b.h);
    }
    let radius = width / 2.0;
    cursor.bounds = Some(Bounds::rect(x - radius - 3.0, y - radius - 3.0, radius * 2.0 + 6.0, radius * 2.0 + 6.0));
    c.stroke_style("rgba(0,0,0,0.45)");
    c.line_width(1.2);
    c.begin_path();
    c.arc(x, y, radius, 0.0, TAU);
    c.stroke();
    c.fill_style("rgba(255,255,255,0.25)");
    c.fill();
}

/// v5: `redrawRemoteLiveStrokePreviews(pageIndex)` (app-v5.js:3437): clears the remote overlay and draws every
/// in-progress stroke other devices have on this page. Returns whether the overlay has ink.
pub fn remote_redraw(c: &mut Cmds, paths: &mut PathCache, overlay: Surf, px: (u32, u32), scale: f64, strokes: &[StrokeRef]) -> bool {
    c.ctx(overlay, false);
    c.set_transform(scale, 0.0, 0.0, scale, 0.0, 0.0);
    c.clear_rect(0.0, 0.0, px.0 as f64 / scale, px.1 as f64 / scale);
    let mut ink = false;
    for s in strokes.iter().filter(|s| !s.is_empty()) {
        ink = true;
        if s.len() == 1 {
            pen::dot(c, s);
        } else {
            pen::draw(c, paths, &StrokeRef { key: None, ..*s });
        }
    }
    ink
}

/// v5: `drawRemoteLiveStrokeAppend(pageIndex, record)` (app-v5.js:3466): the points after `*rendered`
/// (`record.renderedPointCount`), as straight segments; ink → true when something was drawn (the return value).
pub fn remote_append(c: &mut Cmds, overlay: Surf, scale: f64, s: &StrokeRef, rendered: &mut usize) -> bool {
    let n = s.len();
    let start = (*rendered).min(n);
    if start >= n {
        return false;
    }
    c.ctx(overlay, false);
    c.set_transform(scale, 0.0, 0.0, scale, 0.0, 0.0);
    if start == 0 && n == 1 {
        pen::dot(c, s);
        *rendered = 1;
        return true;
    }
    pen::setup(c, s);
    c.line_width(s.width);
    c.begin_path();
    let (x, y) = s.pt(start.saturating_sub(1));
    c.move_to(x, y);
    for i in start.max(1)..n {
        let (x, y) = s.pt(i);
        c.line_to(x, y);
    }
    c.stroke();
    *rendered = n;
    true
}

/// One rectangle of the timeline preview diff.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Highlight {
    pub removed: bool,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// v5: `drawTimelineDiffHighlights(ctx, highlights)` (app-v5.js:13649), on the selected context.
pub fn timeline_highlights(c: &mut Cmds, highlights: &[Highlight]) {
    if highlights.is_empty() {
        return;
    }
    c.save();
    c.line_join(Join::Round);
    for h in highlights {
        c.stroke_style(if h.removed { "#ef4444" } else { "#f59e0b" });
        c.fill_style(if h.removed { "rgba(239, 68, 68, 0.08)" } else { "rgba(245, 158, 11, 0.12)" });
        c.line_width(2.5);
        c.line_dash(if h.removed { &[8.0, 5.0] } else { &[] });
        c.fill_rect(h.x, h.y, h.w, h.h);
        c.stroke_rect(h.x, h.y, h.w, h.h);
    }
    c.restore();
}
