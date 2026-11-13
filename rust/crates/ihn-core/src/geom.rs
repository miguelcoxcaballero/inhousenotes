//! Ink geometry (ARCHITECTURE §2.2, §11.2): bounds, the exact point–segment hit test, swept-capsule eraser tests
//! (BEHAVIOUR-CHANGES B-E6), the area-eraser split, lasso tests and the selection transform. Ported from 5.18.0 and
//! from `rust-core` (`editor.rs`, branch `rust/editor_input`); with a zero-length eraser segment every function
//! reproduces 5.18.0 bit for bit. Polylines and polygons are flat `x0, y0, x1, y1, …` slices.

use crate::jsmath;

/// v5: `Math.max(a, b)` — NaN poisons the result.
#[inline]
pub fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a > b {
        a
    } else {
        b
    }
}

/// v5: `Math.min(a, b)`.
#[inline]
pub fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a < b {
        a
    } else {
        b
    }
}

/// v5: `Math.hypot(dx, dy) <= radius`, with a squared-distance shortcut that only decides cases far from the
/// threshold; anything within 1e-9 (relative) of it is settled by the exact formula.
#[inline]
pub fn within(dx: f64, dy: f64, radius: f64) -> bool {
    let d2 = dx * dx + dy * dy;
    let r2 = radius * radius;
    // A negative radius never contains a point, but its square is positive: leave it to the exact test.
    if radius >= 0.0 && d2 > 1e-250 && d2 < 1e250 && r2 > 1e-250 && r2 < 1e250 {
        if d2 < r2 * (1.0 - 1e-9) {
            return true;
        }
        if d2 > r2 * (1.0 + 1e-9) {
            return false;
        }
    }
    jsmath::hypot(dx, dy) <= radius
}

/// Point of segment `(x1,y1)-(x2,y2)` closest to `(px,py)`, as v5 `distancePointToSegment` computes it.
#[inline]
fn closest(px: f64, py: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> (f64, f64) {
    let dx = x2 - x1;
    let dy = y2 - y1;
    if dx == 0.0 && dy == 0.0 {
        return (x1, y1);
    }
    let t = ((px - x1) * dx + (py - y1) * dy) / (dx * dx + dy * dy);
    let clamped = js_max(0.0, js_min(1.0, t));
    (x1 + clamped * dx, y1 + clamped * dy)
}

/// v5: `distancePointToSegment(px, py, x1, y1, x2, y2)`.
pub fn segment_distance(px: f64, py: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let (cx, cy) = closest(px, py, x1, y1, x2, y2);
    jsmath::hypot(px - cx, py - cy)
}

/// v5: `distancePointToSegment(...) <= radius`.
#[inline]
pub fn point_segment_within(px: f64, py: f64, x1: f64, y1: f64, x2: f64, y2: f64, radius: f64) -> bool {
    let (cx, cy) = closest(px, py, x1, y1, x2, y2);
    within(px - cx, py - cy, radius)
}

/// An eraser sample pair: the segment swept between two consecutive eraser positions. `a == b` is one position.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Sweep {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Sweep {
    pub fn point(x: f64, y: f64) -> Sweep { Sweep { x0: x, y0: y, x1: x, y1: y } }
    #[inline]
    fn is_point(&self) -> bool { self.x0 == self.x1 && self.y0 == self.y1 }

    /// Whether `(px,py)` lies within `radius` of the swept segment (capsule test).
    #[inline]
    pub fn covers(&self, px: f64, py: f64, radius: f64) -> bool {
        point_segment_within(px, py, self.x0, self.y0, self.x1, self.y1, radius)
    }

    /// Whether segment `(x1,y1)-(x2,y2)` passes within `radius` of the swept segment.
    pub fn touches(&self, x1: f64, y1: f64, x2: f64, y2: f64, radius: f64) -> bool {
        if self.is_point() {
            return point_segment_within(self.x0, self.y0, x1, y1, x2, y2, radius);
        }
        // Two segments are closest at an endpoint of one of them unless they cross.
        let cross = |ox: f64, oy: f64, ax: f64, ay: f64, bx: f64, by: f64| (ax - ox) * (by - oy) - (ay - oy) * (bx - ox);
        let opposite = |a: f64, b: f64| (a > 0.0 && b < 0.0) || (a < 0.0 && b > 0.0);
        let d1 = cross(self.x0, self.y0, self.x1, self.y1, x1, y1);
        let d2 = cross(self.x0, self.y0, self.x1, self.y1, x2, y2);
        let d3 = cross(x1, y1, x2, y2, self.x0, self.y0);
        let d4 = cross(x1, y1, x2, y2, self.x1, self.y1);
        (radius >= 0.0 && opposite(d1, d2) && opposite(d3, d4))
            || self.covers(x1, y1, radius)
            || self.covers(x2, y2, radius)
            || point_segment_within(self.x0, self.y0, x1, y1, x2, y2, radius)
            || point_segment_within(self.x1, self.y1, x1, y1, x2, y2, radius)
    }

    /// v5: `strokeMayIntersectCircle` given the stroke bounds and `pad = max(0, radius + extra + width/2)`,
    /// widened to the swept segment.
    #[inline]
    pub fn may_touch(&self, bounds: &[f64; 4], pad: f64) -> bool {
        let (lo_x, hi_x) = if self.x0 < self.x1 { (self.x0, self.x1) } else { (self.x1, self.x0) };
        let (lo_y, hi_y) = if self.y0 < self.y1 { (self.y0, self.y1) } else { (self.y1, self.y0) };
        !(hi_x < bounds[0] - pad || lo_x > bounds[2] + pad || hi_y < bounds[1] - pad || lo_y > bounds[3] + pad)
    }
}

