//! Facades that follow organic plans. A *run* is a chain of consecutive
//! plan edges that turn by less than a tolerance at every shared vertex: a
//! curved street front digitised by OSM as many short facets, a round
//! tower, the drum of a dome. A run is laid out as one developed (unrolled)
//! facade: openings, mouldings and details are generated flat, in
//! `(u = arc length, v = height, z = outward)` coordinates, and then *bent*
//! onto the real polyline by [`Bend`]. Triangles are split along every
//! vertical crease first, so a window straddling a facet boundary gets a
//! folded reveal, a balcony follows the curve, and nothing is skipped
//! because OSM happened to put a node behind it.
//!
//! The same arc-length view drives repeated details along plan polylines
//! ([`Path2`]): corbels, merlons, balusters and gargoyles are spaced along
//! the whole curve, not per facet.

use forja_math::{Vec2, Vec3};

use super::mesh::Geo;
use super::poly;
use crate::geom::Prim;

/// Mapping from a flat facade frame onto a plan polyline.
///
/// Flat coordinates: `x` = arc length along the polyline (in facade order,
/// left to right seen from outside), `y` = height, `z` = distance in front
/// of the wall (negative = into the wall). The polyline is offset with
/// mitred joins, so a plane at depth `z` maps onto the exact parallel
/// offset curve: reveals and through walls stay consistent.
#[derive(Clone, Debug)]
pub struct Bend {
    /// Points in facade order (closed runs repeat the first at the end).
    pub pts: Vec<Vec2>,
    /// Arc length at each point.
    pub s: Vec<f32>,
    /// Mitred unit-distance offsets (n · miter = 1 for both neighbours).
    miter: Vec<Vec2>,
    /// Shading tangent at each vertex for the segment before / after it.
    tan_in: Vec<Vec2>,
    tan_out: Vec<Vec2>,
    pub closed: bool,
}

/// Unit tangent and outward normal of a facade-order segment.
fn seg_frame(a: Vec2, b: Vec2) -> (Vec2, Vec2) {
    let t = (b - a).normalize_or(Vec2::X);
    (t, Vec2::new(-t.y, t.x))
}

impl Bend {
    /// `pts` in facade order (u increasing). Vertices turning less than
    /// `smooth_deg` get smooth shading; sharper ones stay crisp.
    pub fn new(pts: &[Vec2], closed: bool, smooth_deg: f32) -> Option<Bend> {
        let mut p: Vec<Vec2> = Vec::with_capacity(pts.len() + 1);
        for q in pts {
            if p.last().is_none_or(|l: &Vec2| (*l - *q).length() > 1e-5) {
                p.push(*q);
            }
        }
        if closed {
            while p.len() > 2 && (p[0] - p[p.len() - 1]).length() <= 1e-5 {
                p.pop();
            }
            if p.len() < 3 {
                return None;
            }
            p.push(p[0]);
        } else if p.len() < 2 {
            return None;
        }
        let m = p.len();
        let segs = m - 1;
        let frames: Vec<(Vec2, Vec2)> = (0..segs).map(|i| seg_frame(p[i], p[i + 1])).collect();
        let mut s = vec![0.0f32; m];
        for i in 0..segs {
            s[i + 1] = s[i] + (p[i + 1] - p[i]).length();
        }
        let cos_smooth = smooth_deg.to_radians().cos();
        let mut miter = vec![Vec2::ZERO; m];
        let mut tan_in = vec![Vec2::ZERO; m];
        let mut tan_out = vec![Vec2::ZERO; m];
        for j in 0..m {
            // Segments meeting at vertex j.
            let before = if j > 0 { Some(j - 1) } else if closed { Some(segs - 1) } else { None };
            let after = if j < segs { Some(j) } else if closed { Some(0) } else { None };
            match (before, after) {
                (Some(a), Some(b)) => {
                    let (ta, na) = frames[a];
                    let (tb, nb) = frames[b];
                    let k = (1.0 + na.dot(nb)).max(0.2);
                    miter[j] = (na + nb) / k;
                    if ta.dot(tb) >= cos_smooth {
                        let t = (ta + tb).normalize_or(tb);
                        tan_in[j] = t;
                        tan_out[j] = t;
                    } else {
                        tan_in[j] = ta;
                        tan_out[j] = tb;
                    }
                }
                (Some(a), None) => {
                    miter[j] = frames[a].1;
                    tan_in[j] = frames[a].0;
                    tan_out[j] = frames[a].0;
                }
                (None, Some(b)) => {
                    miter[j] = frames[b].1;
                    tan_in[j] = frames[b].0;
                    tan_out[j] = frames[b].0;
                }
                (None, None) => {}
            }
        }
        Some(Bend { pts: p, s, miter, tan_in, tan_out, closed })
    }

