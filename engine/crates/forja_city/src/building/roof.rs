//! Pitched roofs on any lot by weighted offsets.
//!
//! Every edge of the lot carries a roof plane that rises from its eave
//! line at the edge's own pitch; a party wall has an infinite pitch (a
//! vertical plane: the roof stops against the neighbour's medianera, as on
//! real row houses). The roof is the lower envelope of those planes,
//! capped by a flat top at height `cap`:
//!
//! * the face of edge `i` is the strip in front of it, clipped by the
//!   half-planes where its plane is below the plane of every edge `j` whose
//!   strip it meets (the weighted bisectors), and by the eave lines;
//! * the flat top is what no face covers: the lot offset inwards by each
//!   edge's run.
//!
//! All clipping is of convex polygons by half-planes, in f64. For a convex
//! lot this is exactly the weighted straight-skeleton roof; on concave
//! lots locality (only edges whose strips meet) keeps far walls from
//! cutting a face. `Roof::new` checks that faces and top tile the eave
//! outline exactly and returns `None` otherwise, so the caller can fall
//! back to a lower cap or a flat roof.

use forja_math::{Vec2, Vec3};

use crate::geom::Prim;

/// How the roof meets one edge of the lot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slope {
    /// Rise over run; `f32::INFINITY` for a wall the roof stops against.
    pub pitch: f32,
    /// Overhang beyond the wall line (m).
    pub eave: f32,
}

impl Slope {
    /// A party wall: no slope, no overhang.
    pub const WALL: Slope = Slope { pitch: f32::INFINITY, eave: 0.0 };

    fn sloped(&self) -> bool {
        self.pitch.is_finite() && self.pitch > 0.0
    }
}

type P2 = [f64; 2];

/// Half-plane `a · x + b <= 0`.
#[derive(Clone, Copy, Debug)]
struct Half {
    a: P2,
    b: f64,
}

impl Half {
    fn eval(&self, p: P2) -> f64 {
        self.a[0] * p[0] + self.a[1] * p[1] + self.b
    }
}

fn p2(v: Vec2) -> P2 {
    [v.x as f64, v.y as f64]
}

fn v2(p: P2) -> Vec2 {
    Vec2::new(p[0] as f32, p[1] as f32)
}

fn area(poly: &[P2]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| poly[i][0] * poly[(i + 1) % n][1] - poly[(i + 1) % n][0] * poly[i][1]).sum::<f64>() * 0.5
}

/// Sutherland–Hodgman clip of a convex polygon by one half-plane.
fn clip(poly: &[P2], h: Half) -> Vec<P2> {
    let n = poly.len();
    let mut out = Vec::with_capacity(n + 1);
    for i in 0..n {
        let (p, q) = (poly[i], poly[(i + 1) % n]);
        let (fp, fq) = (h.eval(p), h.eval(q));
        if fp <= 0.0 {
            out.push(p);
        }
        if (fp < 0.0 && fq > 0.0) || (fp > 0.0 && fq < 0.0) {
            let t = fp / (fp - fq);
            out.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
        }
    }
    // Drop repeated points left by vertices on the line.
    out.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9);
    if out.len() > 1 && (out[0][0] - out[out.len() - 1][0]).abs() < 1e-9 && (out[0][1] - out[out.len() - 1][1]).abs() < 1e-9 {
        out.pop();
    }
    out
}

/// Whether two convex polygons overlap (separating axis test, with slack).
fn overlap(a: &[P2], b: &[P2], slack: f64) -> bool {
    if a.len() < 3 || b.len() < 2 {
        return false;
    }
    for (poly, other) in [(a, b), (b, a)] {
        let n = poly.len();
        if n < 3 {
            continue;
        }
        let ccw = area(poly) > 0.0;
        for i in 0..n {
            let (p, q) = (poly[i], poly[(i + 1) % n]);
            let d = [q[0] - p[0], q[1] - p[1]];
            let len = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-12);
            // Outward normal of a CCW polygon is (d.y, -d.x).
            let nrm = if ccw { [d[1] / len, -d[0] / len] } else { [-d[1] / len, d[0] / len] };
            let lim = nrm[0] * p[0] + nrm[1] * p[1];
            if other.iter().all(|o| nrm[0] * o[0] + nrm[1] * o[1] > lim + slack) {
                return false;
            }
        }
    }
    true
}