/// v5: `getStrokeBounds` scan → `[minX, minY, maxX, maxY]`. NaN coordinates are skipped by the comparisons.
pub fn bounds(xy: &[f64]) -> [f64; 4] {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in xy.chunks_exact(2) {
        if p[0] < b[0] {
            b[0] = p[0];
        }
        if p[1] < b[1] {
            b[1] = p[1];
        }
        if p[0] > b[2] {
            b[2] = p[0];
        }
        if p[1] > b[3] {
            b[3] = p[1];
        }
    }
    b
}

/// v5: the bbox padding of `strokeHitsPoint` / `eraseAreaAtPoint`: `max(0, radius + 2 + (width || 0) * 0.5)`.
#[inline]
pub fn eraser_pad(radius: f64, width: f64) -> f64 { js_max(0.0, radius + 2.0 + if width.is_nan() { 0.0 } else { width } * 0.5) }

/// v5: `strokeHitsPoint` exact phase — the centre-line polyline within `radius` of the eraser (stroke width is
/// not added); a single-point stroke uses the point distance. Swept between two samples (B-E6).
pub fn polyline_hit(xy: &[f64], sweep: &Sweep, radius: f64) -> bool {
    match xy.len() / 2 {
        0 => false,
        1 => sweep.covers(xy[0], xy[1], radius),
        _ => xy.windows(4).step_by(2).any(|s| sweep.touches(s[0], s[1], s[2], s[3], radius)),
    }
}

/// v5: `splitStrokeByEraser` — the surviving runs of `xy` as half-open point ranges appended to `out`. A point is
/// inside if the eraser covers it; a run restarts at the point after a segment the eraser touches; runs of fewer
/// than two points are dropped. Returns whether the stroke changed (v5: anything but one run with every point).
pub fn split_ranges(xy: &[f64], sweep: &Sweep, radius: f64, out: &mut Vec<(u32, u32)>) -> bool {
    let first = out.len();
    let count = xy.len() / 2;
    let (mut start, mut len) = (0usize, 0usize);
    let mut flush = |start: usize, len: usize| {
        if len > 1 {
            out.push((start as u32, (start + len) as u32));
        }
    };
    for i in 0..count {
        let (px, py) = (xy[i * 2], xy[i * 2 + 1]);
        let inside = sweep.covers(px, py, radius);
        if i > 0 && !inside && sweep.touches(xy[i * 2 - 2], xy[i * 2 - 1], px, py, radius) {
            flush(start, len);
            start = i;
            len = 1;
        } else if !inside {
            if len == 0 {
                start = i;
            }
            len += 1;
        } else {
            flush(start, len);
            len = 0;
        }
    }
    flush(start, len);
    !(out.len() == first + 1 && out[first] == (0, count as u32))
}

