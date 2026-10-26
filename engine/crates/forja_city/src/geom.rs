//! Mesh construction for procedural architecture: oriented boxes, lathes,
//! profile sweeps and cylinders, written into per-material primitives.

use std::collections::HashMap;

use forja_assets::MeshData;
use forja_math::{Mat4, Vec2, Vec3};

/// Triangles for one material.
#[derive(Clone, Default)]
pub struct Prim {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
    pub indices: Vec<u32>,
}

impl Prim {
    pub fn vertex(&mut self, p: Vec3, n: Vec3, uv: Vec2) -> u32 {
        self.positions.push(p);
        self.normals.push(n);
        self.uvs.push(uv);
        self.positions.len() as u32 - 1
    }

    pub fn tri_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Quad `a b c d`, counter-clockwise seen from the front, flat shaded,
    /// with explicit UVs.
    pub fn quad_uv(&mut self, p: [Vec3; 4], uv: [Vec2; 4]) {
        let n = (p[1] - p[0]).cross(p[3] - p[0]);
        let n = if n.length_squared() > 1e-12 { n.normalize() } else { (p[2] - p[1]).cross(p[0] - p[1]).normalize_or(Vec3::Y) };
        let i = self.vertex(p[0], n, uv[0]);
        self.vertex(p[1], n, uv[1]);
        self.vertex(p[2], n, uv[2]);
        self.vertex(p[3], n, uv[3]);
        self.indices.extend([i, i + 1, i + 2, i, i + 2, i + 3]);
    }

    /// Single flat triangle with UVs.
    pub fn tri(&mut self, a: Vec3, b: Vec3, c: Vec3, uv: [Vec2; 3]) {
        let n = (b - a).cross(c - a).normalize_or(Vec3::Y);
        let i = self.vertex(a, n, uv[0]);
        self.vertex(b, n, uv[1]);
        self.vertex(c, n, uv[2]);
        self.indices.extend([i, i + 1, i + 2]);
    }