    pub fn len(&self) -> f32 {
        *self.s.last().unwrap_or(&0.0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() <= 0.0
    }

    pub fn segments(&self) -> usize {
        self.pts.len() - 1
    }

    /// Wraps `x` into `[0, len)` for closed runs (identity otherwise).
    pub fn wrap(&self, x: f32) -> f32 {
        if self.closed { x.rem_euclid(self.len().max(1e-6)) } else { x }
    }

    /// Segment holding arc length `x` (clamped to the ends).
    pub fn locate(&self, x: f32) -> usize {
        let n = self.segments();
        match self.s[1..n].binary_search_by(|v| v.total_cmp(&x)) {
            Ok(i) => i + 1,
            Err(i) => i,
        }
        .min(n - 1)
    }

    /// Point on the polyline at arc length `x` and outward normal (smooth
    /// where the vertex is smooth).
    pub fn frame(&self, x: f32) -> (Vec2, Vec2) {
        let x = self.wrap(x);
        let i = self.locate(x);
        let len = (self.s[i + 1] - self.s[i]).max(1e-6);
        let tau = ((x - self.s[i]) / len).clamp(0.0, 1.0);
        let (t, _) = seg_frame(self.pts[i], self.pts[i + 1]);
        let p = self.pts[i] + t * (x - self.s[i]);
        let ts = self.tan_out[i].lerp(self.tan_in[i + 1], tau).normalize_or(t);
        (p, Vec2::new(-ts.y, ts.x))
    }

    /// Plan normal of the segment holding `x` (exact, faceted).
    pub fn seg_normal(&self, x: f32) -> Vec2 {
        let i = self.locate(self.wrap(x));
        seg_frame(self.pts[i], self.pts[i + 1]).1
    }

    /// World position of flat `(x, y, z)` on segment `i`.
    fn point_on(&self, i: usize, x: f32, y: f32, z: f32) -> Vec3 {
        let len = (self.s[i + 1] - self.s[i]).max(1e-6);
        let (t, _) = seg_frame(self.pts[i], self.pts[i + 1]);
        let tau = ((x - self.s[i]) / len).clamp(0.0, 1.0);
        let base = self.pts[i] + t * (x - self.s[i]);
        let off = self.miter[i].lerp(self.miter[i + 1], tau);
        let q = base + off * z;
        Vec3::new(q.x, y, q.y)
    }

    /// World normal of a flat normal at `(x, z)` on segment `i`, through
    /// the cofactor of the mapping's Jacobian (shading tangents smooth).
    fn normal_on(&self, i: usize, x: f32, z: f32, n: Vec3) -> Vec3 {
        let len = (self.s[i + 1] - self.s[i]).max(1e-6);
        let raw = (x - self.s[i]) / len;
        let tau = raw.clamp(0.0, 1.0);
        let (t, _) = seg_frame(self.pts[i], self.pts[i + 1]);
        let ts = self.tan_out[i].lerp(self.tan_in[i + 1], tau).normalize_or(t);
        let dm = if (0.0..=1.0).contains(&raw) { (self.miter[i + 1] - self.miter[i]) / len } else { Vec2::ZERO };
        let a2 = ts + dm * z;
        // Smooth offsets follow the shading tangent.
        let off = Vec2::new(-ts.y, ts.x) * self.miter[i].lerp(self.miter[i + 1], tau).length();
        let a = Vec3::new(a2.x, 0.0, a2.y);
        let b = Vec3::Y;
        let c = Vec3::new(off.x, 0.0, off.y);
        (b.cross(c) * n.x + c.cross(a) * n.y + a.cross(b) * n.z).normalize_or(Vec3::new(off.x, 0.0, off.y))
    }

    /// Split positions of a closed or open run inside `(lo, hi)`.
    fn creases(&self, lo: f32, hi: f32, out: &mut Vec<f32>) {
        out.clear();
        let eps = 1e-4;
        let n = self.segments();
        if self.closed {
            let l = self.len().max(1e-6);
            let k0 = (lo / l).floor() as i64;
            let k1 = (hi / l).floor() as i64;
            for k in k0..=k1 {
                for j in 0..n {
                    let c = self.s[j] + k as f32 * l;
                    if c > lo + eps && c < hi - eps {
                        out.push(c);
                    }
                }
            }
        } else {
            for j in 1..n {
                let c = self.s[j];
                if c > lo + eps && c < hi - eps {
                    out.push(c);
                }
            }
        }
        out.sort_by(|a, b| a.total_cmp(b));
    }

    /// Bends flat geometry onto the polyline, appending to `dst`.
    pub fn warp(&self, src: &Prim, dst: &mut Prim) {
        #[derive(Clone, Copy)]
        struct V {
            p: Vec3,
            n: Vec3,
            uv: Vec2,
        }
        let lerp = |a: V, b: V, t: f32| V { p: a.p.lerp(b.p, t), n: a.n.lerp(b.n, t), uv: a.uv.lerp(b.uv, t) };
        let l = self.len().max(1e-6);
        let mut cuts = Vec::new();
        let emit = |dst: &mut Prim, tri: [V; 3]| {
            let cx = (tri[0].p.x + tri[1].p.x + tri[2].p.x) / 3.0;
            let (shift, xc) = if self.closed {
                let k = (cx / l).floor();
                (k * l, cx - k * l)
            } else {
                (0.0, cx)
            };
            let i = self.locate(xc);
            let base = dst.positions.len() as u32;
            for v in tri {
                let x = v.p.x - shift;
                dst.positions.push(self.point_on(i, x, v.p.y, v.p.z));
                dst.normals.push(self.normal_on(i, x, v.p.z, v.n));
                dst.uvs.push(v.uv);
            }
            dst.indices.extend([base, base + 1, base + 2]);
        };
        for t in src.indices.chunks_exact(3) {
            let tri = [0, 1, 2].map(|k| {
                let i = t[k] as usize;
                V { p: src.positions[i], n: src.normals[i], uv: src.uvs[i] }
            });
            let lo = tri.iter().map(|v| v.p.x).fold(f32::MAX, f32::min);
            let hi = tri.iter().map(|v| v.p.x).fold(f32::MIN, f32::max);
            self.creases(lo, hi, &mut cuts);
            if cuts.is_empty() {
                emit(dst, tri);
                continue;
            }
            // Slice the triangle along each crease (x = c), left to right.
            let mut rest: Vec<V> = tri.to_vec();
            for &c in &cuts {
                let mut left = Vec::with_capacity(rest.len() + 2);
                let mut right = Vec::with_capacity(rest.len() + 2);
                let m = rest.len();
                for k in 0..m {
                    let (a, b) = (rest[k], rest[(k + 1) % m]);
                    let (da, db) = (a.p.x - c, b.p.x - c);
                    if da <= 0.0 {
                        left.push(a);
                    }
                    if da >= 0.0 {
                        right.push(a);
                    }
                    if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
                        let v = lerp(a, b, da / (da - db));
                        left.push(v);
                        right.push(v);
                    }
                }
                for k in 1..left.len().saturating_sub(1) {
                    emit(dst, [left[0], left[k], left[k + 1]]);
                }
                rest = right;
                if rest.len() < 3 {
                    break;
                }
            }
            for k in 1..rest.len().saturating_sub(1) {
                emit(dst, [rest[0], rest[k], rest[k + 1]]);
            }
        }
    }

