//! Open polylines with arc length, and smooth 1D value noise.
//!
//! Directions follow the ring convention of the whole crate: in `(x, z)`,
//! the *left* of a direction `d` is `(-d.z, d.x)` (the inside of a
//! counter-clockwise ring) and the right is `(d.z, -d.x)` (the outward
//! normal of a ring edge).

use forja_math::Vec2;

/// Left normal of a direction.
pub fn left(d: Vec2) -> Vec2 {
    Vec2::new(-d.y, d.x)
}

/// An open polyline with cumulative arc length.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Polyline {
    pts: Vec<Vec2>,
    s: Vec<f32>,
}

impl Polyline {
    pub fn new(pts: Vec<Vec2>) -> Self {
        let mut s = Vec::with_capacity(pts.len());
        let mut acc = 0.0f32;
        for (i, p) in pts.iter().enumerate() {
            if i > 0 {
                acc += pts[i - 1].distance(*p);
            }
            s.push(acc);
        }
        Polyline { pts, s }
    }

    pub fn pts(&self) -> &[Vec2] {
        &self.pts
    }

    /// Arc length at each vertex.
    pub fn stations(&self) -> &[f32] {
        &self.s
    }

    pub fn len(&self) -> f32 {
        self.s.last().copied().unwrap_or(0.0)
    }

    pub fn is_empty(&self) -> bool {
        self.pts.len() < 2
    }

    /// Segment index containing arc length `s` and the parameter in it.
    pub fn locate(&self, s: f32) -> (usize, f32) {
        let n = self.pts.len();
        if n < 2 {
            return (0, 0.0);
        }
        let s = s.clamp(0.0, self.len());
        let i = match self.s.binary_search_by(|v| v.total_cmp(&s)) {
            Ok(i) => i.min(n - 2),
            Err(i) => i.saturating_sub(1).min(n - 2),
        };
        let l = self.s[i + 1] - self.s[i];
        (i, if l > 0.0 { (s - self.s[i]) / l } else { 0.0 })
    }

    pub fn at(&self, s: f32) -> Vec2 {
        if self.pts.len() < 2 {
            return self.pts.first().copied().unwrap_or(Vec2::ZERO);
        }
        let (i, t) = self.locate(s);
        self.pts[i].lerp(self.pts[i + 1], t)
    }

    /// Unit direction of the segment at `s`.
    pub fn tangent(&self, s: f32) -> Vec2 {
        if self.pts.len() < 2 {
            return Vec2::X;
        }
        let (i, _) = self.locate(s);
        (self.pts[i + 1] - self.pts[i]).normalize_or(Vec2::X)
    }

    /// Left normal at `s`.
    pub fn normal(&self, s: f32) -> Vec2 {
        left(self.tangent(s))
    }

    /// Mitred normal at vertex `i` (unit length times the miter factor,
    /// capped at 2), pointing left.
    pub fn vertex_normal(&self, i: usize) -> Vec2 {
        let n = self.pts.len();
        if n < 2 {
            return Vec2::Y;
        }
        let seg = |k: usize| left((self.pts[k + 1] - self.pts[k]).normalize_or(Vec2::X));
        if i == 0 {
            return seg(0);
        }
        if i >= n - 1 {
            return seg(n - 2);
        }
        let (a, b) = (seg(i - 1), seg(i));
        let m = (a + b).normalize_or(b);
        m / m.dot(b).max(0.5)
    }

    /// Parallel polyline at signed distance `o` (positive to the left).
    pub fn offset(&self, o: f32) -> Polyline {
        Polyline::new((0..self.pts.len()).map(|i| self.pts[i] + self.vertex_normal(i) * o).collect())
    }

    /// Arc length of the closest point and the signed offset of `p` (+ left).
    pub fn project(&self, p: Vec2) -> (f32, f32) {
        let mut best = (f32::INFINITY, 0.0f32, 0.0f32);
        for i in 0..self.pts.len().saturating_sub(1) {
            let (a, b) = (self.pts[i], self.pts[i + 1]);
            let e = b - a;
            let l2 = e.length_squared();
            let t = if l2 > 0.0 { ((p - a).dot(e) / l2).clamp(0.0, 1.0) } else { 0.0 };
            let q = a + e * t;
            let d2 = q.distance_squared(p);
            if d2 < best.0 {
                let side = e.perp_dot(p - a).signum();
                best = (d2, self.s[i] + t * l2.sqrt(), side * d2.sqrt());
            }
        }
        (best.1, best.2)
    }

