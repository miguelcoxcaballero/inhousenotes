//! 2D polygon toolkit in plan coordinates (`Vec2 { x, y }` = metres east,
//! metres south). Rings are open (no repeated closing point). A normalised
//! outer ring is counter-clockwise in (x, z) maths orientation (positive
//! [`area`]), so the outward normal of edge `a → b` is `(d.y, -d.x)`;
//! holes run the other way.

use forja_math::Vec2;

use super::earcut;

pub type Ring = Vec<Vec2>;

/// Outer ring with holes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shape {
    pub outer: Ring,
    pub holes: Vec<Ring>,
}

impl Shape {
    pub fn new(outer: Ring) -> Self {
        Self { outer, holes: Vec::new() }
    }

    /// Normalised copy: deduplicated, outer CCW, holes CW; `None` when the
    /// outer ring is degenerate.
    pub fn normalized(&self) -> Option<Shape> {
        let outer = normalize(&self.outer, true)?;
        let holes = self.holes.iter().filter_map(|h| normalize(h, false)).collect();
        Some(Shape { outer, holes })
    }

    pub fn area(&self) -> f32 {
        area(&self.outer).abs() - self.holes.iter().map(|h| area(h).abs()).sum::<f32>()
    }

    pub fn contains(&self, p: Vec2) -> bool {
        point_in_ring(&self.outer, p) && !self.holes.iter().any(|h| point_in_ring(h, p))
    }

    pub fn bbox(&self) -> (Vec2, Vec2) {
        bbox(&self.outer)
    }

    pub fn map(&self, f: impl Fn(Vec2) -> Vec2) -> Shape {
        Shape { outer: self.outer.iter().map(|p| f(*p)).collect(), holes: self.holes.iter().map(|h| h.iter().map(|p| f(*p)).collect()).collect() }
    }

    /// Triangles (counter-clockwise) over the returned point list.
    pub fn triangulate(&self) -> (Vec<Vec2>, Vec<[u32; 3]>) {
        earcut::triangulate_with_holes(&self.outer, &self.holes)
    }

    /// Every ring with the side it bounds: `(ring, is_hole)`.
    pub fn rings(&self) -> impl Iterator<Item = (&Ring, bool)> {
        std::iter::once((&self.outer, false)).chain(self.holes.iter().map(|h| (h, true)))
    }
}

/// Signed area (positive for counter-clockwise in x, y).
pub fn area(p: &[Vec2]) -> f32 {
    let n = p.len();
    if n < 3 {
        return 0.0;
    }
    // Shift to the first point for precision far from the origin.
    let o = p[0];
    let mut a = 0.0f64;
    for i in 0..n {
        let (u, v) = (p[i] - o, p[(i + 1) % n] - o);
        a += (u.x as f64) * (v.y as f64) - (v.x as f64) * (u.y as f64);
    }
    (a * 0.5) as f32
}

pub fn perimeter(p: &[Vec2]) -> f32 {
    (0..p.len()).map(|i| (p[(i + 1) % p.len()] - p[i]).length()).sum()
}

/// Area centroid (falls back to the vertex mean for degenerate rings).
pub fn centroid(p: &[Vec2]) -> Vec2 {
    let n = p.len();
    if n == 0 {
        return Vec2::ZERO;
    }
    let o = p[0];
    let (mut cx, mut cy, mut a) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..n {
        let (u, v) = (p[i] - o, p[(i + 1) % n] - o);
        let c = (u.x as f64) * (v.y as f64) - (v.x as f64) * (u.y as f64);
        a += c;
        cx += (u.x + v.x) as f64 * c;
        cy += (u.y + v.y) as f64 * c;
    }
    if a.abs() < 1e-9 {
        return p.iter().copied().sum::<Vec2>() / n as f32;
    }
    o + Vec2::new((cx / (3.0 * a)) as f32, (cy / (3.0 * a)) as f32)
}

