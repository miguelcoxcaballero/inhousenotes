//! Contact generation.
//!
//! * Spheres and capsules are "cores" (a point or segment) inflated by a
//!   radius and are handled with closest-point queries.
//! * Boxes, hulls and triangles are polytopes handled with the separating
//!   axis test plus face clipping (Gregorius, GDC 2013/2015), which yields
//!   stable multi-point manifolds for stacking.
//! * Triangle meshes and height fields are decomposed into triangles.
//!
//! Every manifold's normal points from shape A to shape B. Separation is
//! negative when penetrating. Contacts are speculative: anything closer than
//! `margin` is reported so the solver can prevent tunnelling.

use std::sync::OnceLock;

use forja_math::{Aabb, Iso, Vec3};

use crate::shape::{ConvexHull, Shape, cube_topology, unique_dirs};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContactPoint {
    /// World-space point halfway between the two surfaces.
    pub point: Vec3,
    pub separation: f32,
    /// Feature id, stable across frames for warm starting.
    pub id: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Manifold {
    pub normal: Vec3,
    points: [ContactPoint; 4],
    count: u8,
    /// Sub-shape key (triangle index for meshes).
    pub key: u32,
}

impl Manifold {
    pub fn new(normal: Vec3, key: u32) -> Self {
        Self { normal, points: [ContactPoint::default(); 4], count: 0, key }
    }

    pub fn push(&mut self, p: ContactPoint) {
        if (self.count as usize) < 4 {
            self.points[self.count as usize] = p;
            self.count += 1;
        }
    }

    pub fn points(&self) -> &[ContactPoint] {
        &self.points[..self.count as usize]
    }

    pub fn len(&self) -> usize {
        self.count as usize
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn min_separation(&self) -> f32 {
        self.points().iter().map(|p| p.separation).fold(f32::MAX, f32::min)
    }

    fn flipped(mut self) -> Self {
        self.normal = -self.normal;
        self
    }
}

// ---------------------------------------------------------------------------
// Closest-point helpers (Ericson, Real-Time Collision Detection).

pub fn closest_on_segment(p: Vec3, a: Vec3, b: Vec3) -> (Vec3, f32) {
    let ab = b - a;
    let denom = ab.length_squared();
    if denom < 1e-12 {
        return (a, 0.0);
    }
    let t = ((p - a).dot(ab) / denom).clamp(0.0, 1.0);
    (a + ab * t, t)
}

/// Closest points between segments `p1q1` and `p2q2`.
pub fn segment_segment(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (Vec3, Vec3) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.length_squared();
    let e = d2.length_squared();
    let f = d2.dot(r);
    let eps = 1e-12;
    let (s, t);
    if a <= eps && e <= eps {
        return (p1, p2);
    }
    if a <= eps {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= eps {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s0 = if denom > eps { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let mut t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t0 = 0.0;
                s0 = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t0 = 1.0;
                s0 = ((b - c) / a).clamp(0.0, 1.0);
            }
            s = s0;
            t = t0;
        }
    }
    (p1 + d1 * s, p2 + d2 * t)
}

/// Closest point on a convex planar polygon (vertices CCW around `n`).
fn closest_on_polygon(p: Vec3, verts: &[Vec3], n: Vec3) -> Vec3 {
    let mut inside = true;
    for i in 0..verts.len() {
        let a = verts[i];
        let b = verts[(i + 1) % verts.len()];
        if (b - a).cross(p - a).dot(n) < 0.0 {
            inside = false;
            break;
        }
    }
    if inside {
        return p - n * n.dot(p - verts[0]);
    }
    let mut best = verts[0];
    let mut best_d = f32::MAX;
    for i in 0..verts.len() {
        let (c, _) = closest_on_segment(p, verts[i], verts[(i + 1) % verts.len()]);
        let d = (c - p).length_squared();
        if d < best_d {
            best_d = d;
            best = c;
        }
    }
    best
}

// ---------------------------------------------------------------------------
// World-space polytope view.

pub(crate) struct Poly<'a> {
    pub verts: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub ds: Vec<f32>,
    pub faces: &'a [Vec<u32>],
    pub edges: &'a [(u32, u32)],
    pub edge_dirs: Vec<Vec3>,
    pub center: Vec3,
}

fn triangle_topology() -> &'static (Vec<Vec<u32>>, Vec<(u32, u32)>) {
    static TOPO: OnceLock<(Vec<Vec<u32>>, Vec<(u32, u32)>)> = OnceLock::new();
    TOPO.get_or_init(|| (vec![vec![0, 1, 2], vec![0, 2, 1]], vec![(0, 1), (1, 2), (2, 0)]))
}