    /// Convex outline in the XY plane extruded between `z0` and `z1`
    /// (front face at `z1` faces +Z). `uv` maps front-face points.
    pub fn extrude_xy(&mut self, outline: &[Vec2], z0: f32, z1: f32, scale: f32, uv: impl Fn(Vec2) -> Vec2) {
        let n = outline.len();
        if n < 3 {
            return;
        }
        let ccw = signed_area(outline) > 0.0;
        let pts: Vec<Vec2> = if ccw { outline.to_vec() } else { outline.iter().rev().copied().collect() };
        let c = pts.iter().copied().sum::<Vec2>() / n as f32;
        for (z, front) in [(z1, true), (z0, false)] {
            let nrm = if front { Vec3::Z } else { Vec3::NEG_Z };
            let center = self.vertex(c.extend(z), nrm, if front { uv(c) } else { c / scale });
            let first = self.positions.len() as u32;
            for p in &pts {
                self.vertex(p.extend(z), nrm, if front { uv(*p) } else { *p / scale });
            }
            for i in 0..n as u32 {
                let j = (i + 1) % n as u32;
                if front {
                    self.indices.extend([center, first + i, first + j]);
                } else {
                    self.indices.extend([center, first + j, first + i]);
                }
            }
        }
        let mut dist = 0.0;
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let l = (b - a).length();
            let d = (z1 - z0).abs();
            // Outward side face for a CCW outline.
            self.quad_uv([a.extend(z1), a.extend(z0), b.extend(z0), b.extend(z1)], [
                Vec2::new(dist, 0.0) / scale,
                Vec2::new(dist, d) / scale,
                Vec2::new(dist + l, d) / scale,
                Vec2::new(dist + l, 0.0) / scale,
            ]);
            dist += l;
        }
    }

    /// Quad with world-scale planar UVs (`scale` meters per texture repeat)
    /// projected on the quad's own axes, offset by `origin` along them.
    pub fn quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3, scale: f32) {
        let u = (b - a).normalize_or(Vec3::X);
        let v = (d - a).normalize_or(Vec3::Y);
        let uv = |p: Vec3| Vec2::new((p - a).dot(u) + a.dot(u), -((p - a).dot(v) + a.y)) / scale;
        self.quad_uv([a, b, c, d], [uv(a), uv(b), uv(c), uv(d)]);
    }

    /// Box with axes `x y z` (unit vectors) and half extents, world-scale UVs.
    pub fn obox(&mut self, center: Vec3, axes: [Vec3; 3], half: Vec3, scale: f32) {
        let [ax, ay, az] = [axes[0] * half.x, axes[1] * half.y, axes[2] * half.z];
        let c = |x: f32, y: f32, z: f32| center + ax * x + ay * y + az * z;
        self.quad(c(-1., -1., 1.), c(1., -1., 1.), c(1., 1., 1.), c(-1., 1., 1.), scale);
        self.quad(c(1., -1., -1.), c(-1., -1., -1.), c(-1., 1., -1.), c(1., 1., -1.), scale);
        self.quad(c(1., -1., 1.), c(1., -1., -1.), c(1., 1., -1.), c(1., 1., 1.), scale);
        self.quad(c(-1., -1., -1.), c(-1., -1., 1.), c(-1., 1., 1.), c(-1., 1., -1.), scale);
        self.quad(c(-1., 1., 1.), c(1., 1., 1.), c(1., 1., -1.), c(-1., 1., -1.), scale);
        self.quad(c(-1., -1., -1.), c(1., -1., -1.), c(1., -1., 1.), c(-1., -1., 1.), scale);
    }

    /// Axis-aligned box from corners.
    pub fn aabox(&mut self, min: Vec3, max: Vec3, scale: f32) {
        self.obox((min + max) * 0.5, [Vec3::X, Vec3::Y, Vec3::Z], (max - min) * 0.5, scale);
    }

    /// Box between two points with a square/rectangular section (`w` along
    /// `side`, `h` along the third axis). Used for bars, rails and posts.
    pub fn beam(&mut self, a: Vec3, b: Vec3, side: Vec3, w: f32, h: f32, scale: f32) {
        let len = (b - a).length();
        if len < 1e-5 {
            return;
        }
        let z = (b - a) / len;
        let x = (side - z * side.dot(z)).normalize_or(z.any_orthonormal_vector());
        let y = z.cross(x);
        self.obox((a + b) * 0.5, [x, y, z], Vec3::new(w * 0.5, h * 0.5, len * 0.5), scale);
    }

    /// Cylinder from `a` to `b`, smooth sides, optional caps.
    pub fn cylinder(&mut self, a: Vec3, b: Vec3, r: f32, segments: u32, caps: bool) {
        self.cone(a, b, r, r, segments, caps);
    }

    pub fn cone(&mut self, a: Vec3, b: Vec3, ra: f32, rb: f32, segments: u32, caps: bool) {
        let axis = (b - a).normalize_or(Vec3::Y);
        let x = axis.any_orthonormal_vector();
        let y = axis.cross(x);
        let len = (b - a).length();
        let slope = (ra - rb) / len.max(1e-6);
        let base = self.positions.len() as u32;
        for i in 0..=segments {
            let t = i as f32 / segments as f32 * std::f32::consts::TAU;
            let d = x * t.cos() + y * t.sin();
            let n = (d + axis * slope).normalize();
            let u = i as f32 / segments as f32 * ra.max(rb) * std::f32::consts::TAU;
            self.vertex(a + d * ra, n, Vec2::new(u, 0.0));
            self.vertex(b + d * rb, n, Vec2::new(u, -len));
        }
        for i in 0..segments {
            let k = base + i * 2;
            self.indices.extend([k, k + 2, k + 3, k, k + 3, k + 1]);
        }
        if caps {
            for (c, r, n) in [(a, ra, -axis), (b, rb, axis)] {
                if r <= 1e-5 {
                    continue;
                }
                let center = self.vertex(c, n, Vec2::ZERO);
                let first = self.positions.len() as u32;
                for i in 0..=segments {
                    let t = i as f32 / segments as f32 * std::f32::consts::TAU;
                    let d = x * t.cos() + y * t.sin();
                    self.vertex(c + d * r, n, Vec2::new(t.cos(), t.sin()) * r);
                }
                for i in 0..segments {
                    if n == axis {
                        self.indices.extend([center, first + i, first + i + 1]);
                    } else {
                        self.indices.extend([center, first + i + 1, first + i]);
                    }
                }
            }
        }
    }

    /// Surface of revolution around +Y through `origin`. `profile` is
    /// `(radius, height)` from bottom to top; smooth normals.
    pub fn lathe(&mut self, origin: Vec3, profile: &[(f32, f32)], segments: u32) {
        let n = profile.len();
        if n < 2 {
            return;
        }
        // Profile normals from neighbouring segments.
        let normals: Vec<Vec2> = (0..n)
            .map(|i| {
                let a = Vec2::from(profile[i.saturating_sub(1)]);
                let b = Vec2::from(profile[(i + 1).min(n - 1)]);
                let t = (b - a).normalize_or(Vec2::Y);
                Vec2::new(t.y, -t.x)
            })
            .collect();
        let mut v_acc = vec![0.0; n];
        for i in 1..n {
            v_acc[i] = v_acc[i - 1] + (Vec2::from(profile[i]) - Vec2::from(profile[i - 1])).length();
        }
        let base = self.positions.len() as u32;
        for s in 0..=segments {
            let t = s as f32 / segments as f32 * std::f32::consts::TAU;
            let (c, si) = (t.cos(), t.sin());
            for i in 0..n {
                let (r, y) = profile[i];
                let nr = normals[i];
                let p = origin + Vec3::new(c * r, y, si * r);
                let nn = Vec3::new(c * nr.x, nr.y, si * nr.x).normalize_or(Vec3::Y);
                self.vertex(p, nn, Vec2::new(s as f32 / segments as f32 * 2.0, -v_acc[i]));
            }
        }
        let n = n as u32;
        for s in 0..segments {
            for i in 0..n - 1 {
                let a = base + s * n + i;
                let b = base + (s + 1) * n + i;
                self.indices.extend([a, a + 1, b + 1, a, b + 1, b]);
            }
        }
    }

    /// Sweeps a moulding profile along a horizontal polyline. The profile is
    /// `(out, up)` pairs, `out` measured along each point's outward normal
    /// (`normals[i]`, horizontal, already mitred). Flat shaded per profile
    /// segment so mouldings keep crisp edges. `caps` closes open ends.
    pub fn sweep(&mut self, path: &[Vec3], normals: &[Vec3], profile: &[(f32, f32)], closed: bool, caps: bool, scale: f32) {
        let m = path.len();
        if m < 2 || profile.len() < 2 {
            return;
        }
        let segs = if closed { m } else { m - 1 };
        let mut dist = vec![0.0f32; m + 1];
        for i in 0..segs {
            dist[i + 1] = dist[i] + (path[(i + 1) % m] - path[i]).length();
        }
        for k in 0..profile.len() - 1 {
            let (o0, u0) = profile[k];
            let (o1, u1) = profile[k + 1];
            let plen = Vec2::new(o1 - o0, u1 - u0).length();
            for i in 0..segs {
                let j = (i + 1) % m;
                let p = |idx: usize, o: f32, u: f32| path[idx] + normals[idx] * o + Vec3::Y * u;
                let (a, b, c, d) = (p(i, o0, u0), p(j, o0, u0), p(j, o1, u1), p(i, o1, u1));
                let uv = [
                    Vec2::new(dist[i], 0.0) / scale,
                    Vec2::new(dist[i + 1], 0.0) / scale,
                    Vec2::new(dist[i + 1], -plen) / scale,
                    Vec2::new(dist[i], -plen) / scale,
                ];
                // Orient so the face points away from the wall/upwards.
                let n = (b - a).cross(d - a);
                let want = normals[i] * (u1 - u0) - Vec3::Y * (o1 - o0);
                if n.dot(want) >= 0.0 {
                    self.quad_uv([a, b, c, d], uv);
                } else {
                    self.quad_uv([b, a, d, c], [uv[1], uv[0], uv[3], uv[2]]);
                }
            }
        }
        if caps && !closed {
            for (idx, dir) in [(0usize, -1.0f32), (m - 1, 1.0)] {
                let t = if idx == 0 { (path[1] - path[0]).normalize_or(Vec3::X) } else { (path[m - 1] - path[m - 2]).normalize_or(Vec3::X) } * dir;
                let pts: Vec<Vec3> = std::iter::once(path[idx] + Vec3::Y * profile[0].1)
                    .chain(profile.iter().map(|(o, u)| path[idx] + normals[idx] * *o + Vec3::Y * *u))
                    .chain(std::iter::once(path[idx] + Vec3::Y * profile[profile.len() - 1].1))
                    .collect();
                let first = self.positions.len() as u32;
                for p in &pts {
                    let local = *p - path[idx];
                    self.vertex(*p, t, Vec2::new(local.dot(normals[idx]), -local.y) / scale);
                }
                for k in 1..pts.len() as u32 - 1 {
                    let tri = [first, first + k, first + k + 1];
                    let (a, b, c) = (pts[0], pts[k as usize], pts[k as usize + 1]);
                    if (b - a).cross(c - a).dot(t) >= 0.0 {
                        self.indices.extend(tri);
                    } else {
                        self.indices.extend([tri[0], tri[2], tri[1]]);
                    }
                }
            }
        }
    }

    /// Horizontal polygon (any winding) at height `y`, facing up or down.
    pub fn polygon(&mut self, pts: &[Vec2], y: f32, up: bool, scale: f32) {
        let Some(tris) = triangulate(pts) else { return };
        let n = if up { Vec3::Y } else { Vec3::NEG_Y };
        let base = self.positions.len() as u32;
        for p in pts {
            self.vertex(Vec3::new(p.x, y, p.y), n, *p / scale);
        }
        for t in tris {
            let (a, b, c) = (pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]);
            let ccw_up = (b - a).perp_dot(c - a) < 0.0; // (x, z) CCW faces -Y
            if ccw_up == up {
                self.indices.extend([base + t[0], base + t[1], base + t[2]]);
            } else {
                self.indices.extend([base + t[0], base + t[2], base + t[1]]);
            }
        }
    }

    /// Appends `other` transformed by `m` (normals by the inverse transpose).
    pub fn append(&mut self, other: &Prim, m: &Mat4) {
        let nm = m.inverse().transpose();
        let base = self.positions.len() as u32;
        let flip = m.determinant() < 0.0;
        for i in 0..other.positions.len() {
            self.positions.push(m.transform_point3(other.positions[i]));
            self.normals.push(nm.transform_vector3(other.normals[i]).normalize_or(Vec3::Y));
            self.uvs.push(other.uvs[i]);
        }
        for t in other.indices.chunks_exact(3) {
            if flip {
                self.indices.extend([base + t[0], base + t[2], base + t[1]]);
            } else {
                self.indices.extend([base + t[0], base + t[1], base + t[2]]);
            }
        }
    }

    pub fn into_mesh(self, material: usize) -> MeshData {
        let mut m = MeshData { positions: self.positions, normals: self.normals, uvs: self.uvs, indices: self.indices, material, ..Default::default() };
        m.finalize();
        m
    }
}