    /// Bends every material of `src` into `dst`.
    pub fn warp_geo(&self, src: &Geo, dst: &mut Geo) {
        for k in &src.order {
            let p = &src.prims[k];
            if p.indices.is_empty() {
                continue;
            }
            self.warp(p, dst.get(*k));
        }
    }
}

/// Vertices of a ring where the plan turns by more than `tol_deg`
/// (absolute turn, convex or reflex).
pub fn corners(ring: &[Vec2], closed: bool, tol_deg: f32) -> Vec<bool> {
    let n = ring.len();
    let cos_tol = tol_deg.clamp(0.0, 179.0).to_radians().cos();
    (0..n)
        .map(|i| {
            if !closed && (i == 0 || i + 1 == n) {
                return true;
            }
            let a = ring[(i + n - 1) % n];
            let b = ring[i];
            let c = ring[(i + 1) % n];
            let (d0, d1) = ((b - a).normalize_or(Vec2::X), (c - b).normalize_or(Vec2::X));
            d0.dot(d1) < cos_tol
        })
        .collect()
}

/// A plan polyline measured by arc length, split into smooth runs at
/// sharp corners: the frame for details spaced along curved plans.
pub struct Path2 {
    pub pts: Vec<Vec2>,
    pub closed: bool,
    /// Arc length at each point (`pts.len() + 1` entries when closed).
    pub s: Vec<f32>,
    /// `(start, end)` arc lengths of each smooth run.
    pub runs: Vec<(f32, f32)>,
}