    /// Splits segments longer than `max_chord` evenly; every vertex stays.
    pub fn resample(&self, max_chord: f32) -> Polyline {
        let mut out = Vec::with_capacity(self.pts.len());
        for i in 0..self.pts.len() {
            if i > 0 {
                let (a, b) = (self.pts[i - 1], self.pts[i]);
                let k = (a.distance(b) / max_chord).ceil().max(1.0) as usize;
                for j in 1..k {
                    out.push(a.lerp(b, j as f32 / k as f32));
                }
            }
            out.push(self.pts[i]);
        }
        Polyline::new(out)
    }

    /// The piece between arc lengths `s0 < s1`.
    pub fn sub(&self, s0: f32, s1: f32) -> Polyline {
        let (s0, s1) = (s0.clamp(0.0, self.len()), s1.clamp(0.0, self.len()));
        let mut out = vec![self.at(s0)];
        for (i, s) in self.s.iter().enumerate() {
            if *s > s0 && *s < s1 {
                out.push(self.pts[i]);
            }
        }
        out.push(self.at(s1));
        Polyline::new(out)
    }

    pub fn reversed(&self) -> Polyline {
        Polyline::new(self.pts.iter().rev().copied().collect())
    }

    /// Chaikin corner cutting, endpoints fixed.
    pub fn chaikin(&self, n: u32) -> Polyline {
        let mut p = self.pts.clone();
        for _ in 0..n {
            if p.len() < 3 {
                break;
            }
            let mut q = Vec::with_capacity(p.len() * 2);
            q.push(p[0]);
            for w in p.windows(2) {
                q.push(w[0].lerp(w[1], 0.25));
                q.push(w[0].lerp(w[1], 0.75));
            }
            q.push(*p.last().unwrap());
            // Drop the two points that hug the fixed ends.
            q.remove(1);
            q.remove(q.len() - 2);
            p = q;
        }
        Polyline::new(p)
    }

    /// Ramer–Douglas–Peucker, endpoints kept.
    pub fn simplify(&self, eps: f32) -> Polyline {
        let n = self.pts.len();
        if n < 3 {
            return self.clone();
        }
        let mut keep = vec![false; n];
        keep[0] = true;
        keep[n - 1] = true;
        let mut stack = vec![(0usize, n - 1)];
        while let Some((a, b)) = stack.pop() {
            let (pa, pb) = (self.pts[a], self.pts[b]);
            let mut best = (0.0f32, a);
            for i in a + 1..b {
                let d = seg_dist(self.pts[i], pa, pb);
                if d > best.0 {
                    best = (d, i);
                }
            }
            if best.0 > eps {
                keep[best.1] = true;
                stack.push((a, best.1));
                stack.push((best.1, b));
            }
        }
        Polyline::new((0..n).filter(|&i| keep[i]).map(|i| self.pts[i]).collect())
    }
}

/// The region swept by a polyline with half width `half(i)` at vertex `i`:
/// a quad per segment, round joins at bends, square (butt) ends.
pub fn stroke(line: &[Vec2], half: impl Fn(usize) -> f32) -> super::Region {
    let rings = stroke_rings(line, half);
    super::Region::rings(rings.iter().map(|r| r.as_slice()))
}

/// The pieces of `stroke`, for callers that unite many strokes at once.
pub fn stroke_rings(line: &[Vec2], half: impl Fn(usize) -> f32) -> Vec<Vec<Vec2>> {
    let mut rings: Vec<Vec<Vec2>> = Vec::new();
    for k in 0..line.len().saturating_sub(1) {
        let (h0, h1) = (half(k), half(k + 1));
        if h0 <= 0.0 && h1 <= 0.0 {
            continue;
        }
        let nl = left((line[k + 1] - line[k]).normalize_or(Vec2::X));
        rings.push(vec![line[k] - nl * h0, line[k + 1] - nl * h1, line[k + 1] + nl * h1, line[k] + nl * h0]);
    }
    for k in 1..line.len().saturating_sub(1) {
        let h = half(k);
        let (d0, d1) = ((line[k] - line[k - 1]).normalize_or(Vec2::X), (line[k + 1] - line[k]).normalize_or(Vec2::X));
        if h <= 0.0 || d0.dot(d1) > 0.99995 {
            continue;
        }
        // A fan over the outer side of the bend, on the arc of radius h.
        let side = if d0.perp_dot(d1) > 0.0 { -1.0 } else { 1.0 };
        let (a0, a1) = ((left(d0) * side).to_angle(), (left(d1) * side).to_angle());
        let mut da = a1 - a0;
        while da > std::f32::consts::PI {
            da -= std::f32::consts::TAU;
        }
        while da < -std::f32::consts::PI {
            da += std::f32::consts::TAU;
        }
        let n = ((da.abs() / 0.2).ceil() as usize).max(1);
        let mut fan = vec![line[k]];
        for i in 0..=n {
            fan.push(line[k] + Vec2::from_angle(a0 + da * i as f32 / n as f32) * h);
        }
        rings.push(fan);
    }
    rings
}