/// Primitives keyed by material.
#[derive(Clone, Default)]
pub struct Parts<K: std::hash::Hash + Eq + Clone> {
    pub prims: HashMap<K, Prim>,
    pub order: Vec<K>,
}

impl<K: std::hash::Hash + Eq + Clone> Parts<K> {
    pub fn new() -> Self {
        Self { prims: HashMap::new(), order: Vec::new() }
    }

    pub fn get(&mut self, k: K) -> &mut Prim {
        if !self.prims.contains_key(&k) {
            self.order.push(k.clone());
        }
        self.prims.entry(k).or_default()
    }

    pub fn append(&mut self, other: &Parts<K>, m: &Mat4) {
        for k in &other.order {
            let src = &other.prims[k];
            self.get(k.clone()).append(src, m);
        }
    }

    pub fn tri_count(&self) -> usize {
        self.prims.values().map(Prim::tri_count).sum()
    }
}

/// Signed area of a polygon in (x, y).
pub fn signed_area(p: &[Vec2]) -> f32 {
    let mut a = 0.0;
    for i in 0..p.len() {
        let (u, v) = (p[i], p[(i + 1) % p.len()]);
        a += u.x * v.y - v.x * u.y;
    }
    a * 0.5
}

/// Ear clipping for simple polygons (any winding).
pub fn triangulate(input: &[Vec2]) -> Option<Vec<[u32; 3]>> {
    let n = input.len();
    if n < 3 {
        return None;
    }
    let ccw = signed_area(input) > 0.0;
    let mut idx: Vec<u32> = if ccw { (0..n as u32).collect() } else { (0..n as u32).rev().collect() };
    let mut out = Vec::with_capacity(n - 2);
    let cross = |a: Vec2, b: Vec2, c: Vec2| (b - a).perp_dot(c - a);
    let mut guard = 0;
    while idx.len() > 3 {
        guard += 1;
        if guard > n * n + 8 {
            break;
        }
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let (ia, ib, ic) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (a, b, c) = (input[ia as usize], input[ib as usize], input[ic as usize]);
            if cross(a, b, c) <= 1e-9 {
                continue;
            }
            let inside = idx.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let p = input[j as usize];
                cross(a, b, p) >= 0.0 && cross(b, c, p) >= 0.0 && cross(c, a, p) >= 0.0
            });
            if inside {
                continue;
            }
            out.push([ia, ib, ic]);
            idx.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            idx.remove(0);
        }
    }
    if idx.len() == 3 {
        out.push([idx[0], idx[1], idx[2]]);
    }
    (!out.is_empty()).then_some(out)
}