pub fn bbox(p: &[Vec2]) -> (Vec2, Vec2) {
    let mut mn = Vec2::splat(f32::MAX);
    let mut mx = Vec2::splat(f32::MIN);
    for q in p {
        mn = mn.min(*q);
        mx = mx.max(*q);
    }
    (mn, mx)
}

/// Removes duplicate, closing and collinear points and zero-width spikes
/// (a point going out and straight back, common in OSM) and orients the
/// ring (counter-clockwise when `ccw`).
pub fn normalize(ring: &[Vec2], ccw: bool) -> Option<Ring> {
    let mut r: Ring = Vec::with_capacity(ring.len());
    for p in ring {
        if !p.x.is_finite() || !p.y.is_finite() {
            return None;
        }
        if r.last().is_none_or(|q: &Vec2| (*q - *p).length() > 1e-4) {
            r.push(*p);
        }
    }
    while r.len() > 1 && (r[0] - r[r.len() - 1]).length() <= 1e-4 {
        r.pop();
    }
    // Drop collinear points and spikes until stable.
    let mut changed = true;
    while changed && r.len() >= 3 {
        changed = false;
        let n = r.len();
        for i in 0..n {
            let (a, b, c) = (r[(i + n - 1) % n], r[i], r[(i + 1) % n]);
            let (u, v) = (b - a, c - b);
            let cr = u.perp_dot(v);
            let flat = cr.abs() < 1e-6 * u.length().max(1.0) * v.length().max(1.0);
            if flat || (a - c).length() <= 1e-4 {
                r.remove(i);
                // A spike leaves `a` and `c` coincident: drop one of them too.
                if r.len() >= 2 {
                    let m = r.len();
                    let j = i % m;
                    let k = (j + m - 1) % m;
                    if (r[j] - r[k]).length() <= 1e-4 {
                        r.remove(j);
                    }
                }
                changed = true;
                break;
            }
        }
    }
    if r.len() < 3 || area(&r).abs() < 1e-6 {
        return None;
    }
    if (area(&r) > 0.0) != ccw {
        r.reverse();
    }
    Some(r)
}

/// First pair of non-adjacent edges that cross (`None` for a simple ring).
pub fn self_intersection(ring: &[Vec2]) -> Option<(usize, usize)> {
    let n = ring.len();
    if n < 4 {
        return None;
    }
    let cross = |a: Vec2, b: Vec2, c: Vec2, d: Vec2| -> bool {
        let o = |p: Vec2, q: Vec2, r: Vec2| (q - p).perp_dot(r - p);
        let (d1, d2, d3, d4) = (o(c, d, a), o(c, d, b), o(a, b, c), o(a, b, d));
        ((d1 > 1e-9 && d2 < -1e-9) || (d1 < -1e-9 && d2 > 1e-9)) && ((d3 > 1e-9 && d4 < -1e-9) || (d3 < -1e-9 && d4 > 1e-9))
    };
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            if cross(a, b, ring[j], ring[(j + 1) % n]) {
                return Some((i, j));
            }
        }
    }
    None
}

/// Boolean operation on plan shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    Difference,
    Intersection,
}

