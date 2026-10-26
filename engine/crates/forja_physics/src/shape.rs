//! Collision shapes, mass properties and ray casts.

use std::sync::{Arc, OnceLock};

use forja_math::{Aabb, Iso, Mat3, Plane, Ray, Vec2, Vec3, vec3};

use crate::bvh::StaticBvh;

/// Collision shape in its local frame. Capsules are aligned with local +Y.
#[derive(Clone, Debug)]
pub enum Shape {
    Sphere { radius: f32 },
    Cuboid { half: Vec3 },
    Capsule { half_height: f32, radius: f32 },
    Convex(Arc<ConvexHull>),
    /// Static only.
    TriMesh(Arc<TriMesh>),
    /// Static only.
    HeightField(Arc<HeightField>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MassProperties {
    pub mass: f32,
    /// Center of mass in the shape frame.
    pub com: Vec3,
    /// Inertia tensor about `com`, in the shape frame.
    pub inertia: Mat3,
}

impl MassProperties {
    pub const ZERO: Self = Self { mass: 0.0, com: Vec3::ZERO, inertia: Mat3::ZERO };

    /// Combines parts already expressed in the same frame.
    pub fn combine(parts: &[MassProperties]) -> MassProperties {
        let mass: f32 = parts.iter().map(|p| p.mass).sum();
        if mass <= 0.0 {
            return Self::ZERO;
        }
        let com = parts.iter().map(|p| p.com * p.mass).sum::<Vec3>() / mass;
        let mut inertia = Mat3::ZERO;
        for p in parts {
            inertia += p.inertia + parallel_axis(p.mass, p.com - com);
        }
        MassProperties { mass, com, inertia }
    }

    /// Expresses these properties in a parent frame where the shape sits at `iso`.
    pub fn transformed(&self, iso: &Iso) -> MassProperties {
        let r = Mat3::from_quat(iso.rot);
        MassProperties { mass: self.mass, com: iso.transform_point(self.com), inertia: r * self.inertia * r.transpose() }
    }
}

/// Inertia contribution of a point mass at offset `d`.
fn parallel_axis(m: f32, d: Vec3) -> Mat3 {
    let dd = d.dot(d);
    (Mat3::IDENTITY * dd - outer(d, d)) * m
}

fn outer(a: Vec3, b: Vec3) -> Mat3 {
    Mat3::from_cols(a * b.x, a * b.y, a * b.z)
}

/// Ray hit in the frame the query was made in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeRayHit {
    pub t: f32,
    pub normal: Vec3,
    /// Triangle index for meshes, face index for hulls, 0 otherwise.
    pub feature: u32,
}

impl Shape {
    pub fn sphere(radius: f32) -> Self {
        Shape::Sphere { radius }
    }

    pub fn cuboid(half: Vec3) -> Self {
        Shape::Cuboid { half }
    }

    pub fn capsule(half_height: f32, radius: f32) -> Self {
        Shape::Capsule { half_height, radius }
    }

    pub fn is_static_only(&self) -> bool {
        matches!(self, Shape::TriMesh(_) | Shape::HeightField(_))
    }

    pub fn local_aabb(&self) -> Aabb {
        match self {
            Shape::Sphere { radius } => Aabb::from_center_half(Vec3::ZERO, Vec3::splat(*radius)),
            Shape::Cuboid { half } => Aabb::from_center_half(Vec3::ZERO, *half),
            Shape::Capsule { half_height, radius } => {
                Aabb::from_center_half(Vec3::ZERO, vec3(*radius, half_height + radius, *radius))
            }
            Shape::Convex(h) => h.aabb,
            Shape::TriMesh(m) => m.bvh.bounds(),
            Shape::HeightField(h) => h.aabb,
        }
    }