/// Mitred offset normals of a polyline lying in the XZ plane. `outward`
/// picks the side: the normal of segment `a→b` is `(dz, -dx)` times the
/// sign.
pub fn polyline_normals(path: &[Vec3], closed: bool, sign: f32) -> Vec<Vec3> {
    let m = path.len();
    let seg_n = |i: usize| {
        let d = path[(i + 1) % m] - path[i];
        Vec3::new(d.z, 0.0, -d.x).normalize_or(Vec3::Z) * sign
    };
    (0..m)
        .map(|i| {
            let has_prev = closed || i > 0;
            let has_next = closed || i + 1 < m;
            match (has_prev, has_next) {
                (true, true) => {
                    let a = seg_n((i + m - 1) % m);
                    let b = seg_n(i);
                    let n = (a + b).normalize_or(b);
                    n / n.dot(b).max(0.35)
                }
                (false, true) => seg_n(i),
                (true, false) => seg_n(i - 1),
                _ => Vec3::Z,
            }
        })
        .collect()
}

/// Deterministic per-feature randomness.
#[derive(Clone, Copy)]
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03)
    }

    pub fn next_u32(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 32) as u32
    }

    /// Uniform in [0, 1).
    pub fn f(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u32() as usize) % n.max(1)
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.f() < p
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obox_faces_point_outward() {
        let mut p = Prim::default();
        p.obox(Vec3::ZERO, [Vec3::X, Vec3::Y, Vec3::Z], Vec3::ONE, 1.0);
        for t in p.indices.chunks_exact(3) {
            let (a, b, c) = (p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]);
            let n = (b - a).cross(c - a);
            assert!(n.dot((a + b + c) / 3.0) > 0.0);
        }
    }

    #[test]
    fn polygon_winding_matches_request() {
        let sq = [Vec2::new(0., 0.), Vec2::new(1., 0.), Vec2::new(1., 1.), Vec2::new(0., 1.)];
        for up in [true, false] {
            let mut p = Prim::default();
            p.polygon(&sq, 0.0, up, 1.0);
            let t = &p.indices[0..3];
            let (a, b, c) = (p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]);
            assert_eq!((b - a).cross(c - a).y > 0.0, up);
        }
    }

    #[test]
    fn lathe_and_sweep_face_outward() {
        let mut p = Prim::default();
        p.lathe(Vec3::ZERO, &[(0.5, 0.0), (0.5, 1.0)], 12);
        for t in p.indices.chunks_exact(3) {
            let (a, b, c) = (p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]);
            let mid = (a + b + c) / 3.0;
            assert!((b - a).cross(c - a).dot(Vec3::new(mid.x, 0.0, mid.z)) > 0.0);
        }
        let path = [Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0)];
        let n = [Vec3::Z, Vec3::Z];
        let mut s = Prim::default();
        s.sweep(&path, &n, &[(0.0, 0.0), (0.3, 0.0), (0.3, 0.2), (0.0, 0.2)], false, true, 1.0);
        // Front face (out = 0.3) points +Z.
        let front = s.indices.chunks_exact(3).any(|t| {
            let (a, b, c) = (s.positions[t[0] as usize], s.positions[t[1] as usize], s.positions[t[2] as usize]);
            (a.z - 0.3).abs() < 1e-5 && (b.z - 0.3).abs() < 1e-5 && (b - a).cross(c - a).z > 0.0
        });
        assert!(front);
    }
}