impl<'a> Poly<'a> {
    fn from_hull(h: &'a ConvexHull, iso: &Iso, scale: Vec3) -> Self {
        let verts: Vec<Vec3> = h.vertices.iter().map(|v| iso.transform_point(*v * scale)).collect();
        let normals: Vec<Vec3> = h.planes.iter().map(|p| iso.transform_vector(p.normal)).collect();
        let ds = normals.iter().zip(&h.faces).map(|(n, f)| n.dot(verts[f[0] as usize])).collect();
        let edge_dirs = h.edge_dirs.iter().map(|d| iso.transform_vector(*d)).collect();
        let center = verts.iter().copied().sum::<Vec3>() / verts.len() as f32;
        Poly { verts, normals, ds, faces: &h.faces, edges: &h.edges, edge_dirs, center }
    }

    fn cuboid(half: Vec3, iso: &Iso) -> Poly<'static> {
        Poly::from_hull(cube_topology(), iso, half)
    }

    fn triangle(t: [Vec3; 3]) -> Poly<'static> {
        let (faces, edges) = triangle_topology();
        let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or(Vec3::Y);
        let d = n.dot(t[0]);
        let edge_dirs = unique_dirs([t[1] - t[0], t[2] - t[1], t[0] - t[2]].into_iter());
        Poly {
            verts: t.to_vec(),
            normals: vec![n, -n],
            ds: vec![d, -d],
            faces,
            edges,
            edge_dirs,
            center: (t[0] + t[1] + t[2]) / 3.0,
        }
    }

    fn face_verts(&self, f: usize) -> Vec<Vec3> {
        self.faces[f].iter().map(|i| self.verts[*i as usize]).collect()
    }

    fn project(&self, axis: Vec3) -> (f32, f32) {
        self.verts.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| {
            let d = v.dot(axis);
            (lo.min(d), hi.max(d))
        })
    }

    /// Signed distance from `p`, outward normal and closest surface point.
    fn point_query(&self, p: Vec3) -> (f32, Vec3, Vec3, usize) {
        let mut max_d = f32::MIN;
        let mut face = 0;
        for i in 0..self.normals.len() {
            let d = self.normals[i].dot(p) - self.ds[i];
            if d > max_d {
                max_d = d;
                face = i;
            }
        }
        if max_d <= 0.0 {
            let n = self.normals[face];
            return (max_d, n, p - n * max_d, face);
        }
        let mut best = (f32::MAX, Vec3::Y, p, face);
        for i in 0..self.normals.len() {
            if self.normals[i].dot(p) - self.ds[i] <= 0.0 {
                continue;
            }
            let c = closest_on_polygon(p, &self.face_verts(i), self.normals[i]);
            let d = (p - c).length();
            if d < best.0 {
                let n = if d > 1e-7 { (p - c) / d } else { self.normals[i] };
                best = (d, n, c, i);
            }
        }
        best
    }
}

fn shape_poly<'a>(shape: &'a Shape, iso: &Iso) -> Option<Poly<'a>> {
    match shape {
        Shape::Cuboid { half } => Some(Poly::cuboid(*half, iso)),
        Shape::Convex(h) => Some(Poly::from_hull(h, iso, Vec3::ONE)),
        _ => None,
    }
}

