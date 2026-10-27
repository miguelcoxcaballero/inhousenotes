//! Polylines with arc length, offsets, projection and Bézier connectors.

use forja_math::Vec2;

/// Unit left normal of a direction (`(-y, x)`).
#[inline]
pub fn left(d: Vec2) -> Vec2 {
    Vec2::new(-d.y, d.x)
}

/// Unit right normal of a travel direction (`(y, -x)`), the kerb side
/// for right-hand traffic.
#[inline]
pub fn right(d: Vec2) -> Vec2 {
    Vec2::new(d.y, -d.x)
}

/// Polyline with cumulative arc length.
#[derive(Clone, Debug, Default)]
pub struct Polyline {
    pub pts: Vec<Vec2>,
    pub cum: Vec<f32>,
}

impl Polyline {
    pub fn new(pts: Vec<Vec2>) -> Self {
        let mut clean: Vec<Vec2> = Vec::with_capacity(pts.len());
        for p in pts {
            if clean.last().is_none_or(|q: &Vec2| q.distance(p) > 1e-3) {
                clean.push(p);
            }
        }
        if clean.len() == 1 {
            let p = clean[0];
            clean.push(p + Vec2::X * 1e-2);
        }
        let mut cum = Vec::with_capacity(clean.len());
        let mut acc = 0.0;
        for (i, p) in clean.iter().enumerate() {
            if i > 0 {
                acc += clean[i - 1].distance(*p);
            }
            cum.push(acc);
        }
        Self { pts: clean, cum }
    }

    pub fn len(&self) -> f32 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    pub fn is_empty(&self) -> bool {
        self.pts.len() < 2
    }

    fn seg(&self, s: f32) -> usize {
        match self.cum.binary_search_by(|c| c.total_cmp(&s)) {
            Ok(i) => i.min(self.pts.len() - 2),
            Err(i) => i.saturating_sub(1).min(self.pts.len() - 2),
        }
    }

    /// Point and unit tangent at arc length `s`, extrapolated along the end
    /// tangents outside `[0, len]`.
    pub fn sample(&self, s: f32) -> (Vec2, Vec2) {
        let i = self.seg(s.clamp(0.0, self.len()));
        let (a, b) = (self.pts[i], self.pts[i + 1]);
        let l = (self.cum[i + 1] - self.cum[i]).max(1e-6);
        let t = (b - a) / l;
        (a + t * (s - self.cum[i]), t)
    }

    pub fn point(&self, s: f32) -> Vec2 {
        self.sample(s).0
    }

    pub fn start(&self) -> Vec2 {
        self.pts[0]
    }

    pub fn end(&self) -> Vec2 {
        *self.pts.last().expect("polyline")
    }

    pub fn start_dir(&self) -> Vec2 {
        self.sample(0.0).1
    }

    pub fn end_dir(&self) -> Vec2 {
        self.sample(self.len()).1
    }

    /// Offset by `d` to the left (miter joins).
    pub fn offset(&self, d: f32) -> Polyline {
        let n = self.pts.len();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let t0 = if i > 0 { (self.pts[i] - self.pts[i - 1]).normalize_or_zero() } else { Vec2::ZERO };
            let t1 = if i + 1 < n { (self.pts[i + 1] - self.pts[i]).normalize_or_zero() } else { Vec2::ZERO };
            let t = (t0 + t1).normalize_or(t0 + t1);
            let nrm = left(t);
            let cos = nrm.dot(left(if t1 != Vec2::ZERO { t1 } else { t0 })).max(0.3);
            out.push(self.pts[i] + nrm * d / cos);
        }
        Polyline::new(out)
    }

    pub fn reversed(&self) -> Polyline {
        Polyline::new(self.pts.iter().rev().copied().collect())
    }

    /// `(s, signed lateral offset (left positive), distance)` of the closest
    /// point to `p`.
    pub fn project(&self, p: Vec2) -> (f32, f32, f32) {
        let mut best = (0.0, 0.0, f32::MAX);
        for i in 0..self.pts.len() - 1 {
            let (a, b) = (self.pts[i], self.pts[i + 1]);
            let ab = b - a;
            let l2 = ab.length_squared().max(1e-9);
            let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
            let q = a + ab * t;
            let d = q.distance(p);
            if d < best.2 {
                let side = ab.perp_dot(p - a).signum();
                best = (self.cum[i] + t * l2.sqrt(), side * d, d);
            }
        }
        best
    }

    /// Points every `step` metres (ends included).
    pub fn resample(&self, step: f32) -> Vec<(f32, Vec2)> {
        let n = (self.len() / step).ceil().max(1.0) as usize;
        (0..=n).map(|k| {
            let s = self.len() * k as f32 / n as f32;
            (s, self.point(s))
        }).collect()
    }

    /// Axis-aligned bounds.
    pub fn bounds(&self) -> (Vec2, Vec2) {
        let mut lo = Vec2::splat(f32::MAX);
        let mut hi = Vec2::splat(f32::MIN);
        for p in &self.pts {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        (lo, hi)
    }

    /// Largest curvature (1/m) along the polyline.
    pub fn max_curvature(&self) -> f32 {
        let mut k: f32 = 0.0;
        for i in 1..self.pts.len().saturating_sub(1) {
            let a = (self.pts[i] - self.pts[i - 1]).normalize_or_zero();
            let b = (self.pts[i + 1] - self.pts[i]).normalize_or_zero();
            let ang = a.angle_to(b).abs();
            let l = 0.5 * (self.cum[i + 1] - self.cum[i - 1]);
            if l > 1e-3 {
                k = k.max(ang / l);
            }
        }
        k
    }
}