    pub fn aabb(&self, iso: &Iso) -> Aabb {
        match self {
            Shape::Sphere { radius } => Aabb::from_center_half(iso.pos, Vec3::splat(*radius)),
            Shape::Capsule { half_height, radius } => {
                let a = iso.transform_point(vec3(0.0, *half_height, 0.0));
                let b = iso.transform_point(vec3(0.0, -half_height, 0.0));
                Aabb::new(a.min(b), a.max(b)).expanded(*radius)
            }
            Shape::Convex(h) => Aabb::from_points(h.vertices.iter().map(|v| iso.transform_point(*v))),
            _ => self.local_aabb().transformed(iso),
        }
    }

    pub fn mass_properties(&self, density: f32) -> MassProperties {
        use std::f32::consts::PI;
        match self {
            Shape::Sphere { radius: r } => {
                let mass = density * 4.0 / 3.0 * PI * r * r * r;
                MassProperties { mass, com: Vec3::ZERO, inertia: Mat3::from_diagonal(Vec3::splat(0.4 * mass * r * r)) }
            }
            Shape::Cuboid { half: h } => {
                let mass = density * 8.0 * h.x * h.y * h.z;
                let s = h * h;
                MassProperties {
                    mass,
                    com: Vec3::ZERO,
                    inertia: Mat3::from_diagonal(vec3(s.y + s.z, s.x + s.z, s.x + s.y) * (mass / 3.0)),
                }
            }
            Shape::Capsule { half_height: h, radius: r } => {
                let cyl_h = 2.0 * h;
                let mc = density * PI * r * r * cyl_h;
                let ms = density * 4.0 / 3.0 * PI * r * r * r;
                let iy = mc * r * r * 0.5 + ms * 0.4 * r * r;
                let ix = mc * (cyl_h * cyl_h / 12.0 + r * r / 4.0)
                    + ms * (0.4 * r * r + cyl_h * cyl_h / 4.0 + 3.0 * cyl_h * r / 8.0);
                MassProperties { mass: mc + ms, com: Vec3::ZERO, inertia: Mat3::from_diagonal(vec3(ix, iy, ix)) }
            }
            Shape::Convex(h) => h.mass_properties(density),
            Shape::TriMesh(_) | Shape::HeightField(_) => MassProperties::ZERO,
        }
    }

    /// Ray cast in world space against the shape placed at `iso`.
    pub fn raycast(&self, iso: &Iso, ray: &Ray, max_t: f32) -> Option<ShapeRayHit> {
        let local = Ray { origin: iso.inverse_transform_point(ray.origin), dir: iso.inverse_transform_vector(ray.dir) };
        let hit = match self {
            Shape::Sphere { radius } => ray_sphere(&local, Vec3::ZERO, *radius, max_t),
            Shape::Cuboid { half } => ray_box(&local, *half, max_t),
            Shape::Capsule { half_height, radius } => ray_capsule(&local, *half_height, *radius, max_t),
            Shape::Convex(h) => h.raycast(&local, max_t),
            Shape::TriMesh(m) => m.raycast(&local, max_t),
            Shape::HeightField(h) => h.raycast(&local, max_t),
        }?;
        Some(ShapeRayHit { normal: iso.transform_vector(hit.normal), ..hit })
    }
}

fn ray_sphere(ray: &Ray, c: Vec3, r: f32, max_t: f32) -> Option<ShapeRayHit> {
    let m = ray.origin - c;
    let b = m.dot(ray.dir);
    let cc = m.dot(m) - r * r;
    if cc > 0.0 && b > 0.0 {
        return None;
    }
    let disc = b * b - cc;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()).max(0.0);
    if t > max_t {
        return None;
    }
    let n = (ray.at(t) - c).normalize_or(-ray.dir);
    Some(ShapeRayHit { t, normal: n, feature: 0 })
}