impl Path2 {
    pub fn new(pts: &[Vec2], closed: bool, tol_deg: f32) -> Path2 {
        let m = pts.len();
        let segs = if closed { m } else { m.saturating_sub(1) };
        let mut s = vec![0.0f32];
        for i in 0..segs {
            let d = s[i] + (pts[(i + 1) % m] - pts[i]).length();
            s.push(d);
        }
        let total = *s.last().unwrap();
        let cor = corners(pts, closed, tol_deg);
        let ids: Vec<usize> = (0..m).filter(|&i| cor[i]).collect();
        let mut runs = Vec::new();
        if ids.is_empty() {
            runs.push((0.0, total));
        } else if closed {
            for k in 0..ids.len() {
                let a = s[ids[k]];
                let b = if k + 1 < ids.len() { s[ids[k + 1]] } else { total + s[ids[0]] };
                if b > a + 1e-4 {
                    runs.push((a, b));
                }
            }
        } else {
            for w in ids.windows(2) {
                if s[w[1]] > s[w[0]] + 1e-4 {
                    runs.push((s[w[0]], s[w[1]]));
                }
            }
        }
        Path2 { pts: pts.to_vec(), closed, s, runs }
    }

    pub fn total(&self) -> f32 {
        *self.s.last().unwrap_or(&0.0)
    }

    /// A closed path without corners (round towers, drums): one endless run.
    pub fn full_loop(&self) -> bool {
        self.closed && self.runs.len() == 1 && self.runs[0].0 == 0.0 && (self.runs[0].1 - self.total()).abs() < 1e-4 && corners(&self.pts, true, 25.0).iter().all(|c| !c)
    }

    /// Point, outward normal `(d.y, -d.x)` and tangent at arc length `d`.
    pub fn at(&self, d: f32) -> (Vec2, Vec2, Vec2) {
        let m = self.pts.len();
        let total = self.total().max(1e-6);
        let d = if self.closed { d.rem_euclid(total) } else { d.clamp(0.0, total) };
        let segs = self.s.len() - 1;
        let mut j = match self.s[1..].binary_search_by(|v| v.total_cmp(&d)) {
            Ok(i) | Err(i) => i,
        };
        j = j.min(segs - 1);
        let (a, b) = (self.pts[j], self.pts[(j + 1) % m]);
        let len = (self.s[j + 1] - self.s[j]).max(1e-6);
        let t = ((d - self.s[j]) / len).clamp(0.0, 1.0);
        let tan = (b - a).normalize_or(Vec2::X);
        (a.lerp(b, t), Vec2::new(tan.y, -tan.x), tan)
    }

