//! Photo cameras: the projection the renderer uses (look-at, roll,
//! vertical field of view, reverse-Z infinite perspective, optional lens
//! shift) and a solver that recovers a camera from image-to-world point
//! matches (`[[view]] match = [...]`), so a view can be aligned in one
//! step instead of by trial renders. The solver takes constraints (a
//! fixed field of view, e.g. from the photo's EXIF focal length, a fixed
//! eye or eye height prior, a level or upright camera, lens shift for
//! perspective-corrected photos) and reports per-point residuals and
//! warnings for ill-posed matches (nearly coplanar points, cameras
//! solved underground).

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
    /// Lens shift (off-axis principal point) as fractions of the image
    /// width and height: `y > 0` raises the frame (a rising front, as in
    /// perspective-corrected architecture photos), `x > 0` moves it right.
    pub shift: Vec2,
}

impl Cam {
    /// A camera without lens shift.
    pub fn new(eye: Vec3, target: Vec3, fov_y: f32, roll: f32) -> Cam {
        Cam { eye, target, fov_y, roll, shift: Vec2::ZERO }
    }

    /// Same convention as `forja_render::Camera::look_at` plus roll.
    pub fn rotation(&self) -> Quat {
        let dir = (self.target - self.eye).normalize_or(Vec3::NEG_Z);
        let up = if dir.y.abs() > 0.999 { Vec3::Z } else { Vec3::Y };
        Quat::from_mat4(&Mat4::look_at_rh(self.eye, self.eye + dir, up).inverse()) * Quat::from_rotation_z(self.roll.to_radians())
    }

    /// Clip-space offset of the lens shift (identity without shift).
    pub fn shift_matrix(&self) -> Mat4 {
        if self.shift == Vec2::ZERO {
            return Mat4::IDENTITY;
        }
        Mat4::from_translation(Vec3::new(-2.0 * self.shift.x, -2.0 * self.shift.y, 0.0))
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = Mat4::from_rotation_translation(self.rotation(), self.eye).inverse();
        let p = Mat4::perspective_infinite_reverse_rh(self.fov_y.to_radians(), aspect, 0.1) * view;
        if self.shift == Vec2::ZERO { p } else { self.shift_matrix() * p }
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

    /// Pitch of the viewing direction (degrees, positive up).
    pub fn pitch(&self) -> f32 {
        (self.target - self.eye).normalize_or(Vec3::NEG_Z).y.clamp(-1.0, 1.0).asin().to_degrees()
    }

    fn params(&self) -> [f32; NP] {
        let d = (self.target - self.eye).normalize_or(Vec3::NEG_Z);
        let yaw = d.x.atan2(-d.z);
        let pitch = d.y.clamp(-1.0, 1.0).asin();
        [self.eye.x, self.eye.y, self.eye.z, yaw, pitch, self.roll.to_radians(), self.fov_y.to_radians(), self.shift.x, self.shift.y]
    }

    fn from_params(p: &[f32; NP]) -> Cam {
        let (yaw, pitch) = (p[3], p[4].clamp(-1.5, 1.5));
        let d = Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos());
        let eye = Vec3::new(p[0], p[1], p[2]);
        Cam { eye, target: eye + d * 20.0, fov_y: p[6].to_degrees().clamp(5.0, 140.0), roll: p[5].to_degrees(), shift: Vec2::new(p[7], p[8]) }
    }
}

/// Solver parameters: eye x/y/z, yaw, pitch, roll, fov, shift x/y.
const NP: usize = 9;

/// Vertical field of view of a 35 mm-equivalent focal length for a photo
/// of the given aspect (the equivalence is defined on the 43.3 mm
/// diagonal: 2 atan(12 / f) for a 3:2 frame).
pub fn fov_from_focal(focal_35mm: f32, aspect: f32) -> f32 {
    let diag = (aspect * aspect + 1.0).sqrt();
    2.0 * (21.635 / focal_35mm.max(1.0) / diag).atan().to_degrees()
}