/// Cubic Bézier sampled with `n` segments.
pub fn bezier(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, n: usize) -> Vec<Vec2> {
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            p0 * (u * u * u) + p1 * (3.0 * u * u * t) + p2 * (3.0 * u * t * t) + p3 * (t * t * t)
        })
        .collect()
}

/// Intersection parameter of segments `a0-a1` and `b0-b1` (`(ta, tb)` in
/// `[0, 1]`), if they cross.
pub fn segment_hit(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> Option<(f32, f32)> {
    let r = a1 - a0;
    let s = b1 - b0;
    let den = r.perp_dot(s);
    if den.abs() < 1e-9 {
        return None;
    }
    let q = b0 - a0;
    let t = q.perp_dot(s) / den;
    let u = q.perp_dot(r) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some((t, u))
}

/// Oriented rectangle overlap (separating axis), for collision checks.
pub fn obb_overlap(ca: Vec2, fa: Vec2, ha: Vec2, cb: Vec2, fb: Vec2, hb: Vec2) -> bool {
    let axes = [fa, left(fa), fb, left(fb)];
    let corners = |c: Vec2, f: Vec2, h: Vec2| {
        let l = left(f);
        [c + f * h.y + l * h.x, c + f * h.y - l * h.x, c - f * h.y + l * h.x, c - f * h.y - l * h.x]
    };
    let pa = corners(ca, fa, ha);
    let pb = corners(cb, fb, hb);
    for ax in axes {
        let (mut a0, mut a1, mut b0, mut b1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for p in pa {
            let d = p.dot(ax);
            a0 = a0.min(d);
            a1 = a1.max(d);
        }
        for p in pb {
            let d = p.dot(ax);
            b0 = b0.min(d);
            b1 = b1.max(d);
        }
        if a1 < b0 || b1 < a0 {
            return false;
        }
    }
    true
}

/// Distance from `p` to an oriented rectangle (0 inside).
pub fn obb_distance(c: Vec2, f: Vec2, h: Vec2, p: Vec2) -> f32 {
    let d = p - c;
    let lx = d.dot(left(f)).abs() - h.x;
    let ly = d.dot(f).abs() - h.y;
    Vec2::new(lx.max(0.0), ly.max(0.0)).length()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polyline_samples_offsets_and_projects() {
        let p = Polyline::new(vec![Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)]);
        assert!((p.len() - 20.0).abs() < 1e-4);
        assert!(p.point(15.0).distance(Vec2::new(10.0, 5.0)) < 1e-4);
        let o = p.offset(1.0);
        assert!(o.point(0.0).distance(Vec2::new(0.0, 1.0)) < 1e-4);
        let (s, lat, _) = p.project(Vec2::new(5.0, 2.0));
        assert!((s - 5.0).abs() < 1e-4 && (lat - 2.0).abs() < 1e-4);
        assert!(p.point(-2.0).distance(Vec2::new(-2.0, 0.0)) < 1e-4);
    }

    #[test]
    fn obb_overlap_separates() {
        let f = Vec2::Y;
        let h = Vec2::new(0.9, 2.2);
        assert!(obb_overlap(Vec2::ZERO, f, h, Vec2::new(1.5, 0.0), f, h));
        assert!(!obb_overlap(Vec2::ZERO, f, h, Vec2::new(2.0, 0.0), f, h));
        assert!(!obb_overlap(Vec2::ZERO, f, h, Vec2::new(0.0, 4.5), f, h));
    }
}
