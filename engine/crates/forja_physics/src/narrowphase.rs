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

use crate::shape::{ConvexHull, Shape, cube_topology};

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

// ---------------------------------------------------------------------------
// Small inline buffers: the narrowphase runs for every pair and every mesh
// triangle each step, so it must not touch the heap for boxes and triangles.

#[derive(Clone)]
pub(crate) struct Buf<T: Copy + Default, const N: usize> {
    len: usize,
    inline: [T; N],
    heap: Vec<T>,
}

impl<T: Copy + Default, const N: usize> Buf<T, N> {
    #[inline]
    pub fn new() -> Self {
        Self { len: 0, inline: [T::default(); N], heap: Vec::new() }
    }

    #[inline]
    pub fn push(&mut self, v: T) {
        if self.heap.is_empty() && self.len < N {
            self.inline[self.len] = v;
        } else {
            if self.heap.is_empty() {
                self.heap.extend_from_slice(&self.inline[..self.len]);
            }
            self.heap.push(v);
        }
        self.len += 1;
    }

    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
        self.heap.clear();
    }

    #[inline]
    pub fn as_slice(&self) -> &[T] {
        if self.heap.is_empty() { &self.inline[..self.len] } else { &self.heap }
    }

    fn from_iter(it: impl IntoIterator<Item = T>) -> Self {
        let mut b = Self::new();
        for v in it {
            b.push(v);
        }
        b
    }
}

impl<T: Copy + Default, const N: usize> std::ops::Deref for Buf<T, N> {
    type Target = [T];
    #[inline]
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

// ---------------------------------------------------------------------------
// World-space polytope view.

pub(crate) struct Poly<'a> {
    pub verts: Buf<Vec3, 8>,
    pub normals: Buf<Vec3, 8>,
    pub ds: Buf<f32, 8>,
    pub faces: &'a [Vec<u32>],
    pub edges: &'a [(u32, u32)],
    pub edge_dirs: Buf<Vec3, 8>,
    pub center: Vec3,
}

fn triangle_topology() -> &'static (Vec<Vec<u32>>, Vec<(u32, u32)>) {
    static TOPO: OnceLock<(Vec<Vec<u32>>, Vec<(u32, u32)>)> = OnceLock::new();
    TOPO.get_or_init(|| (vec![vec![0, 1, 2], vec![0, 2, 1]], vec![(0, 1), (1, 2), (2, 0)]))
}

impl<'a> Poly<'a> {
    fn from_hull(h: &'a ConvexHull, iso: &Iso, scale: Vec3) -> Self {
        let verts: Buf<Vec3, 8> = Buf::from_iter(h.vertices.iter().map(|v| iso.transform_point(*v * scale)));
        let normals: Buf<Vec3, 8> = if scale == Vec3::ONE {
            Buf::from_iter(h.planes.iter().map(|p| iso.transform_vector(p.normal)))
        } else {
            // Non-uniform scale: normals transform with the inverse scale.
            Buf::from_iter(h.planes.iter().map(|p| iso.transform_vector((p.normal / scale).normalize())))
        };
        let ds = Buf::from_iter(normals.iter().zip(&h.faces).map(|(n, f)| n.dot(verts[f[0] as usize])));
        let edge_dirs = Buf::from_iter(h.edge_dirs.iter().map(|d| iso.transform_vector((*d * scale).normalize())));
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
        let mut edge_dirs = Buf::new();
        for e in [t[1] - t[0], t[2] - t[1], t[0] - t[2]] {
            let e = e.normalize_or_zero();
            if e != Vec3::ZERO && !edge_dirs.iter().any(|o: &Vec3| o.cross(e).length_squared() < 1e-6) {
                edge_dirs.push(e);
            }
        }
        Poly {
            verts: Buf::from_iter(t),
            normals: Buf::from_iter([n, -n]),
            ds: Buf::from_iter([d, -d]),
            faces,
            edges,
            edge_dirs,
            center: (t[0] + t[1] + t[2]) / 3.0,
        }
    }

    #[inline]
    fn face_vert(&self, f: usize, i: usize) -> Vec3 {
        self.verts[self.faces[f][i] as usize]
    }