/// v5: `pointInPolygon` — even–odd ray casting with `+0.00001` in the denominator.
pub fn point_in_polygon(x: f64, y: f64, poly: &[f64]) -> bool {
    let n = poly.len() / 2;
    let mut inside = false;
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (xi, yi, xj, yj) = (poly[i * 2], poly[i * 2 + 1], poly[j * 2], poly[j * 2 + 1]);
        if ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi + 0.00001) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// v5: `strokeIntersectsPolygon` — any point of the stroke inside the lasso.
pub fn polyline_in_polygon(xy: &[f64], poly: &[f64]) -> bool { xy.chunks_exact(2).any(|p| point_in_polygon(p[0], p[1], poly)) }

/// v5: `getImageCorners` — the four corners of a rotated image rectangle (`rotation || 0`).
pub fn image_corners(cx: f64, cy: f64, w: f64, h: f64, rotation: f64) -> [f64; 8] {
    let rot = if rotation.is_nan() { 0.0 } else { rotation };
    let (half_w, half_h, cos, sin) = (w / 2.0, h / 2.0, jsmath::cos(rot), jsmath::sin(rot));
    let mut out = [0.0; 8];
    for (i, (x, y)) in [(-half_w, -half_h), (half_w, -half_h), (half_w, half_h), (-half_w, half_h)].into_iter().enumerate() {
        out[i * 2] = cx + x * cos - y * sin;
        out[i * 2 + 1] = cy + x * sin + y * cos;
    }
    out
}

/// v5: `segmentsIntersect`.
fn segments_intersect(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    let det = (b.0 - a.0) * (d.1 - c.1) - (b.1 - a.1) * (d.0 - c.0);
    if det == 0.0 {
        return false;
    }
    let t = ((c.0 - a.0) * (d.1 - c.1) - (c.1 - a.1) * (d.0 - c.0)) / det;
    let u = ((c.0 - a.0) * (b.1 - a.1) - (c.1 - a.1) * (b.0 - a.0)) / det;
    (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)
}

/// v5: `polygonIntersectsRect` — a corner inside the lasso, a lasso point inside the rectangle, or crossing edges.
pub fn polygon_intersects_rect(poly: &[f64], corners: &[f64; 8]) -> bool {
    let at = |s: &[f64], i: usize| (s[(i % (s.len() / 2)) * 2], s[(i % (s.len() / 2)) * 2 + 1]);
    let n = poly.len() / 2;
    corners.chunks_exact(2).any(|c| point_in_polygon(c[0], c[1], poly))
        || poly.chunks_exact(2).any(|p| point_in_polygon(p[0], p[1], corners))
        || (0..n).any(|i| (0..4).any(|j| segments_intersect(at(poly, i), at(poly, i + 1), at(corners, j), at(corners, j + 1))))
}

/// Accumulator of v5 `getSelectionBounds`: `Math.min`/`Math.max` over points, so a NaN poisons the result.
#[derive(Clone, Copy, Debug)]
pub struct SelectionBounds(pub [f64; 4]);

impl Default for SelectionBounds {
    fn default() -> Self { SelectionBounds([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY]) }
}

impl SelectionBounds {
    pub fn add(&mut self, xy: &[f64]) {
        let b = &mut self.0;
        for p in xy.chunks_exact(2) {
            *b = [js_min(b[0], p[0]), js_min(b[1], p[1]), js_max(b[2], p[0]), js_max(b[3], p[1])];
        }
    }
    /// `[minX, minY, maxX, maxY]`, or `None` when nothing was added (v5: `!isFinite(minX)`).
    pub fn get(&self) -> Option<[f64; 4]> { self.0[0].is_finite().then_some(self.0) }
}

/// v5: the selection transform of `applyTransformToSelection`:
/// `p' = centre + R(rot)·S(sx, sy)·(p − centre) + t`, computed in exactly that order.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Affine {
    pub cx: f64,
    pub cy: f64,
    pub sx: f64,
    pub sy: f64,
    pub rot: f64,
    pub cos: f64,
    pub sin: f64,
    pub tx: f64,
    pub ty: f64,
}