    /// Arc lengths of items every `spacing` metres along each run, keeping
    /// `margin` from the run ends (centred, at least one per run that fits).
    pub fn spaced(&self, spacing: f32, margin: f32) -> Vec<f32> {
        let mut out = Vec::new();
        let sp = spacing.max(0.05);
        if self.full_loop() {
            let l = self.total();
            let n = ((l / sp).round() as usize).max(1);
            return (0..n).map(|k| l * (k as f32 + 0.5) / n as f32).collect();
        }
        for &(a, b) in &self.runs {
            let usable = b - a - 2.0 * margin;
            if usable < 0.0 {
                if b - a > 1e-3 && margin > 0.0 && b - a >= sp * 0.5 {
                    out.push((a + b) * 0.5);
                }
                continue;
            }
            let count = (usable / sp).floor() as usize + 1;
            let step = if count > 1 { usable / (count - 1) as f32 } else { 0.0 };
            for k in 0..count {
                out.push(if count == 1 { (a + b) * 0.5 } else { a + margin + step * k as f32 });
            }
        }
        out
    }

    /// `count` evenly spaced items over each run, half a step from the ends.
    pub fn divided(&self, step: f32) -> Vec<(f32, f32)> {
        let mut out = Vec::new();
        for &(a, b) in &self.runs {
            let n = ((b - a) / step.max(0.05)).round().max(1.0) as usize;
            let w = (b - a) / n as f32;
            for k in 0..n {
                out.push((a + w * (k as f32 + 0.5), w));
            }
        }
        out
    }

    /// Corner points (run ends) with their mitre normals.
    pub fn corner_points(&self) -> Vec<(Vec2, Vec2)> {
        let m = self.pts.len();
        let cor = corners(&self.pts, self.closed, 25.0);
        (0..m)
            .filter(|&i| cor[i])
            .map(|i| {
                let n0 = if i > 0 || self.closed { Some(poly_normal(self.pts[(i + m - 1) % m], self.pts[i])) } else { None };
                let n1 = if i + 1 < m || self.closed { Some(poly_normal(self.pts[i], self.pts[(i + 1) % m])) } else { None };
                let n = match (n0, n1) {
                    (Some(a), Some(b)) => (a + b).normalize_or(b),
                    (Some(a), None) => a,
                    (None, Some(b)) => b,
                    _ => Vec2::Y,
                };
                (self.pts[i], n)
            })
            .collect()
    }
}

fn poly_normal(a: Vec2, b: Vec2) -> Vec2 {
    let d = (b - a).normalize_or(Vec2::X);
    Vec2::new(d.y, -d.x)
}