fn ray_box(ray: &Ray, half: Vec3, max_t: f32) -> Option<ShapeRayHit> {
    let inv = ray.inv_dir();
    let t0 = (-half - ray.origin) * inv;
    let t1 = (half - ray.origin) * inv;
    let tn = t0.min(t1);
    let tf = t0.max(t1);
    let tmin = tn.max_element();
    let tmax = tf.min_element();
    if tmin > tmax || tmax < 0.0 || tmin > max_t {
        return None;
    }
    if tmin < 0.0 {
        return Some(ShapeRayHit { t: 0.0, normal: -ray.dir, feature: 0 });
    }
    let axis = if tn.x >= tn.y && tn.x >= tn.z {
        0
    } else if tn.y >= tn.z {
        1
    } else {
        2
    };
    let mut n = Vec3::ZERO;
    n[axis] = -ray.dir[axis].signum();
    Some(ShapeRayHit { t: tmin, normal: n, feature: axis as u32 })
}

fn ray_capsule(ray: &Ray, h: f32, r: f32, max_t: f32) -> Option<ShapeRayHit> {
    let mut best: Option<ShapeRayHit> = None;
    let mut keep = |hit: Option<ShapeRayHit>| {
        if let Some(hit) = hit
            && best.is_none_or(|b| hit.t < b.t)
        {
            best = Some(hit);
        }
    };
    // Infinite cylinder around Y, clipped to |y| <= h.
    let (o, d) = (Vec2::new(ray.origin.x, ray.origin.z), Vec2::new(ray.dir.x, ray.dir.z));
    let a = d.dot(d);
    if a > 1e-12 {
        let b = o.dot(d);
        let c = o.dot(o) - r * r;
        let disc = b * b - a * c;
        if disc >= 0.0 {
            let t = (-b - disc.sqrt()) / a;
            if (0.0..=max_t).contains(&t) {
                let p = ray.at(t);
                if p.y.abs() <= h {
                    keep(Some(ShapeRayHit { t, normal: vec3(p.x, 0.0, p.z).normalize(), feature: 0 }));
                }
            }
        }
    }
    keep(ray_sphere(ray, vec3(0.0, h, 0.0), r, max_t));
    keep(ray_sphere(ray, vec3(0.0, -h, 0.0), r, max_t));
    best
}

/// Two-sided Möller–Trumbore. The normal faces the ray origin.
pub fn ray_triangle(ray: &Ray, a: Vec3, b: Vec3, c: Vec3, max_t: f32) -> Option<(f32, Vec3)> {
    let e1 = b - a;
    let e2 = c - a;
    let p = ray.dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-10 {
        return None;
    }
    let inv = 1.0 / det;
    let s = ray.origin - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = ray.dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    if t < 0.0 || t > max_t {
        return None;
    }
    let mut n = e1.cross(e2).normalize();
    if n.dot(ray.dir) > 0.0 {
        n = -n;
    }
    Some((t, n))
}

/// Convex polyhedron with polygonal faces.
#[derive(Clone, Debug)]
pub struct ConvexHull {
    pub vertices: Vec<Vec3>,
    /// Face planes, outward normals.
    pub planes: Vec<Plane>,
    /// Vertex indices of each face, counter-clockwise seen from outside.
    pub faces: Vec<Vec<u32>>,
    /// Unique edges.
    pub edges: Vec<(u32, u32)>,
    /// Edge directions with parallel duplicates removed (SAT axes).
    pub edge_dirs: Vec<Vec3>,
    pub aabb: Aabb,
}

#[derive(Debug)]
pub enum HullError {
    TooFewPoints,
    Degenerate,
}

impl std::error::Error for HullError {}

impl std::fmt::Display for HullError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HullError::TooFewPoints => write!(f, "a hull needs at least 4 distinct points"),
            HullError::Degenerate => write!(f, "points are coplanar"),
        }
    }
}

impl ConvexHull {
    /// Maximum input size for [`ConvexHull::from_points`]; larger clouds are
    /// reduced to their extreme points along a fixed set of directions.
    pub const MAX_POINTS: usize = 64;

