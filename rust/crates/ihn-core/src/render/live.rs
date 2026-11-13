//! Live ink: while the pen is down, finished segments go to the page bitmap and the newest one to the cursor
//! overlay; at lift the rest is committed (v5 `updateLiveStroke`, `commitLiveStrokeToCanvas`).
//!
//! Functions that draw on an overlay return `Some(has_ink)` when 5.18.0 calls `setOverlayCanvasInk(overlay,
//! has_ink)` (the caller toggles `visibility`, once per change) and `None` when it does not.

use super::cmd::{Cap, Cmds, Composite, Join, Surf};
use super::{highlighter, or, pen, with_css, PathCache, StrokeRef};
use crate::geom::{js_max, js_min};
use crate::op::Tool;

/// v5 `liveCursorPreviewBounds` entry of one cursor overlay: what the next draw clears first.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// `minX, minY, maxX, maxY` kept by the live highlighter.
    extent: Option<[f64; 4]>,
}

impl Bounds {
    pub fn rect(x: f64, y: f64, w: f64, h: f64) -> Bounds { Bounds { x, y, w, h, extent: None } }
}

/// A cursor overlay canvas and its scale (`dataset.overlayScale`).
pub struct Cursor {
    pub surf: Surf,
    pub scale: f64,
    /// Backing size, for the full clear.
    pub px: (u32, u32),
    pub bounds: Option<Bounds>,
}

/// A page canvas during a contact: `canvas.width / pageWidth`, `canvas.height / pageHeight`.
#[derive(Clone, Copy)]
pub struct PageTarget {
    pub surf: Surf,
    pub sx: f64,
    pub sy: f64,
}

impl PageTarget {
    fn begin(&self, c: &mut Cmds) {
        c.ctx(self.surf, false);
        c.set_transform(self.sx, 0.0, 0.0, self.sy, 0.0, 0.0);
    }
}

/// Per-stroke progress (v5 `stroke.liveCurveIndex`, `finalizedIndex`, `livePreviewPointCount`).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Live {
    pub curve: Option<usize>,
    pub finalized: Option<usize>,
    pub previewed: usize,
}

fn extent(xy: &[f64], mut e: [f64; 4]) -> [f64; 4] {
    for p in xy.chunks_exact(2) {
        e = [js_min(e[0], p[0]), js_min(e[1], p[1]), js_max(e[2], p[0]), js_max(e[3], p[1])];
    }
    e
}

const EMPTY: [f64; 4] = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];

fn padded(e: [f64; 4], padding: f64, keep: bool) -> Bounds {
    Bounds { x: e[0] - padding, y: e[1] - padding, w: js_max(1.0, e[2] - e[0] + padding * 2.0), h: js_max(1.0, e[3] - e[1] + padding * 2.0), extent: keep.then_some(e) }
}

/// v5: `updateLiveStroke(pageIndex, canvas, stroke, previewCanvas, metrics)` (app-v5.js:4660), once per frame.
pub fn update(c: &mut Cmds, live: &mut Live, page: PageTarget, cursor: &mut Cursor, s: &StrokeRef) -> Option<bool> {
    if s.tool == Tool::Highlighter {
        return highlighter_append(c, live, cursor, s);
    }
    match s.len() {
        0 => None,
        1 => {
            if live.finalized.is_none() {
                page.begin(c);
                pen::dot(c, s);
                live.finalized = Some(0);
            }
            None
        }
        n => {
            // v5: finalizeStrokeSegments
            page.begin(c);
            pen::tail(c, s, &mut live.curve, false);
            live.finalized = Some(n - 1);
            tip(c, live, cursor, s)
        }
    }
}