impl Affine {
    pub fn new(centre: (f64, f64), sx: f64, sy: f64, rot: f64, tx: f64, ty: f64) -> Affine {
        Affine { cx: centre.0, cy: centre.1, sx, sy, rot, cos: jsmath::cos(rot), sin: jsmath::sin(rot), tx, ty }
    }
    #[inline]
    pub fn point(&self, px: f64, py: f64) -> (f64, f64) {
        let x = (px - self.cx) * self.sx;
        let y = (py - self.cy) * self.sy;
        (self.cx + (x * self.cos - y * self.sin) + self.tx, self.cy + (x * self.sin + y * self.cos) + self.ty)
    }
    pub fn points(&self, xy: &[f64]) -> Vec<f64> {
        let mut out = Vec::with_capacity(xy.len());
        for p in xy.chunks_exact(2) {
            let (x, y) = self.point(p[0], p[1]);
            out.push(x);
            out.push(y);
        }
        out
    }
    /// Images: centre transformed like a point, `width·sx`, `height·sy`, `(rotation || 0) + rot`.
    pub fn image(&self, cx: f64, cy: f64, w: f64, h: f64, rotation: f64) -> (f64, f64, f64, f64, f64) {
        let (x, y) = self.point(cx, cy);
        (x, y, w * self.sx, h * self.sy, if rotation.is_nan() { 0.0 } else { rotation } + self.rot)
    }
}