    /// Builds the hull of a point cloud. Brute force over point triples, which
    /// is robust and fast enough for collision proxies (<= 64 points).
    pub fn from_points(points: &[Vec3]) -> Result<Self, HullError> {
        let mut pts: Vec<Vec3> = Vec::new();
        let scale = Aabb::from_points(points.iter().copied()).half_extents().max_element().max(1e-3);
        let eps = 1e-4 * scale;
        for p in points {
            if !pts.iter().any(|q| (*q - *p).length_squared() < eps * eps) {
                pts.push(*p);
            }
        }
        if pts.len() > Self::MAX_POINTS {
            pts = extreme_points(&pts, Self::MAX_POINTS);
        }
        if pts.len() < 4 {
            return Err(HullError::TooFewPoints);
        }
        let center = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
        let mut planes: Vec<Plane> = Vec::new();
        let n = pts.len();
        for i in 0..n {
            for j in i + 1..n {
                for k in j + 1..n {
                    let nrm = (pts[j] - pts[i]).cross(pts[k] - pts[i]);
                    let len = nrm.length();
                    if len < eps * scale {
                        continue;
                    }
                    let mut nrm = nrm / len;
                    let mut d = nrm.dot(pts[i]);
                    if nrm.dot(center) > d {
                        nrm = -nrm;
                        d = -d;
                    }
                    if pts.iter().all(|p| nrm.dot(*p) - d <= eps)
                        && !planes.iter().any(|pl| pl.normal.dot(nrm) > 1.0 - 1e-5 && (pl.d - d).abs() < eps)
                    {
                        planes.push(Plane { normal: nrm, d });
                    }
                }
            }
        }
        if planes.len() < 4 {
            return Err(HullError::Degenerate);
        }
        // Keep only vertices that lie on at least 3 planes (true corners).
        let on_plane = |p: Vec3, pl: &Plane| (pl.distance(p)).abs() <= eps * 4.0;
        let corners: Vec<Vec3> =
            pts.iter().copied().filter(|p| planes.iter().filter(|pl| on_plane(*p, pl)).count() >= 3).collect();
        let mut faces = Vec::with_capacity(planes.len());
        for pl in &planes {
            let ids: Vec<u32> = (0..corners.len() as u32).filter(|i| on_plane(corners[*i as usize], pl)).collect();
            faces.push(order_face(&corners, &ids, pl.normal));
        }
        let mut edges: Vec<(u32, u32)> = Vec::new();
        for f in &faces {
            for e in 0..f.len() {
                let (a, b) = (f[e], f[(e + 1) % f.len()]);
                let key = (a.min(b), a.max(b));
                if !edges.contains(&key) {
                    edges.push(key);
                }
            }
        }
        let edge_dirs = unique_dirs(edges.iter().map(|(a, b)| corners[*b as usize] - corners[*a as usize]));
        let aabb = Aabb::from_points(corners.iter().copied());
        Ok(ConvexHull { vertices: corners, planes, faces, edges, edge_dirs, aabb })
    }

    /// Hull of an axis-aligned box.
    pub fn cuboid(half: Vec3) -> Self {
        let unit = unit_cube();
        let mut h = unit.clone();
        for v in &mut h.vertices {
            *v *= half;
        }
        for (pl, f) in h.planes.iter_mut().zip(&h.faces) {
            pl.d = pl.normal.dot(h.vertices[f[0] as usize]);
        }
        h.aabb = Aabb::from_center_half(Vec3::ZERO, half);
        h
    }

    pub fn support(&self, dir: Vec3) -> Vec3 {
        let mut best = self.vertices[0];
        let mut best_d = best.dot(dir);
        for v in &self.vertices[1..] {
            let d = v.dot(dir);
            if d > best_d {
                best_d = d;
                best = *v;
            }
        }
        best
    }