/// v5: `drawLivePenTip(previewCanvas, pageIndex, stroke, metrics)` (app-v5.js:4507): the newest segment.
fn tip(c: &mut Cmds, live: &Live, cursor: &mut Cursor, s: &StrokeRef) -> Option<bool> {
    c.ctx(cursor.surf, false);
    c.set_transform(cursor.scale, 0.0, 0.0, cursor.scale, 0.0, 0.0);
    if let Some(b) = cursor.bounds {
        c.clear_rect(b.x, b.y, b.w, b.h);
    }
    let (last, from) = (s.len() - 1, live.curve.unwrap_or(0));
    if from >= last {
        cursor.bounds = None;
        return Some(false);
    }
    let chain = &s.xy[2 * from.saturating_sub(1)..];
    let offset = (from > 0) as usize;
    pen::setup(c, s);
    c.line_width(s.width);
    c.begin_path();
    c.move_to(chain[2 * offset], chain[2 * offset + 1]);
    pen::segments(c, chain, offset, chain.len() / 2 - 1);
    c.stroke();
    cursor.bounds = Some(padded(extent(chain, EMPTY), js_max(4.0, s.width + 2.0), false));
    Some(true)
}

/// v5: `drawLiveHighlighterAppend(pageIndex, stroke, previewCanvas, metrics)` (app-v5.js:4576).
fn highlighter_append(c: &mut Cmds, live: &mut Live, cursor: &mut Cursor, s: &StrokeRef) -> Option<bool> {
    let n = s.len();
    let start = live.previewed.min(n);
    if start >= n {
        return None;
    }
    c.ctx(cursor.surf, false);
    c.set_transform(cursor.scale, 0.0, 0.0, cursor.scale, 0.0, 0.0);
    let from = start.saturating_sub(1);
    let e = extent(&s.xy[2 * from..], cursor.bounds.and_then(|b| b.extent).unwrap_or(EMPTY));
    if start == 0 && n == 1 {
        pen::dot(c, s);
    } else {
        let (width, inner) = highlighter::widths(s.width);
        c.save();
        c.composite(Composite::Multiply);
        with_css(s.color, |css| c.stroke_style(if css.is_empty() { "#ffde00" } else { css }));
        c.line_cap(Cap::Round);
        c.line_join(Join::Round);
        c.begin_path();
        let (x, y) = s.pt(from);
        c.move_to(x, y);
        for i in start.max(1)..n {
            let (x, y) = s.pt(i);
            c.line_to(x, y);
        }
        c.global_alpha(0.22);
        c.line_width(width);
        c.stroke();
        c.global_alpha(0.1);
        c.line_width(inner);
        c.stroke();
        c.restore();
    }
    cursor.bounds = Some(padded(e, js_max(4.0, or(s.width, 12.0)), true));
    live.previewed = n;
    Some(true)
}

/// v5: `commitLiveStrokeToCanvas(pageIndex, canvas, stroke)` (app-v5.js:4636), at lift. For a highlighter `s` is
/// the stroke as it will be stored.
pub fn commit(c: &mut Cmds, paths: &mut PathCache, live: &mut Live, page: PageTarget, s: &StrokeRef) {
    if s.is_empty() {
        return;
    }
    c.ctx(page.surf, false);
    if s.len() == 1 {
        if live.finalized.is_none() {
            pen::dot(c, s);
        }
        return;
    }
    if s.tool == Tool::Highlighter {
        pen::draw(c, paths, s);
        return;
    }
    c.set_transform(page.sx, 0.0, 0.0, page.sy, 0.0, 0.0);
    pen::tail(c, s, &mut live.curve, true);
}

/// v5: `clearEraserPreview(pageIndex)` (app-v5.js:4708) — also how the live tip is removed at lift. Always
/// followed by `setOverlayCanvasInk(overlay, false)`.
pub fn clear_cursor(c: &mut Cmds, cursor: &mut Cursor) {
    c.ctx(cursor.surf, false);
    match cursor.bounds.take() {
        Some(b) => {
            c.set_transform(cursor.scale, 0.0, 0.0, cursor.scale, 0.0, 0.0);
            c.clear_rect(b.x, b.y, b.w, b.h);
        }
        None => {
            c.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
            c.clear_rect(0.0, 0.0, cursor.px.0 as f64, cursor.px.1 as f64);
        }
    }
}