/// Test command hook: `__IHN_TEST_API__.cmd('core.geom.<name>', args)`.
pub fn test_cmd(_name: &str, _args: &str) -> Option<String> { None }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Rng;

    fn flat(points: &[(f64, f64)]) -> Vec<f64> { points.iter().flat_map(|p| [p.0, p.1]).collect() }
    fn up(v: f64) -> f64 { f64::from_bits(v.to_bits() + 1) }
    fn down(v: f64) -> f64 { f64::from_bits(v.to_bits().saturating_sub(1)) }

    // Ported from rust-core/src/editor.rs and editor_input.rs.
    #[test]
    fn segment_endpoints_and_zero_length() {
        assert!(point_segment_within(5.0, 2.0, 0.0, 0.0, 10.0, 0.0, 2.0));
        assert!(!point_segment_within(5.0, 2.01, 0.0, 0.0, 10.0, 0.0, 2.0));
        assert!(point_segment_within(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0));
        assert!(!point_segment_within(1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0));
        assert!(!point_segment_within(8.0, 10.0, 0.0, 0.0, 0.0, 10.0, 0.0));
        assert!(point_segment_within(8.0, 10.0, 0.0, 10.0, 10.0, 10.0, 0.0));
        let hit = |xy: &[f64], x, y, r| polyline_hit(xy, &Sweep::point(x, y), r);
        assert!(hit(&[0., 0., 10., 0.], 5., 2., 2.));
        assert!(!hit(&[0., 0., 10., 0.], 5., 2.01, 2.));
        assert!(hit(&[0., 0., 0., 0.], 0., 0., 0.));
        assert!(!hit(&[0., 0., 0., 0.], 1., 0., 0.));
        assert!(hit(&[0., 0., 0., 10., 10., 10.], 8., 10., 0.));
        // Single-point stroke: point distance. No points: never.
        assert!(hit(&[3., 4.], 0., 0., 5.) && !hit(&[3., 4.], 0., 0., 4.99) && !hit(&[], 0., 0., 100.));
        assert!(!hit(&[0., 0., 10., 0.], 5., 0., -1.));
    }

    #[test]
    fn bounds_skip_nan_like_the_original_comparisons() {
        assert_eq!(bounds(&[1.0, 5.0, -2.0, f64::NAN, 4.0, 0.0]), [-2.0, 0.0, 4.0, 5.0]);
        assert_eq!(bounds(&[f64::NAN, f64::NAN]), [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY]);
        let mut sel = SelectionBounds::default();
        assert_eq!(sel.get(), None);
        sel.add(&[1.0, 5.0, 4.0, 0.0]);
        assert_eq!(sel.get(), Some([1.0, 0.0, 4.0, 5.0]));
        sel.add(&[f64::NAN, 1.0]);
        assert_eq!(sel.get(), None); // v5: Math.min(NaN) poisons → no box
        assert_eq!(eraser_pad(5.0, 2.0), 8.0);
        assert_eq!(eraser_pad(5.0, f64::NAN), 7.0);
        assert_eq!(eraser_pad(-50.0, 2.0), 0.0);
        let b = [0.0, 0.0, 10.0, 10.0];
        assert!(Sweep::point(18.0, 5.0).may_touch(&b, 8.0) && !Sweep::point(18.01, 5.0).may_touch(&b, 8.0));
        assert!(Sweep { x0: 30.0, y0: 5.0, x1: 18.0, y1: 5.0 }.may_touch(&b, 8.0));
        assert!(!Sweep { x0: 30.0, y0: 5.0, x1: 19.0, y1: 5.0 }.may_touch(&b, 8.0));
    }

    #[test]
    fn shortcut_agrees_with_exact_formula_on_threshold_cases() {
        let mut rng = Rng::new(1);
        for _ in 0..100_000 {
            let mut c = || rng.range(0.0, 100.0);
            let (x1, y1, x2, y2, px, py, other) = (c(), c(), c(), c(), c(), c(), c() / 10.0);
            let exact = segment_distance(px, py, x1, y1, x2, y2);
            for r in [exact, up(exact), down(exact), exact * 1.000000001, exact * 0.999999999, other] {
                assert_eq!(point_segment_within(px, py, x1, y1, x2, y2, r), exact <= r, "{px} {py} {x1} {y1} {x2} {y2} r={r}");
            }
        }
    }

    fn ranges(points: &[(f64, f64)], x: f64, y: f64, r: f64) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        split_ranges(&flat(points), &Sweep::point(x, y), r, &mut out);
        out
    }

    /// Literal transcription of v5 `splitStrokeByEraser`, using only the exact formulas.
    fn reference(points: &[(f64, f64)], x: f64, y: f64, r: f64) -> Vec<(u32, u32)> {
        let (mut segments, mut current): (Vec<(u32, u32)>, Vec<usize>) = (vec![], vec![]);
        let mut close = |current: &Vec<usize>| {
            if current.len() > 1 {
                segments.push((current[0] as u32, current[current.len() - 1] as u32 + 1));
            }
        };
        for i in 0..points.len() {
            let inside = jsmath::hypot(points[i].0 - x, points[i].1 - y) <= r;
            if i > 0 && !inside && segment_distance(x, y, points[i - 1].0, points[i - 1].1, points[i].0, points[i].1) <= r {
                close(&current);
                current = vec![i];
                continue;
            }
            if !inside {
                current.push(i);
            } else {
                close(&current);
                current.clear();
            }
        }
        close(&current);
        segments
    }

    #[test]
    fn split_cuts_around_the_eraser_and_restarts_at_crossings() {
        let line: Vec<(f64, f64)> = (0..11).map(|i| (i as f64 * 10.0, 0.0)).collect();
        assert_eq!(ranges(&line, 50.0, 0.0, 15.0), vec![(0, 4), (7, 11)]);
        assert_eq!(ranges(&line, 500.0, 0.0, 1.0), vec![(0, 11)]);
        assert_eq!(ranges(&line, 50.0, 0.0, 1000.0), vec![]);
        // Crossing between two outside points: the stroke is cut and the second run restarts at the far point.
        assert_eq!(ranges(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (30.0, 0.0)], 15.0, 0.0, 2.0), vec![(0, 2), (2, 4)]);
        // Runs of a single point are dropped.
        assert_eq!(ranges(&[(0.0, 0.0), (10.0, 0.0), (40.0, 0.0)], 5.0, 0.0, 6.0), vec![]);
        assert_eq!(ranges(&[(f64::NAN, 0.0), (1.0, 1.0), (2.0, 2.0)], 100.0, 100.0, 1.0), vec![(0, 3)]);
        assert_eq!(ranges(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)], 1.0, 0.0, -1.0), vec![(0, 3)]);
        // The "changed" result is v5's `segments.length === 1 && all points` test.
        let mut out = Vec::new();
        assert!(!split_ranges(&flat(&line), &Sweep::point(500.0, 0.0), 1.0, &mut out));
        assert!(split_ranges(&flat(&line), &Sweep::point(50.0, 0.0), 15.0, &mut out));
        assert!(split_ranges(&flat(&line), &Sweep::point(50.0, 0.0), 1000.0, &mut out));
        assert_eq!(out, vec![(0, 11), (0, 4), (7, 11)]);
    }

    #[test]
    fn split_matches_the_literal_original_on_random_and_threshold_inputs() {
        let mut rng = Rng::new(9);
        for _ in 0..20_000 {
            let n = 1 + rng.below(40) as usize;
            let pts: Vec<(f64, f64)> =
                (0..n).map(|_| if rng.chance(0.1) { (f64::NAN, rng.range(0.0, 50.0)) } else { (rng.range(0.0, 50.0), rng.range(0.0, 50.0)) }).collect();
            let (x, y) = (rng.range(0.0, 50.0), rng.range(0.0, 50.0));
            let k = 1 + rng.below(n.max(2) as u64 - 1) as usize;
            let mut radii = vec![rng.range(0.0, 20.0), -rng.range(0.0, 20.0), 0.0];
            if k < n {
                let d = segment_distance(x, y, pts[k - 1].0, pts[k - 1].1, pts[k].0, pts[k].1);
                radii.extend([d, up(d), down(d)]);
            }
            let d = jsmath::hypot(pts[0].0 - x, pts[0].1 - y);
            radii.extend([d, up(d)]);
            for r in radii {
                assert_eq!(ranges(&pts, x, y, r), reference(&pts, x, y, r), "{pts:?} {x} {y} {r}");
            }
        }
    }

    /// Brute-force distance between two segments by dense sampling of the eraser path.
    fn sampled_min(sweep: &Sweep, a: (f64, f64), b: (f64, f64)) -> f64 {
        (0..=2000)
            .map(|i| {
                let t = i as f64 / 2000.0;
                segment_distance(sweep.x0 + (sweep.x1 - sweep.x0) * t, sweep.y0 + (sweep.y1 - sweep.y0) * t, a.0, a.1, b.0, b.1)
            })
            .fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn swept_capsule_catches_what_sampled_positions_miss() {
        // B-E6: a fast swipe across a thin stroke; neither sample is within the radius, the swept segment is.
        let stroke = flat(&[(50.0, 0.0), (50.0, 100.0)]);
        let (a, b) = (Sweep::point(10.0, 50.0), Sweep::point(90.0, 50.0));
        let swipe = Sweep { x0: 10.0, y0: 50.0, x1: 90.0, y1: 50.0 };
        assert!(!polyline_hit(&stroke, &a, 5.0) && !polyline_hit(&stroke, &b, 5.0));
        assert!(polyline_hit(&stroke, &swipe, 5.0) && polyline_hit(&stroke, &swipe, 0.0) && !polyline_hit(&stroke, &swipe, -1.0));
        // Parallel pass at distance 6: a capsule of radius 5 misses, 6 hits.
        let pass = Sweep { x0: 44.0, y0: 10.0, x1: 44.0, y1: 90.0 };
        assert!(!polyline_hit(&stroke, &pass, 5.0) && polyline_hit(&stroke, &pass, 6.0));
        // A dot is erased when the capsule covers it.
        assert!(polyline_hit(&[50.0, 53.0], &swipe, 3.0) && !polyline_hit(&[50.0, 53.1], &swipe, 3.0));
        // Area eraser: the swipe cuts the stroke where it passes, keeping both sides.
        let dense: Vec<(f64, f64)> = (0..=10).map(|i| (50.0, i as f64 * 10.0)).collect();
        let mut out = Vec::new();
        assert!(split_ranges(&flat(&dense), &swipe, 5.0, &mut out));
        assert_eq!(out, vec![(0, 5), (6, 11)]);
        out.clear();
        assert!(!split_ranges(&flat(&dense), &Sweep { x0: 10.0, y0: 50.0, x1: 40.0, y1: 50.0 }, 5.0, &mut out));
    }

    #[test]
    fn swept_capsule_agrees_with_dense_sampling() {
        let mut rng = Rng::new(21);
        for _ in 0..3000 {
            let mut c = || rng.range(0.0, 60.0);
            let sweep = Sweep { x0: c(), y0: c(), x1: c(), y1: c() };
            let (a, b, r) = ((c(), c()), (c(), c()), c() / 4.0);
            let d = sampled_min(&sweep, a, b);
            if (d - r).abs() > 0.05 {
                assert_eq!(sweep.touches(a.0, a.1, b.0, b.1, r), d <= r, "{sweep:?} {a:?} {b:?} {r} {d}");
            }
            // A sweep that does not move is exactly the v5 point test.
            let still = Sweep::point(sweep.x0, sweep.y0);
            let exact = segment_distance(sweep.x0, sweep.y0, a.0, a.1, b.0, b.1);
            for radius in [exact, up(exact), down(exact), r] {
                assert_eq!(still.touches(a.0, a.1, b.0, b.1, radius), exact <= radius);
            }
        }
    }

    #[test]
    fn lasso_point_and_image_tests() {
        let square = [0.0, 0.0, 10.0, 0.0, 10.0, 10.0, 0.0, 10.0];
        assert!(point_in_polygon(5.0, 5.0, &square) && !point_in_polygon(15.0, 5.0, &square) && !point_in_polygon(5.0, -1.0, &square));
        assert!(!point_in_polygon(5.0, 5.0, &[]));
        // A concave lasso (a "C"): the notch is outside.
        let c = [0.0, 0.0, 10.0, 0.0, 10.0, 3.0, 3.0, 3.0, 3.0, 7.0, 10.0, 7.0, 10.0, 10.0, 0.0, 10.0];
        assert!(point_in_polygon(1.0, 5.0, &c) && !point_in_polygon(6.0, 5.0, &c) && point_in_polygon(6.0, 1.0, &c));
        // Any point inside selects the stroke.
        assert!(polyline_in_polygon(&[20.0, 20.0, 5.0, 5.0], &square) && !polyline_in_polygon(&[20.0, 20.0, 30.0, 5.0], &square));
        // v5 quirk kept: a segment crossing the lasso with no sample inside is not selected.
        assert!(!polyline_in_polygon(&[-5.0, 5.0, 15.0, 5.0], &square));
        // Images: corner inside, lasso inside the image, or edges crossing.
        let img = image_corners(5.0, 5.0, 4.0, 2.0, 0.0);
        assert_eq!(img, [3.0, 4.0, 7.0, 4.0, 7.0, 6.0, 3.0, 6.0]);
        assert_eq!(image_corners(5.0, 5.0, 4.0, 2.0, f64::NAN), img);
        assert!(polygon_intersects_rect(&square, &img));
        assert!(polygon_intersects_rect(&[4.5, 4.5, 5.5, 4.5, 5.0, 5.5], &img)); // lasso inside the image
        assert!(polygon_intersects_rect(&[5.0, -5.0, 5.2, -5.0, 5.2, 20.0, 5.0, 20.0], &image_corners(5.0, 5.0, 40.0, 2.0, 0.0)));
        assert!(!polygon_intersects_rect(&square, &image_corners(50.0, 50.0, 4.0, 2.0, 0.3)));
        // Rotated by a quarter turn the 4×2 image is 2×4.
        let q = image_corners(0.0, 0.0, 4.0, 2.0, std::f64::consts::FRAC_PI_2);
        assert!((q[0] - 1.0).abs() < 1e-12 && (q[1] + 2.0).abs() < 1e-12);
    }

    #[test]
    fn affine_uses_the_v5_operation_order() {
        // v5: x' = cx + ((px − cx)·sx·cos − (py − cy)·sy·sin) + tx
        let (cx, cy, sx, sy, rot, tx, ty) = (100.3, 55.7, 1.37, 0.82, 0.6, 3.1, -2.2);
        let m = Affine::new((cx, cy), sx, sy, rot, tx, ty);
        let (px, py) = (17.123456789, 91.987654321);
        let (cos, sin) = (jsmath::cos(rot), jsmath::sin(rot));
        let (mut x, mut y) = (px - cx, py - cy);
        x *= sx;
        y *= sy;
        let (rx, ry) = (x * cos - y * sin, x * sin + y * cos);
        assert_eq!(m.point(px, py), (cx + rx + tx, cy + ry + ty));
        assert_eq!(m.points(&[px, py, cx, cy]), vec![cx + rx + tx, cy + ry + ty, cx + 0.0 + tx, cy + 0.0 + ty]);
        // A pure move keeps v5's rounding (centre subtracted and added back), not `p + t`.
        let mv = Affine::new((cx, cy), 1.0, 1.0, 0.0, 5.0, 7.0);
        assert_eq!(mv.point(px, py), (cx + (px - cx) + 5.0, cy + (py - cy) + 7.0));
        let (ix, iy, w, h, r) = m.image(10.0, 20.0, 30.0, 40.0, f64::NAN);
        assert_eq!((ix, iy), m.point(10.0, 20.0));
        assert_eq!((w, h, r), (30.0 * sx, 40.0 * sy, rot));
    }
}