/// The roof over a lot.
#[derive(Clone, Debug)]
pub struct Roof {
    /// Wall points and inward unit normals of every edge.
    lines: Vec<(P2, P2)>,
    slopes: Vec<Slope>,
    /// Height of the flat top above the eave line.
    pub cap: f32,
    /// Sloped faces: `(edge, convex CCW polygon)`.
    pub faces: Vec<(usize, Vec<Vec2>)>,
    /// Flat top, at `cap` (empty when the faces meet in ridges).
    pub top: Vec<Vec<Vec2>>,
    /// Eave outline: the lot grown by each edge's overhang.
    pub outer: Vec<Vec2>,
}

impl Roof {
    /// Roof over the counter-clockwise `ring` with one `Slope` per edge,
    /// rising at most `cap` above the eaves. `None` when the faces and the
    /// top do not tile the eave outline (a lot too irregular for this cap).
    pub fn new(ring: &[Vec2], slopes: &[Slope], cap: f32) -> Option<Roof> {
        let n = ring.len();
        if n < 3 || slopes.len() != n || !(cap > 0.0) || slopes.iter().all(|s| !s.sloped()) {
            return None;
        }
        let lines: Vec<(P2, P2)> = (0..n)
            .map(|i| {
                let (a, b) = (p2(ring[i]), p2(ring[(i + 1) % n]));
                let d = [b[0] - a[0], b[1] - a[1]];
                let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-12);
                (a, [-d[1] / l, d[0] / l])
            })
            .collect();
        let mut roof = Roof { lines, slopes: slopes.to_vec(), cap, faces: Vec::new(), top: Vec::new(), outer: Vec::new() };
        // g_i(x) = distance inside edge i's eave line.
        let outer = roof.offset(|i| -roof.slopes[i].eave as f64)?;
        roof.outer = outer.iter().map(|p| v2(*p)).collect();
        let outer_area = area(&outer);
        let ext = {
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for p in &outer {
                for k in 0..2 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            [[lo[0] - 1.0, lo[1] - 1.0], [hi[0] + 1.0, hi[1] + 1.0]]
        };
        let bbox: Vec<P2> = vec![ext[0], [ext[1][0], ext[0][1]], ext[1], [ext[0][0], ext[1][1]]];
        let mut covered = 0.0;
        for i in 0..n {
            if !roof.slopes[i].sloped() {
                continue;
            }
            let mut face = clip(&bbox, roof.eave_half(i));
            face = clip(&face, roof.cap_half(i));
            // Neighbours first, then the others nearest first: a far wall
            // only counts if the face still reaches its strip.
            let mut order: Vec<usize> = (0..n).filter(|&j| j != i).collect();
            let mid = |k: usize| {
                let (a, b) = (ring[k], ring[(k + 1) % n]);
                (a + b) * 0.5
            };
            order.sort_by(|&a, &b| {
                let adj = |j: usize| j == (i + 1) % n || j == (i + n - 1) % n;
                adj(b).cmp(&adj(a)).then(mid(a).distance(mid(i)).total_cmp(&mid(b).distance(mid(i)))).then(a.cmp(&b))
            });
            for j in order {
                if face.len() < 3 {
                    break;
                }
                let adjacent = j == (i + 1) % n || j == (i + n - 1) % n;
                if !adjacent && !overlap(&face, &roof.zone(j, ring), 1e-6) {
                    continue;
                }
                face = clip(&face, roof.bisector(i, j));
                if face.len() >= 3 {
                    face = clip(&face, roof.eave_half(j));
                }
            }
            if face.len() < 3 {
                continue;
            }
            let a = area(&face);
            if a < 1e-6 {
                continue;
            }
            covered += a;
            roof.faces.push((i, face.iter().map(|p| v2(*p)).collect()));
        }
        // The flat top: the lot offset inwards to the cap line of every
        // sloped edge (party walls stay where they are).
        let tol = 1e-3 * outer_area.max(1.0);
        if (outer_area - covered).abs() > tol {
            let top = roof.offset(|i| {
                let s = roof.slopes[i];
                if s.sloped() { (cap / s.pitch - s.eave) as f64 } else { 0.0 }
            })?;
            covered += area(&top);
            roof.top.push(top.iter().map(|p| v2(*p)).collect());
        }
        ((outer_area - covered).abs() <= tol).then_some(roof)
    }

