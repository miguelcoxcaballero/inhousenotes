//! Photo cameras: the projection the renderer uses (look-at, roll,
//! vertical field of view, reverse-Z infinite perspective) and a solver
//! that recovers a camera from image-to-world point matches (`[[view]]
//! match = [...]`), so a view can be aligned in one step instead of by
//! trial renders.

use forja_math::{Mat4, Quat, Vec2, Vec3};

/// A pinhole camera in the landmark frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cam {
    pub eye: Vec3,
    pub target: Vec3,
    /// Vertical field of view in degrees.
    pub fov_y: f32,
    /// Degrees about the viewing axis.
    pub roll: f32,
}

impl Cam {
    /// Same convention as `forja_render::Camera::look_at` plus roll.
    pub fn rotation(&self) -> Quat {
        let dir = (self.target - self.eye).normalize_or(Vec3::NEG_Z);
        let up = if dir.y.abs() > 0.999 { Vec3::Z } else { Vec3::Y };
        Quat::from_mat4(&Mat4::look_at_rh(self.eye, self.eye + dir, up).inverse()) * Quat::from_rotation_z(self.roll.to_radians())
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = Mat4::from_rotation_translation(self.rotation(), self.eye).inverse();
        Mat4::perspective_infinite_reverse_rh(self.fov_y.to_radians(), aspect, 0.1) * view
    }

    /// Pixel position (from the top-left corner) of `p` in a `w` x `h`
    /// image, or `None` behind the camera.
    pub fn project(&self, p: Vec3, w: f32, h: f32) -> Option<Vec2> {
        let c = self.view_proj(w / h) * p.extend(1.0);
        if c.w <= 1e-3 {
            return None;
        }
        Some(Vec2::new((c.x / c.w * 0.5 + 0.5) * w, (1.0 - (c.y / c.w * 0.5 + 0.5)) * h))
    }

    fn params(&self) -> [f32; 7] {
        let d = (self.target - self.eye).normalize_or(Vec3::NEG_Z);
        let yaw = d.x.atan2(-d.z);
        let pitch = d.y.clamp(-1.0, 1.0).asin();
        [self.eye.x, self.eye.y, self.eye.z, yaw, pitch, self.roll.to_radians(), self.fov_y.to_radians()]
    }

    fn from_params(p: &[f32; 7]) -> Cam {
        let (yaw, pitch) = (p[3], p[4].clamp(-1.5, 1.5));
        let d = Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos());
        let eye = Vec3::new(p[0], p[1], p[2]);
        Cam { eye, target: eye + d * 20.0, fov_y: p[6].to_degrees().clamp(5.0, 140.0), roll: p[5].to_degrees() }
    }
}

/// Vertical field of view of a 35 mm-equivalent focal length for a photo
/// of the given aspect (the equivalence is defined on the 43.3 mm
/// diagonal: 2 atan(12 / f) for a 3:2 frame).
pub fn fov_from_focal(focal_35mm: f32, aspect: f32) -> f32 {
    let diag = (aspect * aspect + 1.0).sqrt();
    2.0 * (21.635 / focal_35mm.max(1.0) / diag).atan().to_degrees()
}

/// Reprojection residuals (pixels) of `cam` over the matches.
fn residuals(cam: &Cam, m: &[(Vec2, Vec3)], w: f32, h: f32, out: &mut Vec<f32>) {
    out.clear();
    for (px, p) in m {
        match cam.project(*p, w, h) {
            Some(q) => {
                out.push(q.x - px.x);
                out.push(q.y - px.y);
            }
            None => {
                out.push(w * 4.0);
                out.push(h * 4.0);
            }
        }
    }
}

fn rms(r: &[f32]) -> f32 {
    (r.iter().map(|v| v * v).sum::<f32>() / r.len().max(1) as f32 * 2.0).sqrt()
}

/// Levenberg–Marquardt from one start.
fn refine(start: Cam, m: &[(Vec2, Vec3)], w: f32, h: f32) -> (Cam, f32) {
    let mut p = start.params();
    let mut r = Vec::new();
    residuals(&Cam::from_params(&p), m, w, h, &mut r);
    let mut cost: f32 = r.iter().map(|v| v * v).sum();
    let mut lambda = 1e-2f32;
    let steps = [0.01f32, 0.01, 0.01, 1e-3, 1e-3, 1e-3, 1e-3];
    let mut rp = Vec::new();
    for _ in 0..200 {
        // Numerical Jacobian (central differences).
        let n = r.len();
        let mut jac = vec![[0.0f32; 7]; n];
        for k in 0..7 {
            let mut a = p;
            let mut b = p;
            a[k] += steps[k];
            b[k] -= steps[k];
            let mut ra = Vec::new();
            let mut rb = Vec::new();
            residuals(&Cam::from_params(&a), m, w, h, &mut ra);
            residuals(&Cam::from_params(&b), m, w, h, &mut rb);
            for i in 0..n {
                jac[i][k] = (ra[i] - rb[i]) / (2.0 * steps[k]);
            }
        }
        // Normal equations (J^T J + lambda diag) dx = -J^T r.
        let mut a = [[0.0f64; 7]; 7];
        let mut g = [0.0f64; 7];
        for i in 0..n {
            for x in 0..7 {
                g[x] += (jac[i][x] * r[i]) as f64;
                for y in 0..7 {
                    a[x][y] += (jac[i][x] * jac[i][y]) as f64;
                }
            }
        }
        let mut improved = false;
        for _ in 0..10 {
            let mut aa = a;
            for (k, row) in aa.iter_mut().enumerate() {
                row[k] += (lambda as f64) * (a[k][k] + 1e-6);
            }
            let Some(dx) = solve7(aa, g.map(|v| -v)) else {
                lambda *= 10.0;
                continue;
            };
            let mut np = p;
            for k in 0..7 {
                np[k] += dx[k] as f32;
            }
            residuals(&Cam::from_params(&np), m, w, h, &mut rp);
            let nc: f32 = rp.iter().map(|v| v * v).sum();
            if nc < cost {
                p = np;
                std::mem::swap(&mut r, &mut rp);
                let gain = cost - nc;
                cost = nc;
                lambda = (lambda * 0.3).max(1e-7);
                improved = gain > cost * 1e-7;
                break;
            }
            lambda *= 10.0;
        }
        if !improved {
            break;
        }
    }
    let c = Cam::from_params(&p);
    (c, rms(&r))
}