/// Core of a round shape: a segment (degenerate for spheres) and a radius.
fn round_core(shape: &Shape, iso: &Iso) -> Option<(Vec3, Vec3, f32)> {
    match shape {
        Shape::Sphere { radius } => Some((iso.pos, iso.pos, *radius)),
        Shape::Capsule { half_height, radius } => {
            let up = iso.transform_vector(Vec3::Y) * *half_height;
            Some((iso.pos - up, iso.pos + up, *radius))
        }
        _ => None,
    }
}

fn rank(s: &Shape) -> u8 {
    match s {
        Shape::Sphere { .. } => 0,
        Shape::Capsule { .. } => 1,
        Shape::Cuboid { .. } => 2,
        Shape::Convex(_) => 3,
        Shape::TriMesh(_) => 4,
        Shape::HeightField(_) => 5,
    }
}

/// Generates manifolds between two shapes. Appends to `out`.
pub fn collide(sa: &Shape, ia: &Iso, sb: &Shape, ib: &Iso, margin: f32, out: &mut Vec<Manifold>) {
    if rank(sa) > rank(sb) {
        let start = out.len();
        collide(sb, ib, sa, ia, margin, out);
        for m in &mut out[start..] {
            *m = m.flipped();
        }
        return;
    }
    match sb {
        Shape::TriMesh(mesh) => {
            let local_aabb = sa.aabb(ia).expanded(margin).transformed(&ib.inverse());
            mesh.query(&local_aabb, |i, t| {
                let tri = t.map(|v| ib.transform_point(v));
                collide_triangle(sa, ia, tri, i, margin, out);
            });
        }
        Shape::HeightField(hf) => {
            let local_aabb = sa.aabb(ia).expanded(margin).transformed(&ib.inverse());
            hf.query(&local_aabb, |i, t| {
                let tri = t.map(|v| ib.transform_point(v));
                collide_triangle(sa, ia, tri, i, margin, out);
            });
        }
        _ => {
            if let Some(m) = collide_convex(sa, ia, sb, ib, margin) {
                out.push(m);
            }
        }
    }
}

fn collide_triangle(sa: &Shape, ia: &Iso, tri: [Vec3; 3], key: u32, margin: f32, out: &mut Vec<Manifold>) {
    let poly = Poly::triangle(tri);
    let m = if let Some((p, q, r)) = round_core(sa, ia) {
        round_vs_poly(p, q, r, &poly, margin).map(Manifold::flipped)
    } else if let Some(pa) = shape_poly(sa, ia) {
        poly_vs_poly(&pa, &poly, margin, true)
    } else {
        None
    };
    if let Some(mut m) = m {
        m.key = key;
        out.push(m);
    }
}

fn collide_convex(sa: &Shape, ia: &Iso, sb: &Shape, ib: &Iso, margin: f32) -> Option<Manifold> {
    match (round_core(sa, ia), round_core(sb, ib)) {
        (Some((p1, q1, r1)), Some((p2, q2, r2))) => {
            let (c1, c2) = segment_segment(p1, q1, p2, q2);
            let d = c2 - c1;
            let len = d.length();
            let sep = len - r1 - r2;
            if sep > margin {
                return None;
            }
            let n = if len > 1e-7 { d / len } else { forja_math::orthonormal_basis((q1 - p1).normalize_or(Vec3::Y)).0 };
            let mut m = Manifold::new(n, 0);
            m.push(ContactPoint { point: c1 + n * (r1 + sep * 0.5), separation: sep, id: 0 });
            Some(m)
        }
        (Some((p, q, r)), None) => {
            let poly = shape_poly(sb, ib)?;
            round_vs_poly(p, q, r, &poly, margin).map(Manifold::flipped)
        }
        (None, Some(_)) => unreachable!("ordered by rank"),
        (None, None) => {
            let pa = shape_poly(sa, ia)?;
            let pb = shape_poly(sb, ib)?;
            poly_vs_poly(&pa, &pb, margin, false)
        }
    }
}