    /// Ring of the lines `g_i = d(i)` (each edge line moved inwards by
    /// `d(i) + eave`), if it is a valid CCW ring with every edge keeping
    /// its direction (no edge collapsed).
    fn offset(&self, d: impl Fn(usize) -> f64) -> Option<Vec<P2>> {
        let n = self.lines.len();
        let line = |i: usize| {
            let (a, nin) = self.lines[i];
            let dd = d(i);
            ([a[0] + nin[0] * dd, a[1] + nin[1] * dd], [nin[1], -nin[0]]) // point, direction
        };
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let (p0, d0) = line((i + n - 1) % n);
            let (p1, d1) = line(i);
            let den = d0[0] * d1[1] - d0[1] * d1[0];
            if den.abs() < 1e-9 {
                out.push(p1);
                continue;
            }
            let t = ((p1[0] - p0[0]) * d1[1] - (p1[1] - p0[1]) * d1[0]) / den;
            out.push([p0[0] + d0[0] * t, p0[1] + d0[1] * t]);
        }
        if area(&out) <= 0.0 {
            return None;
        }
        for i in 0..n {
            let (a, b) = (out[i], out[(i + 1) % n]);
            // The edge keeps its direction d = (nin.y, -nin.x).
            let nin = self.lines[i].1;
            let along = (b[0] - a[0]) * nin[1] - (b[1] - a[1]) * nin[0];
            if along < -1e-6 {
                return None;
            }
        }
        // Simple: no two non-adjacent edges cross.
        let ring: Vec<Vec2> = out.iter().map(|p| v2(*p)).collect();
        for i in 0..n {
            for j in i + 2..n {
                if i == 0 && j == n - 1 {
                    continue;
                }
                if crosses(ring[i], ring[(i + 1) % n], ring[j], ring[(j + 1) % n]) {
                    return None;
                }
            }
        }
        Some(out)
    }

    /// `g_i(x)`: distance of `x` inside the eave line of edge `i`.
    fn g(&self, i: usize, p: P2) -> f64 {
        let (a, nin) = self.lines[i];
        (p[0] - a[0]) * nin[0] + (p[1] - a[1]) * nin[1] + self.slopes[i].eave as f64
    }

    /// Linear form of `g_i` as a half-plane `-g_i <= 0` (inside the eave line).
    fn eave_half(&self, i: usize) -> Half {
        let (a, nin) = self.lines[i];
        let e = self.slopes[i].eave as f64;
        Half { a: [-nin[0], -nin[1]], b: a[0] * nin[0] + a[1] * nin[1] - e }
    }

    /// `pitch_i * g_i <= cap`.
    fn cap_half(&self, i: usize) -> Half {
        let (a, nin) = self.lines[i];
        let (p, e) = (self.slopes[i].pitch as f64, self.slopes[i].eave as f64);
        Half { a: [p * nin[0], p * nin[1]], b: p * (e - (a[0] * nin[0] + a[1] * nin[1])) - self.cap as f64 }
    }

    /// Where the plane of `i` is not above the plane of `j`.
    fn bisector(&self, i: usize, j: usize) -> Half {
        if !self.slopes[j].sloped() {
            return self.eave_half(j);
        }
        let lin = |k: usize| {
            let (a, nin) = self.lines[k];
            let p = self.slopes[k].pitch as f64;
            ([p * nin[0], p * nin[1]], p * (self.slopes[k].eave as f64 - (a[0] * nin[0] + a[1] * nin[1])))
        };
        let (ai, bi) = lin(i);
        let (aj, bj) = lin(j);
        Half { a: [ai[0] - aj[0], ai[1] - aj[1]], b: bi - bj }
    }

    /// The strip where edge `j`'s face can lie: from its eave line to its
    /// cap line, a little beyond its ends (just the wall for party edges).
    fn zone(&self, j: usize, ring: &[Vec2]) -> Vec<P2> {
        let n = ring.len();
        let (a, b) = (p2(ring[j]), p2(ring[(j + 1) % n]));
        let nin = self.lines[j].1;
        let d = [nin[1], -nin[0]];
        let s = self.slopes[j];
        let (lo, hi) = if s.sloped() { (-(s.eave as f64), (self.cap / s.pitch) as f64 - s.eave as f64) } else { (-0.01, 0.01) };
        let ext = (hi - lo).max(0.0) * 2.0;
        let at = |p: P2, along: f64, inn: f64| [p[0] + d[0] * along + nin[0] * inn, p[1] + d[1] * along + nin[1] * inn];
        // d runs along the edge, from a to b.
        vec![at(a, -ext, lo), at(b, ext, lo), at(b, ext, hi), at(a, -ext, hi)]
    }

    /// Height above the eave line of the roof at `p` on face `i`.
    pub fn face_height(&self, i: usize, p: Vec2) -> f32 {
        ((self.slopes[i].pitch as f64 * self.g(i, p2(p))) as f32).min(self.cap)
    }

    /// Height of the roof above the eave line at `p`, if `p` is under it.
    pub fn height_at(&self, p: Vec2) -> Option<f32> {
        for (i, f) in &self.faces {
            if inside_convex(f, p) {
                return Some(self.face_height(*i, p));
            }
        }
        self.top.iter().any(|t| crate::building::point_in(t, p)).then_some(self.cap)
    }

    /// The sloped faces in 3D, with the eave line at height `y`.
    pub fn mesh_faces(&self, slopes: &mut Prim, y: f32, scale: f32) {
        for (i, f) in &self.faces {
            let (a, nin) = self.lines[*i];
            let d = Vec2::new(nin[1] as f32, -nin[0] as f32);
            let o = v2(a);
            let pts: Vec<Vec3> = f.iter().map(|p| Vec3::new(p.x, y + self.face_height(*i, *p), p.y)).collect();
            let uv = |q: &Vec3| {
                let q2 = Vec2::new(q.x, q.z);
                Vec2::new((q2 - o).dot(d), (q2 - o).dot(Vec2::new(nin[0] as f32, nin[1] as f32)) * 1.2) / scale
            };
            for k in 1..pts.len() - 1 {
                let (p0, p1, p2_) = (pts[0], pts[k], pts[k + 1]);
                // CCW in (x, z) faces down; flip to face up.
                slopes.tri(p0, p2_, p1, [uv(&p0), uv(&p2_), uv(&p1)]);
            }
        }
    }

    /// The flat top, at `y + cap`.
    pub fn mesh_top(&self, top: &mut Prim, y: f32, top_scale: f32) {
        for t in &self.top {
            top.polygon(t, y + self.cap, true, top_scale);
        }
    }

    /// The roof's section on every party wall above `wall_top` (heights
    /// relative to the eave line): `(a, b, ha, hb)` segments on the wall
    /// line, for gable infills.
    pub fn gables(&self, wall_top: f32) -> Vec<(usize, Vec2, Vec2, f32, f32)> {
        let mut out = Vec::new();
        let n = self.lines.len();
        for j in 0..n {
            if self.slopes[j].sloped() {
                continue;
            }
            let on = |p: Vec2| self.g(j, p2(p)).abs() < 1e-3;
            let mut segs: Vec<(Vec2, Vec2, f32, f32)> = Vec::new();
            for (i, f) in &self.faces {
                for k in 0..f.len() {
                    let (a, b) = (f[k], f[(k + 1) % f.len()]);
                    if on(a) && on(b) && a.distance(b) > 1e-3 {
                        segs.push((a, b, self.face_height(*i, a), self.face_height(*i, b)));
                    }
                }
            }
            for t in &self.top {
                for k in 0..t.len() {
                    let (a, b) = (t[k], t[(k + 1) % t.len()]);
                    if on(a) && on(b) && a.distance(b) > 1e-3 {
                        segs.push((a, b, self.cap, self.cap));
                    }
                }
            }
            for (a, b, ha, hb) in segs {
                if ha.max(hb) > wall_top + 1e-3 {
                    out.push((j, a, b, ha, hb));
                }
            }
        }
        out
    }
}