/// Union / difference / intersection of two sets of shapes (non-zero
/// fill; holes allowed), as normalised shapes.
pub fn boolean(a: &[Shape], b: &[Shape], op: BoolOp) -> Vec<Shape> {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::core::overlay_rule::OverlayRule;
    use i_overlay::float::single::SingleFloatOverlay;
    let conv = |list: &[Shape]| -> Vec<Vec<Vec<[f64; 2]>>> {
        list.iter()
            .filter_map(|s| s.normalized())
            .map(|s| s.rings().map(|(r, _)| r.iter().map(|p| [p.x as f64, p.y as f64]).collect()).collect())
            .collect()
    };
    let (sa, sb) = (conv(a), conv(b));
    if sa.is_empty() {
        return if op == BoolOp::Union { b.iter().filter_map(|s| s.normalized()).collect() } else { Vec::new() };
    }
    if sb.is_empty() {
        return if op == BoolOp::Intersection { Vec::new() } else { a.iter().filter_map(|s| s.normalized()).collect() };
    }
    let rule = match op {
        BoolOp::Union => OverlayRule::Union,
        BoolOp::Difference => OverlayRule::Difference,
        BoolOp::Intersection => OverlayRule::Intersect,
    };
    let res = sa.overlay(&sb, rule, FillRule::NonZero);
    let mut out = Vec::new();
    for shape in res {
        let mut rings = shape.into_iter().map(|c| c.into_iter().map(|p| Vec2::new(p[0] as f32, p[1] as f32)).collect::<Ring>());
        let Some(outer) = rings.next() else { continue };
        let s = Shape { outer, holes: rings.collect() };
        if let Some(n) = s.normalized() {
            out.push(n);
        }
    }
    out
}