    fn project(&self, axis: Vec3) -> (f32, f32) {
        self.verts.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| {
            let d = v.dot(axis);
            (lo.min(d), hi.max(d))
        })
    }

    /// Closest point on face `f` (a convex polygon) to `p`.
    fn closest_on_face(&self, p: Vec3, f: usize) -> Vec3 {
        let n = self.normals[f];
        let k = self.faces[f].len();
        let mut inside = true;
        for i in 0..k {
            let a = self.face_vert(f, i);
            let b = self.face_vert(f, (i + 1) % k);
            if (b - a).cross(p - a).dot(n) < 0.0 {
                inside = false;
                break;
            }
        }
        if inside {
            return p - n * (n.dot(p) - self.ds[f]);
        }
        let mut best = self.face_vert(f, 0);
        let mut best_d = f32::MAX;
        for i in 0..k {
            let (c, _) = closest_on_segment(p, self.face_vert(f, i), self.face_vert(f, (i + 1) % k));
            let d = (c - p).length_squared();
            if d < best_d {
                best_d = d;
                best = c;
            }
        }
        best
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
            let c = self.closest_on_face(p, i);
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
            let mut soup = TriContacts::default();
            let ctx = TriCtx::new(sa, ia);
            mesh.query(&local_aabb, |i, t| {
                let tri = t.map(|v| ib.transform_point(v));
                ctx.collide(tri, i, margin, &mut soup);
            });
            soup.reduce_into(out);
        }
        Shape::HeightField(hf) => {
            let local_aabb = sa.aabb(ia).expanded(margin).transformed(&ib.inverse());
            let mut soup = TriContacts::default();
            let ctx = TriCtx::new(sa, ia);
            hf.query(&local_aabb, |i, t| {
                let tri = t.map(|v| ib.transform_point(v));
                ctx.collide(tri, i, margin, &mut soup);
            });
            soup.reduce_into(out);
        }
        _ => {
            if let Some(m) = collide_convex(sa, ia, sb, ib, margin) {
                out.push(m);
            }
        }
    }
}

/// The convex side of a shape-vs-triangles query, prepared once per pair.
enum TriCtx<'a> {
    Round(Vec3, Vec3, f32),
    Poly(Poly<'a>),
    None,
}

impl<'a> TriCtx<'a> {
    fn new(sa: &'a Shape, ia: &Iso) -> Self {
        if let Some((p, q, r)) = round_core(sa, ia) {
            TriCtx::Round(p, q, r)
        } else if let Some(pa) = shape_poly(sa, ia) {
            TriCtx::Poly(pa)
        } else {
            TriCtx::None
        }
    }

    fn collide(&self, tri: [Vec3; 3], key: u32, margin: f32, out: &mut TriContacts) {
        let (m, face_contact) = match self {
            TriCtx::Round(p, q, r) => match round_vs_triangle(*p, *q, *r, tri, margin) {
                Some((m, face)) => (Some(m.flipped()), face),
                None => (None, false),
            },
            TriCtx::Poly(pa) => {
                let m = poly_vs_poly(pa, &Poly::triangle(tri), margin, true);
                let tn = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or(Vec3::Y);
                let face = m.is_some_and(|m| m.normal.dot(tn).abs() > 0.999);
                (m, face)
            }
            TriCtx::None => (None, false),
        };
        if let Some(mut m) = m {
            m.key = key;
            out.items.push((m, face_contact));
        }
    }
}

/// Contact reduction for triangle soups (after Jolt's and Bullet's manifold
/// reduction): manifolds from neighbouring triangles with the same normal
/// are merged into one manifold of at most four points, and edge or vertex
/// contacts that duplicate a face contact of an adjacent triangle are dropped
/// (they are the "internal edges" that make objects snag on mesh seams).
#[derive(Default)]
struct TriContacts {
    items: Vec<(Manifold, bool)>,
}

