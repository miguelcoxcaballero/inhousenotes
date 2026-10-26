//! `Region`: exact 2D set algebra on a 1 mm integer grid, the only geometry
//! kernel of the city plan.
//!
//! Every coordinate is snapped to whole millimetres on an absolute grid (not
//! relative to some bounding box), so two regions computed separately share
//! their vertices bit for bit and a region hashes the same however it was
//! built. The booleans, offsets, slicing and line clipping are delegated to
//! `i_overlay` and triangulation to `i_triangle`; both are used only in this
//! file, so swapping the kernel is a one-file change.
//!
//! Conventions: points are `(x, z)` in metres, outer rings are
//! counter-clockwise (`geom::signed_area > 0`) and holes clockwise.

use forja_math::Vec2;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay::{Overlay, ShapeType};
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::i_float::int::angle::Angle;
use i_overlay::i_float::int::point::IntPoint;
use i_overlay::mesh::int::arc::ArcOptions;
use i_overlay::mesh::int::outline::offset::IntOutlineOffset;
use i_overlay::mesh::int::style::{IntLineJoin, IntOutlineStyle};
use i_overlay::string::clip::{ClipRule, IntClip};
use i_overlay::string::slice::IntSlice;
use i_triangle::int::triangulatable::IntTriangulatable;

/// Grid step (m).
pub const GRID: f32 = 0.001;
const SCALE: f64 = 1000.0;
/// Coordinates beyond this (m) are clamped: far inside the i32 engine range.
const LIMIT: f64 = 500_000.0;

type IP = IntPoint<i32>;
type IContour = Vec<IP>;
type IShape = Vec<IContour>;

/// Snaps metres to the millimetre grid.
pub fn to_mm(v: f32) -> i32 {
    ((v as f64).clamp(-LIMIT, LIMIT) * SCALE).round() as i32
}

fn ip(p: Vec2) -> IP {
    IP::new(to_mm(p.x), to_mm(p.y))
}

fn fp(p: IP) -> Vec2 {
    Vec2::new((p.x as f64 / SCALE) as f32, (p.y as f64 / SCALE) as f32)
}

/// A point snapped to the grid.
pub fn snap(p: Vec2) -> Vec2 {
    fp(ip(p))
}

/// One connected piece: an outer ring (CCW) and its holes (CW).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shape {
    pub outer: Vec<Vec2>,
    pub holes: Vec<Vec<Vec2>>,
}

impl Shape {
    pub fn rings(&self) -> impl Iterator<Item = &Vec<Vec2>> {
        std::iter::once(&self.outer).chain(self.holes.iter())
    }
}

/// Offset join style.
#[derive(Clone, Copy, Debug)]
pub enum Join {
    /// Mitred, clipped at this minimum interior angle (radians).
    Miter(f32),
    Round,
    Bevel,
}

/// A set of interior-disjoint shapes on the 1 mm grid.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Region {
    shapes: Vec<IShape>,
}

fn contour_area2(c: &[IP]) -> i64 {
    let n = c.len();
    let mut a = 0i64;
    for i in 0..n {
        let (p, q) = (c[i], c[(i + 1) % n]);
        a += p.x as i64 * q.y as i64 - q.x as i64 * p.y as i64;
    }
    a
}

/// Rotates a contour to start at its smallest vertex, so equal sets print
/// and hash equally whatever the construction order.
fn canon_contour(mut c: IContour) -> IContour {
    if let Some(k) = (0..c.len()).min_by_key(|&i| (c[i].x, c[i].y)) {
        c.rotate_left(k);
    }
    c
}

/// An open path on the grid (repeated points dropped; a path closed on
/// itself keeps its closing segment).
fn path_ip(r: &[Vec2]) -> IContour {
    let mut c: IContour = Vec::with_capacity(r.len());
    for p in r {
        let q = ip(*p);
        if c.last() != Some(&q) {
            c.push(q);
        }
    }
    c
}