/// Round core (point or segment `p..q`, radius `r`) against a polytope.
/// Returned normal points from the polytope to the round shape.
fn round_vs_poly(p: Vec3, q: Vec3, r: f32, poly: &Poly, margin: f32) -> Option<Manifold> {
    let seg = q - p;
    let is_point = seg.length_squared() < 1e-12;
    let eval = |t: f32| poly.point_query(p + seg * t);
    let (t_best, (sd, n, _closest, face)) = if is_point {
        (0.0, eval(0.0))
    } else {
        // Signed distance to a convex set is convex along a segment:
        // golden-section search, then compare with the end points.
        let g = 0.618_034_f32;
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        let mut x1 = hi - g * (hi - lo);
        let mut x2 = lo + g * (hi - lo);
        let mut f1 = eval(x1).0;
        let mut f2 = eval(x2).0;
        for _ in 0..24 {
            if f1 < f2 {
                hi = x2;
                x2 = x1;
                f2 = f1;
                x1 = hi - g * (hi - lo);
                f1 = eval(x1).0;
            } else {
                lo = x1;
                x1 = x2;
                f1 = f2;
                x2 = lo + g * (hi - lo);
                f2 = eval(x2).0;
            }
        }
        let mid = 0.5 * (lo + hi);
        [0.0, mid, 1.0]
            .into_iter()
            .map(|t| (t, eval(t)))
            .min_by(|a, b| a.1.0.total_cmp(&b.1.0))
            .expect("three candidates")
    };
    let sep = sd - r;
    if sep > margin {
        return None;
    }
    let mut m = Manifold::new(n, 0);
    let face_n = poly.normals[face];
    if !is_point && n.dot(face_n) > 0.99 && seg.normalize().dot(face_n).abs() < 0.2 {
        // Capsule resting on a face: clip the segment to the face for two points.
        let fv = poly.face_verts(face);
        let mut a = p;
        let mut b = q;
        let mut ok = true;
        for i in 0..fv.len() {
            let v0 = fv[i];
            let side = (fv[(i + 1) % fv.len()] - v0).cross(face_n);
            let da = side.dot(a - v0);
            let db = side.dot(b - v0);
            if da > 0.0 && db > 0.0 {
                ok = false;
                break;
            }
            if da > 0.0 {
                a = a + (b - a) * (da / (da - db));
            } else if db > 0.0 {
                b = b + (a - b) * (db / (db - da));
            }
        }
        if ok {
            for (id, c) in [a, b].into_iter().enumerate() {
                let dist = face_n.dot(c) - poly.ds[face];
                let s = dist - r;
                if s <= margin {
                    m.push(ContactPoint { point: c - face_n * (r + dist) * 0.5, separation: s, id: id as u32 });
                }
            }
            if !m.is_empty() {
                m.normal = face_n;
                return Some(m);
            }
        }
    }
    let c = p + seg * t_best;
    m.push(ContactPoint { point: c - n * (r + sd) * 0.5, separation: sep, id: 0 });
    Some(m)
}