/// Distance along a plan ray to the nearest crossing of any ring of
/// `shape` (ignoring hits closer than `min`).
pub fn ray_hit(shape: &poly::Shape, o: Vec2, d: Vec2, min: f32) -> Option<(f32, usize, usize, f32)> {
    let mut best: Option<(f32, usize, usize, f32)> = None;
    for (r, (ring, _)) in shape.rings().enumerate() {
        let n = ring.len();
        for i in 0..n {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            let e = b - a;
            let den = d.perp_dot(e);
            if den.abs() < 1e-9 {
                continue;
            }
            let w = a - o;
            let t = w.perp_dot(e) / den;
            let u = w.perp_dot(d) / den;
            if t > min && (-1e-5..=1.0 + 1e-5).contains(&u) && best.is_none_or(|bb| t < bb.0) {
                best = Some((t, r, i, u.clamp(0.0, 1.0)));
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::landmark::mesh::{self, Plane};
    use crate::landmark::poly::Shape;

    #[test]
    fn straight_bend_is_the_plane() {
        // One segment from (10, 0) to (0, 0): u runs west, outward north (-z).
        let b = Bend::new(&[Vec2::new(10.0, 0.0), Vec2::new(0.0, 0.0)], false, 20.0).unwrap();
        let mut flat = Prim::default();
        mesh::planar_shape(&mut flat, &Shape::new(poly::rect(Vec2::new(5.0, 2.0), Vec2::new(10.0, 4.0), 0.0)), &Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y), false, Vec2::ZERO);
        let mut out = Prim::default();
        b.warp(&flat, &mut out);
        assert_eq!(out.tri_count(), flat.tri_count());
        for (p, n) in out.positions.iter().zip(&out.normals) {
            assert!(p.z.abs() < 1e-5 && (0.0..=10.0).contains(&p.x));
            assert!((n.z + 1.0).abs() < 1e-4, "{n:?}");
        }
    }

    #[test]
    fn bent_plane_splits_at_creases_and_keeps_area() {
        // Quarter circle of 8 facets.
        let pts: Vec<Vec2> = (0..=8).map(|k| Vec2::from_angle(k as f32 / 8.0 * std::f32::consts::FRAC_PI_2) * 10.0).collect();
        let b = Bend::new(&pts, false, 20.0).unwrap();
        let l = b.len();
        let mut flat = Prim::default();
        mesh::planar_shape(&mut flat, &Shape::new(poly::rect(Vec2::new(l * 0.5, 3.0), Vec2::new(l, 6.0), 0.0)), &Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y), false, Vec2::ZERO);
        let mut out = Prim::default();
        b.warp(&flat, &mut out);
        let area: f32 = out.indices.chunks_exact(3).map(|t| (out.positions[t[1] as usize] - out.positions[t[0] as usize]).cross(out.positions[t[2] as usize] - out.positions[t[0] as usize]).length() * 0.5).sum();
        assert!((area - l * 6.0).abs() < 1e-2, "{area} vs {}", l * 6.0);
        // Every vertex lies on the arc's polygon (radius between the apothem and 10).
        for p in &out.positions {
            let r = Vec2::new(p.x, p.z).length();
            assert!((9.8..=10.001).contains(&r), "{r}");
        }
        // Smooth normals point away from the centre... or toward it,
        // consistently with the facade's outward side.
        let sgn = out.normals.iter().zip(&out.positions).map(|(n, p)| n.x * p.x + n.z * p.z).sum::<f32>().signum();
        assert!(out.normals.iter().zip(&out.positions).all(|(n, p)| (n.x * p.x + n.z * p.z) * sgn > 0.0));
    }

    #[test]
    fn closed_run_wraps_across_the_seam() {
        let pts = poly::regular(Vec2::ZERO, 5.0, 24, 0.0);
        let b = Bend::new(&pts, true, 20.0).unwrap();
        let l = b.len();
        // A window straddling the seam, built at u in [l - 0.5, l + 0.5].
        let mut flat = Prim::default();
        mesh::planar_shape(&mut flat, &Shape::new(poly::rect(Vec2::new(l, 2.0), Vec2::new(1.0, 1.0), 0.0)), &Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y), false, Vec2::ZERO);
        let mut out = Prim::default();
        b.warp(&flat, &mut out);
        for p in &out.positions {
            let r = Vec2::new(p.x, p.z).length();
            assert!((4.9..=5.001).contains(&r), "{r}");
            assert!((p - Vec3::new(pts[0].x, p.y, pts[0].y)).length() < 0.6);
        }
    }

    #[test]
    fn path_spacing_follows_the_curve() {
        let pts: Vec<Vec2> = (0..=20).map(|k| Vec2::from_angle(k as f32 / 20.0 * std::f32::consts::PI) * 10.0).collect();
        let p = Path2::new(&pts, false, 25.0);
        assert_eq!(p.runs.len(), 1, "a smooth arc is one run");
        let items = p.spaced(2.0, 0.5);
        assert_eq!(items.len(), ((p.total() - 1.0) / 2.0).floor() as usize + 1);
        let sq = Path2::new(&poly::rect(Vec2::ZERO, Vec2::new(10.0, 10.0), 0.0), true, 25.0);
        assert_eq!(sq.runs.len(), 4);
    }

    #[test]
    fn rays_hit_the_opposite_wall() {
        let s = Shape::new(poly::rect(Vec2::ZERO, Vec2::new(10.0, 4.0), 0.0));
        let (t, _, _, _) = ray_hit(&s, Vec2::new(0.0, -2.0), Vec2::new(0.0, 1.0), 1e-3).unwrap();
        assert!((t - 4.0).abs() < 1e-4);
    }
}