/// Constraints on a solved camera. The default solves eye, direction,
/// roll and field of view freely, without lens shift.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Constraints {
    /// Fixed vertical field of view (degrees), e.g. from the EXIF focal length.
    pub fov: Option<f32>,
    /// Fixed eye position.
    pub eye: Option<Vec3>,
    /// Fixed eye height (y) only.
    pub eye_y: Option<f32>,
    /// Soft prior on the eye height: (height, sigma in metres).
    pub eye_height: Option<(f32, f32)>,
    /// Fixed roll (degrees; 0 = level horizon).
    pub roll: Option<f32>,
    /// Fixed pitch (degrees; 0 = upright camera, verticals stay vertical).
    pub pitch: Option<f32>,
    /// Lens shift held fixed at this value (default zero)...
    pub shift: Vec2,
    /// ...or solved: [x, y].
    pub solve_shift: [bool; 2],
}

impl Constraints {
    fn free(&self) -> [bool; NP] {
        let e = self.eye.is_none();
        [e, e && self.eye_y.is_none(), e, true, self.pitch.is_none(), self.roll.is_none(), self.fov.is_none(), self.solve_shift[0], self.solve_shift[1]]
    }

    /// Imposes the fixed values on a camera.
    pub fn apply(&self, mut c: Cam) -> Cam {
        if *self == Constraints::default() {
            return c;
        }
        let dir = (c.target - c.eye).normalize_or(Vec3::NEG_Z);
        if let Some(e) = self.eye {
            c.eye = e;
        }
        if let Some(y) = self.eye_y {
            c.eye.y = y;
        }
        let mut d = dir;
        if let Some(p) = self.pitch {
            let h = Vec2::new(dir.x, dir.z).normalize_or(Vec2::new(0.0, -1.0));
            let p = p.to_radians();
            d = Vec3::new(h.x * p.cos(), p.sin(), h.y * p.cos());
        }
        c.target = c.eye + d * 20.0;
        if let Some(f) = self.fov {
            c.fov_y = f;
        }
        if let Some(r) = self.roll {
            c.roll = r;
        }
        if !self.solve_shift[0] {
            c.shift.x = self.shift.x;
        }
        if !self.solve_shift[1] {
            c.shift.y = self.shift.y;
        }
        c
    }
}

/// Weight (pixels per sigma) of the eye-height prior.
const PRIOR_PX: f32 = 4.0;

/// Reprojection residuals (pixels) of `cam` over the matches, then the
/// prior residuals.
fn residuals(cam: &Cam, m: &[(Vec2, Vec3)], w: f32, h: f32, k: &Constraints, out: &mut Vec<f32>) {
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
    if let Some((y, s)) = k.eye_height
        && k.eye.is_none()
        && k.eye_y.is_none()
    {
        out.push((cam.eye.y - y) / s.max(0.01) * PRIOR_PX);
    }
}

fn rms(r: &[f32]) -> f32 {
    (r.iter().map(|v| v * v).sum::<f32>() / r.len().max(1) as f32 * 2.0).sqrt()
}