/// Point inside a convex CCW polygon (boundary included).
fn inside_convex(poly: &[Vec2], p: Vec2) -> bool {
    let n = poly.len();
    (0..n).all(|i| (poly[(i + 1) % n] - poly[i]).perp_dot(p - poly[i]) >= -1e-5)
}

fn crosses(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    let o = |p: Vec2, q: Vec2, r: Vec2| (q - p).perp_dot(r - p);
    let (d1, d2, d3, d4) = (o(c, d, a), o(c, d, b), o(a, b, c), o(a, b, d));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// Gable infill: the part of the roof section `a -> b` (heights `ha`,
/// `hb` above `y`) that rises above `wall_top`, on the wall plane facing
/// `out`.
#[allow(clippy::too_many_arguments)]
pub fn gable(prim: &mut Prim, a: Vec2, b: Vec2, ha: f32, hb: f32, y: f32, wall_top: f32, out: Vec2, scale: f32) {
    let v3 = |p: Vec2, h: f32| Vec3::new(p.x, y + h, p.y);
    // Clip the quad (a@wall_top, b@wall_top, b@hb, a@ha) to above wall_top.
    let mut poly: Vec<(Vec2, f32)> = Vec::new();
    let (wa, wb) = (ha - wall_top, hb - wall_top);
    if wa > 0.0 && wb > 0.0 {
        poly.extend([(a, wall_top), (b, wall_top), (b, hb), (a, ha)]);
    } else if wa > 0.0 {
        let t = wa / (wa - wb);
        poly.extend([(a, wall_top), (a.lerp(b, t), wall_top), (a, ha)]);
    } else if wb > 0.0 {
        let t = wa / (wa - wb);
        poly.extend([(a.lerp(b, t), wall_top), (b, wall_top), (b, hb)]);
    } else {
        return;
    }
    let pts: Vec<Vec3> = poly.iter().map(|(p, h)| v3(*p, *h)).collect();
    let dir = (b - a).normalize_or(Vec2::X);
    let uv = |q: &Vec3| Vec2::new(Vec2::new(q.x, q.z).dot(dir), -q.y) / scale;
    let want = Vec3::new(out.x, 0.0, out.y);
    for k in 1..pts.len() - 1 {
        let (p0, p1, p2_) = (pts[0], pts[k], pts[k + 1]);
        if (p1 - p0).cross(p2_ - p0).dot(want) >= 0.0 {
            prim.tri(p0, p1, p2_, [uv(&p0), uv(&p1), uv(&p2_)]);
        } else {
            prim.tri(p0, p2_, p1, [uv(&p0), uv(&p2_), uv(&p1)]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sq(w: f32, d: f32) -> Vec<Vec2> {
        vec![Vec2::new(0.0, 0.0), Vec2::new(w, 0.0), Vec2::new(w, d), Vec2::new(0.0, d)]
    }

    #[test]
    fn row_house_roof_pitches_to_street_and_patio() {
        // Street at z = 0 (edge 0), party walls at x = 0 and x = 6, patio at z = 8.
        let s = Slope { pitch: 0.42, eave: 0.4 };
        let r = Roof::new(&sq(6.0, 8.0), &[s, Slope::WALL, s, Slope::WALL], 0.42 * 6.0).unwrap();
        // Two faces meeting at a ridge at mid depth, no flat top.
        assert_eq!(r.faces.len(), 2);
        assert!(r.top.is_empty());
        let ridge = r.height_at(Vec2::new(3.0, 4.0)).unwrap();
        assert!((ridge - 0.42 * 4.4).abs() < 1e-3, "{ridge}");
        // Nothing crosses the party walls; eaves overhang street and patio.
        for (_, f) in &r.faces {
            assert!(f.iter().all(|p| p.x >= -1e-4 && p.x <= 6.0 + 1e-4));
        }
        assert!(r.outer.iter().any(|p| p.y < -0.39) && r.outer.iter().any(|p| p.y > 8.39));
        // Gables stand on both party walls.
        let g = r.gables(0.5);
        assert_eq!(g.iter().filter(|s| s.0 == 1).count(), 2);
    }

    #[test]
    fn deep_lot_gets_a_flat_top() {
        let s = Slope { pitch: 0.42, eave: 0.4 };
        let r = Roof::new(&sq(10.0, 20.0), &[s; 4], 0.42 * 4.0).unwrap();
        assert_eq!(r.faces.len(), 4);
        assert_eq!(r.top.len(), 1);
        assert!((r.height_at(Vec2::new(5.0, 10.0)).unwrap() - 0.42 * 4.0).abs() < 1e-4);
    }
}
