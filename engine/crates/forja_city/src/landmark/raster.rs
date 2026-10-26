//! Small software rasterizer for comparing models with photos: depth,
//! face normals and element ids per pixel, and a line drawing of the
//! silhouette, depth breaks and creases.

use forja_math::{Mat4, Vec3, Vec4};

pub struct Raster {
    pub w: usize,
    pub h: usize,
    /// Linear view depth (f32::INFINITY = empty).
    pub depth: Vec<f32>,
    pub normal: Vec<Vec3>,
    pub id: Vec<u32>,
}

impl Raster {
    pub fn covered(&self, x: usize, y: usize) -> bool {
        self.depth[y * self.w + x].is_finite()
    }
}

fn clip_near(v: &[(Vec4, ()); 3], near: f32) -> Vec<Vec4> {
    let mut out = Vec::with_capacity(4);
    for i in 0..3 {
        let (a, b) = (v[i].0, v[(i + 1) % 3].0);
        let (da, db) = (a.w - near, b.w - near);
        if da >= 0.0 {
            out.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            let t = da / (da - db);
            out.push(a + (b - a) * t);
        }
    }
    out
}

/// Rasterizes triangles `(a, b, c, id)` seen through `view_proj` into a
/// `w × h` buffer.
pub fn rasterize<'a>(tris: impl Iterator<Item = ([Vec3; 3], u32)>, view_proj: Mat4, w: usize, h: usize) -> Raster {
    let mut r = Raster { w, h, depth: vec![f32::INFINITY; w * h], normal: vec![Vec3::ZERO; w * h], id: vec![u32::MAX; w * h] };
    let near = 0.05;
    for (t, id) in tris {
        let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or(Vec3::ZERO);
        if n == Vec3::ZERO {
            continue;
        }
        let cv = [(view_proj * t[0].extend(1.0), ()), (view_proj * t[1].extend(1.0), ()), (view_proj * t[2].extend(1.0), ())];
        if cv.iter().all(|c| c.0.w < near) {
            continue;
        }
        let poly = clip_near(&cv, near);
        if poly.len() < 3 {
            continue;
        }
        let scr: Vec<(f32, f32, f32)> = poly
            .iter()
            .map(|c| {
                let iw = 1.0 / c.w;
                ((c.x * iw * 0.5 + 0.5) * w as f32, (1.0 - (c.y * iw * 0.5 + 0.5)) * h as f32, iw)
            })
            .collect();
        for k in 1..scr.len() - 1 {
            let (a, b, c) = (scr[0], scr[k], scr[k + 1]);
            let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
            if area.abs() < 1e-9 {
                continue;
            }
            let x0 = a.0.min(b.0).min(c.0).floor().max(0.0) as usize;
            let x1 = (a.0.max(b.0).max(c.0).ceil() as i64).clamp(0, w as i64) as usize;
            let y0 = a.1.min(b.1).min(c.1).floor().max(0.0) as usize;
            let y1 = (a.1.max(b.1).max(c.1).ceil() as i64).clamp(0, h as i64) as usize;
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            for y in y0..y1 {
                let py = y as f32 + 0.5;
                for x in x0..x1 {
                    let px = x as f32 + 0.5;
                    let w0 = ((b.0 - px) * (c.1 - py) - (b.1 - py) * (c.0 - px)) / area;
                    let w1 = ((c.0 - px) * (a.1 - py) - (c.1 - py) * (a.0 - px)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let iw = w0 * a.2 + w1 * b.2 + w2 * c.2;
                    let d = 1.0 / iw.max(1e-9);
                    let k = y * w + x;
                    if d < r.depth[k] {
                        r.depth[k] = d;
                        r.normal[k] = n;
                        r.id[k] = id;
                    }
                }
            }
        }
    }
    r
}

/// Edge strength per pixel (1 = silhouette or depth break, 0.6 = crease).
pub fn edges(r: &Raster, crease_deg: f32) -> Vec<f32> {
    let (w, h) = (r.w, r.h);
    let mut e = vec![0.0f32; w * h];
    let cos_c = crease_deg.to_radians().cos();
    for y in 0..h {
        for x in 0..w {
            let k = y * w + x;
            for (dx, dy) in [(1usize, 0usize), (0, 1)] {
                let (nx, ny) = (x + dx, y + dy);
                if nx >= w || ny >= h {
                    continue;
                }
                let j = ny * w + nx;
                let (a, b) = (r.depth[k], r.depth[j]);
                let s = match (a.is_finite(), b.is_finite()) {
                    (false, false) => 0.0,
                    (true, false) | (false, true) => 1.0,
                    _ => {
                        if (a - b).abs() > 0.03 * a.min(b) + 0.05 {
                            0.9
                        } else if r.normal[k].dot(r.normal[j]) < cos_c {
                            0.55
                        } else if r.id[k] != r.id[j] {
                            0.35
                        } else {
                            0.0
                        }
                    }
                };
                if s > 0.0 {
                    e[k] = e[k].max(s);
                    e[j] = e[j].max(s);
                }
            }
        }
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quad_in_front_of_camera_is_covered_with_outline() {
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_infinite_reverse_rh(1.0, 1.0, 0.1);
        let vp = proj * view;
        let q = [Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 0.0), Vec3::new(-1.0, 1.0, 0.0)];
        let tris = vec![([q[0], q[1], q[2]], 1), ([q[0], q[2], q[3]], 1)];
        let r = rasterize(tris.into_iter(), vp, 64, 64);
        assert!(r.covered(32, 32));
        assert!(!r.covered(1, 1));
        assert!((r.depth[32 * 64 + 32] - 5.0).abs() < 0.05);
        let e = edges(&r, 30.0);
        assert!(e.iter().filter(|v| **v >= 1.0).count() > 40);
        assert_eq!(e[32 * 64 + 32], 0.0);
    }
}