/// A closed ring on the grid (no repeated closing point).
fn ring_ip(r: &[Vec2]) -> IContour {
    let mut c = path_ip(r);
    while c.len() > 1 && c.first() == c.last() {
        c.pop();
    }
    c
}

fn arc(r_mm: i32) -> ArcOptions {
    // Step keeping the sagitta under 4 mm.
    let r = (r_mm.unsigned_abs() as f64 / SCALE).max(0.01);
    let step = 2.0 * (1.0 - (0.004 / r).min(1.0)).acos();
    ArcOptions { max_step: Angle::from_radians(step.clamp(0.02, 0.5)).unwrap_or(ArcOptions::MAX_STEP), rotation_precision: 6 }
}

impl Region {
    pub fn empty() -> Self {
        Self::default()
    }

    fn from_int(shapes: Vec<IShape>) -> Self {
        let mut shapes: Vec<IShape> = shapes
            .into_iter()
            .filter(|s| !s.is_empty() && s[0].len() >= 3 && contour_area2(&s[0]) != 0)
            .map(|s| {
                let mut it = s.into_iter().filter(|c| c.len() >= 3).map(canon_contour);
                let outer = it.next().unwrap_or_default();
                let mut holes: Vec<IContour> = it.collect();
                holes.sort_by_key(|h| (h[0].x, h[0].y));
                std::iter::once(outer).chain(holes).collect()
            })
            .collect();
        shapes.sort_by_key(|s| (s[0][0].x, s[0][0].y, s[0].len()));
        Region { shapes }
    }

    /// The region inside one closed ring (any winding, self-intersections
    /// resolved by the non-zero rule).
    pub fn ring(r: &[Vec2]) -> Self {
        Self::rings(std::iter::once(r))
    }

