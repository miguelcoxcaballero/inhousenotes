//! Pitched roofs on any lot by weighted offsets.
//!
//! Every edge of the lot carries a roof plane that rises from its eave
//! line at the edge's own pitch; a party wall has an infinite pitch (a
//! vertical plane: the roof stops against the neighbour's medianera, as on
//! real row houses). The roof is made of those planes, capped by a flat
//! top at height `cap`:
//!
//! * the face of edge `i` is the strip in front of it between the
//!   weighted bisectors with its two neighbours (hips at convex corners,
//!   valleys at reflex ones), clipped where the plane of any other edge
//!   whose own such strip it overlaps is lower (ridges), and by party walls;
//! * the flat top is what no face covers: the lot offset inwards by each
//!   edge's run.
//!
//! All clipping is of convex polygons by half-planes, in f64. For a convex
//! lot this is exactly the weighted straight-skeleton roof; on concave
//! lots valleys and locality (only edges whose strips overlap) keep far
//! walls from cutting a face. `Roof::new` checks that faces and top tile the eave
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
        // The local face of every edge: its strip between the bisectors
        // with its two neighbours. A far edge only cuts a face that its own
        // local face overlaps, which is exact on convex lots (a true face
        // lies inside its local face) and keeps the planes of far edges of
        // a concave front from cutting in where their wavefront never gets.
        let local: Vec<Vec<P2>> = (0..n).map(|j| roof.local(j, &bbox)).collect();
        let mut covered = 0.0;
        for i in 0..n {
            if !roof.slopes[i].sloped() {
                continue;
            }
            let mut face = local[i].clone();
            // The others nearest first.
            let mid = |k: usize| {
                let (a, b) = (ring[k], ring[(k + 1) % n]);
                (a + b) * 0.5
            };
            let mut order: Vec<usize> = (0..n).filter(|&j| j != i && j != (i + 1) % n && j != (i + n - 1) % n).collect();
            order.sort_by(|&a, &b| mid(a).distance(mid(i)).total_cmp(&mid(b).distance(mid(i))).then(a.cmp(&b)));
            for j in order {
                if face.len() < 3 {
                    break;
                }
                if !overlap(&face, &local[j], -1e-4) {
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

    /// The face edge `j` would have if only its neighbours bounded it: its
    /// strip from the eave line to the cap line between the bisectors with
    /// both neighbours. A party wall's is a thin band along it, a little
    /// longer than the wall so it also bounds the eaves.
    fn local(&self, j: usize, bbox: &[P2]) -> Vec<P2> {
        let n = self.lines.len();
        if !self.slopes[j].sloped() {
            let (a, nin) = self.lines[j];
            let d = [nin[1], -nin[0]];
            let len = {
                let b = self.lines[(j + 1) % n].0;
                ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
            };
            let at = |along: f64, inn: f64| [a[0] + d[0] * along + nin[0] * inn, a[1] + d[1] * along + nin[1] * inn];
            let ext = 0.6;
            return vec![at(-ext, -0.01), at(len + ext, -0.01), at(len + ext, 0.01), at(-ext, 0.01)];
        }
        let mut f = clip(bbox, self.eave_half(j));
        f = clip(&f, self.cap_half(j));
        for k in [(j + n - 1) % n, (j + 1) % n] {
            if f.len() < 3 {
                break;
            }
            f = clip(&f, self.toward(j, self.bisector(j, k)));
        }
        f
    }

    /// `h` turned to keep edge `j`'s own wall: at a convex vertex the
    /// face of `j` is where its plane is the lower (a hip), at a reflex one
    /// where it is the higher (a valley). Both bisectors pass through the
    /// corner of the eave outline, so no other clip is needed there.
    fn toward(&self, j: usize, h: Half) -> Half {
        let n = self.lines.len();
        let (a, nin) = self.lines[j];
        let b = self.lines[(j + 1) % n].0;
        let mid = [(a[0] + b[0]) * 0.5 + nin[0] * 0.01, (a[1] + b[1]) * 0.5 + nin[1] * 0.01];
        if h.eval(mid) > 0.0 { Half { a: [-h.a[0], -h.a[1]], b: -h.b } } else { h }
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

    /// Vertical infills on the party lines: above `wall_top` where the
    /// roof rises over the medianera, and, past the end of a party wall
    /// (a re-entrant corner), between the lower slope that stops on the
    /// wall's line and the higher roof beyond it. Heights are relative to
    /// the eave line.
    pub fn gables(&self, wall_top: f32) -> Vec<Gable> {
        let mut out = Vec::new();
        let n = self.lines.len();
        for j in 0..n {
            if self.slopes[j].sloped() {
                continue;
            }
            let (o, nin) = (v2(self.lines[j].0), v2(self.lines[j].1));
            let d = Vec2::new(nin.y, -nin.x);
            let len = (v2(self.lines[(j + 1) % n].0) - o).dot(d);
            let on = |p: Vec2| (p - o).dot(nin).abs() < 1e-3;
            // Sections of the roof on the line: (s0, s1, h0, h1, side).
            let mut secs: Vec<(f32, f32, f32, f32, f32)> = Vec::new();
            let mut add = |poly: &[Vec2], h: &dyn Fn(Vec2) -> f32| {
                let c = poly.iter().copied().sum::<Vec2>() / poly.len() as f32;
                let side = if (c - o).dot(nin) >= 0.0 { 1.0 } else { -1.0 };
                for k in 0..poly.len() {
                    let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
                    if on(a) && on(b) && a.distance(b) > 1e-3 {
                        let (sa, sb) = ((a - o).dot(d), (b - o).dot(d));
                        let (s0, s1, h0, h1) = if sa < sb { (sa, sb, h(a), h(b)) } else { (sb, sa, h(b), h(a)) };
                        secs.push((s0, s1, h0, h1, side));
                    }
                }
            };
            for (i, f) in &self.faces {
                add(f, &|p| self.face_height(*i, p));
            }
            for t in &self.top {
                add(t, &|_| self.cap);
            }
            let mut cuts: Vec<f32> = secs.iter().flat_map(|s| [s.0, s.1]).chain([0.0, len]).collect();
            cuts.sort_by(f32::total_cmp);
            cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
            for w in cuts.windows(2) {
                let (x0, x1) = (w[0], w[1]);
                let xm = (x0 + x1) * 0.5;
                let at = |s: &(f32, f32, f32, f32, f32), x: f32| s.2 + (s.3 - s.2) * ((x - s.0) / (s.1 - s.0).max(1e-6));
                let here: Vec<&(f32, f32, f32, f32, f32)> = secs.iter().filter(|s| s.0 <= xm && s.1 >= xm).collect();
                if here.is_empty() {
                    continue;
                }
                let hi_of = |x: f32| here.iter().map(|s| at(s, x)).fold(f32::NEG_INFINITY, f32::max);
                let (lo, side) = if xm > 0.0 && xm < len {
                    // On the medianera, facing the neighbour.
                    ([wall_top; 2], -1.0)
                } else if here.len() >= 2 {
                    // Past the wall's end: from the lower section, facing it.
                    let low = here.iter().min_by(|a, b| (at(a, x0) + at(a, x1)).total_cmp(&(at(b, x0) + at(b, x1)))).unwrap();
                    ([at(low, x0), at(low, x1)], low.4)
                } else {
                    continue;
                };
                let hi = [hi_of(x0), hi_of(x1)];
                if hi[0] - lo[0] > 1e-3 || hi[1] - lo[1] > 1e-3 {
                    out.push(Gable { wall: j, a: o + d * x0, b: o + d * x1, lo, hi, out: nin * side });
                }
            }
        }
        out
    }
}

/// A vertical infill on the line of party wall `wall`, from `a` to `b`,
/// between heights `lo` and `hi` (at `a` and at `b`), facing `out`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gable {
    pub wall: usize,
    pub a: Vec2,
    pub b: Vec2,
    pub lo: [f32; 2],
    pub hi: [f32; 2],
    pub out: Vec2,
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

/// Meshes a gable infill with its eave line at height `y` (only where
/// `hi` is above `lo`).
pub fn gable(prim: &mut Prim, g: &Gable, y: f32, scale: f32) {
    let v3 = |p: Vec2, h: f32| Vec3::new(p.x, y + h, p.y);
    let (da, db) = (g.hi[0] - g.lo[0], g.hi[1] - g.lo[1]);
    let poly: Vec<Vec3> = if da > 0.0 && db > 0.0 {
        vec![v3(g.a, g.lo[0]), v3(g.b, g.lo[1]), v3(g.b, g.hi[1]), v3(g.a, g.hi[0])]
    } else if da > 0.0 {
        let t = da / (da - db);
        vec![v3(g.a, g.lo[0]), v3(g.a.lerp(g.b, t), g.lo[0] + (g.lo[1] - g.lo[0]) * t), v3(g.a, g.hi[0])]
    } else if db > 0.0 {
        let t = da / (da - db);
        vec![v3(g.a.lerp(g.b, t), g.lo[0] + (g.lo[1] - g.lo[0]) * t), v3(g.b, g.lo[1]), v3(g.b, g.hi[1])]
    } else {
        return;
    };
    let dir = (g.b - g.a).normalize_or(Vec2::X);
    let uv = |q: &Vec3| Vec2::new(Vec2::new(q.x, q.z).dot(dir), -q.y) / scale;
    let want = Vec3::new(g.out.x, 0.0, g.out.y);
    for k in 1..poly.len() - 1 {
        let (p0, p1, p2_) = (poly[0], poly[k], poly[k + 1]);
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
        assert_eq!(g.iter().filter(|s| s.wall == 1).count(), 2);
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
