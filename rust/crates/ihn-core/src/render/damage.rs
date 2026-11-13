//! Damage-region repaint while erasing (v5 `recordEraserDamage` / `repaintEraserDamage`).

use super::cmd::{Cmds, Composite, Surf};
use super::page::{self, PageView};
use super::template::Templates;
use super::{or, pen, PathCache, StrokeRef};
use crate::geom::{js_max, js_min};

/// The rectangle of a page that erased strokes covered: `[minX, minY, maxX, maxY]`.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub struct Damage(pub Option<[f64; 4]>);

/// v5: `Math.max(3, Number(stroke.width || 1) * 1.5)`.
fn pad(width: f64) -> f64 { js_max(3.0, or(width, 1.0) * 1.5) }

impl Damage {
    /// v5: `recordEraserDamage(pageIndex, stroke)` (app-v5.js:12313) for a removed stroke.
    pub fn add(&mut self, bounds: [f64; 4], width: f64) {
        if !bounds[0].is_finite() {
            return;
        }
        let p = pad(width);
        let mut next = [bounds[0] - p, bounds[1] - p, bounds[2] + p, bounds[3] + p];
        if let Some(prev) = self.0 {
            next = [js_min(next[0], prev[0]), js_min(next[1], prev[1]), js_max(next[2], prev[2]), js_max(next[3], prev[3])];
        }
        self.0 = Some(next);
    }
    pub fn take(&mut self) -> Option<[f64; 4]> { self.0.take() }
}

fn intersects(s: &StrokeRef, d: &[f64; 4]) -> bool {
    let (b, p) = (s.bounds, pad(s.width));
    !s.is_empty() && b[2] + p >= d[0] && b[0] - p <= d[2] && b[3] + p >= d[1] && b[1] - p <= d[3]
}

/// v5: `repaintEraserDamage(pageIndex, damage)` (app-v5.js:12330): background and surviving ink inside the
/// damaged rectangle. `view` holds the strokes that remain; `scale` is the canvas' render scale. The caller falls
/// back to a full repaint when the canvas has no complete paint, as 5.18.0 does.
pub fn repaint(c: &mut Cmds, tpl: &mut Templates, paths: &mut PathCache, canvas: Surf, view: &PageView, scale: f64, d: [f64; 4]) {
    c.ctx(canvas, false);
    c.save();
    c.set_transform(scale, 0.0, 0.0, scale, 0.0, 0.0);
    c.begin_path();
    c.rect(d[0], d[1], d[2] - d[0], d[3] - d[1]);
    c.clip();
    c.clear_rect(d[0], d[1], d[2] - d[0], d[3] - d[1]);
    c.composite(Composite::SourceOver);
    c.global_alpha(1.0);
    page::background(c, tpl, canvas, view, scale);
    let covers: Vec<StrokeRef> = view.items.covers.iter().filter(|s| intersects(s, &d)).copied().collect();
    pen::covers(c, paths, &covers);
    page::images(c, &view.items.images);
    for s in view.items.strokes.iter().filter(|s| intersects(s, &d)) {
        pen::draw(c, paths, s);
    }
    c.restore();
}