    /// The union of closed rings (non-zero rule).
    pub fn rings<'a>(rings: impl IntoIterator<Item = &'a [Vec2]>) -> Self {
        let mut ov = Overlay::<i32>::new(64);
        for r in rings {
            let mut c = ring_ip(r);
            if c.len() < 3 {
                continue;
            }
            if contour_area2(&c) < 0 {
                c.reverse();
            }
            ov.add_source(&c, ShapeType::Subject);
        }
        Self::from_int(ov.overlay(OverlayRule::Subject, FillRule::NonZero))
    }

    /// Shapes given as outer ring plus holes (orientations are fixed here).
    pub fn from_shapes(s: &[Shape]) -> Self {
        let mut ov = Overlay::<i32>::new(64);
        for sh in s {
            let mut outer = ring_ip(&sh.outer);
            if outer.len() < 3 {
                continue;
            }
            if contour_area2(&outer) < 0 {
                outer.reverse();
            }
            let mut shape = vec![outer];
            for h in &sh.holes {
                let mut c = ring_ip(h);
                if c.len() >= 3 {
                    if contour_area2(&c) > 0 {
                        c.reverse();
                    }
                    shape.push(c);
                }
            }
            ov.add_source(&shape, ShapeType::Subject);
        }
        Self::from_int(ov.overlay(OverlayRule::Subject, FillRule::NonZero))
    }

    /// One overlay for any number of regions.
    pub fn union_all<'a>(it: impl IntoIterator<Item = &'a Region>) -> Self {
        let mut ov = Overlay::<i32>::new(256);
        for r in it {
            ov.add_source(&r.shapes, ShapeType::Subject);
        }
        Self::from_int(ov.overlay(OverlayRule::Subject, FillRule::NonZero))
    }

    fn op(&self, o: &Region, rule: OverlayRule) -> Self {
        Self::from_int(Overlay::from_subj_and_clip(&self.shapes, &o.shapes).overlay(rule, FillRule::NonZero))
    }

    pub fn union(&self, o: &Region) -> Self {
        if o.is_empty() {
            return self.clone();
        }
        if self.is_empty() {
            return o.clone();
        }
        self.op(o, OverlayRule::Union)
    }

    pub fn and(&self, o: &Region) -> Self {
        if self.is_empty() || o.is_empty() {
            return Region::empty();
        }
        self.op(o, OverlayRule::Intersect)
    }

    pub fn minus(&self, o: &Region) -> Self {
        if self.is_empty() || o.is_empty() || !self.bbox_overlaps(o) {
            return self.clone();
        }
        self.op(o, OverlayRule::Difference)
    }

    /// Grows (`d > 0`) or erodes (`d < 0`) the region by `d` metres.
    pub fn offset(&self, d: f32, join: Join) -> Self {
        let dm = to_mm(d);
        if dm == 0 || self.is_empty() {
            return self.clone();
        }
        let j = match join {
            Join::Miter(a) => IntLineJoin::Miter(Angle::from_radians(a.clamp(0.09, 3.0) as f64).unwrap_or(Angle::from_bits(1 << 26))),
            Join::Round => IntLineJoin::Round(arc(dm)),
            Join::Bevel => IntLineJoin::Bevel,
        };
        let style = IntOutlineStyle::new(dm).line_join(j);
        match self.shapes.outline(&style) {
            Ok(s) => Self::from_int(s),
            Err(_) => Region::empty(),
        }
    }

    /// Removes every neck and sliver thinner than `2r` and keeps corners:
    /// round erosion then mitred dilation.
    pub fn opening(&self, r: f32) -> Self {
        // Dilating back can only add area inside the eroded set's mitred
        // hull; intersect to stay within the original.
        self.offset(-r, Join::Round).offset(r, Join::Miter(10f32.to_radians())).and(self)
    }

    /// Fills gaps and notches narrower than `2r`.
    pub fn closing(&self, r: f32) -> Self {
        self.offset(r, Join::Round).offset(-r, Join::Miter(10f32.to_radians())).union(self)
    }

    /// Exact partition of the region by open polylines. A cut that does not
    /// separate anything (dangling) leaves the region whole.
    pub fn slice(&self, cuts: &[Vec<Vec2>]) -> Vec<Shape> {
        let paths: Vec<Vec<IP>> = cuts.iter().map(|c| path_ip(c)).filter(|c| c.len() >= 2).collect();
        let out = if paths.is_empty() { self.shapes.clone() } else { self.shapes.slice_by_paths(&paths, FillRule::NonZero) };
        Self::from_int(out).shapes()
    }

    /// The parts of open polylines strictly inside the region.
    pub fn clip(&self, lines: &[Vec<Vec2>]) -> Vec<Vec<Vec2>> {
        let paths: Vec<Vec<IP>> = lines.iter().map(|c| path_ip(c)).filter(|c| c.len() >= 2).collect();
        if paths.is_empty() || self.is_empty() {
            return Vec::new();
        }
        let mut out = self.shapes.clip_paths(&paths, FillRule::NonZero, ClipRule { invert: false, boundary_included: false });
        out.sort_by_key(|p| p.first().map(|q| (q.x, q.y)));
        out.into_iter().map(|p| p.into_iter().map(fp).collect()).collect()
    }

    /// Drops ring vertices closer than `tol` to the chord of their
    /// neighbours (Ramer–Douglas–Peucker per ring), then cleans up.
    pub fn simplify(&self, tol: f32) -> Self {
        let shapes: Vec<Shape> = self
            .shapes()
            .into_iter()
            .map(|s| Shape { outer: rdp_ring(&s.outer, tol), holes: s.holes.iter().map(|h| rdp_ring(h, tol)).collect() })
            .collect();
        Region::from_shapes(&shapes)
    }

    pub fn shapes(&self) -> Vec<Shape> {
        self.shapes
            .iter()
            .map(|s| Shape { outer: s[0].iter().copied().map(fp).collect(), holes: s[1..].iter().map(|h| h.iter().copied().map(fp).collect()).collect() })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.shapes.len()
    }

    pub fn components(&self) -> Vec<Region> {
        self.shapes.iter().map(|s| Region { shapes: vec![s.clone()] }).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    /// Area in m².
    pub fn area(&self) -> f64 {
        let a2: i128 = self.shapes.iter().flatten().map(|c| contour_area2(c) as i128).sum();
        a2 as f64 * 0.5 / (SCALE * SCALE)
    }

    pub fn bbox(&self) -> Option<[Vec2; 2]> {
        let mut it = self.shapes.iter().flat_map(|s| s[0].iter());
        let f = *it.next()?;
        let (mut lo, mut hi) = ((f.x, f.y), (f.x, f.y));
        for p in it {
            lo = (lo.0.min(p.x), lo.1.min(p.y));
            hi = (hi.0.max(p.x), hi.1.max(p.y));
        }
        Some([fp(IP::new(lo.0, lo.1)), fp(IP::new(hi.0, hi.1))])
    }

    fn bbox_overlaps(&self, o: &Region) -> bool {
        match (self.bbox(), o.bbox()) {
            (Some(a), Some(b)) => a[0].x <= b[1].x && b[0].x <= a[1].x && a[0].y <= b[1].y && b[0].y <= a[1].y,
            _ => false,
        }
    }

    pub fn intersects(&self, o: &Region) -> bool {
        self.bbox_overlaps(o) && !self.and(o).is_empty()
    }

    /// Edges of every ring as segments in metres (f64), outer CCW, holes CW:
    /// the inside is always to the left.
    pub fn edges(&self) -> impl Iterator<Item = ([f64; 2], [f64; 2])> + '_ {
        self.shapes.iter().flatten().flat_map(|c| {
            let n = c.len();
            (0..n).map(move |i| {
                let (p, q) = (c[i], c[(i + 1) % n]);
                ([p.x as f64 / SCALE, p.y as f64 / SCALE], [q.x as f64 / SCALE, q.y as f64 / SCALE])
            })
        })
    }

    /// Point in region (even-odd over all rings; boundary counts as outside
    /// or inside arbitrarily, within float precision).
    pub fn contains(&self, p: Vec2) -> bool {
        let (x, y) = (p.x as f64, p.y as f64);
        let mut inside = false;
        for (a, b) in self.edges() {
            if (a[1] > y) != (b[1] > y) {
                let t = (y - a[1]) / (b[1] - a[1]);
                if x < a[0] + t * (b[0] - a[0]) {
                    inside = !inside;
                }
            }
        }
        inside
    }

    /// Distance from `p` to the region's boundary.
    pub fn boundary_distance(&self, p: Vec2) -> f32 {
        let (x, y) = (p.x as f64, p.y as f64);
        let mut best = f64::INFINITY;
        for (a, b) in self.edges() {
            best = best.min(seg_dist2(x, y, a, b));
        }
        best.sqrt() as f32
    }

    /// Signed distance: positive inside.
    pub fn signed_distance(&self, p: Vec2) -> f32 {
        let d = self.boundary_distance(p);
        if self.contains(p) { d } else { -d }
    }

    /// Whether a disc lies entirely inside.
    pub fn contains_disc(&self, p: Vec2, r: f32) -> bool {
        self.contains(p) && self.boundary_distance(p) >= r
    }

    /// Distance along `dir` (unit) from `from` to the first boundary
    /// crossing, if within `max`.
    pub fn ray(&self, from: Vec2, dir: Vec2, max: f32) -> Option<f32> {
        let (ox, oy, dx, dy) = (from.x as f64, from.y as f64, dir.x as f64, dir.y as f64);
        let mut best = max as f64;
        let mut hit = false;
        for (a, b) in self.edges() {
            let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
            let den = dx * ey - dy * ex;
            if den.abs() < 1e-12 {
                continue;
            }
            let (wx, wy) = (a[0] - ox, a[1] - oy);
            let t = (wx * ey - wy * ex) / den;
            let u = (wx * dy - wy * dx) / den;
            if t > 1e-6 && (-1e-9..=1.0 + 1e-9).contains(&u) && t < best {
                best = t;
                hit = true;
            }
        }
        hit.then_some(best as f32)
    }

    /// Centre and radius of the largest inscribed circle (pole of
    /// inaccessibility, quadtree search to 1 cm), over all components.
    pub fn pole(&self) -> Option<(Vec2, f32)> {
        let mut best: Option<(Vec2, f32)> = None;
        for c in self.components() {
            if let Some(p) = c.pole_one(0.01) {
                if best.is_none_or(|b| p.1 > b.1) {
                    best = Some(p);
                }
            }
        }
        best
    }

    fn pole_one(&self, precision: f32) -> Option<(Vec2, f32)> {
        let [lo, hi] = self.bbox()?;
        let size = hi - lo;
        let cell = size.x.min(size.y).max(0.01);
        let mut heap = std::collections::BinaryHeap::new();
        let push = |heap: &mut std::collections::BinaryHeap<Cell>, c: Vec2, h: f32| {
            let d = self.signed_distance(c);
            heap.push(Cell { pot: d + h * std::f32::consts::SQRT_2, d, c, h });
        };
        let mut x = lo.x;
        while x < hi.x {
            let mut y = lo.y;
            while y < hi.y {
                push(&mut heap, Vec2::new(x + cell * 0.5, y + cell * 0.5), cell * 0.5);
                y += cell;
            }
            x += cell;
        }
        let mut best = (Vec2::ZERO, f32::NEG_INFINITY);
        let mut guard = 0;
        while let Some(k) = heap.pop() {
            if k.d > best.1 {
                best = (k.c, k.d);
            }
            if k.pot - best.1 <= precision {
                continue;
            }
            guard += 1;
            if guard > 4000 {
                break;
            }
            let hh = k.h * 0.5;
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                push(&mut heap, k.c + Vec2::new(sx, sy) * hh, hh);
            }
        }
        (best.1 > 0.0).then_some(best)
    }

    /// Triangles covering the region (holes respected): points and CCW
    /// index triples.
    pub fn triangulate(&self) -> (Vec<Vec2>, Vec<[u32; 3]>) {
        let mut pts = Vec::new();
        let mut tris = Vec::new();
        for s in &self.shapes {
            let t = s.triangulate().into_triangulation::<u32>();
            let base = pts.len() as u32;
            pts.extend(t.points.iter().map(|p| fp(*p)));
            for c in t.indices.chunks_exact(3) {
                let (a, b, cc) = (pts[(base + c[0]) as usize], pts[(base + c[1]) as usize], pts[(base + c[2]) as usize]);
                if (b - a).perp_dot(cc - a) >= 0.0 {
                    tris.push([base + c[0], base + c[1], base + c[2]]);
                } else {
                    tris.push([base + c[0], base + c[2], base + c[1]]);
                }
            }
        }
        (pts, tris)
    }

    /// Exact content hash (integer coordinates).
    pub fn hash64(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut eat = |v: i64| {
            h ^= v as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        };
        for s in &self.shapes {
            eat(s.len() as i64);
            for c in s {
                eat(c.len() as i64);
                for p in c {
                    eat(p.x as i64);
                    eat(p.y as i64);
                }
            }
        }
        h
    }
}

