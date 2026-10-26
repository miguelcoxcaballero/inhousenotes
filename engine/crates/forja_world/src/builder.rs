//! Procedural mesh building helpers.

use forja_assets::MeshData;
use forja_math::{Vec2, Vec3};

#[derive(Default)]
pub struct MeshBuilder {
    pub mesh: MeshData,
}

impl MeshBuilder {
    pub fn new(material: usize) -> Self {
        Self { mesh: MeshData { material, ..Default::default() } }
    }

    pub fn vertex(&mut self, p: Vec3, n: Vec3, uv: Vec2) -> u32 {
        self.mesh.positions.push(p);
        self.mesh.normals.push(n);
        self.mesh.uvs.push(uv);
        self.mesh.positions.len() as u32 - 1
    }

    /// Quad `a b c d` counter-clockwise seen from the front, flat shaded.
    /// UVs are world-scale (`uv_scale` meters per unit).
    pub fn quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3, uv_scale: f32) {
        let n = (b - a).cross(d - a).normalize_or(Vec3::Y);
        let w = (b - a).length() / uv_scale;
        let h = (d - a).length() / uv_scale;
        let i = self.vertex(a, n, Vec2::new(0.0, h));
        self.vertex(b, n, Vec2::new(w, h));
        self.vertex(c, n, Vec2::new(w, 0.0));
        self.vertex(d, n, Vec2::new(0.0, 0.0));
        self.mesh.indices.extend([i, i + 1, i + 2, i, i + 2, i + 3]);
    }

    /// Axis-aligned box.
    pub fn cuboid(&mut self, center: Vec3, half: Vec3, uv_scale: f32) {
        let c = |x: f32, y: f32, z: f32| center + half * Vec3::new(x, y, z);
        self.quad(c(-1., -1., 1.), c(1., -1., 1.), c(1., 1., 1.), c(-1., 1., 1.), uv_scale); // +Z
        self.quad(c(1., -1., -1.), c(-1., -1., -1.), c(-1., 1., -1.), c(1., 1., -1.), uv_scale); // -Z
        self.quad(c(1., -1., 1.), c(1., -1., -1.), c(1., 1., -1.), c(1., 1., 1.), uv_scale); // +X
        self.quad(c(-1., -1., -1.), c(-1., -1., 1.), c(-1., 1., 1.), c(-1., 1., -1.), uv_scale); // -X
        self.quad(c(-1., 1., 1.), c(1., 1., 1.), c(1., 1., -1.), c(-1., 1., -1.), uv_scale); // +Y
        self.quad(c(-1., -1., -1.), c(1., -1., -1.), c(1., -1., 1.), c(-1., -1., 1.), uv_scale); // -Y
    }

    /// Horizontal polygon (XZ, counter-clockwise seen from above) at height `y`.
    pub fn polygon(&mut self, pts: &[Vec2], y: f32, up: bool, uv_scale: f32) -> bool {
        let Some(tris) = triangulate(pts) else { return false };
        let n = if up { Vec3::Y } else { Vec3::NEG_Y };
        let base = self.mesh.positions.len() as u32;
        for p in pts {
            self.vertex(Vec3::new(p.x, y, p.y), n, *p / uv_scale);
        }
        for t in tris {
            // CCW in (x, z) seen from +Y is clockwise in a right-handed frame
            // looking down, so flip for upward faces.
            if up {
                self.mesh.indices.extend([base + t[0], base + t[2], base + t[1]]);
            } else {
                self.mesh.indices.extend([base + t[0], base + t[1], base + t[2]]);
            }
        }
        true
    }

    pub fn finish(mut self) -> MeshData {
        self.mesh.finalize();
        self.mesh
    }

    pub fn is_empty(&self) -> bool {
        self.mesh.indices.is_empty()
    }
}

/// Signed area in the XZ plane (positive = counter-clockwise with x right, z down... i.e. CCW in (x, y=z)).
pub fn signed_area(p: &[Vec2]) -> f32 {
    let mut a = 0.0;
    for i in 0..p.len() {
        let (u, v) = (p[i], p[(i + 1) % p.len()]);
        a += u.x * v.y - v.x * u.y;
    }
    a * 0.5
}

/// Ear clipping for simple polygons. Input is made counter-clockwise.
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
        if guard > n * n {
            return if out.is_empty() { None } else { Some(out) }; // degenerate input
        }
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let (ia, ib, ic) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (a, b, c) = (input[ia as usize], input[ib as usize], input[ic as usize]);
            if cross(a, b, c) <= 1e-9 {
                continue; // reflex or collinear
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
            // Remove a collinear vertex and retry.
            let m = idx.len();
            let pos = (0..m).find(|&i| {
                let (a, b, c) = (input[idx[(i + m - 1) % m] as usize], input[idx[i] as usize], input[idx[(i + 1) % m] as usize]);
                cross(a, b, c).abs() <= 1e-9
            });
            match pos {
                Some(p) => {
                    idx.remove(p);
                }
                None => return if out.is_empty() { None } else { Some(out) },
            }
        }
    }
    out.push([idx[0], idx[1], idx[2]]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangulates_concave_l_shape() {
        let l = [
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(2.0, 1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(1.0, 2.0),
            Vec2::new(0.0, 2.0),
        ];
        let t = triangulate(&l).unwrap();
        assert_eq!(t.len(), 4);
        let area: f32 = t.iter().map(|t| signed_area(&[l[t[0] as usize], l[t[1] as usize], l[t[2] as usize]]).abs()).sum();
        assert!((area - 3.0).abs() < 1e-5);
        let mut rev = l;
        rev.reverse();
        assert_eq!(triangulate(&rev).unwrap().len(), 4);
    }

    #[test]
    fn polygon_faces_up() {
        let mut b = MeshBuilder::new(0);
        b.polygon(&[Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)], 0.0, true, 1.0);
        let m = b.mesh;
        let (p0, p1, p2) = (m.positions[m.indices[0] as usize], m.positions[m.indices[1] as usize], m.positions[m.indices[2] as usize]);
        assert!((p1 - p0).cross(p2 - p0).y > 0.0);
    }
}