/// Even-odd point in ring test.
pub fn point_in_ring(ring: &[Vec2], p: Vec2) -> bool {
    let n = ring.len();
    let mut inside = false;
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (ring[i], ring[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub fn is_convex(ring: &[Vec2]) -> bool {
    let n = ring.len();
    if n < 4 {
        return true;
    }
    let s = area(ring).signum();
    (0..n).all(|i| {
        let (a, b, c) = (ring[i], ring[(i + 1) % n], ring[(i + 2) % n]);
        (b - a).perp_dot(c - b) * s >= -1e-5
    })
}

/// Outward unit normal of edge `i` of a counter-clockwise ring.
pub fn edge_normal(ring: &[Vec2], i: usize) -> Vec2 {
    let d = ring[(i + 1) % ring.len()] - ring[i];
    Vec2::new(d.y, -d.x).normalize_or(Vec2::Y)
}

/// Compass azimuth (degrees clockwise from north, north = -z) of a plan
/// direction.
pub fn azimuth(dir: Vec2) -> f32 {
    let a = dir.x.atan2(-dir.y).to_degrees();
    if a < 0.0 { a + 360.0 } else { a }
}

/// Plan direction of a compass azimuth.
pub fn from_azimuth(deg: f32) -> Vec2 {
    let r = deg.to_radians();
    Vec2::new(r.sin(), -r.cos())
}

/// Nearest compass label (n, ne, e, ...).
pub fn compass(deg: f32) -> &'static str {
    const L: [&str; 8] = ["n", "ne", "e", "se", "s", "sw", "w", "nw"];
    L[(((deg.rem_euclid(360.0)) + 22.5) / 45.0) as usize % 8]
}

/// Offsets a ring outward by `d` (negative insets) with mitred joins
/// (limited to `4 d`). Edges that flip or collapse are removed and the
/// offset repeated, so moderate insets of concave rings stay valid.
pub fn offset(ring: &[Vec2], d: f32) -> Option<Ring> {
    let ccw = area(ring) > 0.0;
    let mut r = normalize(ring, true)?;
    if d.abs() < 1e-6 {
        return Some(if ccw { r } else { r.into_iter().rev().collect() });
    }
    for _ in 0..r.len().max(4) {
        let n = r.len();
        if n < 3 {
            return None;
        }
        let normals: Vec<Vec2> = (0..n).map(|i| edge_normal(&r, i)).collect();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let (n0, n1) = (normals[(i + n - 1) % n], normals[i]);
            let bis = (n0 + n1).normalize_or(n1);
            let c = bis.dot(n1).max(0.25);
            out.push(r[i] + bis * (d / c));
        }
        // Edges whose direction reversed collapsed: drop them and retry.
        let flipped: Vec<usize> = (0..n).filter(|&i| (out[(i + 1) % n] - out[i]).dot(r[(i + 1) % n] - r[i]) <= 0.0).collect();
        if flipped.is_empty() {
            let res = normalize(&out, true)?;
            if (area(&res) > 0.0) && (d > 0.0 || area(&res) < area(&r)) {
                return Some(if ccw { res } else { res.into_iter().rev().collect() });
            }
            return None;
        }
        // Remove the vertex at the end of each collapsed edge.
        let mut keep: Vec<Vec2> = Vec::with_capacity(n);
        for i in 0..n {
            if !flipped.contains(&((i + n - 1) % n)) {
                keep.push(r[i]);
            }
        }
        if keep.len() == n || keep.len() < 3 {
            return None;
        }
        r = keep;
    }
    None
}

/// Shape grown by `d` (outer offset outward, holes shrunk; holes that
/// close up disappear). Negative `d` shrinks.
pub fn grow_shape(s: &Shape, d: f32) -> Shape {
    if d.abs() < 1e-6 {
        return s.clone();
    }
    let outer = offset(&s.outer, d).unwrap_or_else(|| s.outer.clone());
    let holes = s.holes.iter().filter_map(|h| offset(h, -d)).collect();
    Shape { outer, holes }
}

/// Douglas-Peucker simplification of a closed ring.
pub fn simplify(ring: &[Vec2], tol: f32) -> Ring {
    let n = ring.len();
    if n < 5 || tol <= 0.0 {
        return ring.to_vec();
    }
    // Split at the two farthest-apart vertices.
    let (mut a, mut b, mut best) = (0, 0, -1.0);
    for i in 0..n {
        let d = (ring[i] - ring[0]).length_squared();
        if d > best {
            best = d;
            a = i;
        }
    }
    best = -1.0;
    for i in 0..n {
        let d = (ring[i] - ring[a]).length_squared();
        if d > best {
            best = d;
            b = i;
        }
    }
    let (lo, hi) = (a.min(b), a.max(b));
    let mut keep = vec![false; n];
    keep[lo] = true;
    keep[hi] = true;
    fn dp(pts: &[Vec2], idx: &[usize], tol: f32, keep: &mut [bool]) {
        if idx.len() < 3 {
            return;
        }
        let (a, b) = (pts[idx[0]], pts[idx[idx.len() - 1]]);
        let (mut far, mut fd) = (0, -1.0);
        for (k, &i) in idx.iter().enumerate().take(idx.len() - 1).skip(1) {
            let d = seg_dist(pts[i], a, b);
            if d > fd {
                fd = d;
                far = k;
            }
        }
        if fd > tol {
            keep[idx[far]] = true;
            dp(pts, &idx[..=far], tol, keep);
            dp(pts, &idx[far..], tol, keep);
        }
    }
    let first: Vec<usize> = (lo..=hi).collect();
    let second: Vec<usize> = (hi..n).chain(0..=lo).collect();
    dp(ring, &first, tol, &mut keep);
    dp(ring, &second, tol, &mut keep);
    let out: Ring = (0..n).filter(|&i| keep[i]).map(|i| ring[i]).collect();
    if out.len() >= 3 { out } else { ring.to_vec() }
}

/// Distance from `p` to segment `a b`.
pub fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
    (a + ab * t - p).length()
}