    pub fn mass_properties(&self, density: f32) -> MassProperties {
        // Covariance integration over tetrahedra fanned from an interior point
        // (Blow & Binstock).
        let origin = self.vertices.iter().copied().sum::<Vec3>() / self.vertices.len() as f32;
        let canonical = Mat3::from_cols_array(&[2.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 2.0]) * (1.0 / 120.0);
        let mut cov = Mat3::ZERO;
        let mut vol = 0.0;
        let mut com = Vec3::ZERO;
        for f in &self.faces {
            let a = self.vertices[f[0] as usize] - origin;
            for i in 1..f.len() - 1 {
                let b = self.vertices[f[i] as usize] - origin;
                let c = self.vertices[f[i + 1] as usize] - origin;
                let m = Mat3::from_cols(a, b, c);
                let det = m.determinant();
                cov += m * canonical * m.transpose() * det;
                let v = det / 6.0;
                vol += v;
                com += (a + b + c) * (v / 4.0);
            }
        }
        if vol <= 0.0 {
            return MassProperties::ZERO;
        }
        com /= vol;
        let mass = density * vol;
        cov = cov * density - outer(com, com) * mass;
        let inertia = Mat3::IDENTITY * (cov.x_axis.x + cov.y_axis.y + cov.z_axis.z) - cov;
        MassProperties { mass, com: com + origin, inertia }
    }

    fn raycast(&self, ray: &Ray, max_t: f32) -> Option<ShapeRayHit> {
        let (mut t_in, mut t_out) = (0.0f32, max_t);
        let mut normal = -ray.dir;
        let mut feature = 0;
        for (i, pl) in self.planes.iter().enumerate() {
            let denom = pl.normal.dot(ray.dir);
            let dist = pl.distance(ray.origin);
            if denom.abs() < 1e-12 {
                if dist > 0.0 {
                    return None;
                }
                continue;
            }
            let t = -dist / denom;
            if denom < 0.0 {
                if t > t_in {
                    t_in = t;
                    normal = pl.normal;
                    feature = i as u32;
                }
            } else {
                t_out = t_out.min(t);
            }
            if t_in > t_out {
                return None;
            }
        }
        Some(ShapeRayHit { t: t_in, normal, feature })
    }
}

fn unit_cube() -> &'static ConvexHull {
    static CUBE: OnceLock<ConvexHull> = OnceLock::new();
    CUBE.get_or_init(|| {
        let pts: Vec<Vec3> = (0..8)
            .map(|i| {
                vec3(
                    if i & 1 != 0 { 1.0 } else { -1.0 },
                    if i & 2 != 0 { 1.0 } else { -1.0 },
                    if i & 4 != 0 { 1.0 } else { -1.0 },
                )
            })
            .collect();
        ConvexHull::from_points(&pts).expect("cube hull")
    })
}

/// Shared polytope topology for boxes.
pub(crate) fn cube_topology() -> &'static ConvexHull {
    unit_cube()
}

pub(crate) fn unique_dirs(dirs: impl Iterator<Item = Vec3>) -> Vec<Vec3> {
    let mut out: Vec<Vec3> = Vec::new();
    for d in dirs {
        let d = d.normalize_or_zero();
        if d == Vec3::ZERO {
            continue;
        }
        if !out.iter().any(|o| o.cross(d).length_squared() < 1e-6) {
            out.push(d);
        }
    }
    out
}

fn extreme_points(pts: &[Vec3], max: usize) -> Vec<Vec3> {
    // Fibonacci sphere directions.
    let mut out: Vec<Vec3> = Vec::new();
    let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
    let dirs = max * 2;
    for i in 0..dirs {
        let y = 1.0 - 2.0 * (i as f32 + 0.5) / dirs as f32;
        let r = (1.0 - y * y).sqrt();
        let th = golden * i as f32;
        let d = vec3(r * th.cos(), y, r * th.sin());
        let p = *pts.iter().max_by(|a, b| a.dot(d).total_cmp(&b.dot(d))).expect("non-empty");
        if !out.contains(&p) {
            out.push(p);
        }
        if out.len() == max {
            break;
        }
    }
    out
}