/// Quadtree cell of the pole search, ordered by potential.
struct Cell {
    pot: f32,
    d: f32,
    c: Vec2,
    h: f32,
}

impl PartialEq for Cell {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == std::cmp::Ordering::Equal
    }
}
impl Eq for Cell {}
impl PartialOrd for Cell {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Cell {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.pot.total_cmp(&o.pot).then(o.c.x.total_cmp(&self.c.x)).then(o.c.y.total_cmp(&self.c.y))
    }
}

fn seg_dist2(x: f64, y: f64, a: [f64; 2], b: [f64; 2]) -> f64 {
    let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
    let l2 = ex * ex + ey * ey;
    let t = if l2 > 0.0 { (((x - a[0]) * ex + (y - a[1]) * ey) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (dx, dy) = (a[0] + ex * t - x, a[1] + ey * t - y);
    dx * dx + dy * dy
}

/// Closed-ring Ramer–Douglas–Peucker that keeps the extreme vertex.
pub fn rdp_ring(r: &[Vec2], tol: f32) -> Vec<Vec2> {
    let n = r.len();
    if n <= 3 || tol <= 0.0 {
        return r.to_vec();
    }
    // Split at vertex 0 and the vertex farthest from it.
    let far = (1..n).max_by(|&a, &b| r[a].distance_squared(r[0]).total_cmp(&r[b].distance_squared(r[0]))).unwrap_or(n / 2);
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[far] = true;
    let ext: Vec<Vec2> = r.iter().copied().chain(std::iter::once(r[0])).collect();
    rdp_mark(&ext, 0, far, tol, &mut keep);
    let mut keep2 = vec![false; n + 1];
    rdp_mark(&ext, far, n, tol, &mut keep2);
    for i in far..n {
        keep[i] |= keep2[i];
    }
    let out: Vec<Vec2> = (0..n).filter(|&i| keep[i]).map(|i| r[i]).collect();
    if out.len() >= 3 { out } else { r.to_vec() }
}

fn rdp_mark(p: &[Vec2], a: usize, b: usize, tol: f32, keep: &mut [bool]) {
    if b <= a + 1 {
        return;
    }
    let (pa, pb) = (p[a], p[b]);
    let (mut best, mut idx) = (0.0f32, a);
    for (i, q) in p.iter().enumerate().take(b).skip(a + 1) {
        let d = seg_dist2(q.x as f64, q.y as f64, [pa.x as f64, pa.y as f64], [pb.x as f64, pb.y as f64]).sqrt() as f32;
        if d > best {
            best = d;
            idx = i;
        }
    }
    if best > tol {
        keep[idx] = true;
        rdp_mark(p, a, idx, tol, keep);
        rdp_mark(p, idx, b, tol, keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f32, z0: f32, x1: f32, z1: f32) -> Vec<Vec2> {
        vec![Vec2::new(x0, z0), Vec2::new(x1, z0), Vec2::new(x1, z1), Vec2::new(x0, z1)]
    }

    fn poly(seed: u64, c: Vec2, r: f32, n: usize) -> Vec<Vec2> {
        let mut rng = crate::geom::Rng::new(seed);
        (0..n).map(|i| c + Vec2::from_angle(i as f32 / n as f32 * std::f32::consts::TAU + rng.range(-0.2, 0.2)) * r * rng.range(0.3, 1.0)).collect()
    }

    /// Brute-force oracle: point classification against the definition.
    fn inside_ring(r: &[Vec2], p: Vec2) -> bool {
        let mut c = false;
        let n = r.len();
        for i in 0..n {
            let (a, b) = (r[i], r[(i + 1) % n]);
            if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                c = !c;
            }
        }
        c
    }

    fn near_ring(r: &[Vec2], p: Vec2, eps: f32) -> bool {
        let n = r.len();
        (0..n).any(|i| seg_dist2(p.x as f64, p.y as f64, [r[i].x as f64, r[i].y as f64], [r[(i + 1) % n].x as f64, r[(i + 1) % n].y as f64]).sqrt() < eps as f64)
    }

    #[test]
    fn booleans_match_a_sampling_oracle() {
        for seed in 0..24u64 {
            let a = poly(seed, Vec2::new(0.0, 0.0), 10.0, 7 + seed as usize % 5);
            let b = poly(seed + 100, Vec2::new(3.0, 1.0), 9.0, 5 + seed as usize % 7);
            let (ra, rb) = (Region::ring(&a), Region::ring(&b));
            let ops: [(&str, Region, fn(bool, bool) -> bool); 3] = [
                ("union", ra.union(&rb), |x, y| x || y),
                ("and", ra.and(&rb), |x, y| x && y),
                ("minus", ra.minus(&rb), |x, y| x && !y),
            ];
            for (name, r, f) in ops {
                let mut x = -12.0;
                while x < 14.0 {
                    let mut z = -12.0;
                    while z < 14.0 {
                        let p = Vec2::new(x, z);
                        // Even-odd within each input ring (they may self-intersect: use the
                        // region of each input as the definition for non-zero == even-odd on simple rings).
                        let (ia, ib) = (ra.contains(p), rb.contains(p));
                        if !near_ring(&a, p, 0.01) && !near_ring(&b, p, 0.01) {
                            assert_eq!(r.contains(p), f(ia, ib), "{name} seed {seed} at {p} ia {ia} ib {ib} a {a:?} b {b:?} r {:?}", r.shapes());
                        }
                        z += 0.25;
                    }
                    x += 0.25;
                }
            }
            // Area identity.
            let (u, i) = (ra.union(&rb).area(), ra.and(&rb).area());
            // Intersection points round to the millimetre grid.
            assert!((u + i - ra.area() - rb.area()).abs() < 0.02, "seed {seed}: {u} {i} {} {}", ra.area(), rb.area());
        }
    }

    #[test]
    fn simple_rings_agree_with_the_raw_definition() {
        let a = rect(0.0, 0.0, 10.0, 6.0);
        let r = Region::ring(&a);
        for (x, z) in [(1.0, 1.0), (9.9, 5.9), (-0.1, 3.0), (5.0, 6.1)] {
            let p = Vec2::new(x, z);
            assert_eq!(r.contains(p), inside_ring(&a, p));
        }
        assert!((r.area() - 60.0).abs() < 1e-9);
        // CW input gives the same region.
        let mut cw = a.clone();
        cw.reverse();
        assert_eq!(Region::ring(&cw), r);
    }

    #[test]
    fn degenerate_soups_do_not_break_anything() {
        let soups: Vec<Vec<Vec2>> = vec![
            vec![Vec2::ZERO, Vec2::ZERO, Vec2::new(1.0, 0.0)],
            vec![Vec2::ZERO, Vec2::new(5.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(5.0, 0.0)],
            vec![Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 0.0001), Vec2::new(0.0, 0.0)],
            // 2° wedge.
            vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.0, 0.7)],
            // Bow tie.
            vec![Vec2::ZERO, Vec2::new(4.0, 4.0), Vec2::new(4.0, 0.0), Vec2::new(0.0, 4.0)],
        ];
        for s in &soups {
            let r = Region::ring(s);
            assert!(r.area() >= 0.0);
            let o = r.offset(-0.2, Join::Round);
            let _ = o.offset(0.2, Join::Miter(0.3));
            let _ = r.union(&Region::ring(&rect(0.0, 0.0, 2.0, 2.0)));
            let _ = r.triangulate();
            let _ = r.pole();
        }
        // Collinear overlapping rectangles merge.
        let u = Region::rings([rect(0.0, 0.0, 10.0, 2.0).as_slice(), rect(5.0, 0.0, 15.0, 2.0).as_slice()]);
        assert_eq!(u.len(), 1);
        assert!((u.area() - 30.0).abs() < 1e-6);
    }

    #[test]
    fn opening_removes_necks_keeps_corners() {
        // Two 10 m squares joined by a 1 m neck.
        let r = Region::rings([rect(0.0, 0.0, 10.0, 10.0).as_slice(), rect(10.0, 4.5, 14.0, 5.5).as_slice(), rect(14.0, 0.0, 24.0, 10.0).as_slice()]);
        assert_eq!(r.len(), 1);
        let o = r.opening(1.0);
        assert_eq!(o.len(), 2, "neck removed");
        // Corners kept: the square's corner point is still inside.
        assert!(o.contains(Vec2::new(0.05, 0.05)) && o.contains(Vec2::new(23.95, 9.95)));
        // The opening leaves a small rounded nub where the neck met each square.
        assert!((o.area() - 200.0).abs() < 0.5, "{}", o.area());
    }

    #[test]
    fn slice_partitions_with_t_cuts() {
        let r = Region::ring(&rect(0.0, 0.0, 10.0, 10.0));
        let cuts = vec![vec![Vec2::new(-1.0, 5.0), Vec2::new(11.0, 5.0)], vec![Vec2::new(5.0, -1.0), Vec2::new(5.0, 5.0)], vec![Vec2::new(3.0, 5.0), Vec2::new(3.2, 11.0)]];
        let p = r.slice(&cuts);
        assert_eq!(p.len(), 4);
        let a: f64 = p.iter().map(|s| Region::from_shapes(std::slice::from_ref(s)).area()).sum();
        assert!((a - 100.0).abs() < 1e-6);
        // Dangling cut leaves it whole.
        assert_eq!(r.slice(&[vec![Vec2::new(-1.0, 5.0), Vec2::new(4.0, 5.0)]]).len(), 1);
        // Shared vertices are identical bit for bit.
        let all: Vec<Vec2> = p.iter().flat_map(|s| s.outer.clone()).collect();
        let t = all.iter().filter(|q| (q.x - 5.0).abs() < 1e-6 && (q.y - 5.0).abs() < 1e-6).count();
        assert!(t >= 3, "triple point shared by three pieces ({t})");
    }

    #[test]
    fn clip_and_ray_and_triangulate_with_holes() {
        let shape = Shape { outer: rect(0.0, 0.0, 10.0, 10.0), holes: vec![rect(3.0, 3.0, 6.0, 6.0)] };
        let r = Region::from_shapes(&[shape]);
        assert!((r.area() - 91.0).abs() < 1e-9);
        let (pts, tris) = r.triangulate();
        let at: f32 = tris.iter().map(|t| (pts[t[1] as usize] - pts[t[0] as usize]).perp_dot(pts[t[2] as usize] - pts[t[0] as usize]) * 0.5).sum();
        assert!((at - 91.0).abs() < 1e-3, "{at}");
        let l = r.clip(&[vec![Vec2::new(-5.0, 4.0), Vec2::new(15.0, 4.0)]]);
        assert_eq!(l.len(), 2);
        let len: f32 = l.iter().map(|p| p.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>()).sum();
        assert!((len - 7.0).abs() < 1e-3);
        assert!((r.ray(Vec2::new(1.0, 4.0), Vec2::X, 50.0).unwrap() - 2.0).abs() < 1e-4);
        assert!(r.ray(Vec2::new(-1.0, 4.0), -Vec2::X, 50.0).is_none());
    }

    #[test]
    fn pole_is_inside_and_maximal() {
        let r = Region::ring(&rect(0.0, 0.0, 20.0, 8.0));
        let (c, rad) = r.pole().unwrap();
        assert!((rad - 4.0).abs() < 0.02, "{rad}");
        assert!(r.contains(c) && (c.y - 4.0).abs() < 0.05);
        // L shape: the pole lies in the thicker arm.
        let l = Region::rings([rect(0.0, 0.0, 30.0, 4.0).as_slice(), rect(0.0, 0.0, 12.0, 12.0).as_slice()]);
        let (c, rad) = l.pole().unwrap();
        assert!((rad - 6.0).abs() < 0.02 && c.x < 12.0, "{c} {rad}");
    }

    #[test]
    fn grid_is_absolute() {
        let a = Region::ring(&rect(0.0004, 0.0, 1.0006, 1.0));
        let b = Region::ring(&rect(100.0004, 0.0, 101.0006, 1.0)).union(&a);
        let s = b.shapes();
        assert!(s.iter().flat_map(|s| s.outer.iter()).all(|p| ((p.x * 1000.0).round() - p.x * 1000.0).abs() < 1e-3));
        assert_eq!(a.hash64(), Region::ring(&rect(0.0, 0.0, 1.001, 1.0)).hash64());
    }
}