/// SAT + clipping between two polytopes. Normal points from `a` to `b`.
fn poly_vs_poly(a: &Poly, b: &Poly, margin: f32, b_is_triangle: bool) -> Option<Manifold> {
    let face_query = |x: &Poly, y: &Poly| {
        let mut best = (f32::MIN, 0usize);
        for i in 0..x.normals.len() {
            let sep = y.project(x.normals[i]).0 - x.ds[i];
            if sep > best.0 {
                best = (sep, i);
            }
        }
        best
    };
    let (sep_a, face_a) = face_query(a, b);
    if sep_a > margin {
        return None;
    }
    let (mut sep_b, mut face_b) = face_query(b, a);
    if sep_b > margin {
        return None;
    }
    if b_is_triangle {
        // Prefer the triangle's front face to avoid snagging on internal
        // mesh edges.
        let sep_front = a.project(b.normals[0]).0 - b.ds[0];
        if face_b != 0 && sep_front >= sep_b - 0.02 {
            sep_b = sep_front;
            face_b = 0;
        }
    }
    let mut best_edge = (f32::MIN, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
    for da in &a.edge_dirs {
        for db in &b.edge_dirs {
            let axis = da.cross(*db);
            let len = axis.length();
            if len < 1e-4 {
                continue;
            }
            let mut axis = axis / len;
            if axis.dot(b.center - a.center) < 0.0 {
                axis = -axis;
            }
            let sep = b.project(axis).0 - a.project(axis).1;
            if sep > margin {
                return None;
            }
            if sep > best_edge.0 {
                best_edge = (sep, axis, *da, *db);
            }
        }
    }
    let face_best = sep_a.max(sep_b);
    if best_edge.0 > face_best + 0.005 {
        let (sep, axis, da, db) = best_edge;
        let pick = |p: &Poly, dir: Vec3, sign: f32| {
            let mut best = (f32::MIN, Vec3::ZERO, Vec3::ZERO);
            for (i, j) in p.edges {
                let (v0, v1) = (p.verts[*i as usize], p.verts[*j as usize]);
                if (v1 - v0).normalize_or_zero().cross(dir).length_squared() > 1e-4 {
                    continue;
                }
                let s = sign * axis.dot((v0 + v1) * 0.5);
                if s > best.0 {
                    best = (s, v0, v1);
                }
            }
            (best.1, best.2)
        };
        let (a0, a1) = pick(a, da, 1.0);
        let (b0, b1) = pick(b, db, -1.0);
        let (ca, cb) = segment_segment(a0, a1, b0, b1);
        let mut m = Manifold::new(axis, 0);
        m.push(ContactPoint { point: (ca + cb) * 0.5, separation: sep, id: 0xE000_0000 });
        return Some(m);
    }
    let (reference, incident, ref_face, flip) =
        if sep_b > sep_a + 0.001 { (b, a, face_b, true) } else { (a, b, face_a, false) };
    let ref_n = reference.normals[ref_face];
    let ref_d = reference.ds[ref_face];
    let mut inc_face = 0;
    let mut min_dot = f32::MAX;
    for i in 0..incident.normals.len() {
        let d = incident.normals[i].dot(ref_n);
        if d < min_dot {
            min_dot = d;
            inc_face = i;
        }
    }
    let mut poly: Vec<(Vec3, u32)> = incident.faces[inc_face]
        .iter()
        .map(|i| (incident.verts[*i as usize], *i))
        .collect();
    let ref_verts = reference.face_verts(ref_face);
    let mut next = Vec::with_capacity(poly.len() + 4);
    for e in 0..ref_verts.len() {
        let v0 = ref_verts[e];
        let side = (ref_verts[(e + 1) % ref_verts.len()] - v0).cross(ref_n);
        next.clear();
        for k in 0..poly.len() {
            let (p0, id0) = poly[k];
            let (p1, _) = poly[(k + 1) % poly.len()];
            let d0 = side.dot(p0 - v0);
            let d1 = side.dot(p1 - v0);
            if d0 <= 0.0 {
                next.push((p0, id0));
            }
            if (d0 <= 0.0) != (d1 <= 0.0) {
                let t = d0 / (d0 - d1);
                next.push((p0 + (p1 - p0) * t, 0x8000 | ((e as u32) << 8) | k as u32));
            }
        }
        std::mem::swap(&mut poly, &mut next);
        if poly.is_empty() {
            return None;
        }
    }
    let tag = ((ref_face as u32) << 16) | ((flip as u32) << 31);
    let mut pts: Vec<ContactPoint> = poly
        .iter()
        .filter_map(|(p, id)| {
            let s = ref_n.dot(*p) - ref_d;
            (s <= margin).then(|| ContactPoint { point: *p - ref_n * (s * 0.5), separation: s, id: tag | id })
        })
        .collect();
    if pts.is_empty() {
        return None;
    }
    let normal = if flip { -ref_n } else { ref_n };
    reduce(&mut pts, normal);
    let mut m = Manifold::new(normal, 0);
    for p in pts {
        m.push(p);
    }
    Some(m)
}

/// Keeps at most four points that preserve depth and contact area.
fn reduce(pts: &mut Vec<ContactPoint>, n: Vec3) {
    if pts.len() <= 4 {
        return;
    }
    let mut keep = Vec::with_capacity(4);
    let i0 = (0..pts.len()).min_by(|a, b| pts[*a].separation.total_cmp(&pts[*b].separation)).expect("non-empty");
    keep.push(i0);
    let p0 = pts[i0].point;
    let i1 = (0..pts.len()).max_by(|a, b| (pts[*a].point - p0).length_squared().total_cmp(&(pts[*b].point - p0).length_squared())).expect("non-empty");
    keep.push(i1);
    let p1 = pts[i1].point;
    let area = |q: Vec3| (p1 - p0).cross(q - p0).dot(n);
    let i2 = (0..pts.len()).max_by(|a, b| area(pts[*a].point).abs().total_cmp(&area(pts[*b].point).abs())).expect("non-empty");
    keep.push(i2);
    let p2 = pts[i2].point;
    let sign = area(p2).signum();
    let outside = |q: Vec3| {
        let e = [(p0, p1), (p1, p2), (p2, p0)];
        e.iter().map(|(s, t)| (*t - *s).cross(q - *s).dot(n) * sign).fold(f32::MAX, f32::min)
    };
    if let Some(i3) = (0..pts.len()).filter(|i| !keep.contains(i)).min_by(|a, b| outside(pts[*a].point).total_cmp(&outside(pts[*b].point)))
        && outside(pts[i3].point) < 0.0
    {
        keep.push(i3);
    }
    let kept: Vec<ContactPoint> = keep.into_iter().map(|i| pts[i]).collect();
    *pts = kept;
}

/// Collides `shape` at `iso` against every candidate and reports manifolds
/// with their candidate index. Helper for character controllers and queries.
pub fn collide_many<'a>(
    shape: &Shape,
    iso: &Iso,
    candidates: impl Iterator<Item = (usize, &'a Shape, Iso)>,
    margin: f32,
    out: &mut Vec<(usize, Manifold)>,
) {
    let mut tmp = Vec::new();
    for (i, s, ci) in candidates {
        tmp.clear();
        collide(shape, iso, s, &ci, margin, &mut tmp);
        out.extend(tmp.iter().map(|m| (i, *m)));
    }
}

/// AABB helper re-exported for callers that only use narrowphase.
pub fn shape_aabb(shape: &Shape, iso: &Iso) -> Aabb {
    shape.aabb(iso)
}

#[cfg(test)]
mod tests {
    use super::*;
    use forja_math::{Quat, vec3};
    use std::sync::Arc;

    fn one(sa: &Shape, ia: Iso, sb: &Shape, ib: Iso) -> Option<Manifold> {
        let mut out = Vec::new();
        collide(sa, &ia, sb, &ib, 0.0, &mut out);
        out.into_iter().next()
    }

    #[test]
    fn sphere_sphere() {
        let s = Shape::sphere(1.0);
        let m = one(&s, Iso::IDENTITY, &s, Iso::from_pos(vec3(1.5, 0.0, 0.0))).unwrap();
        assert!((m.normal - Vec3::X).length() < 1e-5);
        assert!((m.points()[0].separation + 0.5).abs() < 1e-5);
    }

    #[test]
    fn box_on_box_has_four_points() {
        let ground = Shape::cuboid(vec3(10.0, 1.0, 10.0));
        let b = Shape::cuboid(Vec3::splat(0.5));
        let m = one(&ground, Iso::from_pos(vec3(0.0, -1.0, 0.0)), &b, Iso::from_pos(vec3(0.0, 0.49, 0.0))).unwrap();
        assert_eq!(m.len(), 4);
        assert!((m.normal - Vec3::Y).length() < 1e-4, "{:?}", m.normal);
        for p in m.points() {
            assert!((p.separation + 0.01).abs() < 1e-4);
        }
        // Reversed order flips the normal.
        let m2 = one(&b, Iso::from_pos(vec3(0.0, 0.49, 0.0)), &ground, Iso::from_pos(vec3(0.0, -1.0, 0.0))).unwrap();
        assert!((m2.normal + Vec3::Y).length() < 1e-4);
    }

    #[test]
    fn rotated_box_edge_contact() {
        let b = Shape::cuboid(Vec3::splat(0.5));
        let rot = Quat::from_rotation_z(std::f32::consts::FRAC_PI_4) * Quat::from_rotation_x(std::f32::consts::FRAC_PI_4);
        let m = one(&b, Iso::IDENTITY, &b, Iso::new(vec3(0.0, 1.15, 0.0), rot));
        let m = m.expect("touching");
        assert!(m.normal.y > 0.7);
    }

    #[test]
    fn separated_boxes_do_not_touch() {
        let b = Shape::cuboid(Vec3::splat(0.5));
        assert!(one(&b, Iso::IDENTITY, &b, Iso::from_pos(vec3(1.2, 0.0, 0.0))).is_none());
    }

    #[test]
    fn capsule_lying_on_box_has_two_points() {
        let ground = Shape::cuboid(vec3(10.0, 1.0, 10.0));
        let cap = Shape::capsule(1.0, 0.25);
        let iso = Iso::new(vec3(0.0, 0.24, 0.0), Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        let m = one(&ground, Iso::from_pos(vec3(0.0, -1.0, 0.0)), &cap, iso).unwrap();
        assert_eq!(m.len(), 2);
        assert!((m.normal - Vec3::Y).length() < 1e-4);
    }

    #[test]
    fn sphere_inside_box_pushes_out_nearest_face() {
        let b = Shape::cuboid(Vec3::splat(1.0));
        let s = Shape::sphere(0.2);
        let m = one(&b, Iso::IDENTITY, &s, Iso::from_pos(vec3(0.0, 0.9, 0.0))).unwrap();
        assert!((m.normal - Vec3::Y).length() < 1e-4);
        assert!((m.points()[0].separation + 0.3).abs() < 1e-4);
    }

    #[test]
    fn box_on_trimesh() {
        let mesh = Shape::TriMesh(Arc::new(crate::shape::TriMesh::new(
            vec![vec3(-10.0, 0.0, -10.0), vec3(10.0, 0.0, -10.0), vec3(10.0, 0.0, 10.0), vec3(-10.0, 0.0, 10.0)],
            vec![[0, 2, 1], [0, 3, 2]],
        )));
        let b = Shape::cuboid(Vec3::splat(0.5));
        let mut out = Vec::new();
        collide(&b, &Iso::from_pos(vec3(0.3, 0.49, 0.2)), &mesh, &Iso::IDENTITY, 0.0, &mut out);
        assert!(!out.is_empty());
        for m in &out {
            assert!(m.normal.y < -0.99, "{:?}", m.normal);
        }
    }

    #[test]
    fn sphere_on_trimesh_normal_points_into_mesh() {
        let mesh = Shape::TriMesh(Arc::new(crate::shape::TriMesh::new(
            vec![vec3(-10.0, 0.0, -10.0), vec3(10.0, 0.0, -10.0), vec3(10.0, 0.0, 10.0), vec3(-10.0, 0.0, 10.0)],
            vec![[0, 2, 1], [0, 3, 2]],
        )));
        let mut out = Vec::new();
        collide(&Shape::sphere(0.5), &Iso::from_pos(vec3(0.0, 0.45, 0.0)), &mesh, &Iso::IDENTITY, 0.0, &mut out);
        assert!(!out.is_empty());
        assert!(out.iter().all(|m| m.normal.y < -0.99));
    }

    #[test]
    fn hull_vs_hull() {
        let h = Shape::Convex(Arc::new(ConvexHull::cuboid(Vec3::splat(0.5))));
        let m = one(&h, Iso::IDENTITY, &h, Iso::from_pos(vec3(0.0, 0.95, 0.0))).unwrap();
        assert_eq!(m.len(), 4);
        assert!((m.points()[0].separation + 0.05).abs() < 1e-4);
    }
}