impl TriContacts {
    fn reduce_into(mut self, out: &mut Vec<Manifold>) {
        if self.items.len() <= 1 {
            out.extend(self.items.into_iter().map(|(m, _)| m));
            return;
        }
        // Face contacts first so they claim their points.
        self.items.sort_by_key(|(m, face)| (!face, m.key));
        let mut owned: Vec<(Manifold, Buf<ContactPoint, 16>)> = Vec::new();
        for (m, face) in &self.items {
            // Drop edge contacts that sit on (or under) an existing face contact.
            if !face {
                let dup = owned.iter().any(|(g, pts)| {
                    g.normal.dot(m.normal) > 0.7
                        && m.points().iter().all(|p| pts.iter().any(|q| (q.point - p.point).length_squared() < 0.05 * 0.05) || p.separation >= g.min_separation_of(pts))
                });
                if dup {
                    continue;
                }
            }
            match owned.iter_mut().find(|(g, _)| g.normal.dot(m.normal) > 0.995) {
                Some((_, pts)) => {
                    for p in m.points() {
                        // Shared edges and corners report the same point twice.
                        if !pts.iter().any(|q| (q.point - p.point).length_squared() < 1e-4) {
                            pts.push(ContactPoint { id: p.id ^ m.key.wrapping_mul(0x9E37_79B9), ..*p });
                        }
                    }
                }
                None => {
                    let mut pts = Buf::new();
                    for p in m.points() {
                        pts.push(ContactPoint { id: p.id ^ m.key.wrapping_mul(0x9E37_79B9), ..*p });
                    }
                    owned.push((*m, pts));
                }
            }
        }
        for (g, pts) in owned {
            let mut kept: Buf<ContactPoint, 16> = Buf::new();
            reduce(&pts, g.normal, &mut kept);
            let mut m = Manifold::new(g.normal, normal_key(g.normal));
            for p in kept.iter() {
                m.push(*p);
            }
            out.push(m);
        }
    }
}

impl Manifold {
    fn min_separation_of(&self, pts: &[ContactPoint]) -> f32 {
        pts.iter().map(|p| p.separation).fold(f32::MAX, f32::min)
    }
}