/// Levenberg–Marquardt from one start, over the free parameters.
fn refine(start: Cam, m: &[(Vec2, Vec3)], w: f32, h: f32, k: &Constraints) -> (Cam, f32) {
    let free = k.free();
    let idx: Vec<usize> = (0..NP).filter(|i| free[*i]).collect();
    let nf = idx.len();
    let mut p = k.apply(start).params();
    let mut r = Vec::new();
    residuals(&Cam::from_params(&p), m, w, h, k, &mut r);
    let mut cost: f32 = r.iter().map(|v| v * v).sum();
    let mut lambda = 1e-2f32;
    let steps = [0.01f32, 0.01, 0.01, 1e-3, 1e-3, 1e-3, 1e-3, 1e-4, 1e-4];
    let mut rp = Vec::new();
    for _ in 0..200 {
        if nf == 0 {
            break;
        }
        // Numerical Jacobian (central differences).
        let n = r.len();
        let mut jac = vec![vec![0.0f32; nf]; n];
        for (x, &kk) in idx.iter().enumerate() {
            let mut a = p;
            let mut b = p;
            a[kk] += steps[kk];
            b[kk] -= steps[kk];
            let mut ra = Vec::new();
            let mut rb = Vec::new();
            residuals(&Cam::from_params(&a), m, w, h, k, &mut ra);
            residuals(&Cam::from_params(&b), m, w, h, k, &mut rb);
            for i in 0..n {
                jac[i][x] = (ra[i] - rb[i]) / (2.0 * steps[kk]);
            }
        }
        // Normal equations (J^T J + lambda diag) dx = -J^T r.
        let mut a = vec![vec![0.0f64; nf]; nf];
        let mut g = vec![0.0f64; nf];
        for i in 0..n {
            for x in 0..nf {
                g[x] += (jac[i][x] * r[i]) as f64;
                for y in 0..nf {
                    a[x][y] += (jac[i][x] * jac[i][y]) as f64;
                }
            }
        }
        let mut improved = false;
        for _ in 0..10 {
            let mut aa = a.clone();
            for (q, row) in aa.iter_mut().enumerate() {
                row[q] += (lambda as f64) * (a[q][q] + 1e-6);
            }
            let Some(dx) = solve_n(aa, g.iter().map(|v| -v).collect()) else {
                lambda *= 10.0;
                continue;
            };
            let mut np = p;
            for (x, &kk) in idx.iter().enumerate() {
                np[kk] += dx[x] as f32;
            }
            residuals(&Cam::from_params(&np), m, w, h, k, &mut rp);
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
    (c, rms(&r[..(m.len() * 2).min(r.len())]))
}

/// Gaussian elimination with partial pivoting.
fn solve_n(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for c in 0..n {
        let piv = (c..n).max_by(|i, j| a[*i][c].abs().total_cmp(&a[*j][c].abs()))?;
        if a[piv][c].abs() < 1e-12 {
            return None;
        }
        a.swap(c, piv);
        b.swap(c, piv);
        for r in c + 1..n {
            let f = a[r][c] / a[c][c];
            for k in c..n {
                a[r][k] -= f * a[c][k];
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = vec![0.0f64; n];
    for c in (0..n).rev() {
        let mut s = b[c];
        for k in c + 1..n {
            s -= a[c][k] * x[k];
        }
        x[c] = s / a[c][c];
    }
    Some(x)
}

/// A solved camera with diagnostics.
#[derive(Clone, Debug)]
pub struct Solved {
    pub cam: Cam,
    /// RMS reprojection error (pixels).
    pub rms: f32,
    /// Reprojection error of each match (pixels).
    pub errors: Vec<f32>,
    /// Where each match point lands with the solved camera.
    pub reprojected: Vec<Option<Vec2>>,
    /// Thickness of the match points across their best-fit plane divided
    /// by their extent (0 = coplanar).
    pub planarity: f32,
    pub warnings: Vec<String>,
}

/// Thickness of a point set across its best-fit plane relative to its
/// largest extent: sqrt(smallest / largest eigenvalue of the covariance).
pub fn planarity(pts: &[Vec3]) -> f32 {
    if pts.len() < 3 {
        return 0.0;
    }
    let c = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
    let mut m = [[0.0f64; 3]; 3];
    for p in pts {
        let d = *p - c;
        let d = [d.x as f64, d.y as f64, d.z as f64];
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += d[i] * d[j];
            }
        }
    }
    let ev = sym3_eigenvalues(m);
    let (lo, hi) = (ev.iter().copied().fold(f64::MAX, f64::min).max(0.0), ev.iter().copied().fold(0.0, f64::max));
    if hi <= 1e-12 { 0.0 } else { (lo / hi).sqrt() as f32 }
}

/// Eigenvalues of a symmetric 3 x 3 matrix (closed form).
fn sym3_eigenvalues(a: [[f64; 3]; 3]) -> [f64; 3] {
    let p1 = a[0][1] * a[0][1] + a[0][2] * a[0][2] + a[1][2] * a[1][2];
    if p1 < 1e-18 {
        return [a[0][0], a[1][1], a[2][2]];
    }
    let q = (a[0][0] + a[1][1] + a[2][2]) / 3.0;
    let p2 = (a[0][0] - q).powi(2) + (a[1][1] - q).powi(2) + (a[2][2] - q).powi(2) + 2.0 * p1;
    let p = (p2 / 6.0).sqrt();
    let mut bm = a;
    for (i, row) in bm.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (a[i][j] - if i == j { q } else { 0.0 }) / p;
        }
    }
    let det = bm[0][0] * (bm[1][1] * bm[2][2] - bm[1][2] * bm[2][1]) - bm[0][1] * (bm[1][0] * bm[2][2] - bm[1][2] * bm[2][0]) + bm[0][2] * (bm[1][0] * bm[2][1] - bm[1][1] * bm[2][0]);
    let phi = (det / 2.0).clamp(-1.0, 1.0).acos() / 3.0;
    let e1 = q + 2.0 * p * phi.cos();
    let e3 = q + 2.0 * p * (phi + 2.0 * std::f64::consts::PI / 3.0).cos();
    [e1, 3.0 * q - e1 - e3, e3]
}

/// Below this [`planarity`] the matches are "nearly coplanar".
pub const COPLANAR: f32 = 0.06;

/// Camera from at least four pixel/world matches in a `w` x `h` photo.
/// `hints` are starting cameras (the view as written, the sources.toml
/// camera); a ring of generic starts around the points is added. Returns
/// the best camera and its RMS reprojection error in pixels.
pub fn solve(m: &[(Vec2, Vec3)], w: f32, h: f32, hints: &[Cam]) -> Option<(Cam, f32)> {
    solve_with(m, w, h, hints, &Constraints::default()).map(|s| (s.cam, s.rms))
}

/// [`solve`] with constraints, per-point errors and warnings.
pub fn solve_with(m: &[(Vec2, Vec3)], w: f32, h: f32, hints: &[Cam], k: &Constraints) -> Option<Solved> {
    let free = k.free().iter().filter(|f| **f).count();
    // Each match gives two equations; the free parameters need as many.
    let min_pts = (free.div_ceil(2)).clamp(2, 4);
    if m.len() < min_pts {
        return None;
    }
    let c = m.iter().map(|x| x.1).sum::<Vec3>() / m.len() as f32;
    let ext = m.iter().map(|x| (x.1 - c).length()).fold(1.0, f32::max);
    let mut starts: Vec<Cam> = hints.to_vec();
    let start_y = k.eye_y.or(k.eye_height.map(|e| e.0)).unwrap_or(1.6);
    let start_fov = |f: f32| k.fov.unwrap_or(f);
    for kk in 0..8 {
        let a = kk as f32 / 8.0 * std::f32::consts::TAU;
        for (dist, fov) in [(2.5f32, 50.0f32), (5.0, 35.0)] {
            let eye = k.eye.unwrap_or(Vec3::new(c.x + a.sin() * ext * dist, start_y, c.z - a.cos() * ext * dist));
            starts.push(Cam::new(eye, c, start_fov(fov), 0.0));
        }
        if let Some(e) = k.eye {
            // Only the direction varies: aim around the points.
            let d = poly_dir(a);
            starts.push(Cam::new(e, e + Vec3::new(d.x, 0.0, d.y) * 20.0 + Vec3::Y * 2.0, start_fov(50.0), 0.0));
        }
    }
    let mut best: Option<(Cam, f32)> = None;
    for s in starts {
        let (cam, e) = refine(s, m, w, h, k);
        if e.is_finite() && best.is_none_or(|b| e < b.1) {
            best = Some((cam, e));
        }
    }
    let (cam, rms) = best?;
    let reprojected: Vec<Option<Vec2>> = m.iter().map(|(_, p)| cam.project(*p, w, h)).collect();
    let errors: Vec<f32> = m.iter().zip(&reprojected).map(|((px, _), q)| q.map(|q| (q - *px).length()).unwrap_or(f32::INFINITY)).collect();
    let pts: Vec<Vec3> = m.iter().map(|x| x.1).collect();
    let pl = planarity(&pts);
    let mut warnings = Vec::new();
    let constrained = k.fov.is_some() || k.eye.is_some() || k.eye_y.is_some() || k.eye_height.is_some();
    if pl < COPLANAR && !constrained {
        warnings.push(format!(
            "the {} match points are nearly coplanar (thickness {:.0} % of their extent): the camera is ambiguous; add points off that plane (another facade, a tower top, the ground in depth) or constrain it (focal_35mm with fix = [\"fov\"], eye_height, level, upright)",
            m.len(),
            pl * 100.0
        ));
    }
    if cam.eye.y < -0.2 {
        warnings.push(format!("the camera was solved {:.1} m underground: the matches are ambiguous or wrong; set eye_height = 1.6 (or fix the eye) and check the points with the largest errors", -cam.eye.y));
    }
    if m.len() < 5 && free >= 7 {
        warnings.push(format!("only {} matches for {free} free camera parameters: use 6+ points spread in depth or constrain the camera", m.len()));
    }
    if let Some((i, e)) = errors.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))
        && *e > (rms * 2.5).max(8.0)
    {
        warnings.push(format!("match {i} is off by {e:.0} px (rms {rms:.1}): check its pixel and world coordinates"));
    }
    Some(Solved { cam, rms, errors, reprojected, planarity: pl, warnings })
}