/// Orders coplanar vertex ids counter-clockwise around `n`, dropping
/// collinear and interior points (2D monotone chain).
fn order_face(verts: &[Vec3], ids: &[u32], n: Vec3) -> Vec<u32> {
    let (u, v) = forja_math::orthonormal_basis(n);
    let mut p: Vec<(Vec2, u32)> = ids.iter().map(|i| (Vec2::new(verts[*i as usize].dot(u), verts[*i as usize].dot(v)), *i)).collect();
    p.sort_by(|a, b| a.0.x.total_cmp(&b.0.x).then(a.0.y.total_cmp(&b.0.y)));
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let mut hull: Vec<(Vec2, u32)> = Vec::new();
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &(Vec2, u32)>> = if pass == 0 { Box::new(p.iter()) } else { Box::new(p.iter().rev()) };
        for q in iter {
            while hull.len() >= start + 2 && cross(hull[hull.len() - 2].0, hull[hull.len() - 1].0, q.0) <= 1e-9 {
                hull.pop();
            }
            hull.push(*q);
        }
        hull.pop();
    }
    // (u, v, n) is right-handed, so CCW in (u, v) is CCW seen from +n.
    let mut out: Vec<u32> = hull.into_iter().map(|h| h.1).collect();
    if out.len() >= 3 {
        let a = verts[out[0] as usize];
        let b = verts[out[1] as usize];
        let c = verts[out[2] as usize];
        if (b - a).cross(c - a).dot(n) < 0.0 {
            out.reverse();
        }
    }
    out
}

/// Static triangle mesh with its own BVH.
#[derive(Clone, Debug)]
pub struct TriMesh {
    pub vertices: Vec<Vec3>,
    pub indices: Vec<[u32; 3]>,
    pub bvh: StaticBvh,
}

impl TriMesh {
    pub fn new(vertices: Vec<Vec3>, indices: Vec<[u32; 3]>) -> Self {
        let aabbs: Vec<Aabb> = indices
            .iter()
            .map(|t| Aabb::from_points(t.iter().map(|i| vertices[*i as usize])).expanded(1e-4))
            .collect();
        let bvh = StaticBvh::build(&aabbs);
        Self { vertices, indices, bvh }
    }

    #[inline]
    pub fn triangle(&self, i: u32) -> [Vec3; 3] {
        let t = self.indices[i as usize];
        [self.vertices[t[0] as usize], self.vertices[t[1] as usize], self.vertices[t[2] as usize]]
    }

    pub fn query(&self, aabb: &Aabb, mut f: impl FnMut(u32, [Vec3; 3])) {
        self.bvh.query(aabb, |i| f(i, self.triangle(i)));
    }

    fn raycast(&self, ray: &Ray, max_t: f32) -> Option<ShapeRayHit> {
        let mut best = None;
        self.bvh.raycast(ray, max_t, |i, max_t| {
            let [a, b, c] = self.triangle(i);
            match ray_triangle(ray, a, b, c, max_t) {
                Some((t, n)) => {
                    best = Some(ShapeRayHit { t, normal: n, feature: i });
                    t
                }
                None => max_t,
            }
        });
        best
    }
}

/// Regular height grid. Local origin at the grid center; samples are
/// row-major with rows along +Z and columns along +X.
#[derive(Clone, Debug)]
pub struct HeightField {
    pub rows: usize,
    pub cols: usize,
    pub heights: Vec<f32>,
    /// World size of one cell along X and Z.
    pub cell: Vec2,
    pub aabb: Aabb,
}

impl HeightField {
    pub fn new(rows: usize, cols: usize, heights: Vec<f32>, cell: Vec2) -> Self {
        assert!(rows >= 2 && cols >= 2 && heights.len() == rows * cols);
        let (lo, hi) = heights.iter().fold((f32::MAX, f32::MIN), |(lo, hi), h| (lo.min(*h), hi.max(*h)));
        let half = Vec2::new((cols - 1) as f32 * cell.x, (rows - 1) as f32 * cell.y) * 0.5;
        let aabb = Aabb::new(vec3(-half.x, lo, -half.y), vec3(half.x, hi, half.y));
        Self { rows, cols, heights, cell, aabb }
    }

    #[inline]
    pub fn point(&self, r: usize, c: usize) -> Vec3 {
        vec3(self.aabb.min.x + c as f32 * self.cell.x, self.heights[r * self.cols + c], self.aabb.min.z + r as f32 * self.cell.y)
    }