/// Stable key of a merged manifold: its quantised normal (octahedral
/// encoding, 8 bits per axis), so warm starting survives triangles entering
/// and leaving the group.
fn normal_key(n: Vec3) -> u32 {
    let l1 = n.x.abs() + n.y.abs() + n.z.abs();
    let (mut x, mut y) = (n.x / l1, n.z / l1);
    if n.y < 0.0 {
        let (ox, oy) = (x, y);
        x = (1.0 - oy.abs()) * ox.signum();
        y = (1.0 - ox.abs()) * oy.signum();
    }
    let q = |v: f32| (((v * 0.5 + 0.5) * 63.0).round() as u32).min(63);
    0x4000_0000 | (q(x) << 8) | q(y)
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

#[inline]
fn inside_triangle(p: Vec3, t: &[Vec3; 3], n: Vec3) -> bool {
    (t[1] - t[0]).cross(p - t[0]).dot(n) >= 0.0 && (t[2] - t[1]).cross(p - t[1]).dot(n) >= 0.0 && (t[0] - t[2]).cross(p - t[2]).dot(n) >= 0.0
}

/// Round core (segment `p..q`, radius `r`) against a triangle, analytic
/// (closest points between the segment and the triangle's face, edges and
/// corners). The normal points from the triangle to the round shape.
/// Returns the manifold and whether it is a face contact.
fn round_vs_triangle(p: Vec3, q: Vec3, r: f32, t: [Vec3; 3], margin: f32) -> Option<(Manifold, bool)> {
    let e1 = t[1] - t[0];
    let e2 = t[2] - t[0];
    let nn = e1.cross(e2);
    let nl = nn.length();
    if nl < 1e-12 {
        return None;
    }
    let n = nn / nl;
    let (dp, dq) = (n.dot(p - t[0]), n.dot(q - t[0]));
    // Quick reject: the whole core is farther than r + margin from the plane.
    let reach = r + margin;
    if (dp > reach && dq > reach) || (dp < -reach && dq < -reach) {
        return None;
    }
    let seg = q - p;
    // Segment crosses the plane inside the triangle: deep penetration. Push
    // out towards the side holding most of the core.
    if (dp > 0.0) != (dq > 0.0) && dp != dq {
        let s = dp / (dp - dq);
        let x = p + seg * s;
        if inside_triangle(x, &t, n) {
            let side = if dp.abs() + dq.abs() > 0.0 && (dp + dq) >= 0.0 { 1.0 } else { -1.0 };
            let deep = if side > 0.0 { dp.min(dq) } else { -dp.max(dq) };
            let deep_pt = if (dp * side) < (dq * side) { p } else { q };
            let nrm = n * side;
            let mut m = Manifold::new(nrm, 0);
            let sep = deep - r;
            m.push(ContactPoint { point: deep_pt - nrm * (deep + r) * 0.5 + nrm * 0.0, separation: sep, id: 0 });
            return Some((m, true));
        }
    }
    // Closest pair among: end points over the face, segment vs each edge.
    let mut best = (f32::MAX, Vec3::ZERO, Vec3::ZERO, true);
    for (e, d) in [(p, dp), (q, dq)] {
        let proj = e - n * d;
        if inside_triangle(proj, &t, n) && d.abs() < best.0 {
            best = (d.abs(), e, proj, true);
        }
    }
    for i in 0..3 {
        let (a, b) = (t[i], t[(i + 1) % 3]);
        let (cs, ct) = segment_segment(p, q, a, b);
        let d = (cs - ct).length();
        if d < best.0 - 1e-6 {
            best = (d, cs, ct, false);
        }
    }
    let (dist, cs, ct, mut face) = best;
    let sep = dist - r;
    if sep > margin {
        return None;
    }
    let nrm = if face {
        let s = n.dot(cs - ct);
        if s >= 0.0 { n } else { -n }
    } else if dist > 1e-6 {
        let v = (cs - ct) / dist;
        // Nearly along the face normal: treat as a face contact.
        if v.dot(n).abs() > 0.999 {
            face = true;
        }
        v
    } else {
        n
    };
    let mut m = Manifold::new(nrm, 0);
    // Capsule lying on the face: two points from the segment clipped to it.
    if face && seg.length_squared() > 1e-8 && seg.normalize().dot(n).abs() < 0.2 {
        let (mut a, mut b) = (p, q);
        let mut ok = true;
        for i in 0..3 {
            let v0 = t[i];
            let side = (t[(i + 1) % 3] - v0).cross(n);
            // Points with side.dot(x - v0) > 0 are outside this edge.
            let (da, db) = (side.dot(a - v0), side.dot(b - v0));
            if da > 0.0 && db > 0.0 {
                ok = false;
                break;
            }
            if da > 0.0 {
                a += (b - a) * (da / (da - db));
            } else if db > 0.0 {
                b += (a - b) * (db / (db - da));
            }
        }
        if ok {
            for (id, c) in [a, b].into_iter().enumerate() {
                let d = nrm.dot(c - t[0]);
                let s = d - r;
                if s <= margin {
                    m.push(ContactPoint { point: c - nrm * (r + d) * 0.5, separation: s, id: id as u32 + 1 });
                }
            }
            if !m.is_empty() {
                return Some((m, true));
            }
        }
    }
    m.push(ContactPoint { point: cs - nrm * (r + dist) * 0.5, separation: sep, id: 0 });
    Some((m, face))
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
        for _ in 0..20 {
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
        let k = poly.faces[face].len();
        let mut a = p;
        let mut b = q;
        let mut ok = true;
        for i in 0..k {
            let v0 = poly.face_vert(face, i);
            let side = (poly.face_vert(face, (i + 1) % k) - v0).cross(face_n);
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
    for da in a.edge_dirs.iter() {
        for db in b.edge_dirs.iter() {
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
    let mut poly: Buf<(Vec3, u32), 16> = Buf::from_iter(incident.faces[inc_face].iter().map(|i| (incident.verts[*i as usize], *i)));
    let k_ref = reference.faces[ref_face].len();
    let mut next: Buf<(Vec3, u32), 16> = Buf::new();
    for e in 0..k_ref {
        let v0 = reference.face_vert(ref_face, e);
        let side = (reference.face_vert(ref_face, (e + 1) % k_ref) - v0).cross(ref_n);
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
    let mut pts: Buf<ContactPoint, 16> = Buf::new();
    for (p, id) in poly.iter() {
        let s = ref_n.dot(*p) - ref_d;
        if s <= margin {
            pts.push(ContactPoint { point: *p - ref_n * (s * 0.5), separation: s, id: tag | id });
        }
    }
    if pts.is_empty() {
        return None;
    }
    let normal = if flip { -ref_n } else { ref_n };
    let mut kept: Buf<ContactPoint, 16> = Buf::new();
    reduce(&pts, normal, &mut kept);
    let mut m = Manifold::new(normal, 0);
    for p in kept.iter() {
        m.push(*p);
    }
    Some(m)
}

/// Keeps at most four points that preserve depth and contact area: the
/// deepest point, the farthest from it, the one maximising the triangle
/// area, and the one farthest outside that triangle.
fn reduce(pts: &[ContactPoint], n: Vec3, out: &mut Buf<ContactPoint, 16>) {
    if pts.len() <= 4 {
        for p in pts {
            out.push(*p);
        }
        return;
    }
    let mut keep: Buf<usize, 4> = Buf::new();
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
    let mut seen: Buf<usize, 4> = Buf::new();
    for i in keep.iter() {
        if !seen.contains(i) {
            seen.push(*i);
            out.push(pts[*i]);
        }
    }
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