/// Gaussian elimination with partial pivoting on a 7 x 7 system.
fn solve7(mut a: [[f64; 7]; 7], mut b: [f64; 7]) -> Option<[f64; 7]> {
    for c in 0..7 {
        let piv = (c..7).max_by(|i, j| a[*i][c].abs().total_cmp(&a[*j][c].abs()))?;
        if a[piv][c].abs() < 1e-12 {
            return None;
        }
        a.swap(c, piv);
        b.swap(c, piv);
        for r in c + 1..7 {
            let f = a[r][c] / a[c][c];
            for k in c..7 {
                a[r][k] -= f * a[c][k];
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = [0.0f64; 7];
    for c in (0..7).rev() {
        let mut s = b[c];
        for k in c + 1..7 {
            s -= a[c][k] * x[k];
        }
        x[c] = s / a[c][c];
    }
    Some(x)
}

/// Camera from at least four pixel/world matches in a `w` x `h` photo.
/// `hints` are starting cameras (the view as written, the sources.toml
/// camera); a ring of generic starts around the points is added. Returns
/// the best camera and its RMS reprojection error in pixels.
pub fn solve(m: &[(Vec2, Vec3)], w: f32, h: f32, hints: &[Cam]) -> Option<(Cam, f32)> {
    if m.len() < 4 {
        return None;
    }
    let c = m.iter().map(|x| x.1).sum::<Vec3>() / m.len() as f32;
    let ext = m.iter().map(|x| (x.1 - c).length()).fold(1.0, f32::max);
    let mut starts: Vec<Cam> = hints.to_vec();
    for k in 0..8 {
        let a = k as f32 / 8.0 * std::f32::consts::TAU;
        for (dist, fov) in [(2.5f32, 50.0f32), (5.0, 35.0)] {
            let eye = Vec3::new(c.x + a.sin() * ext * dist, 1.6, c.z - a.cos() * ext * dist);
            starts.push(Cam { eye, target: c, fov_y: fov, roll: 0.0 });
        }
    }
    let mut best: Option<(Cam, f32)> = None;
    for s in starts {
        let (cam, e) = refine(s, m, w, h);
        if e.is_finite() && best.is_none_or(|b| e < b.1) {
            best = Some((cam, e));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_a_known_camera() {
        let truth = Cam { eye: Vec3::new(12.0, 1.6, 30.0), target: Vec3::new(-2.0, 9.0, 0.0), fov_y: 48.0, roll: 1.5 };
        let (w, h) = (1600.0, 1067.0);
        // Corners and features of a 20 x 14 x 25 m building.
        let pts = [
            Vec3::new(-10.0, 0.0, 7.0),
            Vec3::new(10.0, 0.0, 7.0),
            Vec3::new(10.0, 25.0, 7.0),
            Vec3::new(-10.0, 25.0, 7.0),
            Vec3::new(10.0, 0.0, -7.0),
            Vec3::new(10.0, 25.0, -7.0),
            Vec3::new(0.0, 32.0, 0.0),
        ];
        let m: Vec<(Vec2, Vec3)> = pts.iter().map(|p| (truth.project(*p, w, h).unwrap(), *p)).collect();
        let (cam, err) = solve(&m, w, h, &[]).unwrap();
        assert!(err < 0.5, "rms {err}");
        assert!((cam.eye - truth.eye).length() < 0.3, "{cam:?}");
        assert!((cam.fov_y - truth.fov_y).abs() < 0.5, "{cam:?}");
        for p in pts {
            let (a, b) = (cam.project(p, w, h).unwrap(), truth.project(p, w, h).unwrap());
            assert!((a - b).length() < 1.0);
        }
    }

    #[test]
    fn focal_length_to_fov() {
        assert!((fov_from_focal(24.0, 1.5) - 2.0 * (12.0f32 / 24.0).atan().to_degrees()).abs() < 0.05);
    }
}