/// Distance from `p` to segment `ab`.
pub fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let e = b - a;
    let l2 = e.length_squared();
    let t = if l2 > 0.0 { ((p - a).dot(e) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (a + e * t).distance(p)
}

/// SplitMix64 finaliser.
pub fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Smooth value noise in [-1, 1] with unit feature size (cubic fade
/// between hashed lattice values).
pub fn noise1(seed: u64, x: f32) -> f32 {
    let i = x.floor();
    let f = x - i;
    let h = |k: i64| (mix64(seed ^ (k as u64).wrapping_mul(0xA24B_AED4_963E_E407)) >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0;
    let (a, b) = (h(i as i64), h(i as i64 + 1));
    let u = f * f * (3.0 - 2.0 * f);
    a + (b - a) * u
}

/// Smooth 2D value noise in [-1, 1].
pub fn noise2(seed: u64, p: Vec2) -> f32 {
    let (i, j) = (p.x.floor(), p.y.floor());
    let (fx, fy) = (p.x - i, p.y - j);
    let h = |a: i64, b: i64| {
        let k = mix64(seed ^ (a as u64).wrapping_mul(0xA24B_AED4_963E_E407) ^ (b as u64).wrapping_mul(0x9FB2_1C65_1E98_DF25));
        (k >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
    };
    let (i, j) = (i as i64, j as i64);
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = h(i, j) + (h(i + 1, j) - h(i, j)) * ux;
    let b = h(i, j + 1) + (h(i + 1, j + 1) - h(i, j + 1)) * ux;
    a + (b - a) * uy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_length_projection_and_offset() {
        let p = Polyline::new(vec![Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)]);
        assert_eq!(p.len(), 20.0);
        assert_eq!(p.at(15.0), Vec2::new(10.0, 5.0));
        let (s, o) = p.project(Vec2::new(4.0, 2.0));
        assert!((s - 4.0).abs() < 1e-5 && (o - 2.0).abs() < 1e-5, "left is +");
        let (_, o) = p.project(Vec2::new(4.0, -1.0));
        assert!((o + 1.0).abs() < 1e-5);
        let off = p.offset(1.0);
        assert!(off.pts()[1].distance(Vec2::new(9.0, 1.0)) < 1e-4, "mitred inner corner {:?}", off.pts()[1]);
        let r = p.resample(3.0);
        assert!(r.pts().windows(2).all(|w| w[0].distance(w[1]) <= 3.0 + 1e-4));
        assert!(r.pts().contains(&Vec2::new(10.0, 0.0)));
        let sub = p.sub(5.0, 12.0);
        assert!((sub.len() - 7.0).abs() < 1e-4);
    }

    #[test]
    fn chaikin_and_simplify_keep_ends() {
        let p = Polyline::new(vec![Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0), Vec2::new(20.0, 10.0)]);
        let c = p.chaikin(3);
        assert_eq!(c.pts()[0], Vec2::ZERO);
        assert_eq!(*c.pts().last().unwrap(), Vec2::new(20.0, 10.0));
        assert!(c.len() < p.len());
        let s = c.simplify(0.3);
        assert!(s.pts().len() < c.pts().len());
        assert!(s.pts().iter().all(|q| { let (_, o) = c.project(*q); o.abs() < 1e-3 }));
    }

    #[test]
    fn noise_is_smooth_and_bounded() {
        let mut prev = noise1(5, 0.0);
        for i in 1..1000 {
            let v = noise1(5, i as f32 * 0.01);
            assert!((-1.0..=1.0).contains(&v));
            assert!((v - prev).abs() < 0.05);
            prev = v;
        }
        assert_ne!(noise1(5, 0.5), noise1(6, 0.5));
        let v = noise2(3, Vec2::new(0.3, 7.7));
        assert!((-1.0..=1.0).contains(&v));
    }
}