/// Convex hull (counter-clockwise, monotone chain).
pub fn convex_hull(pts: &[Vec2]) -> Ring {
    let mut p: Vec<Vec2> = pts.to_vec();
    p.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    p.dedup_by(|a, b| (*a - *b).length() < 1e-6);
    if p.len() < 3 {
        return p;
    }
    let mut lower: Vec<Vec2> = Vec::new();
    for q in &p {
        while lower.len() >= 2 && (lower[lower.len() - 1] - lower[lower.len() - 2]).perp_dot(*q - lower[lower.len() - 1]) <= 0.0 {
            lower.pop();
        }
        lower.push(*q);
    }
    let mut upper: Vec<Vec2> = Vec::new();
    for q in p.iter().rev() {
        while upper.len() >= 2 && (upper[upper.len() - 1] - upper[upper.len() - 2]).perp_dot(*q - upper[upper.len() - 1]) <= 0.0 {
            upper.pop();
        }
        upper.push(*q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Minimum-area enclosing rectangle (counter-clockwise), from the hull's
/// edge directions.
pub fn min_rect(pts: &[Vec2]) -> Ring {
    let hull = convex_hull(pts);
    if hull.len() < 3 {
        return hull;
    }
    let mut best: Option<(f32, Ring)> = None;
    for i in 0..hull.len() {
        let d = (hull[(i + 1) % hull.len()] - hull[i]).normalize_or(Vec2::X);
        let nrm = Vec2::new(-d.y, d.x);
        let (mut a0, mut a1, mut b0, mut b1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for q in &hull {
            let (a, b) = (q.dot(d), q.dot(nrm));
            a0 = a0.min(a);
            a1 = a1.max(a);
            b0 = b0.min(b);
            b1 = b1.max(b);
        }
        let ar = (a1 - a0) * (b1 - b0);
        if best.as_ref().is_none_or(|(x, _)| ar < *x) {
            let p = |a: f32, b: f32| d * a + nrm * b;
            best = Some((ar, vec![p(a0, b0), p(a1, b0), p(a1, b1), p(a0, b1)]));
        }
    }
    best.map(|b| b.1).unwrap_or(hull)
}

/// Regular polygon with `n` sides. `rotation` (degrees, counter-clockwise
/// seen from above) is applied to the default orientation in which edge 0
/// faces south (+z) when `n` is even.
pub fn regular(center: Vec2, radius: f32, n: usize, rotation_deg: f32) -> Ring {
    let n = n.max(3);
    let step = std::f32::consts::TAU / n as f32;
    // Vertices at increasing (x, z) maths angles make a CCW ring; edge i's
    // normal points at angle a_i + step/2. Edge 0 faces +z (angle pi/2),
    // and a counter-clockwise rotation seen from above lowers the angle.
    let start = std::f32::consts::FRAC_PI_2 - step * 0.5 - rotation_deg.to_radians();
    (0..n)
        .map(|i| {
            let a = start + step * i as f32;
            center + Vec2::new(a.cos(), a.sin()) * radius
        })
        .collect()
}

/// Rotates a plan vector counter-clockwise seen from above (+x towards
/// -z) by `deg`.
pub fn rotate(v: Vec2, deg: f32) -> Vec2 {
    let (s, c) = deg.to_radians().sin_cos();
    Vec2::new(v.x * c + v.y * s, -v.x * s + v.y * c)
}

/// Axis-aligned rectangle centred at `c`, size `(w along x, d along z)`.
pub fn rect(c: Vec2, size: Vec2, rotation_deg: f32) -> Ring {
    let h = size * 0.5;
    let r: Ring = vec![Vec2::new(h.x, h.y), Vec2::new(-h.x, h.y), Vec2::new(-h.x, -h.y), Vec2::new(h.x, -h.y)];
    let mut r: Ring = r.into_iter().map(|p| c + rotate(p, rotation_deg)).collect();
    if area(&r) < 0.0 {
        r.reverse();
    }
    r
}

/// Sutherland-Hodgman clip of a ring against the half plane
/// `dot(p, n) <= d`.
pub fn clip_half_plane(ring: &[Vec2], n: Vec2, d: f32) -> Ring {
    let mut out = Vec::with_capacity(ring.len() + 2);
    let m = ring.len();
    for i in 0..m {
        let (a, b) = (ring[i], ring[(i + 1) % m]);
        let (da, db) = (a.dot(n) - d, b.dot(n) - d);
        if da <= 0.0 {
            out.push(a);
        }
        if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
            let t = da / (da - db);
            out.push(a + (b - a) * t);
        }
    }
    out
}

/// Clips `subject` to a convex counter-clockwise `clip` ring.
pub fn clip_convex(subject: &[Vec2], clip: &[Vec2]) -> Ring {
    let mut out = subject.to_vec();
    for i in 0..clip.len() {
        if out.len() < 3 {
            return Vec::new();
        }
        let (a, b) = (clip[i], clip[(i + 1) % clip.len()]);
        let nrm = Vec2::new(b.y - a.y, a.x - b.x).normalize_or(Vec2::X);
        out = clip_half_plane(&out, nrm, a.dot(nrm));
    }
    out
}

// ------------------------------------------------------------ boolean clip

#[derive(Clone, Copy, Debug)]
struct GhVert {
    p: Vec2,
    next: usize,
    prev: usize,
    is_x: bool,
    entry: bool,
    neighbor: usize,
    visited: bool,
}

fn gh_list(ring: &[Vec2]) -> Vec<GhVert> {
    let n = ring.len();
    ring.iter()
        .enumerate()
        .map(|(i, p)| GhVert { p: *p, next: (i + 1) % n, prev: (i + n - 1) % n, is_x: false, entry: false, neighbor: 0, visited: false })
        .collect()
}

fn seg_intersect(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> Option<(f32, f32)> {
    let r = b - a;
    let s = d - c;
    let den = r.perp_dot(s);
    if den.abs() < 1e-12 {
        return None;
    }
    let t = (c - a).perp_dot(s) / den;
    let u = (c - a).perp_dot(r) / den;
    (t > 0.0 && t < 1.0 && u > 0.0 && u < 1.0).then_some((t, u))
}

fn degenerate_contact(s: &[Vec2], c: &[Vec2]) -> bool {
    // A vertex of one ring lying on (or extremely close to) the other.
    let near = |r: &[Vec2], p: Vec2| (0..r.len()).any(|i| seg_dist(p, r[i], r[(i + 1) % r.len()]) < 1e-4);
    s.iter().any(|p| near(c, *p)) || c.iter().any(|p| near(s, *p))
}

/// Intersection of two simple rings (Greiner-Hormann). Degenerate contacts
/// are resolved by nudging the clip ring by a fraction of a millimetre.
/// Returns zero or more counter-clockwise rings.
pub fn intersect(subject: &[Vec2], clip: &[Vec2]) -> Vec<Ring> {
    let (Some(s0), Some(c0)) = (normalize(subject, true), normalize(clip, true)) else { return Vec::new() };
    let mut c = c0.clone();
    let mut tries = 0;
    while degenerate_contact(&s0, &c) && tries < 6 {
        tries += 1;
        let k = 0.0003 * tries as f32;
        c = c0.iter().map(|p| *p + Vec2::new(k * 0.7071, k * 0.5133)).collect();
    }
    gh_intersect(&s0, &c)
}

fn gh_intersect(s: &[Vec2], c: &[Vec2]) -> Vec<Ring> {
    let mut sl = gh_list(s);
    let mut cl = gh_list(c);
    let (ns, nc) = (s.len(), c.len());
    // Intersections per original edge.
    let mut s_ins: Vec<Vec<(f32, usize)>> = vec![Vec::new(); ns];
    let mut c_ins: Vec<Vec<(f32, usize)>> = vec![Vec::new(); nc];
    let mut xs: Vec<(Vec2, f32, f32)> = Vec::new();
    for i in 0..ns {
        for j in 0..nc {
            if let Some((t, u)) = seg_intersect(s[i], s[(i + 1) % ns], c[j], c[(j + 1) % nc]) {
                let k = xs.len();
                xs.push((s[i] + (s[(i + 1) % ns] - s[i]) * t, t, u));
                s_ins[i].push((t, k));
                c_ins[j].push((u, k));
            }
        }
    }
    if xs.is_empty() {
        if s.iter().all(|p| point_in_ring(c, *p)) {
            return vec![s.to_vec()];
        }
        if c.iter().all(|p| point_in_ring(s, *p)) {
            return vec![c.to_vec()];
        }
        return Vec::new();
    }
    // Insert intersection vertices into both lists.
    let mut s_node = vec![0usize; xs.len()];
    let mut c_node = vec![0usize; xs.len()];
    let insert = |list: &mut Vec<GhVert>, edges: &mut [Vec<(f32, usize)>], node_of: &mut [usize], xs: &[(Vec2, f32, f32)]| {
        for (i, ins) in edges.iter_mut().enumerate() {
            ins.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut prev = i;
            let end = list[i].next;
            for &(_, k) in ins.iter() {
                let idx = list.len();
                list.push(GhVert { p: xs[k].0, next: end, prev, is_x: true, entry: false, neighbor: 0, visited: false });
                list[prev].next = idx;
                prev = idx;
                node_of[k] = idx;
            }
            list[prev].next = end;
            list[end].prev = prev;
        }
    };
    insert(&mut sl, &mut s_ins, &mut s_node, &xs);
    insert(&mut cl, &mut c_ins, &mut c_node, &xs);
    for k in 0..xs.len() {
        sl[s_node[k]].neighbor = c_node[k];
        cl[c_node[k]].neighbor = s_node[k];
    }
    // Entry / exit flags.
    for (list, other, n0) in [(&mut sl, c, 0usize), (&mut cl, s, 0usize)] {
        let mut inside = point_in_ring(other, list[n0].p);
        let mut v = n0;
        loop {
            if list[v].is_x {
                list[v].entry = !inside;
                inside = !inside;
            }
            v = list[v].next;
            if v == n0 {
                break;
            }
        }
    }
    // Trace.
    let mut out = Vec::new();
    loop {
        let Some(start) = (0..sl.len()).find(|&i| sl[i].is_x && !sl[i].visited) else { break };
        let mut ring: Ring = Vec::new();
        let mut cur = start;
        let mut on_s = true;
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 4 * (sl.len() + cl.len()) {
                break;
            }
            {
                let list = if on_s { &mut sl } else { &mut cl };
                list[cur].visited = true;
            }
            let (entry, nb) = {
                let list = if on_s { &sl } else { &cl };
                (list[cur].entry, list[cur].neighbor)
            };
            // Walk forward on entry, backward on exit.
            loop {
                let list = if on_s { &mut sl } else { &mut cl };
                cur = if entry { list[cur].next } else { list[cur].prev };
                ring.push(list[cur].p);
                list[cur].visited = true;
                if list[cur].is_x {
                    break;
                }
            }
            let nb2 = if on_s { sl[cur].neighbor } else { cl[cur].neighbor };
            let _ = nb;
            on_s = !on_s;
            cur = nb2;
            if on_s && cur == start || (!on_s && cl[cur].neighbor == start) {
                break;
            }
        }
        sl[start].visited = true;
        if let Some(r) = normalize(&ring, true) {
            out.push(r);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sq(x0: f32, y0: f32, s: f32) -> Ring {
        vec![Vec2::new(x0, y0), Vec2::new(x0 + s, y0), Vec2::new(x0 + s, y0 + s), Vec2::new(x0, y0 + s)]
    }

    #[test]
    fn normalize_orients_and_cleans() {
        let r = vec![Vec2::new(0., 0.), Vec2::new(0., 1.), Vec2::new(0., 2.), Vec2::new(2., 2.), Vec2::new(2., 0.), Vec2::new(0., 0.)];
        let n = normalize(&r, true).unwrap();
        assert_eq!(n.len(), 4);
        assert!(area(&n) > 0.0);
        let h = normalize(&r, false).unwrap();
        assert!(area(&h) < 0.0);
    }

    #[test]
    fn offset_square_and_concave() {
        let o = offset(&sq(0., 0., 10.), -1.0).unwrap();
        assert!((area(&o) - 64.0).abs() < 1e-3);
        let g = offset(&sq(0., 0., 10.), 1.0).unwrap();
        assert!((area(&g) - 144.0).abs() < 1e-3);
        // L shape inset stays an L.
        let l = vec![Vec2::new(0., 0.), Vec2::new(10., 0.), Vec2::new(10., 4.), Vec2::new(4., 4.), Vec2::new(4., 10.), Vec2::new(0., 10.)];
        let li = offset(&l, -1.0).unwrap();
        assert_eq!(li.len(), 6);
        assert!((area(&li) - (8.0 * 2.0 + 2.0 * 6.0)).abs() < 1e-2, "{}", area(&li));
    }

    #[test]
    fn outward_normal_convention() {
        let r = normalize(&sq(0., 0., 1.), true).unwrap();
        for i in 0..4 {
            let mid = (r[i] + r[(i + 1) % 4]) * 0.5;
            assert!(!point_in_ring(&r, mid + edge_normal(&r, i) * 0.1));
        }
        assert_eq!(compass(azimuth(Vec2::new(0., 1.))), "s");
        assert_eq!(compass(azimuth(Vec2::new(1., 0.))), "e");
        let oct = regular(Vec2::ZERO, 5.0, 8, 0.0);
        assert_eq!(compass(azimuth(edge_normal(&oct, 0))), "s");
        assert!((rotate(Vec2::X, 90.0) - Vec2::new(0.0, -1.0)).length() < 1e-5, "rotate is CCW from above: east -> north");
    }

    #[test]
    fn intersect_overlapping_and_nested() {
        let a = sq(0., 0., 10.);
        let b = sq(5., 5., 10.);
        let r = intersect(&a, &b);
        assert_eq!(r.len(), 1);
        assert!((area(&r[0]) - 25.0).abs() < 1e-2, "{}", area(&r[0]));
        let inner = sq(2., 2., 3.);
        assert!((area(&intersect(&a, &inner)[0]) - 9.0).abs() < 1e-3);
        assert!(intersect(&a, &sq(20., 20., 2.)).is_empty());
        // Shared edge (degenerate) still works.
        let c = sq(0., 0., 5.);
        let r = intersect(&a, &c);
        assert!((area(&r[0]) - 25.0).abs() < 0.05, "{}", area(&r[0]));
        // Concave subject split into two pieces.
        let u = vec![Vec2::new(0., 0.), Vec2::new(9., 0.), Vec2::new(9., 9.), Vec2::new(6., 9.), Vec2::new(6., 3.), Vec2::new(3., 3.), Vec2::new(3., 9.), Vec2::new(0., 9.)];
        let band = vec![Vec2::new(-1., 5.), Vec2::new(10., 5.), Vec2::new(10., 7.), Vec2::new(-1., 7.)];
        let r = intersect(&u, &band);
        assert_eq!(r.len(), 2);
        let tot: f32 = r.iter().map(|x| area(x)).sum();
        assert!((tot - 12.0).abs() < 1e-2);
    }

    #[test]
    fn hull_rect_simplify() {
        let pts = vec![Vec2::new(0., 0.), Vec2::new(4., 0.), Vec2::new(2., 1.), Vec2::new(4., 2.), Vec2::new(0., 2.)];
        assert_eq!(convex_hull(&pts).len(), 4);
        let r = min_rect(&pts);
        assert!((area(&r) - 8.0).abs() < 1e-3);
        let mut noisy = Vec::new();
        for i in 0..40 {
            noisy.push(Vec2::new(i as f32 * 0.25, if i % 2 == 0 { 0.0 } else { 0.01 }));
        }
        noisy.extend([Vec2::new(10., 5.), Vec2::new(0., 5.)]);
        assert!(simplify(&noisy, 0.05).len() <= 5);
    }
}