impl Rng {
    /// A generator keyed by a stable id instead of drawn from one
    /// sequential stream: `seed` (the city), `stage` (what is being decided)
    /// and `key` (a geometric id such as a quantized position), mixed with
    /// SplitMix64. A local change then only changes local decisions.
    pub fn keyed(seed: u64, stage: u64, key: u64) -> Self {
        let mix = |mut z: u64| {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        let s = mix(mix(mix(seed) ^ stage.wrapping_mul(0xD6E8_FEB8_6659_FD93)) ^ key);
        Rng(s | 1)
    }
}

/// Stable key of a point quantized to 1 cm.
pub fn point_key(p: Vec2) -> u64 {
    let x = (p.x * 100.0).round() as i64 as u64;
    let z = (p.y * 100.0).round() as i64 as u64;
    x.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ z.rotate_left(32).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
}

impl Prim {
    /// Area-weighted smooth normals for the vertices from `first` on (a
    /// height field laid flat with up normals, then lifted).
    pub fn recompute_normals_from(&mut self, first: u32) {
        let f = first as usize;
        for n in &mut self.normals[f..] {
            *n = Vec3::ZERO;
        }
        for t in self.indices.chunks_exact(3) {
            if (t[0] as usize) < f {
                continue;
            }
            let (a, b, c) = (self.positions[t[0] as usize], self.positions[t[1] as usize], self.positions[t[2] as usize]);
            let n = (b - a).cross(c - a);
            for &i in t {
                self.normals[i as usize] += n;
            }
        }
        for n in &mut self.normals[f..] {
            *n = n.try_normalize().unwrap_or(Vec3::Y);
        }
    }
}