fn poly_dir(a: f32) -> Vec2 {
    Vec2::new(a.sin(), -a.cos())
}

/// 35 mm-equivalent focal length from a JPEG's EXIF data
/// (`FocalLengthIn35mmFilm`, tag 0xA405), if present.
pub fn exif_focal_35mm(bytes: &[u8]) -> Option<f32> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        if marker == 0xD9 || marker == 0xDA {
            return None;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        let seg = bytes.get(i + 4..i + 2 + len)?;
        if marker == 0xE1 && seg.starts_with(b"Exif\0\0") {
            return tiff_focal_35mm(&seg[6..]);
        }
        i += 2 + len;
    }
    None
}

fn tiff_focal_35mm(t: &[u8]) -> Option<f32> {
    let le = match t.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |o: usize| -> Option<u16> {
        let b = t.get(o..o + 2)?;
        Some(if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    };
    let u32_at = |o: usize| -> Option<u32> {
        let b = t.get(o..o + 4)?;
        Some(if le { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) })
    };
    // Finds `tag` in the IFD at `ifd`: (type, value offset of the entry).
    let find = |ifd: usize, tag: u16| -> Option<(u16, usize)> {
        let n = u16_at(ifd)? as usize;
        (0..n.min(512)).map(|k| ifd + 2 + k * 12).find(|&e| u16_at(e) == Some(tag)).and_then(|e| Some((u16_at(e + 2)?, e + 8)))
    };
    let ifd0 = u32_at(4)? as usize;
    let (_, p) = find(ifd0, 0x8769)?;
    let exif = u32_at(p)? as usize;
    let (ty, v) = find(exif, 0xA405)?;
    let f = match ty {
        3 => u16_at(v)? as f32,
        4 => u32_at(v)? as f32,
        _ => return None,
    };
    (f > 0.0).then_some(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn building_points() -> Vec<Vec3> {
        // Corners and features of a 20 x 14 x 25 m building.
        vec![
            Vec3::new(-10.0, 0.0, 7.0),
            Vec3::new(10.0, 0.0, 7.0),
            Vec3::new(10.0, 25.0, 7.0),
            Vec3::new(-10.0, 25.0, 7.0),
            Vec3::new(10.0, 0.0, -7.0),
            Vec3::new(10.0, 25.0, -7.0),
            Vec3::new(0.0, 32.0, 0.0),
        ]
    }

    #[test]
    fn recovers_a_known_camera() {
        let truth = Cam::new(Vec3::new(12.0, 1.6, 30.0), Vec3::new(-2.0, 9.0, 0.0), 48.0, 1.5);
        let (w, h) = (1600.0, 1067.0);
        let pts = building_points();
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

    #[test]
    fn fixed_fov_and_eye_height_resolve_a_coplanar_facade() {
        // Only points on one facade plane (z = 7): ambiguous without
        // constraints; with the focal length fixed and an eye height prior
        // the camera is recovered, and the free solve warns.
        let truth = Cam::new(Vec3::new(6.0, 1.6, 40.0), Vec3::new(0.0, 10.0, 7.0), 46.0, 0.0);
        let (w, h) = (1500.0, 1000.0);
        let pts: Vec<Vec3> = [[-10.0, 0.0], [10.0, 0.0], [10.0, 25.0], [-10.0, 25.0], [0.0, 12.0], [-5.0, 6.0]].iter().map(|p| Vec3::new(p[0], p[1], 7.0)).collect();
        let m: Vec<(Vec2, Vec3)> = pts.iter().map(|p| (truth.project(*p, w, h).unwrap(), *p)).collect();
        let free = solve_with(&m, w, h, &[], &Constraints::default()).unwrap();
        assert!(free.planarity < COPLANAR);
        assert!(free.warnings.iter().any(|w| w.contains("coplanar")), "{:?}", free.warnings);
        let k = Constraints { fov: Some(46.0), eye_height: Some((1.6, 0.2)), roll: Some(0.0), ..Default::default() };
        let s = solve_with(&m, w, h, &[], &k).unwrap();
        assert!(s.rms < 0.5, "rms {}", s.rms);
        assert!((s.cam.eye - truth.eye).length() < 0.5, "{:?}", s.cam);
        assert_eq!(s.cam.fov_y, 46.0);
        assert_eq!(s.cam.roll, 0.0);
        assert!(!s.warnings.iter().any(|w| w.contains("coplanar")));
        assert_eq!(s.errors.len(), m.len());
        assert!(s.errors.iter().all(|e| *e < 1.0));
    }

    #[test]
    fn upright_camera_with_lens_shift() {
        // A perspective-corrected photo: level, upright camera with the
        // frame raised (verticals stay vertical).
        let mut truth = Cam::new(Vec3::new(4.0, 1.7, 45.0), Vec3::new(4.0, 1.7, 0.0), 50.0, 0.0);
        truth.shift = Vec2::new(0.0, 0.22);
        let (w, h) = (1200.0, 1600.0);
        let pts = building_points();
        let m: Vec<(Vec2, Vec3)> = pts.iter().map(|p| (truth.project(*p, w, h).unwrap(), *p)).collect();
        // Verticals project vertical: both ends of an edge share x.
        let (a, b) = (truth.project(pts[0], w, h).unwrap(), truth.project(pts[3], w, h).unwrap());
        assert!((a.x - b.x).abs() < 1e-2);
        let k = Constraints { pitch: Some(0.0), roll: Some(0.0), solve_shift: [false, true], ..Default::default() };
        let s = solve_with(&m, w, h, &[], &k).unwrap();
        assert!(s.rms < 0.5, "rms {} {:?}", s.rms, s.cam);
        assert!((s.cam.shift.y - 0.22).abs() < 0.01, "{:?}", s.cam);
        assert!((s.cam.eye - truth.eye).length() < 0.5, "{:?}", s.cam);
        assert!(s.cam.pitch().abs() < 1e-3);
    }

    #[test]
    fn fixed_eye_solves_only_the_direction() {
        let truth = Cam::new(Vec3::new(-8.0, 12.0, 35.0), Vec3::new(0.0, 8.0, 0.0), 40.0, -2.0);
        let (w, h) = (1600.0, 1200.0);
        let m: Vec<(Vec2, Vec3)> = building_points().iter().map(|p| (truth.project(*p, w, h).unwrap(), *p)).collect();
        let k = Constraints { eye: Some(truth.eye), ..Default::default() };
        let s = solve_with(&m[..3], w, h, &[], &k).unwrap();
        assert_eq!(s.cam.eye, truth.eye);
        assert!(s.rms < 0.5 && (s.cam.fov_y - 40.0).abs() < 0.3 && (s.cam.roll + 2.0).abs() < 0.3, "{:?} {}", s.cam, s.rms);
    }

    #[test]
    fn planarity_of_point_sets() {
        let flat: Vec<Vec3> = (0..9).map(|k| Vec3::new((k % 3) as f32 * 5.0, (k / 3) as f32 * 4.0, 2.0)).collect();
        assert!(planarity(&flat) < 1e-3);
        assert!(planarity(&building_points()) > 0.2);
    }

    #[test]
    fn exif_focal_length() {
        // Minimal JPEG: SOI, APP1 Exif (little endian TIFF) with IFD0 ->
        // ExifIFD -> FocalLengthIn35mmFilm = 26, then EOI.
        let mut tiff: Vec<u8> = b"II*\0".to_vec();
        tiff.extend(8u32.to_le_bytes());
        // IFD0 at 8: one entry (0x8769, LONG, 1, offset 26), next 0.
        tiff.extend(1u16.to_le_bytes());
        tiff.extend(0x8769u16.to_le_bytes());
        tiff.extend(4u16.to_le_bytes());
        tiff.extend(1u32.to_le_bytes());
        tiff.extend(26u32.to_le_bytes());
        tiff.extend(0u32.to_le_bytes());
        // Exif IFD at 26: (0xA405, SHORT, 1, 26).
        tiff.extend(1u16.to_le_bytes());
        tiff.extend(0xA405u16.to_le_bytes());
        tiff.extend(3u16.to_le_bytes());
        tiff.extend(1u32.to_le_bytes());
        tiff.extend([26u8, 0, 0, 0]);
        tiff.extend(0u32.to_le_bytes());
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(&tiff);
        let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xE1];
        jpg.extend(((app1.len() + 2) as u16).to_be_bytes());
        jpg.extend(&app1);
        jpg.extend([0xFF, 0xD9]);
        assert_eq!(exif_focal_35mm(&jpg), Some(26.0));
        assert_eq!(exif_focal_35mm(&[0xFF, 0xD8, 0xFF, 0xD9]), None);
        assert_eq!(exif_focal_35mm(b"not a jpeg"), None);
    }
}