    /// Triangle `i`: cell index * 2 + (0 | 1).
    pub fn triangle(&self, i: u32) -> [Vec3; 3] {
        let cell = (i / 2) as usize;
        let (r, c) = (cell / (self.cols - 1), cell % (self.cols - 1));
        let p00 = self.point(r, c);
        let p01 = self.point(r, c + 1);
        let p10 = self.point(r + 1, c);
        let p11 = self.point(r + 1, c + 1);
        // CCW seen from +Y.
        if i % 2 == 0 { [p00, p10, p11] } else { [p00, p11, p01] }
    }

    /// Height at a local XZ position (bilinear over the triangle split).
    pub fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        let fx = (x - self.aabb.min.x) / self.cell.x;
        let fz = (z - self.aabb.min.z) / self.cell.y;
        if fx < 0.0 || fz < 0.0 || fx > (self.cols - 1) as f32 || fz > (self.rows - 1) as f32 {
            return None;
        }
        let c = (fx as usize).min(self.cols - 2);
        let r = (fz as usize).min(self.rows - 2);
        let (u, v) = (fx - c as f32, fz - r as f32);
        let h = |r, c| self.heights[r * self.cols + c];
        Some(if v >= u {
            h(r, c) + (h(r + 1, c) - h(r, c)) * v + (h(r + 1, c + 1) - h(r + 1, c)) * u
        } else {
            h(r, c) + (h(r, c + 1) - h(r, c)) * u + (h(r + 1, c + 1) - h(r, c + 1)) * v
        })
    }

    pub fn query(&self, aabb: &Aabb, mut f: impl FnMut(u32, [Vec3; 3])) {
        if !aabb.intersects(&self.aabb) {
            return;
        }
        let c0 = (((aabb.min.x - self.aabb.min.x) / self.cell.x).floor().max(0.0)) as usize;
        let c1 = (((aabb.max.x - self.aabb.min.x) / self.cell.x).floor() as isize).clamp(0, self.cols as isize - 2) as usize;
        let r0 = (((aabb.min.z - self.aabb.min.z) / self.cell.y).floor().max(0.0)) as usize;
        let r1 = (((aabb.max.z - self.aabb.min.z) / self.cell.y).floor() as isize).clamp(0, self.rows as isize - 2) as usize;
        for r in r0..=r1.min(self.rows - 2) {
            for c in c0..=c1.min(self.cols - 2) {
                let cell = (r * (self.cols - 1) + c) as u32;
                for k in 0..2 {
                    let tri = self.triangle(cell * 2 + k);
                    let lo = tri[0].y.min(tri[1].y).min(tri[2].y);
                    let hi = tri[0].y.max(tri[1].y).max(tri[2].y);
                    if hi >= aabb.min.y && lo <= aabb.max.y {
                        f(cell * 2 + k, tri);
                    }
                }
            }
        }
    }

    fn raycast(&self, ray: &Ray, max_t: f32) -> Option<ShapeRayHit> {
        // March the ray in chunks of a few cells and test the covered triangles.
        let t_start = self.aabb.ray_hit(ray, max_t)?;
        let step = self.cell.x.min(self.cell.y) * 4.0;
        let mut t = t_start;
        while t <= max_t {
            let t_end = (t + step).min(max_t);
            let seg = Aabb::from_points([ray.at(t), ray.at(t_end)]).expanded(1e-3);
            let mut best: Option<ShapeRayHit> = None;
            self.query(&seg, |i, [a, b, c]| {
                if let Some((th, n)) = ray_triangle(ray, a, b, c, best.map_or(t_end + 1e-3, |b| b.t)) {
                    best = Some(ShapeRayHit { t: th, normal: n, feature: i });
                }
            });
            if best.is_some() {
                return best;
            }
            if t_end >= max_t {
                break;
            }
            t = t_end;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forja_math::Quat;

    #[test]
    fn cube_hull_topology() {
        let h = ConvexHull::cuboid(vec3(1.0, 2.0, 3.0));
        assert_eq!(h.vertices.len(), 8);
        assert_eq!(h.faces.len(), 6);
        assert_eq!(h.edges.len(), 12);
        assert_eq!(h.edge_dirs.len(), 3);
        for (pl, f) in h.planes.iter().zip(&h.faces) {
            assert_eq!(f.len(), 4);
            let a = h.vertices[f[0] as usize];
            let b = h.vertices[f[1] as usize];
            let c = h.vertices[f[2] as usize];
            assert!((b - a).cross(c - a).dot(pl.normal) > 0.0, "face not CCW");
        }
    }

    #[test]
    fn hull_mass_matches_box() {
        let half = vec3(0.5, 1.0, 2.0);
        let a = Shape::cuboid(half).mass_properties(2.0);
        let b = ConvexHull::cuboid(half).mass_properties(2.0);
        assert!((a.mass - b.mass).abs() < 1e-3);
        assert!(b.com.length() < 1e-4);
        for i in 0..3 {
            assert!((a.inertia.col(i) - b.inertia.col(i)).length() < 1e-3, "{:?} vs {:?}", a.inertia, b.inertia);
        }
    }

    #[test]
    fn hull_from_cloud_drops_interior_points() {
        let mut pts = vec![];
        for i in 0..8 {
            pts.push(vec3(if i & 1 != 0 { 1.0 } else { -1.0 }, if i & 2 != 0 { 1.0 } else { -1.0 }, if i & 4 != 0 { 1.0 } else { -1.0 }));
        }
        pts.push(Vec3::ZERO);
        pts.push(vec3(0.0, 1.0, 0.0)); // on a face, not a corner
        let h = ConvexHull::from_points(&pts).unwrap();
        assert_eq!(h.vertices.len(), 8);
        assert_eq!(h.faces.len(), 6);
    }

    #[test]
    fn raycasts() {
        let iso = Iso::new(vec3(0.0, 0.0, 10.0), Quat::from_rotation_y(0.3));
        let ray = Ray::new(Vec3::ZERO, Vec3::Z);
        for shape in [
            Shape::sphere(1.0),
            Shape::cuboid(Vec3::ONE),
            Shape::capsule(1.0, 1.0),
            Shape::Convex(Arc::new(ConvexHull::cuboid(Vec3::ONE))),
        ] {
            let hit = shape.raycast(&iso, &ray, 100.0).unwrap_or_else(|| panic!("{shape:?} missed"));
            assert!(hit.t > 8.5 && hit.t < 9.1, "{shape:?} t={}", hit.t);
            assert!(hit.normal.z < -0.9 || matches!(shape, Shape::Cuboid { .. } | Shape::Convex(_)));
        }
    }

    #[test]
    fn heightfield_ray_and_height() {
        let n = 33;
        let heights: Vec<f32> = (0..n * n).map(|i| ((i % n) as f32 * 0.2).sin()).collect();
        let hf = HeightField::new(n, n, heights, Vec2::splat(1.0));
        let ray = Ray::new(vec3(3.3, 10.0, -2.7), Vec3::NEG_Y);
        let hit = hf.raycast(&ray, 100.0).unwrap();
        let h = hf.height_at(3.3, -2.7).unwrap();
        assert!((ray.at(hit.t).y - h).abs() < 1e-3);
    }

    #[test]
    fn trimesh_raycast() {
        let m = TriMesh::new(
            vec![vec3(-10.0, 0.0, -10.0), vec3(10.0, 0.0, -10.0), vec3(10.0, 0.0, 10.0), vec3(-10.0, 0.0, 10.0)],
            vec![[0, 2, 1], [0, 3, 2]],
        );
        let hit = Shape::TriMesh(Arc::new(m)).raycast(&Iso::IDENTITY, &Ray::new(vec3(1.0, 5.0, 2.0), Vec3::NEG_Y), 10.0);
        assert!((hit.unwrap().t - 5.0).abs() < 1e-5);
    }
}
